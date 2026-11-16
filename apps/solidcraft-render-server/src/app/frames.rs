//! Frames out (PROTOCOL.md, "Frame rules"): the painted texture read back, compared with the
//! last frame sent, the changed rectangles encoded, against credits. While credits are 0 the
//! changed region keeps growing (it is relative to the last frame *sent*), so nothing is lost.
//! After a quiet spell, what went out as JPEG goes again losslessly. One frame is kept on the
//! CPU: the read-back is compared where it was mapped, and what changed is copied over the last
//! frame sent before it is encoded from there.

use std::time::{Duration, Instant};

use egui_wgpu::wgpu;
use rayon::prelude::*;

use crate::adapters::encode;
use crate::adapters::readback::Readback;
use crate::domain::codec::{self, LossyRegion, REFINE_AFTER_MS};
use crate::domain::damage::Grid;
use crate::domain::geometry::FrameSize;
use crate::domain::pacing::Credits;
use crate::domain::wire::{FrameRect, FrameTiming, ServerMsg};

/// A frame ready to go: its header and payload.
pub struct Outgoing {
    pub msg: ServerMsg,
    pub payload: Vec<u8>,
}

#[derive(Default)]
pub struct FrameStream {
    seq: u64,
    credits: Credits,
    readback: Readback,
    /// The true pixels of the last frame sent (not what JPEG made of them), tight RGBA rows.
    sent: Vec<u8>,
    sent_size: Option<[u32; 2]>,
    need_full: bool,
    /// Painted since the last frame went out, held for lack of a credit.
    unsent: bool,
    lossy: LossyRegion,
    /// When the region `lossy` holds was last added to or touched by input.
    lossy_since: Option<Instant>,
}

impl FrameStream {
    /// The next frame covers everything (a resize, the pane shown again, the first one).
    pub fn invalidate(&mut self) {
        self.need_full = true;
    }

    pub fn ack(&mut self) {
        self.credits.give_back();
    }

    /// A frame is waiting for a credit that has now come back.
    pub fn held_and_sendable(&self) -> bool {
        self.unsent && self.credits.available()
    }

    /// Input arrived: refining now would be overwritten at once.
    pub fn activity(&mut self, now: Instant) {
        if !self.lossy.is_empty() {
            self.lossy_since = Some(now);
        }
    }

    /// The frame for the texture just painted (`pass_ms`, `paint_ms`: that pass's), or `None`
    /// when nothing changed or no credit is left (then it is held).
    pub fn send_painted(
        &mut self,
        rs: &egui_wgpu::RenderState,
        texture: &wgpu::Texture,
        size: &FrameSize,
        input_seq: u64,
        pass_ms: f32,
        paint_ms: f32,
    ) -> Result<Option<Outgoing>, String> {
        if !self.credits.available() {
            self.unsent = true;
            return Ok(None);
        }
        // Taken now: a read that fails is not retried in a loop, the next paint tries again.
        self.unsent = false;
        let px = size.px();
        let grid = Grid { w: px[0], h: px[1] };
        let full = self.need_full || self.sent_size != Some(px) || self.sent.len() != grid.w as usize * grid.h as usize * 4;
        let sent = &mut self.sent;
        let t = Instant::now();
        let (rects, readback, diff) = self.readback.with_mapped(rs, texture, px, |pixels| {
            let readback = ms(t);
            let t = Instant::now();
            let rects = if full {
                vec![grid.full()]
            } else {
                let prev = &*sent;
                let rows: Vec<Vec<bool>> = (0..grid.tiles_y()).into_par_iter().map(|ty| grid.row_changes(prev, pixels, ty)).collect();
                grid.rects(&rows)
            };
            pixels.copy_rects(sent, &rects);
            (rects, readback, ms(t))
        })?;
        if rects.is_empty() {
            return Ok(None);
        }
        let t = Instant::now();
        let pieces = codec::plan(&rects, false);
        // `sent` already holds these pixels: a frame that fails to encode is sent whole next time.
        self.need_full = true;
        self.sent_size = Some(px);
        let (rects, payload) = encode::encode_pieces(&self.sent, px[0], &pieces)?;
        let encode = ms(t);
        self.need_full = false;
        if full {
            self.lossy.clear();
        }
        self.lossy.add(&pieces);
        self.lossy_since = (!self.lossy.is_empty()).then(Instant::now);
        let timing = FrameTiming { pass: pass_ms, paint: paint_ms, readback, diff, encode };
        Ok(Some(self.frame(size, input_seq, full, rects, timing, payload)))
    }

    /// When the lossless refinement is due, if one awaits and could go out now (no newer frame
    /// held, a credit left, no full frame pending): the loop never waits on one that cannot.
    pub fn refine_at(&self) -> Option<Instant> {
        if self.unsent || self.need_full || !self.credits.available() {
            return None;
        }
        self.lossy_since.map(|t| t + Duration::from_millis(REFINE_AFTER_MS))
    }

    /// The region sent as JPEG, again as PNG from the true pixels of the last frame sent; no
    /// GPU work.
    pub fn refine(&mut self, size: &FrameSize, input_seq: u64) -> Result<Option<Outgoing>, String> {
        if self.refine_at().is_none() {
            return Ok(None);
        }
        if self.lossy.is_empty() || self.sent_size != Some(size.px()) {
            // Nothing to refine, or a new size: its full frame replaces everything anyway.
            self.lossy_since = None;
            return Ok(None);
        }
        self.lossy_since = None;
        let t = Instant::now();
        let pieces = codec::plan(&self.lossy.take(), true);
        let (rects, payload) = encode::encode_pieces(&self.sent, size.px_w, &pieces)?;
        let timing = FrameTiming { encode: ms(t), ..FrameTiming::default() };
        Ok(Some(self.frame(size, input_seq, false, rects, timing, payload)))
    }

    fn frame(&mut self, size: &FrameSize, input_seq: u64, full: bool, rects: Vec<FrameRect>, timing: FrameTiming, payload: Vec<u8>) -> Outgoing {
        self.credits.spend();
        self.seq += 1;
        let msg = ServerMsg::Frame { seq: self.seq, input_seq, w: size.px_w, h: size.px_h, dpr: size.dpr, full, rects, timing };
        Outgoing { msg, payload }
    }
}

fn ms(since: Instant) -> f32 {
    since.elapsed().as_secs_f32() * 1000.0
}
