//! Crash recovery: finalizes session directories left behind by a killed/crashed process.
//! Intermediates are fragmented MP4, so everything up to the last complete fragment
//! (keyframe interval for video, 1 s for audio) is recoverable even after `kill -9`.

use crate::args::{expected_audio_streams, MuxAudioInput, MuxSpec};
use crate::audio::AudioSourceKind;
use crate::finalize::{mux_to_final, platform_string, video_metadata, VideoMetaInput};
use crate::manifest::*;
use crate::naming::candidate_names;
use crate::probe::{codec_from_name, run_ffprobe, Expectation};
use crate::EngineInner;
use capture_core::*;
use fs2::FileExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// True if another live session (any process) holds the session lock.
fn is_locked(dir: &Path) -> bool {
    match std::fs::OpenOptions::new().read(true).write(true).open(dir.join(LOCK_FILE)) {
        Ok(f) => match f.try_lock_exclusive() {
            Ok(()) => {
                let _ = f.unlock();
                false
            }
            Err(_) => true,
        },
        Err(_) => false,
    }
}

pub(crate) fn recover_sessions(engine: &EngineInner, output_dir: &Path) -> Vec<CaptureResult<MediaMetadata>> {
    let root = output_dir.join(SESSIONS_DIR);
    let Ok(rd) = std::fs::read_dir(&root) else { return Vec::new() };
    let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    dirs.sort();
    let mut results = Vec::new();
    for dir in dirs {
        if is_locked(&dir) {
            continue; // recording in progress
        }
        let manifest = read_manifest(&dir);
        if matches!(manifest.as_ref().map(|m| m.state), Some(ManifestState::Completed)) {
            let _ = std::fs::remove_dir_all(&dir);
            continue;
        }
        // Already finalized into a file that failed validation; data kept for diagnosis only.
        if let Some(m) = &manifest {
            if m.state == ManifestState::FailedValidation && m.final_path.as_deref().map(|p| Path::new(p).is_file()).unwrap_or(false) {
                continue;
            }
        }
        results.push(recover_one(engine, output_dir, &dir, manifest));
    }
    results
}

fn non_empty(p: &Path) -> bool {
    std::fs::metadata(p).map(|m| m.len() > 0).unwrap_or(false)
}

fn recover_one(engine: &EngineInner, output_dir: &Path, dir: &Path, manifest: Option<SessionManifest>) -> CaptureResult<MediaMetadata> {
    let ffmpeg = engine.ffmpeg.clone().ok_or_else(|| CaptureError::new(CaptureErrorCode::EncoderUnavailable, "ffmpeg not found"))?;
    let ffprobe = engine.ffprobe.clone().ok_or_else(|| CaptureError::new(CaptureErrorCode::EncoderUnavailable, "ffprobe not found"))?;
    let id = manifest.as_ref().map(|m| m.session_id.clone()).unwrap_or_else(|| dir.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default());
    let video = dir.join(VIDEO_FILE);
    let vprobe = if non_empty(&video) { run_ffprobe(&ffprobe, &video).ok().filter(|p| p.video.is_some()) } else { None };
    let Some(vprobe) = vprobe else {
        // Killed before the first fragment reached the disk: nothing to salvage.
        let _ = std::fs::remove_dir_all(dir);
        return Err(CaptureError::new(CaptureErrorCode::Interrupted, format!("session {id} contains no recoverable video; its data was removed"))
            .with_detail("sessionId", id));
    };

    // Audio intermediates listed in the manifest (or found on disk) that ffprobe can read.
    let candidates: Vec<(String, PathBuf)> = match &manifest {
        Some(m) if !m.audio.is_empty() => m.audio.iter().map(|a| (a.label.clone(), dir.join(&a.file))).collect(),
        _ => [AudioSourceKind::Microphone, AudioSourceKind::System].iter().map(|k| (k.label().to_string(), dir.join(k.file_name()))).collect(),
    };
    let audio: Vec<MuxAudioInput> = candidates
        .into_iter()
        .filter(|(_, p)| non_empty(p) && run_ffprobe(&ffprobe, p).map(|i| !i.audio.is_empty()).unwrap_or(false))
        .map(|(label, path)| MuxAudioInput { path, label })
        .collect();

    let track_mode = manifest.as_ref().map(|m| m.track_mode).unwrap_or(TrackMode::Separate);
    let codec = manifest.as_ref().and_then(|m| m.video.as_ref().map(|v| v.codec)).or_else(|| vprobe.video.as_ref().and_then(|v| codec_from_name(&v.codec_name)));
    let size = manifest.as_ref().and_then(|m| m.video.as_ref().map(|v| Size { width: v.width, height: v.height }));
    let video_duration = vprobe.duration_sec.or_else(|| vprobe.video.as_ref().and_then(|v| v.duration_sec));
    let expectation = Expectation {
        duration_sec: video_duration,
        duration_tolerance_sec: video_duration.map_or(1.0, |d| (d * 0.02).max(1.0)),
        size,
        codec,
        audio_streams: Some(expected_audio_streams(audio.len(), track_mode)),
    };
    let spec = MuxSpec {
        video: video.clone(),
        audio,
        track_mode,
        audio_bitrate_kbps: manifest.as_ref().map_or(160, |m| m.audio_bitrate_kbps),
        hevc: codec == Some(VideoCodec::H265),
        title: None,
        out_path: PathBuf::new(),
    };
    let template = format!("{} (recovered)", manifest.as_ref().map_or("CaptureOne {date} {time}", |m| m.file_name_template.as_str()));
    let created_local = manifest
        .as_ref()
        .and_then(|m| chrono::DateTime::parse_from_rfc3339(&m.created_local).ok())
        .map(|d| d.with_timezone(&chrono::Local))
        .unwrap_or_else(chrono::Local::now);
    let source = manifest.as_ref().map(|m| m.source_name.clone()).unwrap_or_default();
    let names = candidate_names(&template, &created_local, &source, "mp4");
    let outcome = mux_to_final(&ffmpeg, &ffprobe, &spec, output_dir, &id, names, &expectation, Duration::from_secs(1800))?;
    let created_at = manifest.as_ref().map(|m| m.created_at.clone()).unwrap_or_else(now_iso8601);
    let meta = video_metadata(VideoMetaInput {
        id: &id,
        path: &outcome.path,
        size_bytes: outcome.size_bytes,
        probe: &outcome.probe,
        validation: Some(outcome.validation.clone()),
        created_at,
        source_name: (!source.is_empty()).then_some(source),
        platform: Some(platform_string()),
        thumbnail: outcome.thumbnail.clone(),
    });
    if outcome.validation.valid {
        let _ = std::fs::remove_dir_all(dir);
    } else if let Some(mut m) = manifest {
        m.state = ManifestState::FailedValidation;
        m.final_path = Some(meta.path.clone());
        let _ = write_manifest(dir, &m);
    }
    Ok(meta)
}
