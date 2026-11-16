//! What changed between the last frame sent and this one: tiles of [`TILE`]² device pixels
//! compared byte for byte (egui reports no damage regions), merged into few rectangles. The new
//! frame is compared where it was read back ([`Pixels`], rows padded), and only what changed is
//! copied into the last frame sent.

/// Tile side, device pixels.
pub const TILE: u32 = 64;
/// More rectangles than this and one bounding rectangle is cheaper to send and to draw.
pub const MAX_RECTS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub fn area(&self) -> u64 {
        u64::from(self.w) * u64::from(self.h)
    }

    pub fn union(&self, o: &Rect) -> Rect {
        let (x, y) = (self.x.min(o.x), self.y.min(o.y));
        let (r, b) = ((self.x + self.w).max(o.x + o.w), (self.y + self.h).max(o.y + o.h));
        Rect { x, y, w: r - x, h: b - y }
    }
}

/// A frame's RGBA rows as read back: `w`×`h` pixels, rows `stride` bytes apart.
#[derive(Clone, Copy)]
pub struct Pixels<'a> {
    pub data: &'a [u8],
    pub stride: usize,
    pub w: u32,
    pub h: u32,
}

impl Pixels<'_> {
    /// Every row is there.
    pub fn is_whole(&self) -> bool {
        self.h == 0 || self.data.len() >= self.stride * (self.h as usize - 1) + self.w as usize * 4
    }

    /// Row `y`'s pixels (empty past the end).
    pub fn row(&self, y: u32) -> &[u8] {
        let start = y as usize * self.stride;
        self.data.get(start..start + self.w as usize * 4).unwrap_or_default()
    }

    /// As tight rows in `out`.
    pub fn copy_tight(&self, out: &mut Vec<u8>) {
        out.clear();
        out.reserve(self.w as usize * self.h as usize * 4);
        for y in 0..self.h {
            out.extend_from_slice(self.row(y));
        }
    }

    /// `rects` of these pixels into `dst`, a frame of the same size in tight rows (made that
    /// size, zeroed, if it is not).
    pub fn copy_rects(&self, dst: &mut Vec<u8>, rects: &[Rect]) {
        let tight = self.w as usize * 4;
        dst.resize(tight * self.h as usize, 0);
        for r in rects {
            let (a, b) = (r.x as usize * 4, (r.x + r.w) as usize * 4);
            for y in r.y..r.y + r.h {
                let start = y as usize * tight;
                if let (Some(to), Some(from)) = (dst.get_mut(start + a..start + b), self.row(y).get(a..b)) {
                    to.copy_from_slice(from);
                }
            }
        }
    }
}

/// The tile grid of a `w`×`h` frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grid {
    pub w: u32,
    pub h: u32,
}

impl Grid {
    pub fn tiles_x(&self) -> u32 {
        self.w.div_ceil(TILE)
    }

    pub fn tiles_y(&self) -> u32 {
        self.h.div_ceil(TILE)
    }

    /// Which tiles of tile row `ty` differ between `prev` (tight RGBA rows) and `next`. The
    /// caller may compare the rows in parallel.
    pub fn row_changes(&self, prev: &[u8], next: Pixels<'_>, ty: u32) -> Vec<bool> {
        let stride = self.w as usize * 4;
        let mut dirty = vec![false; self.tiles_x() as usize];
        let (y0, y1) = (ty * TILE, ((ty + 1) * TILE).min(self.h));
        for y in y0..y1 {
            let old = prev.get(y as usize * stride..(y as usize + 1) * stride).unwrap_or_default();
            let new = next.row(y);
            for (tx, d) in dirty.iter_mut().enumerate().filter(|(_, d)| !**d) {
                let a = tx * TILE as usize * 4;
                let b = ((tx + 1) * TILE as usize * 4).min(stride);
                *d = old.get(a..b) != new.get(a..b);
            }
        }
        dirty
    }

    /// The changed tiles (`rows[ty][tx]`) as rectangles: runs along each tile row, then runs of
    /// the same columns in consecutive rows merged; past [`MAX_RECTS`], their bounding rectangle.
    pub fn rects(&self, rows: &[Vec<bool>]) -> Vec<Rect> {
        let mut done: Vec<Rect> = Vec::new();
        // Rectangles still growing downwards, by their tile columns.
        let mut open: Vec<(u32, u32, Rect)> = Vec::new();
        for (ty, row) in rows.iter().enumerate() {
            let ty = ty as u32;
            let runs = runs(row);
            let mut next_open = Vec::with_capacity(runs.len());
            for (a, b) in runs {
                let tile = self.tile_rect(a, b, ty);
                match open.iter().position(|(oa, ob, _)| (*oa, *ob) == (a, b)) {
                    Some(i) => {
                        let (_, _, r) = open.swap_remove(i);
                        next_open.push((a, b, r.union(&tile)));
                    }
                    None => next_open.push((a, b, tile)),
                }
            }
            done.extend(open.drain(..).map(|(_, _, r)| r));
            open = next_open;
        }
        done.extend(open.into_iter().map(|(_, _, r)| r));
        if done.len() > MAX_RECTS {
            let all = done.iter().skip(1).fold(done[0], |acc, r| acc.union(r));
            return vec![all];
        }
        done
    }

    /// The whole frame.
    pub fn full(&self) -> Rect {
        Rect { x: 0, y: 0, w: self.w, h: self.h }
    }

    /// Tiles `a..b` of tile row `ty`, clipped to the frame.
    fn tile_rect(&self, a: u32, b: u32, ty: u32) -> Rect {
        let (x, y) = (a * TILE, ty * TILE);
        Rect { x, y, w: (b * TILE).min(self.w) - x, h: ((ty + 1) * TILE).min(self.h) - y }
    }
}

/// The runs of `true` in `row`, as tile column ranges `a..b`.
fn runs(row: &[bool]) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, &d) in row.iter().chain([false].iter()).enumerate() {
        match (d, start) {
            (true, None) => start = Some(i as u32),
            (false, Some(a)) => {
                out.push((a, i as u32));
                start = None;
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn changes(grid: Grid, prev: &[u8], next: &[u8]) -> Vec<Rect> {
        let next = Pixels { data: next, stride: grid.w as usize * 4, w: grid.w, h: grid.h };
        let rows: Vec<Vec<bool>> = (0..grid.tiles_y()).map(|ty| grid.row_changes(prev, next, ty)).collect();
        grid.rects(&rows)
    }

    fn poke(buf: &mut [u8], grid: Grid, x: u32, y: u32) {
        buf[(y * grid.w + x) as usize * 4] ^= 0xff;
    }

    #[test]
    fn nothing_changed_nothing_sent() {
        let grid = Grid { w: 100, h: 70 };
        let frame = vec![7u8; 100 * 70 * 4];
        assert!(changes(grid, &frame, &frame).is_empty());
    }

    #[test]
    fn a_pixel_marks_its_tile_clipped_to_the_frame() {
        let grid = Grid { w: 100, h: 70 };
        let prev = vec![0u8; 100 * 70 * 4];
        let mut next = prev.clone();
        poke(&mut next, grid, 99, 69);
        assert_eq!(changes(grid, &prev, &next), vec![Rect { x: 64, y: 64, w: 36, h: 6 }]);
    }

    #[test]
    fn neighbouring_tiles_merge_across_and_down() {
        let grid = Grid { w: 256, h: 256 };
        let prev = vec![0u8; 256 * 256 * 4];
        let mut next = prev.clone();
        for (x, y) in [(10, 10), (70, 10), (10, 70), (70, 70), (200, 200)] {
            poke(&mut next, grid, x, y);
        }
        let mut got = changes(grid, &prev, &next);
        got.sort_by_key(|r| (r.y, r.x));
        assert_eq!(got, vec![Rect { x: 0, y: 0, w: 128, h: 128 }, Rect { x: 192, y: 192, w: 64, h: 64 }]);
    }

    #[test]
    fn many_scattered_tiles_become_one_rectangle() {
        let grid = Grid { w: 64 * 20, h: 64 * 4 };
        let prev = vec![0u8; (grid.w * grid.h * 4) as usize];
        let mut next = prev.clone();
        // A checkerboard of tiles: 40 single tiles, none of which merge.
        for ty in 0..4 {
            for k in 0..10 {
                poke(&mut next, grid, (2 * k + ty % 2) * 64, ty * 64);
            }
        }
        assert_eq!(changes(grid, &prev, &next), vec![grid.full()]);
    }

    /// Read-back rows are padded: compared and copied by row, the padding never counts, and
    /// after copying what changed the last frame sent equals the new one.
    #[test]
    fn padded_rows_compare_and_copy_by_row() {
        let grid = Grid { w: 100, h: 70 };
        let stride = 512;
        let mut padded = vec![0xeeu8; stride * 70];
        for y in 0..70 {
            padded[y * stride..y * stride + 400].fill(0);
        }
        padded[69 * stride + 99 * 4] = 9;
        let next = Pixels { data: &padded, stride, w: 100, h: 70 };
        let mut sent = vec![0u8; 100 * 70 * 4];
        let rows: Vec<Vec<bool>> = (0..grid.tiles_y()).map(|ty| grid.row_changes(&sent, next, ty)).collect();
        let rects = grid.rects(&rows);
        assert_eq!(rects, vec![Rect { x: 64, y: 64, w: 36, h: 6 }]);
        next.copy_rects(&mut sent, &rects);
        let mut tight = Vec::new();
        next.copy_tight(&mut tight);
        assert_eq!(sent, tight);
        assert!(!Pixels { data: &padded[..stride * 69], stride, w: 100, h: 70 }.is_whole());
    }
}
