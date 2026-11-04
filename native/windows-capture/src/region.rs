//! Region capture by cropping display frames (platform independent).

use capture_core::{CaptureError, Rect, StreamEvent, VideoFrame, VideoSink};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Wraps a sink and crops every display frame to `rect` (display-relative physical pixels).
/// `Resized` events are derived from the cropped size, not forwarded from the display.
pub struct CroppingSink {
    inner: Arc<dyn VideoSink>,
    rect: Rect,
    last_size: Mutex<Option<(u32, u32)>>,
    failed: AtomicBool,
}

impl CroppingSink {
    pub fn new(inner: Arc<dyn VideoSink>, rect: Rect) -> Self {
        Self {
            inner,
            rect,
            last_size: Mutex::new(None),
            failed: AtomicBool::new(false),
        }
    }
}

impl VideoSink for CroppingSink {
    fn on_frame(&self, frame: VideoFrame) {
        if self.failed.load(Ordering::Acquire) {
            return;
        }
        match frame.crop(self.rect) {
            Ok(cropped) => {
                let size = (cropped.width, cropped.height);
                let previous = {
                    let mut last = self.last_size.lock().unwrap_or_else(|e| e.into_inner());
                    last.replace(size)
                };
                if previous.is_some_and(|p| p != size) {
                    self.inner.on_event(StreamEvent::Resized { width: size.0, height: size.1 });
                }
                self.inner.on_frame(cropped);
            }
            Err(_) => {
                if !self.failed.swap(true, Ordering::AcqRel) {
                    self.inner.on_event(StreamEvent::Error(CaptureError::invalid_config(format!(
                        "region {}x{}+{}+{} is no longer inside the display ({}x{})",
                        self.rect.width, self.rect.height, self.rect.x, self.rect.y, frame.width, frame.height
                    ))));
                }
            }
        }
    }

    fn on_event(&self, event: StreamEvent) {
        match event {
            StreamEvent::Resized { .. } => {}
            other => self.inner.on_event(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Collect {
        frames: Mutex<Vec<(u32, u32, u8)>>,
        events: Mutex<Vec<String>>,
    }
    impl VideoSink for Collect {
        fn on_frame(&self, f: VideoFrame) {
            self.frames.lock().unwrap().push((f.width, f.height, f.data[0]));
        }
        fn on_event(&self, e: StreamEvent) {
            self.events.lock().unwrap().push(format!("{e:?}"));
        }
    }

    fn frame(w: u32, h: u32) -> VideoFrame {
        let mut data = vec![0u8; (w * h * 4) as usize];
        for (i, px) in data.chunks_exact_mut(4).enumerate() {
            px[0] = (i % 251) as u8;
        }
        VideoFrame::new_bgra(w, h, w * 4, data).unwrap()
    }

    #[test]
    fn crops_and_reports_size_changes() {
        let sink = Arc::new(Collect::default());
        let crop = CroppingSink::new(
            sink.clone(),
            Rect {
                x: 2,
                y: 1,
                width: 4,
                height: 2,
            },
        );
        crop.on_frame(frame(8, 4));
        crop.on_event(StreamEvent::Resized { width: 5, height: 4 }); // swallowed
        crop.on_frame(frame(5, 4)); // display shrank: region clamps to 3x2
        crop.on_event(StreamEvent::SourceClosed);
        let frames = sink.frames.lock().unwrap().clone();
        assert_eq!(frames[0], (4, 2, 10)); // pixel (2,1) of an 8-wide frame = index 10
        assert_eq!((frames[1].0, frames[1].1), (3, 2));
        let events = sink.events.lock().unwrap().clone();
        assert_eq!(events, vec!["Resized { width: 3, height: 2 }".to_string(), "SourceClosed".to_string()]);
    }

    #[test]
    fn region_outside_display_errors_once() {
        let sink = Arc::new(Collect::default());
        let crop = CroppingSink::new(
            sink.clone(),
            Rect {
                x: 100,
                y: 100,
                width: 16,
                height: 16,
            },
        );
        crop.on_frame(frame(8, 8));
        crop.on_frame(frame(8, 8));
        assert!(sink.frames.lock().unwrap().is_empty());
        let events = sink.events.lock().unwrap().clone();
        assert_eq!(events.len(), 1);
        assert!(events[0].contains("no longer inside"));
    }
}
