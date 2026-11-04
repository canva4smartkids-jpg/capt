//! Writing and validating screenshots obtained from a TV.

use capture_core::*;
use std::path::{Path, PathBuf};

fn random_id() -> String {
    let mut b = [0u8; 8];
    if getrandom::fill(&mut b).is_err() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        b = (nanos as u64).to_le_bytes();
    }
    hex::encode(b)
}

fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let first = dir.join(format!("{stem}.{ext}"));
    if !first.exists() {
        return first;
    }
    (2..10_000).map(|n| dir.join(format!("{stem} ({n}).{ext}"))).find(|p| !p.exists()).unwrap_or_else(|| dir.join(format!("{stem}-{}.{ext}", random_id())))
}

/// Validates `bytes` by fully decoding them, writes the file (atomically) and returns metadata.
pub fn save_screenshot(bytes: &[u8], output_dir: &Path, source_name: &str, platform: &str) -> CaptureResult<MediaMetadata> {
    let (format, ext, container) = match bytes {
        [0xFF, 0xD8, 0xFF, ..] => (ImageFormat::Jpeg, "jpg", "jpeg"),
        [0x89, b'P', b'N', b'G', ..] => (ImageFormat::Png, "png", "png"),
        _ => return Err(CaptureError::new(CaptureErrorCode::Network, "the TV returned data that is not a JPEG or PNG image")),
    };
    let img = image::load_from_memory(bytes).map_err(|e| CaptureError::new(CaptureErrorCode::Network, format!("TV screenshot does not decode: {e}")))?;
    let (width, height) = (img.width(), img.height());
    if width == 0 || height == 0 {
        return Err(CaptureError::new(CaptureErrorCode::Network, "TV screenshot has zero size"));
    }
    let max_luma = img.to_luma8().pixels().map(|p| p.0[0]).max().unwrap_or(0);
    let mut issues = Vec::new();
    if max_luma < 16 {
        issues.push(ValidationIssue {
            severity: IssueSeverity::Warning,
            code: "BLANK_FRAME".into(),
            message: "The TV returned an all-black picture. Protected (HDCP/DRM) content and some inputs are blanked by the TV.".into(),
        });
    }
    std::fs::create_dir_all(output_dir).map_err(CaptureError::from)?;
    let created_at = now_iso8601();
    let stamp: String = created_at.chars().filter(|c| c.is_ascii_digit()).collect();
    let safe_source: String = source_name.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
    let stem = format!("{} {}-{}", safe_source.trim(), &stamp[..8.min(stamp.len())], &stamp[8.min(stamp.len())..]);
    let path = unique_path(output_dir, stem.trim(), ext);
    let tmp = path.with_extension(format!("{ext}.part"));
    std::fs::write(&tmp, bytes).map_err(CaptureError::from)?;
    std::fs::rename(&tmp, &path).map_err(CaptureError::from)?;
    let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(MediaMetadata {
        id: random_id(),
        kind: MediaKind::Screenshot,
        path: path.display().to_string(),
        file_name,
        title: format!("{source_name} screenshot"),
        created_at: created_at.clone(),
        size_bytes: bytes.len() as u64,
        width,
        height,
        duration_ms: None,
        fps: None,
        video_codec: None,
        video_bitrate_kbps: None,
        audio_tracks: vec![],
        image_format: Some(format),
        container: container.into(),
        source_name: Some(source_name.to_string()),
        platform: Some(platform.to_string()),
        validation: Some(MediaValidation { valid: true, checked_at: created_at, tool: "image-rs decode".into(), issues }),
        thumbnail_path: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg(w: u32, h: u32, v: u8) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb([v, v / 2, 255 - v]));
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img).write_to(&mut out, image::ImageFormat::Jpeg).unwrap();
        out.into_inner()
    }

    #[test]
    fn saves_and_flags_black() {
        let dir = tempfile::tempdir().unwrap();
        let m = save_screenshot(&jpeg(64, 36, 200), dir.path(), "LG OLED", "webos").unwrap();
        assert_eq!((m.width, m.height, m.container.as_str()), (64, 36, "jpeg"));
        assert!(std::path::Path::new(&m.path).exists());
        assert!(m.validation.as_ref().unwrap().issues.is_empty());
        let black = image::RgbImage::from_pixel(32, 32, image::Rgb([0, 0, 0]));
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(black).write_to(&mut out, image::ImageFormat::Png).unwrap();
        let m2 = save_screenshot(&out.into_inner(), dir.path(), "LG OLED", "webos").unwrap();
        assert_eq!(m2.validation.unwrap().issues[0].code, "BLANK_FRAME");
        assert_ne!(m.path, m2.path);
        assert!(save_screenshot(b"<html>", dir.path(), "x", "webos").is_err());
        assert!(save_screenshot(&[0xFF, 0xD8, 0xFF, 0, 0], dir.path(), "x", "webos").is_err(), "truncated JPEG is rejected");
    }
}
