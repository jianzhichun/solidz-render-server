//! The pane's size in CSS pixels (= UI points) and the frame's in device pixels, at most
//! [`MAX_DEVICE_SIDE`] per side: a larger pane is drawn at a lower dpr, which frames report.

use super::wire::MAX_DEVICE_SIDE;

/// Largest pane side in CSS px (the host's pane checks refuse more): the UI is laid out over the
/// whole pane, so input positions keep matching it, and only the drawing dpr drops (to ≥ 0.25).
pub const MAX_CSS_SIDE: u32 = 16_384;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameSize {
    /// The pane, CSS px.
    pub css_w: u32,
    pub css_h: u32,
    /// Device pixels per CSS pixel actually drawn (≤ the pane's).
    pub dpr: f32,
    /// The frame, device px.
    pub px_w: u32,
    pub px_h: u32,
}

impl FrameSize {
    /// The frame for a pane of `css_w`×`css_h` at `dpr` (a nonsensical dpr counts as 1).
    pub fn new(css_w: u32, css_h: u32, dpr: f32) -> Self {
        let (css_w, css_h) = (css_w.clamp(1, MAX_CSS_SIDE), css_h.clamp(1, MAX_CSS_SIDE));
        let asked = if dpr.is_finite() && dpr > 0.0 { dpr.clamp(0.25, 8.0) } else { 1.0 };
        let fits = MAX_DEVICE_SIDE as f32 / css_w.max(css_h) as f32;
        let dpr = asked.min(fits);
        let px = |css: u32| ((css as f32 * dpr).round() as u32).clamp(1, MAX_DEVICE_SIDE);
        FrameSize { css_w, css_h, dpr, px_w: px(css_w), px_h: px(css_h) }
    }

    pub fn px(&self) -> [u32; 2] {
        [self.px_w, self.px_h]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retina_and_oversized_panes() {
        let r1 = FrameSize::new(1280, 800, 2.0);
        assert_eq!((r1.px_w, r1.px_h, r1.dpr), (2560, 1600, 2.0));
        let wide = FrameSize::new(3000, 1000, 2.0);
        assert!(wide.px_w <= MAX_DEVICE_SIDE && wide.dpr < 2.0, "{wide:?}");
        // Wider than a frame may be even at dpr 1: the UI still spans the pane, drawn below 1.
        let huge = FrameSize::new(6000, 1000, 1.0);
        assert!(huge.css_w == 6000 && huge.px_w == MAX_DEVICE_SIDE && huge.dpr < 1.0, "{huge:?}");
        assert_eq!(FrameSize::new(0, 0, f32::NAN).px(), [1, 1]);
    }
}
