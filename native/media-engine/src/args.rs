//! ffmpeg command-line builders (pure functions, unit tested).

use crate::encoders::{encoder_args, encoder_filter_tail, encoder_pre_input_args, is_hevc, VideoEncodeParams};
use capture_core::{Size, TrackMode};
use std::path::{Path, PathBuf};

fn s<T: ToString>(v: T) -> String {
    v.to_string()
}

fn p(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

/// Fragmented MP4 flags for crash-safe intermediates.
pub const FRAG_VIDEO_MOVFLAGS: &str = "+frag_keyframe+empty_moov+default_base_moof";

#[derive(Debug, Clone)]
pub struct VideoEncodeSpec {
    pub input: Size,
    pub output: Size,
    pub fps: u32,
    pub encoder: String,
    pub bitrate_kbps: u32,
    pub keyframe_interval_sec: f64,
    pub out_path: PathBuf,
}

/// Raw BGRA frames on stdin -> scaled, color-converted, encoded fragmented MP4.
pub fn video_encode_args(v: &VideoEncodeSpec) -> Vec<String> {
    let mut a: Vec<String> = vec!["-hide_banner".into(), "-nostdin".into(), "-loglevel".into(), "error".into()];
    a.extend(encoder_pre_input_args(&v.encoder));
    a.extend([
        s("-f"),
        s("rawvideo"),
        s("-pix_fmt"),
        s("bgra"),
        s("-s"),
        format!("{}x{}", v.input.width, v.input.height),
        s("-r"),
        s(v.fps),
        s("-i"),
        s("-"),
    ]);
    // Always run the scaler: it converts sRGB BGRA to BT.709 limited-range YUV even at 1:1.
    let vf = format!(
        "scale={}:{}:flags=bicubic:out_color_matrix=bt709:out_range=tv,{}",
        v.output.width,
        v.output.height,
        encoder_filter_tail(&v.encoder)
    );
    a.extend([s("-vf"), vf]);
    a.extend(encoder_args(&VideoEncodeParams {
        encoder: v.encoder.clone(),
        bitrate_kbps: v.bitrate_kbps,
        fps: v.fps,
        keyframe_interval_sec: v.keyframe_interval_sec,
        pixel_rate: v.output.width as u64 * v.output.height as u64 * v.fps as u64,
    }));
    a.extend(["-colorspace", "bt709", "-color_primaries", "bt709", "-color_trc", "bt709", "-color_range", "tv"].map(s));
    if is_hevc(&v.encoder) {
        a.extend([s("-tag:v"), s("hvc1")]);
    }
    a.extend([s("-an"), s("-movflags"), s(FRAG_VIDEO_MOVFLAGS), s("-flush_packets"), s("1"), s("-f"), s("mp4"), s("-y"), p(&v.out_path)]);
    a
}

#[derive(Debug, Clone)]
pub struct AudioEncodeSpec {
    pub sample_rate: u32,
    pub channels: u16,
    pub bitrate_kbps: u32,
    pub out_path: PathBuf,
}

/// f32le PCM on stdin -> AAC in a fragmented .m4a (1 s fragments).
pub fn audio_encode_args(a: &AudioEncodeSpec) -> Vec<String> {
    vec![
        s("-hide_banner"),
        s("-nostdin"),
        s("-loglevel"),
        s("error"),
        s("-f"),
        s("f32le"),
        s("-ar"),
        s(a.sample_rate),
        s("-ac"),
        s(a.channels),
        s("-i"),
        s("-"),
        s("-c:a"),
        s("aac"),
        s("-b:a"),
        format!("{}k", a.bitrate_kbps),
        s("-movflags"),
        s("+empty_moov+default_base_moof"),
        s("-frag_duration"),
        s("1000000"),
        s("-flush_packets"),
        s("1"),
        s("-f"),
        s("mp4"),
        s("-y"),
        p(&a.out_path),
    ]
}

#[derive(Debug, Clone)]
pub struct MuxAudioInput {
    pub path: PathBuf,
    pub label: String,
}

#[derive(Debug, Clone)]
pub struct MuxSpec {
    pub video: PathBuf,
    pub audio: Vec<MuxAudioInput>,
    pub track_mode: TrackMode,
    pub audio_bitrate_kbps: u32,
    pub hevc: bool,
    pub title: Option<String>,
    pub out_path: PathBuf,
}

/// Number of audio streams the muxed file will contain.
pub fn expected_audio_streams(n_sources: usize, mode: TrackMode) -> usize {
    match mode {
        TrackMode::Separate => n_sources,
        TrackMode::Mixed => n_sources.min(1),
    }
}

/// Final mux: stream-copies video (and audio unless several sources are mixed with amix).
pub fn mux_args(m: &MuxSpec) -> Vec<String> {
    let mut a: Vec<String> = vec![s("-hide_banner"), s("-nostdin"), s("-loglevel"), s("error"), s("-i"), p(&m.video)];
    for inp in &m.audio {
        a.extend([s("-i"), p(&inp.path)]);
    }
    a.extend([s("-map"), s("0:v:0")]);
    let mix = m.track_mode == TrackMode::Mixed && m.audio.len() > 1;
    if mix {
        let inputs: String = (1..=m.audio.len()).map(|i| format!("[{i}:a]")).collect();
        let filter = format!("{inputs}amix=inputs={}:duration=longest:dropout_transition=0:normalize=0[aout]", m.audio.len());
        a.extend([s("-filter_complex"), filter, s("-map"), s("[aout]")]);
        a.extend([s("-c:v"), s("copy"), s("-c:a"), s("aac"), s("-b:a"), format!("{}k", m.audio_bitrate_kbps)]);
        let label = m.audio.iter().map(|i| i.label.as_str()).collect::<Vec<_>>().join(" + ");
        a.extend([s("-metadata:s:a:0"), format!("title={label}"), s("-metadata:s:a:0"), format!("handler_name={label}")]);
    } else {
        for i in 0..m.audio.len() {
            a.extend([s("-map"), format!("{}:a:0", i + 1)]);
        }
        a.extend([s("-c"), s("copy")]);
        for (i, inp) in m.audio.iter().enumerate() {
            a.extend([format!("-metadata:s:a:{i}"), format!("title={}", inp.label), format!("-metadata:s:a:{i}"), format!("handler_name={}", inp.label)]);
        }
    }
    if m.audio.len() > 1 && !mix {
        // First track is default, others are alternates.
        a.extend([s("-disposition:a:0"), s("default")]);
        for i in 1..m.audio.len() {
            a.extend([format!("-disposition:a:{i}"), s("0")]);
        }
    }
    if m.hevc {
        a.extend([s("-tag:v"), s("hvc1")]);
    }
    if let Some(t) = &m.title {
        a.extend([s("-metadata"), format!("title={t}")]);
    }
    a.extend([s("-metadata"), s("comment=Recorded with CaptureOne")]);
    a.extend([s("-movflags"), s("+faststart"), s("-f"), s("mp4"), s("-y"), p(&m.out_path)]);
    a
}

/// JPEG thumbnail from within the first second.
pub fn thumbnail_args(input: &Path, seek_sec: f64, max_width: u32, out: &Path) -> Vec<String> {
    let mut a = vec![s("-hide_banner"), s("-nostdin"), s("-loglevel"), s("error")];
    if seek_sec > 0.0 {
        a.extend([s("-ss"), format!("{seek_sec:.3}")]);
    }
    a.extend([
        s("-i"),
        p(input),
        s("-frames:v"),
        s("1"),
        s("-vf"),
        format!("scale='min({max_width},iw)':-2"),
        s("-q:v"),
        s("4"),
        s("-f"),
        s("image2"),
        s("-y"),
        p(out),
    ]);
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn joined(v: &[String]) -> String {
        v.join(" ")
    }

    #[test]
    fn video_args_shape() {
        let a = video_encode_args(&VideoEncodeSpec {
            input: Size { width: 1920, height: 1080 },
            output: Size { width: 1280, height: 720 },
            fps: 30,
            encoder: "libx264".into(),
            bitrate_kbps: 4000,
            keyframe_interval_sec: 2.0,
            out_path: "/tmp/s/video.mp4".into(),
        });
        let j = joined(&a);
        assert!(j.contains("-f rawvideo -pix_fmt bgra -s 1920x1080 -r 30 -i -"), "{j}");
        assert!(j.contains("scale=1280:720:flags=bicubic:out_color_matrix=bt709:out_range=tv,format=yuv420p"), "{j}");
        assert!(j.contains("-movflags +frag_keyframe+empty_moov+default_base_moof"));
        assert!(j.ends_with("-f mp4 -y /tmp/s/video.mp4"));
        assert!(!j.contains("hvc1"));
        // -i - must come before the codec args
        let i_pos = a.iter().position(|x| x == "-i").unwrap();
        let c_pos = a.iter().position(|x| x == "-c:v").unwrap();
        assert!(i_pos < c_pos);
    }

    #[test]
    fn hevc_is_tagged_hvc1() {
        let a = video_encode_args(&VideoEncodeSpec {
            input: Size { width: 640, height: 480 },
            output: Size { width: 640, height: 480 },
            fps: 60,
            encoder: "libx265".into(),
            bitrate_kbps: 2000,
            keyframe_interval_sec: 1.0,
            out_path: "v.mp4".into(),
        });
        assert!(joined(&a).contains("-tag:v hvc1"));
    }

    #[test]
    fn audio_args_shape() {
        let a = audio_encode_args(&AudioEncodeSpec { sample_rate: 48000, channels: 2, bitrate_kbps: 160, out_path: "a.m4a".into() });
        let j = joined(&a);
        assert!(j.contains("-f f32le -ar 48000 -ac 2 -i - -c:a aac -b:a 160k"));
        assert!(j.contains("-frag_duration 1000000"));
    }

    fn mux(mode: TrackMode, n: usize) -> String {
        let audio = (0..n).map(|i| MuxAudioInput { path: format!("a{i}.m4a").into(), label: if i == 0 { "Microphone".into() } else { "System audio".into() } }).collect();
        joined(&mux_args(&MuxSpec { video: "v.mp4".into(), audio, track_mode: mode, audio_bitrate_kbps: 160, hevc: false, title: None, out_path: "out.mp4".into() }))
    }

    #[test]
    fn mux_separate_maps_each_track() {
        let j = mux(TrackMode::Separate, 2);
        assert!(j.contains("-map 0:v:0 -map 1:a:0 -map 2:a:0 -c copy"), "{j}");
        assert!(j.contains("-metadata:s:a:0 title=Microphone"));
        assert!(j.contains("-metadata:s:a:1 title=System audio"));
        assert!(j.contains("-movflags +faststart"));
        assert!(!j.contains("amix"));
        assert_eq!(expected_audio_streams(2, TrackMode::Separate), 2);
    }

    #[test]
    fn mux_mixed_uses_amix_and_reencodes_audio_only() {
        let j = mux(TrackMode::Mixed, 2);
        assert!(j.contains("[1:a][2:a]amix=inputs=2"), "{j}");
        assert!(j.contains("-c:v copy -c:a aac -b:a 160k"));
        assert_eq!(expected_audio_streams(2, TrackMode::Mixed), 1);
    }

    #[test]
    fn mux_mixed_single_source_is_copied() {
        let j = mux(TrackMode::Mixed, 1);
        assert!(!j.contains("amix") && j.contains("-map 1:a:0 -c copy"));
        let none = mux(TrackMode::Mixed, 0);
        assert!(none.contains("-map 0:v:0 -c copy") && expected_audio_streams(0, TrackMode::Mixed) == 0);
    }

    #[test]
    fn thumbnail_args_shape() {
        let j = joined(&thumbnail_args(Path::new("in.mp4"), 0.5, 480, Path::new("t.jpg")));
        assert!(j.contains("-ss 0.500 -i in.mp4 -frames:v 1"));
        assert!(j.contains("scale='min(480,iw)':-2"));
    }
}
