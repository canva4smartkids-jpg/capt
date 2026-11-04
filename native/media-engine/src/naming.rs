//! Output file naming: template expansion, sanitization and no-overwrite commits.

use capture_core::{CaptureError, CaptureResult};
use chrono::{DateTime, Local};
use std::path::{Path, PathBuf};

/// Expands `{date}` (YYYY-MM-DD), `{time}` (HH-MM-SS), `{source}` and `{n}` and sanitizes
/// the result into a safe file stem (no extension).
pub fn expand_template(template: &str, now: &DateTime<Local>, source: &str, n: u32) -> String {
    let raw = template
        .replace("{date}", &now.format("%Y-%m-%d").to_string())
        .replace("{time}", &now.format("%H-%M-%S").to_string())
        .replace("{source}", source)
        .replace("{n}", &n.to_string());
    sanitize_stem(&raw)
}

/// Removes characters that are invalid in file names on Windows/macOS/Linux, trims
/// trailing dots/spaces, avoids reserved device names and limits the length.
pub fn sanitize_stem(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') { '_' } else { c })
        .collect();
    out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    while out.ends_with('.') || out.ends_with(' ') {
        out.pop();
    }
    while out.starts_with('.') {
        out.remove(0);
    }
    if out.chars().count() > 180 {
        out = out.chars().take(180).collect();
    }
    const RESERVED: &[&str] = &["CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "LPT1", "LPT2", "LPT3"];
    if out.is_empty() {
        out = "CaptureOne".into();
    } else if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(&out)) {
        out = format!("{out}_");
    }
    out
}

/// Candidate file names in order: with `{n}` in the template the counter increments
/// (1, 2, ...); otherwise the plain name, then "name (1)", "name (2)", ...
pub fn candidate_names(template: &str, now: &DateTime<Local>, source: &str, ext: &str) -> impl Iterator<Item = String> {
    let has_n = template.contains("{n}");
    let template = template.to_string();
    let source = source.to_string();
    let now = *now;
    let ext = ext.to_string();
    let base = expand_template(&template, &now, &source, 1);
    (0u32..10_000).map(move |i| {
        if has_n {
            format!("{}.{ext}", expand_template(&template, &now, &source, i + 1))
        } else if i == 0 {
            format!("{base}.{ext}")
        } else {
            format!("{base} ({i}).{ext}")
        }
    })
}

/// Moves `tmp` to the first free name from `names` inside `dir`, never overwriting an
/// existing file (hard link + unlink is atomic and fails if the target exists; falls back
/// to an existence check + rename on file systems without hard links).
pub fn persist_no_clobber(tmp: &Path, dir: &Path, names: impl Iterator<Item = String>) -> CaptureResult<PathBuf> {
    for name in names {
        let target = dir.join(&name);
        match std::fs::hard_link(tmp, &target) {
            Ok(()) => {
                let _ = std::fs::remove_file(tmp);
                return Ok(target);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => {
                if target.exists() {
                    continue;
                }
                std::fs::rename(tmp, &target)?;
                return Ok(target);
            }
        }
    }
    Err(CaptureError::io("could not find a free output file name"))
}

/// Temporary file inside `dir` (same file system as the final target).
pub fn temp_path(dir: &Path, id: &str, ext: &str) -> PathBuf {
    dir.join(format!(".captureone-{id}.partial.{ext}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 3, 7, 9, 5, 2).single().unwrap()
    }

    #[test]
    fn expands_tokens() {
        assert_eq!(expand_template("CaptureOne {date} {time}", &t(), "x", 1), "CaptureOne 2026-03-07 09-05-02");
        assert_eq!(expand_template("{source} #{n}", &t(), "Display: DP-1", 3), "Display_ DP-1 #3");
    }

    #[test]
    fn sanitizes() {
        assert_eq!(sanitize_stem("a/b\\c:d*e?f\"g<h>i|j"), "a_b_c_d_e_f_g_h_i_j");
        assert_eq!(sanitize_stem("  ..hidden.  "), "hidden");
        assert_eq!(sanitize_stem(""), "CaptureOne");
        assert_eq!(sanitize_stem("CON"), "CON_");
        assert_eq!(sanitize_stem("tab\there\nnewline"), "tab_here_newline");
        assert!(sanitize_stem(&"x".repeat(500)).len() <= 180);
    }

    #[test]
    fn candidates_without_and_with_counter() {
        let c: Vec<_> = candidate_names("Rec {date}", &t(), "s", "mp4").take(3).collect();
        assert_eq!(c, vec!["Rec 2026-03-07.mp4", "Rec 2026-03-07 (1).mp4", "Rec 2026-03-07 (2).mp4"]);
        let n: Vec<_> = candidate_names("Shot {n}", &t(), "s", "png").take(2).collect();
        assert_eq!(n, vec!["Shot 1.png", "Shot 2.png"]);
    }

    #[test]
    fn never_overwrites() {
        let dir = std::env::temp_dir().join(format!("captureone-naming-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Rec.mp4"), b"existing").unwrap();
        std::fs::write(dir.join("Rec (1).mp4"), b"existing1").unwrap();
        let tmp = temp_path(&dir, "abc", "mp4");
        std::fs::write(&tmp, b"new").unwrap();
        let out = persist_no_clobber(&tmp, &dir, candidate_names("Rec", &t(), "", "mp4")).unwrap();
        assert_eq!(out.file_name().unwrap(), "Rec (2).mp4");
        assert_eq!(std::fs::read(dir.join("Rec.mp4")).unwrap(), b"existing");
        assert_eq!(std::fs::read(dir.join("Rec (1).mp4")).unwrap(), b"existing1");
        assert_eq!(std::fs::read(&out).unwrap(), b"new");
        assert!(!tmp.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
