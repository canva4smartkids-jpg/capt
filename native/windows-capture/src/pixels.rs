//! Row-pitch handling for mapped D3D11 staging textures (platform independent).

/// Copies `height` rows of `row_bytes` each out of a buffer whose rows start every `pitch` bytes,
/// producing tightly packed rows (the "padding removed" layout `VideoFrame` prefers).
///
/// `src` must hold at least `pitch * (height - 1) + row_bytes` bytes — the last row of a mapped
/// subresource is not guaranteed to be padded.
pub fn pack_rows(src: &[u8], pitch: usize, row_bytes: usize, height: usize) -> Result<Vec<u8>, String> {
    if height == 0 || row_bytes == 0 {
        return Err("empty image".into());
    }
    if pitch < row_bytes {
        return Err(format!("row pitch {pitch} is smaller than the row size {row_bytes}"));
    }
    let needed = required_len(pitch, row_bytes, height);
    if src.len() < needed {
        return Err(format!("mapped buffer too small: {} bytes, need {needed}", src.len()));
    }
    if pitch == row_bytes {
        return Ok(src[..row_bytes * height].to_vec());
    }
    let mut out = Vec::with_capacity(row_bytes * height);
    for y in 0..height {
        let start = y * pitch;
        out.extend_from_slice(&src[start..start + row_bytes]);
    }
    Ok(out)
}

/// Minimum number of readable bytes for an image of `height` rows at `pitch`.
pub fn required_len(pitch: usize, row_bytes: usize, height: usize) -> usize {
    if height == 0 {
        0
    } else {
        pitch * (height - 1) + row_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_padding() {
        // 2x3 BGRA image stored with a 12-byte pitch (4 bytes of padding per row).
        let mut src = Vec::new();
        for y in 0..3u8 {
            src.extend_from_slice(&[y, y, y, y, y + 10, y + 10, y + 10, y + 10]);
            if y < 2 {
                src.extend_from_slice(&[0xEE; 4]);
            }
        }
        let out = pack_rows(&src, 12, 8, 3).unwrap();
        assert_eq!(out.len(), 24);
        assert_eq!(&out[0..8], &[0, 0, 0, 0, 10, 10, 10, 10]);
        assert_eq!(&out[16..24], &[2, 2, 2, 2, 12, 12, 12, 12]);
        assert!(!out.contains(&0xEE));
    }

    #[test]
    fn packed_input_is_copied_verbatim() {
        let src: Vec<u8> = (0..32).collect();
        assert_eq!(pack_rows(&src, 8, 8, 4).unwrap(), src);
        // Extra trailing bytes are ignored.
        let mut longer = src.clone();
        longer.extend_from_slice(&[1, 2, 3]);
        assert_eq!(pack_rows(&longer, 8, 8, 4).unwrap(), src);
    }

    #[test]
    fn rejects_bad_geometry() {
        assert!(pack_rows(&[0; 16], 4, 8, 2).is_err());
        assert!(pack_rows(&[0; 10], 8, 8, 2).is_err());
        assert!(pack_rows(&[], 8, 8, 0).is_err());
        assert_eq!(required_len(16, 8, 3), 40);
        assert!(pack_rows(&[0; 40], 16, 8, 3).is_ok());
    }
}
