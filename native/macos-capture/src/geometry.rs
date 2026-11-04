//! Points ↔ pixels math.
//!
//! macOS lays out displays and windows in a global coordinate space measured in **points**
//! (origin at the top-left of the main display, y grows downwards in CoreGraphics/SCK).
//! The CaptureOne contract speaks **physical pixels** (`CaptureSource.width/height`, region
//! rects relative to the display's top-left). The backing scale of a display is
//! `pixel width of its current mode / width in points` (2.0 on Retina, 1.0 otherwise, other
//! values for some scaled modes).

use capture_core::{CaptureError, CaptureResult, Rect};

/// A rectangle in points (CoreGraphics `CGRect`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl PointRect {
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self { x, y, width, height }
    }
    pub fn center(&self) -> (f64, f64) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }
    pub fn contains(&self, px: f64, py: f64) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.width && py < self.y + self.height
    }
    pub fn intersection_area(&self, o: &PointRect) -> f64 {
        let w = (self.x + self.width).min(o.x + o.width) - self.x.max(o.x);
        let h = (self.y + self.height).min(o.y + o.height) - self.y.max(o.y);
        if w > 0.0 && h > 0.0 {
            w * h
        } else {
            0.0
        }
    }
}

/// Backing scale factor from a display's pixel width and point width. Falls back to 1.0
/// for nonsensical inputs (e.g. a display mode that could not be read).
pub fn scale_factor(pixel_width: u32, point_width: f64) -> f64 {
    if pixel_width == 0 || !(point_width.is_finite() && point_width > 0.0) {
        return 1.0;
    }
    let s = pixel_width as f64 / point_width;
    if (0.25..=8.0).contains(&s) {
        s
    } else {
        1.0
    }
}

/// Points → whole pixels (rounded, at least 1).
pub fn points_to_pixels(points: f64, scale: f64) -> u32 {
    let v = (points * scale).round();
    if v.is_finite() && v >= 1.0 {
        v.min(u32::MAX as f64) as u32
    } else {
        1
    }
}

pub fn pixels_to_points(pixels: f64, scale: f64) -> f64 {
    if scale > 0.0 {
        pixels / scale
    } else {
        pixels
    }
}

/// Pixel size of a window whose frame is `frame` (points) on a display with `scale`.
pub fn window_pixel_size(frame: &PointRect, scale: f64) -> (u32, u32) {
    (points_to_pixels(frame.width, scale), points_to_pixels(frame.height, scale))
}

/// A region capture plan: ScreenCaptureKit's `sourceRect` (points, relative to the display's
/// top-left) and the output size in pixels, after clamping the requested pixel rect to the
/// display.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegionPlan {
    pub source_rect: PointRect,
    /// The clamped region in display pixels (what `VideoFrame::crop` would use).
    pub pixel_rect: Rect,
}

/// Clamps `rect` (physical pixels, relative to the display) to a display of
/// `display_px` pixels and converts it to a display-local `sourceRect` in points.
pub fn region_plan(rect: Rect, display_px: (u32, u32), scale: f64) -> CaptureResult<RegionPlan> {
    let (dw, dh) = (display_px.0 as i64, display_px.1 as i64);
    let x0 = (rect.x as i64).clamp(0, dw);
    let y0 = (rect.y as i64).clamp(0, dh);
    let x1 = (rect.x as i64 + rect.width as i64).clamp(0, dw);
    let y1 = (rect.y as i64 + rect.height as i64).clamp(0, dh);
    if x1 <= x0 || y1 <= y0 {
        return Err(CaptureError::invalid_config("region is outside the captured display"));
    }
    let pixel_rect = Rect { x: x0 as i32, y: y0 as i32, width: (x1 - x0) as u32, height: (y1 - y0) as u32 };
    let source_rect = PointRect::new(
        pixels_to_points(x0 as f64, scale),
        pixels_to_points(y0 as f64, scale),
        pixels_to_points((x1 - x0) as f64, scale),
        pixels_to_points((y1 - y0) as f64, scale),
    );
    Ok(RegionPlan { source_rect, pixel_rect })
}

/// Index of the display a window belongs to: the one containing the window's centre, else
/// the one with the largest overlap, else `None`.
pub fn display_for_window(window: &PointRect, displays: &[PointRect]) -> Option<usize> {
    let (cx, cy) = window.center();
    if let Some(i) = displays.iter().position(|d| d.contains(cx, cy)) {
        return Some(i);
    }
    displays
        .iter()
        .enumerate()
        .map(|(i, d)| (i, d.intersection_area(window)))
        .filter(|(_, a)| *a > 0.0)
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retina_scale() {
        assert_eq!(scale_factor(2880, 1440.0), 2.0);
        assert_eq!(scale_factor(1920, 1920.0), 1.0);
        // "More space" scaled mode: 1680pt backed by a 3360px framebuffer.
        assert_eq!(scale_factor(3360, 1680.0), 2.0);
        assert_eq!(scale_factor(0, 1440.0), 1.0);
        assert_eq!(scale_factor(2880, 0.0), 1.0);
        assert_eq!(scale_factor(100_000, 1.0), 1.0);
    }

    #[test]
    fn point_pixel_conversion() {
        assert_eq!(points_to_pixels(1440.0, 2.0), 2880);
        assert_eq!(points_to_pixels(100.4, 1.0), 100);
        assert_eq!(points_to_pixels(0.1, 1.0), 1);
        assert_eq!(points_to_pixels(f64::NAN, 2.0), 1);
        assert_eq!(pixels_to_points(2880.0, 2.0), 1440.0);
        assert_eq!(pixels_to_points(10.0, 0.0), 10.0);
        assert_eq!(window_pixel_size(&PointRect::new(10.0, 20.0, 800.0, 600.5), 2.0), (1600, 1201));
    }

    #[test]
    fn region_to_source_rect() {
        let plan = region_plan(Rect { x: 100, y: 50, width: 640, height: 480 }, (2880, 1800), 2.0).unwrap();
        assert_eq!(plan.source_rect, PointRect::new(50.0, 25.0, 320.0, 240.0));
        assert_eq!(plan.pixel_rect, Rect { x: 100, y: 50, width: 640, height: 480 });
        // odd pixel coordinates map to half points on a 2x display
        let odd = region_plan(Rect { x: 1, y: 3, width: 17, height: 16 }, (2880, 1800), 2.0).unwrap();
        assert_eq!(odd.source_rect, PointRect::new(0.5, 1.5, 8.5, 8.0));
    }

    #[test]
    fn region_is_clamped_to_display() {
        let plan = region_plan(Rect { x: -20, y: 1700, width: 200, height: 400 }, (1920, 1800), 1.0).unwrap();
        assert_eq!(plan.pixel_rect, Rect { x: 0, y: 1700, width: 180, height: 100 });
        assert_eq!(plan.source_rect, PointRect::new(0.0, 1700.0, 180.0, 100.0));
        assert!(region_plan(Rect { x: 5000, y: 0, width: 10, height: 10 }, (1920, 1080), 1.0).is_err());
        assert!(region_plan(Rect { x: -50, y: 0, width: 50, height: 10 }, (1920, 1080), 1.0).is_err());
        // extreme values must not overflow
        assert!(region_plan(Rect { x: i32::MAX, y: i32::MAX, width: u32::MAX, height: u32::MAX }, (1920, 1080), 1.0).is_err());
        let all = region_plan(Rect { x: i32::MIN, y: i32::MIN, width: u32::MAX, height: u32::MAX }, (1920, 1080), 1.0).unwrap();
        assert_eq!(all.pixel_rect, Rect { x: 0, y: 0, width: 1920, height: 1080 });
    }

    #[test]
    fn picks_display_for_window() {
        let displays = [PointRect::new(0.0, 0.0, 1440.0, 900.0), PointRect::new(1440.0, -200.0, 1920.0, 1080.0)];
        assert_eq!(display_for_window(&PointRect::new(100.0, 100.0, 400.0, 300.0), &displays), Some(0));
        assert_eq!(display_for_window(&PointRect::new(1500.0, 0.0, 400.0, 300.0), &displays), Some(1));
        // centre on display 1, but straddling
        assert_eq!(display_for_window(&PointRect::new(1300.0, 100.0, 600.0, 300.0), &displays), Some(1));
        // centre outside every display (below display 1), but overlapping display 1 only
        assert_eq!(display_for_window(&PointRect::new(1500.0, 800.0, 400.0, 400.0), &displays), Some(1));
        assert_eq!(display_for_window(&PointRect::new(-5000.0, -5000.0, 10.0, 10.0), &displays), None);
        assert_eq!(display_for_window(&PointRect::new(0.0, 0.0, 10.0, 10.0), &[]), None);
    }
}
