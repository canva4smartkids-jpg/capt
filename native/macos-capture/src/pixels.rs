//! BGRA row handling for `CVPixelBuffer` / bitmap memory.
//!
//! CoreVideo pads rows (`CVPixelBufferGetBytesPerRow` is usually a multiple of 64 bytes and
//! larger than `width * 4`), so frames are copied row by row into a tightly packed buffer
//! before being handed to the engine as a `VideoFrame`.

use capture_core::{CaptureError, CaptureResult};

pub const BGRA_BYTES_PER_PIXEL: usize = 4;

/// `kCVPixelFormatType_32BGRA` (`'BGRA'`).
pub const PIXEL_FORMAT_32BGRA: u32 = u32::from_be_bytes(*b"BGRA");

/// Minimum number of readable bytes for `height` rows of `width` BGRA pixels that are
/// `bytes_per_row` apart (the last row need not be padded).
pub fn required_len(width: usize, height: usize, bytes_per_row: usize) -> Option<usize> {
    if height == 0 {
        return Some(0);
    }
    bytes_per_row.checked_mul(height - 1)?.checked_add(width.checked_mul(BGRA_BYTES_PER_PIXEL)?)
}

/// Copies `height` rows of `width` BGRA pixels, `bytes_per_row` apart in `src`, into a tightly
/// packed vector (`stride == width * 4`).
pub fn pack_bgra_rows(src: &[u8], width: usize, height: usize, bytes_per_row: usize) -> CaptureResult<Vec<u8>> {
    if width == 0 || height == 0 {
        return Err(CaptureError::internal("pixel buffer has zero size"));
    }
    let row = width
        .checked_mul(BGRA_BYTES_PER_PIXEL)
        .ok_or_else(|| CaptureError::internal("pixel buffer width overflows"))?;
    if bytes_per_row < row {
        return Err(CaptureError::internal(format!("bytesPerRow {bytes_per_row} is smaller than width*4 ({row})")));
    }
    let needed = required_len(width, height, bytes_per_row).ok_or_else(|| CaptureError::internal("pixel buffer size overflows"))?;
    if src.len() < needed {
        return Err(CaptureError::internal(format!(
            "pixel buffer too small: {width}x{height}, bytesPerRow {bytes_per_row}, {} bytes available, {needed} needed",
            src.len()
        )));
    }
    if bytes_per_row == row {
        return Ok(src[..row * height].to_vec());
    }
    let mut out = Vec::with_capacity(row * height);
    for y in 0..height {
        let start = y * bytes_per_row;
        out.extend_from_slice(&src[start..start + row]);
    }
    Ok(out)
}

/// Renders a FourCC (`OSType`) such as `'BGRA'` or `'420v'` for diagnostics.
pub fn fourcc_to_string(code: u32) -> String {
    let bytes = code.to_be_bytes();
    if bytes.iter().all(|b| b.is_ascii_graphic() || *b == b' ') {
        bytes.iter().map(|&b| b as char).collect()
    } else {
        format!("{code:#010x}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fourcc_constants() {
        assert_eq!(PIXEL_FORMAT_32BGRA, 0x4247_5241); // kCVPixelFormatType_32BGRA
        assert_eq!(fourcc_to_string(PIXEL_FORMAT_32BGRA), "BGRA");
        assert_eq!(fourcc_to_string(u32::from_be_bytes(*b"420v")), "420v");
        assert_eq!(fourcc_to_string(1), "0x00000001");
    }

    #[test]
    fn packs_padded_rows() {
        // 2x3 image, rows padded to 12 bytes (8 data + 4 padding).
        let mut src = Vec::new();
        for y in 0..3u8 {
            src.extend_from_slice(&[y, 1, 2, 3, y, 5, 6, 7]);
            src.extend_from_slice(&[0xEE; 4]);
        }
        let out = pack_bgra_rows(&src, 2, 3, 12).unwrap();
        assert_eq!(out.len(), 2 * 3 * 4);
        assert!(!out.contains(&0xEE));
        assert_eq!(&out[16..24], &[2, 1, 2, 3, 2, 5, 6, 7]);
    }

    #[test]
    fn last_row_needs_no_padding() {
        let src = vec![9u8; 12 + 8]; // two rows: first padded to 12, second exactly 8 bytes
        assert_eq!(required_len(2, 2, 12), Some(20));
        assert_eq!(pack_bgra_rows(&src, 2, 2, 12).unwrap().len(), 16);
        assert!(pack_bgra_rows(&src[..19], 2, 2, 12).is_err());
    }

    #[test]
    fn tight_rows_are_copied_directly() {
        let src: Vec<u8> = (0..32).collect();
        assert_eq!(pack_bgra_rows(&src, 4, 2, 16).unwrap(), src);
    }

    #[test]
    fn rejects_invalid_geometry() {
        let src = vec![0u8; 64];
        assert!(pack_bgra_rows(&src, 0, 1, 16).is_err());
        assert!(pack_bgra_rows(&src, 1, 0, 16).is_err());
        assert!(pack_bgra_rows(&src, 4, 1, 15).is_err());
        assert!(pack_bgra_rows(&src, 4, 5, 16).is_err());
        assert_eq!(required_len(usize::MAX, 2, 4), None);
    }
}
