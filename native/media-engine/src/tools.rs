//! Locating and running the ffmpeg / ffprobe executables.

use capture_core::{CaptureError, CaptureErrorCode, CaptureResult};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// Creates a `Command` that never opens a console window (Windows GUI apps).
pub fn new_command(program: &Path) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Output of a finished child process.
#[derive(Debug)]
pub struct CmdOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl CmdOutput {
    pub fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).trim().to_string()
    }
}

/// Runs `cmd` to completion (stdin closed), killing it after `timeout`.
pub fn run_with_timeout(mut cmd: Command, timeout: Duration) -> CaptureResult<CmdOutput> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let program = cmd.get_program().to_string_lossy().to_string();
    let mut child = cmd
        .spawn()
        .map_err(|e| CaptureError::new(CaptureErrorCode::EncoderUnavailable, format!("failed to run {program}: {e}")))?;
    let out = child.stdout.take();
    let err = child.stderr.take();
    let t_out = std::thread::spawn(move || read_all(out));
    let t_err = std::thread::spawn(move || read_all(err));
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = t_out.join();
                    let _ = t_err.join();
                    return Err(CaptureError::new(CaptureErrorCode::Internal, format!("{program} timed out after {}s", timeout.as_secs())));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => return Err(CaptureError::io(e)),
        }
    };
    let stdout = t_out.join().unwrap_or_default();
    let stderr = t_err.join().unwrap_or_default();
    Ok(CmdOutput { status, stdout, stderr })
}

fn read_all<R: Read>(r: Option<R>) -> Vec<u8> {
    let mut v = Vec::new();
    if let Some(mut r) = r {
        let _ = r.read_to_end(&mut v);
    }
    v
}

/// Locates a tool: explicit path, then `$env_var`, then a sidecar next to the current
/// executable (plain name or Tauri's `<name>-<target-triple>`), then `PATH` and common
/// install locations (GUI apps on macOS do not inherit the shell PATH).
pub fn locate_tool(explicit: Option<&Path>, env_var: &str, name: &str) -> Option<PathBuf> {
    if let Some(p) = explicit {
        return p.is_file().then(|| p.to_path_buf());
    }
    if let Some(v) = std::env::var_os(env_var) {
        let p = PathBuf::from(v);
        if p.is_file() {
            return Some(p);
        }
    }
    let suffix = std::env::consts::EXE_SUFFIX;
    let exe_name = format!("{name}{suffix}");
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        let cand = dir.join(&exe_name);
        if cand.is_file() {
            return Some(cand);
        }
        if let Ok(rd) = std::fs::read_dir(&dir) {
            let prefix = format!("{name}-");
            for entry in rd.flatten() {
                let fname = entry.file_name().to_string_lossy().to_string();
                if let Some(rest) = fname.strip_prefix(&prefix) {
                    let triple = rest.strip_suffix(suffix).unwrap_or(rest);
                    if triple.matches('-').count() >= 2 && entry.path().is_file() {
                        return Some(entry.path());
                    }
                }
            }
        }
    }
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    for extra in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/opt/local/bin"] {
        dirs.push(PathBuf::from(extra));
    }
    dirs.into_iter().map(|d| d.join(&exe_name)).find(|c| c.is_file())
}

/// `ffmpeg -version` first line, e.g. "6.1.1-3ubuntu5".
pub fn tool_version(path: &Path) -> Option<String> {
    let mut cmd = new_command(path);
    cmd.args(["-hide_banner", "-version"]);
    let out = run_with_timeout(cmd, Duration::from_secs(15)).ok()?;
    if !out.status.success() {
        return None;
    }
    parse_version_line(&String::from_utf8_lossy(&out.stdout))
}

pub fn parse_version_line(stdout: &str) -> Option<String> {
    let line = stdout.lines().next()?;
    let mut it = line.split_whitespace();
    let _name = it.next()?;
    if it.next()? != "version" {
        return None;
    }
    it.next().map(str::to_string)
}

/// Maps ffmpeg stderr to the most specific error code.
pub fn classify_ffmpeg_error(stderr: &str, default: CaptureErrorCode) -> CaptureErrorCode {
    let s = stderr.to_ascii_lowercase();
    if s.contains("no space left on device") || s.contains("disk full") || s.contains("not enough space") {
        CaptureErrorCode::StorageFull
    } else if s.contains("permission denied") {
        CaptureErrorCode::IoError
    } else {
        default
    }
}

/// Keeps the last `max` lines of a child's stderr (read on a background thread).
pub struct StderrTail {
    lines: std::sync::Arc<parking_lot::Mutex<std::collections::VecDeque<String>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl StderrTail {
    pub fn spawn<R: Read + Send + 'static>(reader: R, max: usize) -> Self {
        let lines = std::sync::Arc::new(parking_lot::Mutex::new(std::collections::VecDeque::new()));
        let l2 = lines.clone();
        let thread = std::thread::Builder::new()
            .name("ffmpeg-stderr".into())
            .spawn(move || {
                use std::io::BufRead;
                let br = std::io::BufReader::new(reader);
                for line in br.split(b'\n').map_while(Result::ok) {
                    let text = String::from_utf8_lossy(&line).trim_end().to_string();
                    if text.is_empty() {
                        continue;
                    }
                    log::debug!("ffmpeg: {text}");
                    let mut g = l2.lock();
                    if g.len() >= max {
                        g.pop_front();
                    }
                    g.push_back(text);
                }
            })
            .ok();
        Self { lines, thread }
    }

    /// Waits for the reader to hit EOF (child exited) and returns the collected text.
    pub fn finish(mut self) -> String {
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        self.text()
    }

    pub fn text(&self) -> String {
        self.lines.lock().iter().cloned().collect::<Vec<_>>().join("\n")
    }
}

/// Waits for a child with a timeout; kills it if it does not exit in time.
pub fn wait_child(child: &mut std::process::Child, timeout: Duration) -> CaptureResult<ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(s)) => return Ok(s),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(CaptureError::new(CaptureErrorCode::EncoderFailed, "ffmpeg did not exit in time and was killed"));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => return Err(CaptureError::io(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_parsing() {
        assert_eq!(parse_version_line("ffmpeg version 6.1.1-3ubuntu5 Copyright (c) 2000-2023").as_deref(), Some("6.1.1-3ubuntu5"));
        assert_eq!(parse_version_line("ffprobe version n7.0 Copyright").as_deref(), Some("n7.0"));
        assert_eq!(parse_version_line("garbage"), None);
        assert_eq!(parse_version_line(""), None);
    }

    #[test]
    fn error_classification() {
        assert_eq!(classify_ffmpeg_error("av_interleaved_write_frame(): No space left on device", CaptureErrorCode::EncoderFailed), CaptureErrorCode::StorageFull);
        assert_eq!(classify_ffmpeg_error("Unknown encoder", CaptureErrorCode::EncoderFailed), CaptureErrorCode::EncoderFailed);
    }

    #[test]
    fn locate_explicit_missing_is_none() {
        assert!(locate_tool(Some(Path::new("/definitely/not/here/ffmpeg")), "CAPTUREONE_TEST_NOPE", "ffmpeg").is_none());
    }
}
