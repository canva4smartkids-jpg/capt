//! Screenshots (PNG/JPEG via the `image` crate, validated by decoding) and source thumbnails.

use crate::finalize::{file_time_iso, platform_string};
use crate::naming::{candidate_names, persist_no_clobber, temp_path};
use base64::Engine as _;
use capture_core::*;
use image::ImageEncoder;
use std::path::Path;

/// Resolved capture target: the source (for names/sizes) and, for regions, the rect plus
/// its size clamped to the display.
pub(crate) struct TargetInfo {
    pub source: CaptureSource,
    pub region: Option<(Rect, Size)>,
}

pub(crate) fn resolve_target(backend: &dyn CaptureBackend, ffmpeg: Option<&Path>, target: &CaptureTarget) -> CaptureResult<TargetInfo> {
    match target {
        CaptureTarget::Display { display_id } => Ok(TargetInfo { source: find_source(backend, SourceKind::Display, display_id)?, region: None }),
        CaptureTarget::Window { window_id } => Ok(TargetInfo { source: find_source(backend, SourceKind::Window, window_id)?, region: None }),
        CaptureTarget::Region { display_id, rect } => {
            let source = find_source(backend, SourceKind::Display, display_id)?;
            let size = clamp_region(*rect, source.width, source.height)
                .ok_or_else(|| CaptureError::invalid_config(format!("region {rect:?} is outside display '{display_id}' ({}x{})", source.width, source.height)))?;
            Ok(TargetInfo { source, region: Some((*rect, size)) })
        }
        CaptureTarget::Device { device_id } => Ok(TargetInfo { source: crate::device::find_device(ffmpeg, device_id)?, region: None }),
    }
}

/// Size of `rect` intersected with a `w`x`h` frame, `None` if empty.
pub(crate) fn clamp_region(rect: Rect, w: u32, h: u32) -> Option<Size> {
    let x0 = rect.x.max(0) as i64;
    let y0 = rect.y.max(0) as i64;
    let x1 = (rect.x as i64 + rect.width as i64).min(w as i64);
    let y1 = (rect.y as i64 + rect.height as i64).min(h as i64);
    (x1 > x0 && y1 > y0).then(|| Size { width: (x1 - x0) as u32, height: (y1 - y0) as u32 })
}

/// Crops a frame to the region unless the backend already delivered exactly the region.
pub(crate) fn apply_region(frame: VideoFrame, region: Option<(Rect, Size)>) -> CaptureResult<VideoFrame> {
    match region {
        Some((_, size)) if frame.width == size.width && frame.height == size.height => Ok(frame),
        Some((rect, _)) => frame.crop(rect),
        None => Ok(frame),
    }
}

fn rgb_bytes(frame: &VideoFrame) -> Vec<u8> {
    let packed = frame.packed_bgra();
    let mut out = Vec::with_capacity(packed.len() / 4 * 3);
    for px in packed.chunks_exact(4) {
        out.extend_from_slice(&[px[2], px[1], px[0]]);
    }
    out
}

fn encode_image(frame: &VideoFrame, format: ImageFormat, jpeg_quality: u8) -> CaptureResult<Vec<u8>> {
    let rgb = rgb_bytes(frame);
    let mut buf = Vec::new();
    let res = match format {
        ImageFormat::Png => image::codecs::png::PngEncoder::new_with_quality(&mut buf, image::codecs::png::CompressionType::Fast, image::codecs::png::FilterType::Adaptive)
            .write_image(&rgb, frame.width, frame.height, image::ExtendedColorType::Rgb8),
        ImageFormat::Jpeg => image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, jpeg_quality.clamp(1, 100)).write_image(&rgb, frame.width, frame.height, image::ExtendedColorType::Rgb8),
    };
    res.map_err(|e| CaptureError::new(CaptureErrorCode::EncoderFailed, format!("image encoding failed: {e}")))?;
    Ok(buf)
}

/// Decodes an image file and checks its dimensions; warns if all pixels are identical
/// (blank capture, e.g. protected content or a missing compositor).
pub(crate) fn validate_image(path: &Path, expected: Option<Size>) -> (MediaValidation, Option<(u32, u32, ImageFormat)>) {
    let mut issues = Vec::new();
    let mut dims = None;
    let decoded = image::ImageReader::open(path).map_err(|e| e.to_string()).and_then(|r| r.with_guessed_format().map_err(|e| e.to_string()));
    match decoded {
        Ok(reader) => {
            let fmt = match reader.format() {
                Some(image::ImageFormat::Png) => Some(ImageFormat::Png),
                Some(image::ImageFormat::Jpeg) => Some(ImageFormat::Jpeg),
                _ => None,
            };
            match (fmt, reader.decode()) {
                (Some(fmt), Ok(img)) => {
                    let (w, h) = (img.width(), img.height());
                    dims = Some((w, h, fmt));
                    if let Some(sz) = expected {
                        if (w, h) != (sz.width, sz.height) {
                            issues.push(ValidationIssue { severity: IssueSeverity::Error, code: "DIMENSION_MISMATCH".into(), message: format!("image is {w}x{h}, expected {}x{}", sz.width, sz.height) });
                        }
                    }
                    let rgb = img.to_rgb8();
                    let first = rgb.pixels().next().copied();
                    if let Some(first) = first {
                        if rgb.pixels().all(|p| *p == first) {
                            issues.push(ValidationIssue {
                                severity: IssueSeverity::Warning,
                                code: "UNIFORM_IMAGE".into(),
                                message: "every pixel has the same color (blank or protected content?)".into(),
                            });
                        }
                    }
                }
                (None, _) => issues.push(ValidationIssue { severity: IssueSeverity::Error, code: "UNKNOWN_FORMAT".into(), message: "not a PNG or JPEG file".into() }),
                (_, Err(e)) => issues.push(ValidationIssue { severity: IssueSeverity::Error, code: "DECODE_FAILED".into(), message: format!("image does not decode: {e}") }),
            }
        }
        Err(e) => issues.push(ValidationIssue { severity: IssueSeverity::Error, code: "UNREADABLE".into(), message: e }),
    }
    let valid = !issues.iter().any(|i| i.severity == IssueSeverity::Error);
    (MediaValidation { valid, checked_at: now_iso8601(), tool: "image-rs decode".into(), issues }, dims)
}

pub(crate) fn screenshot(backend: &dyn CaptureBackend, ffmpeg: Option<&Path>, req: &ScreenshotRequest, output_dir: &Path) -> CaptureResult<MediaMetadata> {
    let info = resolve_target(backend, ffmpeg, &req.target)?;
    let frame = grab(backend, ffmpeg, &req.target, req.show_cursor)?;
    let frame = apply_region(frame, info.region)?;
    let bytes = encode_image(&frame, req.format, req.jpeg_quality)?;
    std::fs::create_dir_all(output_dir)?;
    let id = uuid::Uuid::new_v4().to_string();
    let ext = match req.format {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpg",
    };
    let tmp = temp_path(output_dir, &id, ext);
    if let Err(e) = std::fs::write(&tmp, &bytes) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    let template = req.file_name_template.clone().unwrap_or_else(|| "CaptureOne {date} {time}".into());
    let now = chrono::Local::now();
    let path = persist_no_clobber(&tmp, output_dir, candidate_names(&template, &now, &info.source.name, ext))?;
    let (validation, _) = validate_image(&path, Some(Size { width: frame.width, height: frame.height }));
    let size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(bytes.len() as u64);
    Ok(MediaMetadata {
        id,
        kind: MediaKind::Screenshot,
        path: path.to_string_lossy().to_string(),
        file_name: path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default(),
        title: path.file_stem().map(|f| f.to_string_lossy().to_string()).unwrap_or_default(),
        created_at: now_iso8601(),
        size_bytes,
        width: frame.width,
        height: frame.height,
        duration_ms: None,
        fps: None,
        video_codec: None,
        video_bitrate_kbps: None,
        audio_tracks: Vec::new(),
        image_format: Some(req.format),
        container: ext_container(req.format).into(),
        source_name: Some(info.source.name),
        platform: Some(platform_string()),
        validation: Some(validation),
        thumbnail_path: None,
    })
}

fn ext_container(f: ImageFormat) -> &'static str {
    match f {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpeg",
    }
}

/// Metadata + decode validation for an existing PNG/JPEG file.
pub(crate) fn inspect_image(path: &Path) -> CaptureResult<MediaMetadata> {
    let (validation, dims) = validate_image(path, None);
    let (w, h, fmt) = dims.unwrap_or((0, 0, ImageFormat::Png));
    Ok(MediaMetadata {
        id: uuid::Uuid::new_v4().to_string(),
        kind: MediaKind::Screenshot,
        path: path.to_string_lossy().to_string(),
        file_name: path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default(),
        title: path.file_stem().map(|f| f.to_string_lossy().to_string()).unwrap_or_default(),
        created_at: file_time_iso(path),
        size_bytes: std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
        width: w,
        height: h,
        duration_ms: None,
        fps: None,
        video_codec: None,
        video_bitrate_kbps: None,
        audio_tracks: Vec::new(),
        image_format: dims.map(|d| d.2),
        container: ext_container(fmt).into(),
        source_name: None,
        platform: None,
        validation: Some(validation),
        thumbnail_path: None,
    })
}

/// One frame of any target; devices are read through ffmpeg.
fn grab(backend: &dyn CaptureBackend, ffmpeg: Option<&Path>, target: &CaptureTarget, show_cursor: bool) -> CaptureResult<VideoFrame> {
    match target {
        CaptureTarget::Device { device_id } => {
            let ff = ffmpeg.ok_or_else(|| CaptureError::new(CaptureErrorCode::EncoderUnavailable, "ffmpeg was not found (needed for capture devices)"))?;
            crate::device::grab_frame(ff, device_id)
        }
        _ => backend.screenshot(target, show_cursor),
    }
}

/// PNG data URL of a display/window scaled to at most `max_width` pixels wide.
pub(crate) fn source_thumbnail(backend: &dyn CaptureBackend, ffmpeg: Option<&Path>, kind: SourceKind, source_id: &str, max_width: u32) -> CaptureResult<String> {
    let target = match kind {
        SourceKind::Display => CaptureTarget::Display { display_id: source_id.to_string() },
        SourceKind::Window => CaptureTarget::Window { window_id: source_id.to_string() },
        SourceKind::Device => CaptureTarget::Device { device_id: source_id.to_string() },
    };
    let frame = grab(backend, ffmpeg, &target, false)?;
    let img = image::RgbImage::from_raw(frame.width, frame.height, rgb_bytes(&frame)).ok_or_else(|| CaptureError::internal("frame buffer size mismatch"))?;
    let max_w = max_width.clamp(16, 4096);
    let img = if frame.width > max_w {
        let h = ((frame.height as u64 * max_w as u64) / frame.width as u64).max(1) as u32;
        image::imageops::thumbnail(&img, max_w, h)
    } else {
        img
    };
    let mut buf = Vec::new();
    image::codecs::png::PngEncoder::new(&mut buf)
        .write_image(img.as_raw(), img.width(), img.height(), image::ExtendedColorType::Rgb8)
        .map_err(|e| CaptureError::new(CaptureErrorCode::EncoderFailed, format!("thumbnail encoding failed: {e}")))?;
    Ok(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&buf)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_clamping() {
        assert_eq!(clamp_region(Rect { x: 10, y: 10, width: 100, height: 50 }, 1920, 1080), Some(Size { width: 100, height: 50 }));
        assert_eq!(clamp_region(Rect { x: 1900, y: 1000, width: 100, height: 100 }, 1920, 1080), Some(Size { width: 20, height: 80 }));
        assert_eq!(clamp_region(Rect { x: -10, y: 0, width: 30, height: 30 }, 1920, 1080), Some(Size { width: 20, height: 30 }));
        assert_eq!(clamp_region(Rect { x: 2000, y: 0, width: 30, height: 30 }, 1920, 1080), None);
    }

    #[test]
    fn encodes_and_validates_png_and_jpeg() {
        let (w, h) = (64u32, 48u32);
        let mut data = vec![0u8; (w * h * 4) as usize];
        for (i, px) in data.chunks_exact_mut(4).enumerate() {
            px.copy_from_slice(&[(i % 256) as u8, (i / 7 % 256) as u8, 128, 255]);
        }
        let frame = VideoFrame::new_bgra(w, h, w * 4, data).unwrap();
        let dir = std::env::temp_dir().join(format!("captureone-shot-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        for (fmt, ext) in [(ImageFormat::Png, "png"), (ImageFormat::Jpeg, "jpg")] {
            let bytes = encode_image(&frame, fmt, 85).unwrap();
            let p = dir.join(format!("x.{ext}"));
            std::fs::write(&p, bytes).unwrap();
            let (v, dims) = validate_image(&p, Some(Size { width: w, height: h }));
            assert!(v.valid, "{:?}", v.issues);
            assert!(!v.issues.iter().any(|i| i.code == "UNIFORM_IMAGE"));
            assert_eq!(dims.unwrap().2, fmt);
            let wrong = validate_image(&p, Some(Size { width: 1, height: 1 })).0;
            assert!(!wrong.valid);
        }
        // a uniform image is flagged with a warning only
        let flat = VideoFrame::new_bgra(8, 8, 32, vec![50; 256]).unwrap();
        let p = dir.join("flat.png");
        std::fs::write(&p, encode_image(&flat, ImageFormat::Png, 90).unwrap()).unwrap();
        let (v, _) = validate_image(&p, None);
        assert!(v.valid && v.issues.iter().any(|i| i.code == "UNIFORM_IMAGE"));
        std::fs::write(dir.join("bad.png"), b"not an image").unwrap();
        assert!(!validate_image(&dir.join("bad.png"), None).0.valid);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn apply_region_detects_native_crop() {
        let full = VideoFrame::new_bgra(100, 100, 400, vec![1; 40_000]).unwrap();
        let r = Some((Rect { x: 10, y: 10, width: 20, height: 30 }, Size { width: 20, height: 30 }));
        let c = apply_region(full, r).unwrap();
        assert_eq!((c.width, c.height), (20, 30));
        let native = VideoFrame::new_bgra(20, 30, 80, vec![1; 2400]).unwrap();
        let n = apply_region(native, r).unwrap();
        assert_eq!((n.width, n.height), (20, 30));
    }
}
