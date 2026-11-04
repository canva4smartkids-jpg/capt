//! Runtime smoke test for the Windows backend, used by `.github/workflows/windows.yml` on the
//! `windows-latest` runner. It drives the real `CaptureBackend` (no mocks) and prints JSON
//! evidence; the workflow then checks the produced media with ffprobe.
//!
//! ```text
//! wgc_smoke info                              # capabilities, permissions, sources, endpoints
//! wgc_smoke screenshot <out.png> [display-id] # one WGC frame of a display
//! wgc_smoke region <out.png>                  # 200x120 region of the first display
//! wgc_smoke window <needle> <out.png>         # first window whose title/app contains <needle>
//! wgc_smoke record <seconds> <out.mp4>        # display stream paced to 30 fps, piped to ffmpeg
//! wgc_smoke closed-test                       # stream notepad, kill it, expect SourceClosed
//! wgc_smoke restart-test                      # start/stop a stream 3 times (cleanup)
//! wgc_smoke audio <seconds> <out.wav>         # WASAPI loopback
//! ```

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("wgc_smoke only runs on Windows");
}

#[cfg(target_os = "windows")]
fn main() {
    if let Err(e) = win::run() {
        eprintln!("wgc_smoke failed: {e}");
        println!("{}", serde_json::json!({ "ok": false, "error": e }));
        std::process::exit(1);
    }
}

#[cfg(target_os = "windows")]
mod win {
    use capture_core::*;
    use serde_json::json;
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use windows_capture_backend::{create_backend, list_render_endpoints};

    type R<T> = Result<T, String>;

    fn s<E: std::fmt::Display>(e: E) -> String {
        e.to_string()
    }

    pub fn run() -> R<()> {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let backend = create_backend().map_err(s)?;
        match args.first().map(String::as_str) {
            Some("info") => info(&*backend),
            Some("screenshot") => screenshot(&*backend, arg(&args, 1)?, args.get(2).map(String::as_str)),
            Some("region") => region(&*backend, arg(&args, 1)?),
            Some("window") => window(&*backend, arg(&args, 1)?, arg(&args, 2)?),
            Some("record") => record(&*backend, arg(&args, 1)?.parse().map_err(s)?, arg(&args, 2)?),
            Some("closed-test") => closed_test(&*backend),
            Some("restart-test") => restart_test(&*backend),
            Some("audio") => audio(&*backend, arg(&args, 1)?.parse().map_err(s)?, arg(&args, 2)?),
            _ => Err("usage: wgc_smoke info|screenshot|region|window|record|closed-test|restart-test|audio ...".into()),
        }
    }

    fn arg(args: &[String], i: usize) -> R<&str> {
        args.get(i).map(String::as_str).ok_or_else(|| format!("missing argument #{i}"))
    }

    fn first_display(b: &dyn CaptureBackend) -> R<CaptureSource> {
        b.list_sources(&[SourceKind::Display])
            .map_err(s)?
            .into_iter()
            .next()
            .ok_or_else(|| "no displays found".to_string())
    }

    /// Basic content statistics so the logs show whether the frame is plausibly real.
    fn stats(f: &VideoFrame) -> serde_json::Value {
        let packed = f.packed_bgra();
        let mut sum = 0u64;
        let mut distinct = std::collections::HashSet::new();
        for (i, px) in packed.chunks_exact(4).enumerate() {
            sum += (px[0] as u64 + px[1] as u64 * 2 + px[2] as u64) / 4;
            if i % 97 == 0 && distinct.len() < 4096 {
                distinct.insert([px[0], px[1], px[2]]);
            }
        }
        let n = (f.width as u64 * f.height as u64).max(1);
        json!({ "width": f.width, "height": f.height, "stride": f.stride, "meanLuma": sum / n, "sampledDistinctColors": distinct.len() })
    }

    fn write_png(f: &VideoFrame, path: &str) -> R<()> {
        if let Some(dir) = std::path::Path::new(path).parent() {
            std::fs::create_dir_all(dir).map_err(s)?;
        }
        image::save_buffer(path, &f.to_rgba(), f.width, f.height, image::ExtendedColorType::Rgba8).map_err(s)
    }

    fn info(b: &dyn CaptureBackend) -> R<()> {
        let caps = b.capabilities();
        let displays = b.list_sources(&[SourceKind::Display]).map_err(s)?;
        let windows = b.list_sources(&[SourceKind::Window]).map_err(s)?;
        let endpoints = match list_render_endpoints() {
            Ok(v) => json!(v
                .iter()
                .map(|e| json!({ "id": e.id, "name": e.name, "isDefault": e.is_default }))
                .collect::<Vec<_>>()),
            Err(e) => json!({ "error": e }),
        };
        let perms: Vec<_> = [PermissionKind::ScreenCapture, PermissionKind::Microphone, PermissionKind::SystemAudio]
            .into_iter()
            .map(|k| b.permission_status(k))
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "ok": !displays.is_empty(),
                "capabilities": caps,
                "permissions": perms,
                "displays": displays,
                "windows": windows,
                "renderEndpoints": endpoints,
            }))
            .map_err(s)?
        );
        if displays.is_empty() {
            return Err("list_sources returned no displays".into());
        }
        if let Some(d) = displays.iter().find(|d| d.width == 0 || d.height == 0) {
            return Err(format!("display {} has a zero size", d.id));
        }
        Ok(())
    }

    fn screenshot(b: &dyn CaptureBackend, out: &str, id: Option<&str>) -> R<()> {
        let display = match id {
            Some(id) => find_source(b, SourceKind::Display, id).map_err(s)?,
            None => first_display(b)?,
        };
        let t = Instant::now();
        let frame = b
            .screenshot(
                &CaptureTarget::Display {
                    display_id: display.id.clone(),
                },
                true,
            )
            .map_err(s)?;
        let elapsed = t.elapsed().as_millis();
        write_png(&frame, out)?;
        println!(
            "{}",
            json!({ "ok": true, "source": display, "frame": stats(&frame), "elapsedMs": elapsed, "file": out })
        );
        if (frame.width, frame.height) != (display.width, display.height) {
            return Err(format!(
                "frame {}x{} does not match the listed display size {}x{}",
                frame.width, frame.height, display.width, display.height
            ));
        }
        Ok(())
    }

    fn region(b: &dyn CaptureBackend, out: &str) -> R<()> {
        let display = first_display(b)?;
        let rect = Rect {
            x: 10,
            y: 10,
            width: 200,
            height: 120,
        };
        let frame = b
            .screenshot(
                &CaptureTarget::Region {
                    display_id: display.id.clone(),
                    rect,
                },
                false,
            )
            .map_err(s)?;
        write_png(&frame, out)?;
        println!(
            "{}",
            json!({ "ok": true, "display": display.id, "rect": rect, "frame": stats(&frame), "file": out })
        );
        if (frame.width, frame.height) != (200, 120) {
            return Err(format!("region frame is {}x{}, expected 200x120", frame.width, frame.height));
        }
        Ok(())
    }

    fn find_window(b: &dyn CaptureBackend, needle: &str) -> R<CaptureSource> {
        let needle = needle.to_lowercase();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let found = b
                .list_sources(&[SourceKind::Window])
                .map_err(s)?
                .into_iter()
                .find(|w| w.name.to_lowercase().contains(&needle) || w.app_name.as_deref().is_some_and(|a| a.to_lowercase().contains(&needle)));
            if let Some(w) = found {
                return Ok(w);
            }
            if Instant::now() > deadline {
                return Err(format!("no window matching '{needle}'"));
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    fn window(b: &dyn CaptureBackend, needle: &str, out: &str) -> R<()> {
        let w = find_window(b, needle)?;
        let frame = b.screenshot(&CaptureTarget::Window { window_id: w.id.clone() }, false).map_err(s)?;
        write_png(&frame, out)?;
        println!("{}", json!({ "ok": true, "window": w, "frame": stats(&frame), "file": out }));
        Ok(())
    }

    #[derive(Default)]
    struct Latest {
        frame: Mutex<Option<VideoFrame>>,
        delivered: AtomicU64,
        events: Mutex<Vec<String>>,
    }

    impl VideoSink for Latest {
        fn on_frame(&self, frame: VideoFrame) {
            self.delivered.fetch_add(1, Ordering::Relaxed);
            *self.frame.lock().unwrap_or_else(|e| e.into_inner()) = Some(frame);
        }
        fn on_event(&self, event: StreamEvent) {
            self.events.lock().unwrap_or_else(|e| e.into_inner()).push(format!("{event:?}"));
        }
    }

    /// Streams the first display for `secs`, duplicating the latest frame at 30 fps (like the
    /// engine's pacer) into ffmpeg/libx264, while moving the cursor so WGC sees changes.
    fn record(b: &dyn CaptureBackend, secs: u64, out: &str) -> R<()> {
        let display = first_display(b)?;
        let sink = Arc::new(Latest::default());
        let opts = VideoStreamOptions { fps: 30, show_cursor: true };
        let started = Instant::now();
        let mut handle = b
            .start_video(
                &CaptureTarget::Display {
                    display_id: display.id.clone(),
                },
                &opts,
                sink.clone(),
            )
            .map_err(s)?;
        // Wait for the first frame.
        let first = loop {
            if let Some(f) = sink.frame.lock().unwrap_or_else(|e| e.into_inner()).clone() {
                break f;
            }
            if started.elapsed() > Duration::from_secs(5) {
                let _ = handle.stop();
                return Err("no frame within 5 s of start_video".into());
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let first_frame_ms = started.elapsed().as_millis();
        let (w, h) = (first.width, first.height);
        let ffmpeg = std::env::var("FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
        let mut child = Command::new(&ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "bgra",
                "-s",
                &format!("{w}x{h}"),
                "-framerate",
                "30",
                "-i",
                "-",
            ])
            .args(["-c:v", "libx264", "-preset", "veryfast", "-pix_fmt", "yuv420p", "-movflags", "+faststart", out])
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot start {ffmpeg}: {e}"))?;
        let mut stdin = child.stdin.take().ok_or("ffmpeg stdin unavailable")?;
        let total = secs * 30;
        let tick = Duration::from_nanos(1_000_000_000 / 30);
        let t0 = Instant::now();
        let mut written = 0u64;
        let mut size_mismatch = 0u64;
        while written < total {
            let target = t0 + tick * written as u32;
            if let Some(wait) = target.checked_duration_since(Instant::now()) {
                std::thread::sleep(wait);
            }
            // Move the cursor in a circle so the compositor produces new frames.
            let a = written as f64 / 10.0;
            let (cx, cy) = (
                display.x.unwrap_or(0) + (w as f64 / 2.0 + a.cos() * 100.0) as i32,
                display.y.unwrap_or(0) + (h as f64 / 2.0 + a.sin() * 100.0) as i32,
            );
            // SAFETY: plain Win32 call.
            let _ = unsafe { windows::Win32::UI::WindowsAndMessaging::SetCursorPos(cx, cy) };
            let frame = sink.frame.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_else(|| first.clone());
            let frame = if (frame.width, frame.height) == (w, h) {
                frame
            } else {
                size_mismatch += 1;
                first.clone()
            };
            stdin.write_all(&frame.packed_bgra()).map_err(|e| format!("writing to ffmpeg failed: {e}"))?;
            written += 1;
        }
        drop(stdin);
        let stop_started = Instant::now();
        handle.stop().map_err(s)?;
        let stop_ms = stop_started.elapsed().as_millis();
        let status = child.wait().map_err(s)?;
        let delivered = sink.delivered.load(Ordering::Relaxed);
        let last = sink.frame.lock().unwrap_or_else(|e| e.into_inner()).clone();
        println!(
            "{}",
            json!({
                "ok": status.success() && delivered > 0,
                "display": display,
                "framesDeliveredByBackend": delivered,
                "framesWrittenAt30fps": written,
                "sizeMismatchedFrames": size_mismatch,
                "firstFrameMs": first_frame_ms,
                "stopMs": stop_ms,
                "events": *sink.events.lock().unwrap_or_else(|e| e.into_inner()),
                "firstFrame": stats(&first),
                "lastFrame": last.as_ref().map(stats),
                "ffmpegExit": status.code(),
                "file": out,
            })
        );
        if !status.success() {
            return Err(format!("ffmpeg exited with {status}"));
        }
        if delivered == 0 {
            return Err("the backend delivered no frames".into());
        }
        Ok(())
    }

    fn closed_test(b: &dyn CaptureBackend) -> R<()> {
        let mut child = Command::new("notepad.exe").spawn().map_err(|e| format!("cannot start notepad: {e}"))?;
        let result = (|| -> R<serde_json::Value> {
            let w = find_window(b, "notepad")?;
            let sink = Arc::new(Latest::default());
            let mut handle = b
                .start_video(
                    &CaptureTarget::Window { window_id: w.id.clone() },
                    &VideoStreamOptions { fps: 30, show_cursor: false },
                    sink.clone(),
                )
                .map_err(s)?;
            std::thread::sleep(Duration::from_millis(1500));
            let before = sink.delivered.load(Ordering::Relaxed);
            // Close the window (kill the process that owns it).
            let pid_kill = Command::new("taskkill").args(["/F", "/IM", "notepad.exe"]).output().map_err(s)?;
            let killed_at = Instant::now();
            let mut closed_after = None;
            while killed_at.elapsed() < Duration::from_secs(8) {
                if sink.events.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|e| e == "SourceClosed") {
                    closed_after = Some(killed_at.elapsed().as_millis());
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            let stop = handle.stop();
            Ok(json!({
                "ok": closed_after.is_some() && stop.is_ok(),
                "window": w,
                "framesBeforeClose": before,
                "taskkillExit": pid_kill.status.code(),
                "sourceClosedAfterMs": closed_after,
                "events": *sink.events.lock().unwrap_or_else(|e| e.into_inner()),
                "stopAfterClose": stop.map(|_| "ok".to_string()).unwrap_or_else(|e| e.to_string()),
            }))
        })();
        let _ = child.kill();
        let _ = child.wait();
        let v = result?;
        println!("{v}");
        if v["ok"] != json!(true) {
            return Err("SourceClosed was not reported after the captured window closed".into());
        }
        Ok(())
    }

    fn restart_test(b: &dyn CaptureBackend) -> R<()> {
        let display = first_display(b)?;
        let mut runs = Vec::new();
        for i in 0..3 {
            let sink = Arc::new(Latest::default());
            let t = Instant::now();
            let mut handle = b
                .start_video(
                    &CaptureTarget::Display {
                        display_id: display.id.clone(),
                    },
                    &VideoStreamOptions { fps: 30, show_cursor: true },
                    sink.clone(),
                )
                .map_err(s)?;
            std::thread::sleep(Duration::from_millis(700));
            let stop_t = Instant::now();
            handle.stop().map_err(s)?;
            let stop_ms = stop_t.elapsed().as_millis();
            // Stopping twice must be harmless; dropping after stop too.
            handle.stop().map_err(s)?;
            drop(handle);
            let after = sink.delivered.load(Ordering::Relaxed);
            std::thread::sleep(Duration::from_millis(200));
            let later = sink.delivered.load(Ordering::Relaxed);
            runs.push(json!({ "run": i, "frames": after, "framesAfterStop": later - after, "stopMs": stop_ms, "totalMs": t.elapsed().as_millis() }));
            if after == 0 {
                return Err(format!("run {i}: no frames delivered"));
            }
            if later != after {
                return Err(format!("run {i}: frames were delivered after stop()"));
            }
        }
        // Drop without stop must also clean up.
        {
            let sink = Arc::new(Latest::default());
            let _handle = b
                .start_video(
                    &CaptureTarget::Display { display_id: display.id },
                    &VideoStreamOptions { fps: 30, show_cursor: true },
                    sink,
                )
                .map_err(s)?;
            std::thread::sleep(Duration::from_millis(300));
        }
        println!("{}", json!({ "ok": true, "runs": runs, "dropWithoutStop": "ok" }));
        Ok(())
    }

    #[derive(Default)]
    struct AudioCollect {
        samples: Mutex<Vec<f32>>,
        format: Mutex<Option<(u32, u16)>>,
        buffers: AtomicU64,
        events: Mutex<Vec<String>>,
    }

    impl AudioSink for AudioCollect {
        fn on_audio(&self, buffer: AudioBuffer) {
            self.buffers.fetch_add(1, Ordering::Relaxed);
            *self.format.lock().unwrap_or_else(|e| e.into_inner()) = Some((buffer.sample_rate, buffer.channels));
            self.samples.lock().unwrap_or_else(|e| e.into_inner()).extend_from_slice(&buffer.samples);
        }
        fn on_event(&self, event: StreamEvent) {
            self.events.lock().unwrap_or_else(|e| e.into_inner()).push(format!("{event:?}"));
        }
    }

    fn write_wav_f32(path: &str, rate: u32, channels: u16, samples: &[f32]) -> R<()> {
        let data_len = (samples.len() * 4) as u32;
        let mut out = Vec::with_capacity(44 + data_len as usize);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_len).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * channels as u32 * 4).to_le_bytes());
        out.extend_from_slice(&(channels * 4).to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_len.to_le_bytes());
        for v in samples {
            out.extend_from_slice(&v.to_le_bytes());
        }
        std::fs::write(path, out).map_err(s)
    }

    /// Either captures loopback audio, or — when the machine has no output device — verifies the
    /// backend reports that honestly (SOURCE_NOT_FOUND + systemAudio unsupported).
    fn audio(b: &dyn CaptureBackend, secs: u64, out: &str) -> R<()> {
        let level = b.capabilities().level(FeatureKey::SystemAudio);
        let sink = Arc::new(AudioCollect::default());
        let opts = AudioStreamOptions {
            sample_rate: 48_000,
            channels: 2,
            device_id: None,
        };
        match b.start_system_audio(&opts, sink.clone()) {
            Ok(mut handle) => {
                std::thread::sleep(Duration::from_secs(secs));
                handle.stop().map_err(s)?;
                let samples = sink.samples.lock().unwrap_or_else(|e| e.into_inner()).clone();
                let fmt = *sink.format.lock().unwrap_or_else(|e| e.into_inner());
                if let Some((rate, ch)) = fmt {
                    write_wav_f32(out, rate, ch, &samples)?;
                }
                let peak = samples.iter().fold(0f32, |m, v| m.max(v.abs()));
                println!(
                    "{}",
                    json!({ "ok": true, "mode": "captured", "capability": level, "buffers": sink.buffers.load(Ordering::Relaxed), "samples": samples.len(),
                            "format": fmt, "peak": peak, "events": *sink.events.lock().unwrap_or_else(|e| e.into_inner()), "file": fmt.map(|_| out) })
                );
                Ok(())
            }
            Err(e) => {
                let honest = e.code == CaptureErrorCode::SourceNotFound && level == SupportLevel::Unsupported;
                println!("{}", json!({ "ok": honest, "mode": "unavailable", "capability": level, "error": e }));
                if honest {
                    Ok(())
                } else {
                    Err(format!("start_system_audio failed ({e}) while capabilities report systemAudio = {level:?}"))
                }
            }
        }
    }
}
