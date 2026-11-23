//! How each changed rectangle travels (design §2.3): small ones raw (no encode at all), large
//! ones as JPEG stripes the page decodes concurrently; lossless PNG for snapshots and for the
//! idle refinement that replaces what JPEG blurred.

use super::damage::Rect;
use super::wire::Codec;

/// Rectangles up to this many pixels go raw: at most 256 KiB, cheaper than any encode.
pub const RAW_MAX_PIXELS: u64 = 65_536;
/// Stripe height for large rectangles: each stripe is its own image, encoded in parallel here
/// and decoded in parallel by the page.
pub const STRIPE_ROWS: u32 = 256;
/// JPEG quality while things move.
pub const JPEG_QUALITY: u8 = 80;
/// Quiet time after which the JPEG-covered region is sent again losslessly.
pub const REFINE_AFTER_MS: u64 = 200;

/// One image of a frame: where it goes and how it is encoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece {
    pub rect: Rect,
    pub codec: Codec,
}

/// The pieces for `rects`. `lossless`: large rectangles as PNG instead of JPEG (refinement).
pub fn plan(rects: &[Rect], lossless: bool) -> Vec<Piece> {
    let big = if lossless { Codec::Png } else { Codec::Jpeg };
    let mut out = Vec::new();
    for r in rects.iter().filter(|r| r.area() > 0) {
        if r.area() <= RAW_MAX_PIXELS {
            out.push(Piece { rect: *r, codec: Codec::Rgba });
            continue;
        }
        let mut y = r.y;
        while y < r.y + r.h {
            let h = STRIPE_ROWS.min(r.y + r.h - y);
            out.push(Piece { rect: Rect { x: r.x, y, w: r.w, h }, codec: big });
            y += h;
        }
    }
    out
}

/// The region sent lossily since the last refinement.
#[derive(Debug, Default)]
pub struct LossyRegion {
    rects: Vec<Rect>,
}

impl LossyRegion {
    pub fn add(&mut self, pieces: &[Piece]) {
        self.rects.extend(pieces.iter().filter(|p| p.codec == Codec::Jpeg).map(|p| p.rect));
        // Bounded: past a few dozen, their union is as good.
        if self.rects.len() > 64 {
            let all = self.rects.iter().skip(1).fold(self.rects[0], |a, r| a.union(r));
            self.rects = vec![all];
        }
    }

    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }

    pub fn take(&mut self) -> Vec<Rect> {
        std::mem::take(&mut self.rects)
    }

    pub fn clear(&mut self) {
        self.rects.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_rectangles_go_raw_and_large_ones_in_stripes() {
        let caret = Rect { x: 5, y: 5, w: 2, h: 20 };
        let view = Rect { x: 0, y: 10, w: 1000, h: 600 };
        let pieces = plan(&[caret, view], false);
        assert_eq!(pieces[0], Piece { rect: caret, codec: Codec::Rgba });
        let stripes = &pieces[1..];
        assert!(stripes.iter().all(|p| p.codec == Codec::Jpeg && p.rect.h <= STRIPE_ROWS && p.rect.w == 1000));
        assert_eq!(stripes.iter().map(|p| p.rect.h).sum::<u32>(), 600);
        assert_eq!((stripes[0].rect.y, stripes.last().map(|p| p.rect.y + p.rect.h)), (10, Some(610)));
        assert!(plan(&[view], true).iter().all(|p| p.codec == Codec::Png));
    }

    #[test]
    fn only_lossy_pieces_need_refining() {
        let mut lossy = LossyRegion::default();
        lossy.add(&plan(&[Rect { x: 0, y: 0, w: 10, h: 10 }], false));
        assert!(lossy.is_empty(), "raw pieces are exact");
        lossy.add(&plan(&[Rect { x: 0, y: 0, w: 400, h: 400 }], false));
        assert_eq!(lossy.take().len(), 2);
        assert!(lossy.is_empty());
    }
}
