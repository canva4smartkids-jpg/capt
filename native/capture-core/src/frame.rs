//! Raw media buffers exchanged between capture backends and the media engine.

use crate::types::{CaptureError, Rect};
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// 8-bit B,G,R,A in memory order (DXGI_FORMAT_B8G8R8A8_UNORM, kCVPixelFormatType_32BGRA, X11 ZPixmap depth 24/32 on little-endian).
    Bgra8,
}

/// One captured video frame in system memory.
#[derive(Clone)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    /// Bytes per row (>= width * 4).
    pub stride: u32,
    pub format: PixelFormat,
    pub data: Arc<Vec<u8>>,
    /// Monotonic capture time, used by the encoder pacer for A/V sync.
    pub captured_at: Instant,
}

impl std::fmt::Debug for VideoFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoFrame").field("width", &self.width).field("height", &self.height).field("stride", &self.stride).finish()
    }
}

impl VideoFrame {
    pub fn new_bgra(width: u32, height: u32, stride: u32, data: Vec<u8>) -> Result<Self, CaptureError> {
        if width == 0 || height == 0 {
            return Err(CaptureError::internal("frame has zero size"));
        }
        if (stride as usize) < width as usize * 4 || data.len() < stride as usize * (height as usize - 1) + width as usize * 4 {
            return Err(CaptureError::internal(format!("frame buffer too small: {}x{} stride {} len {}", width, height, stride, data.len())));
        }
        Ok(Self { width, height, stride, format: PixelFormat::Bgra8, data: Arc::new(data), captured_at: Instant::now() })
    }

    /// Returns tightly packed BGRA rows (stride == width*4), copying only when needed.
    pub fn packed_bgra(&self) -> std::borrow::Cow<'_, [u8]> {
        let row = self.width as usize * 4;
        if self.stride as usize == row {
            std::borrow::Cow::Borrowed(&self.data[..row * self.height as usize])
        } else {
            let mut out = Vec::with_capacity(row * self.height as usize);
            for y in 0..self.height as usize {
                let start = y * self.stride as usize;
                out.extend_from_slice(&self.data[start..start + row]);
            }
            std::borrow::Cow::Owned(out)
        }
    }

    /// Converts to tightly packed RGBA (for PNG/JPEG encoding). Alpha is forced opaque.
    pub fn to_rgba(&self) -> Vec<u8> {
        let packed = self.packed_bgra();
        let mut out = Vec::with_capacity(packed.len());
        for px in packed.chunks_exact(4) {
            out.extend_from_slice(&[px[2], px[1], px[0], 255]);
        }
        out
    }

    /// Crops to `rect` (clamped to the frame). Errors if the intersection is empty.
    pub fn crop(&self, rect: Rect) -> Result<VideoFrame, CaptureError> {
        let x0 = rect.x.max(0) as u32;
        let y0 = rect.y.max(0) as u32;
        let x1 = ((rect.x as i64 + rect.width as i64).min(self.width as i64)).max(0) as u32;
        let y1 = ((rect.y as i64 + rect.height as i64).min(self.height as i64)).max(0) as u32;
        if x1 <= x0 || y1 <= y0 {
            return Err(CaptureError::invalid_config("region is outside the captured display"));
        }
        let (w, h) = (x1 - x0, y1 - y0);
        let row = w as usize * 4;
        let mut out = Vec::with_capacity(row * h as usize);
        for y in y0..y1 {
            let start = y as usize * self.stride as usize + x0 as usize * 4;
            out.extend_from_slice(&self.data[start..start + row]);
        }
        Ok(VideoFrame { width: w, height: h, stride: w * 4, format: self.format, data: Arc::new(out), captured_at: self.captured_at })
    }
}

/// Interleaved 32-bit float PCM.
#[derive(Debug, Clone)]
pub struct AudioBuffer {
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Vec<f32>,
    /// Best-effort monotonic time of the first sample.
    pub captured_at: Instant,
}

impl AudioBuffer {
    pub fn frames(&self) -> usize {
        if self.channels == 0 {
            0
        } else {
            self.samples.len() / self.channels as usize
        }
    }
}
