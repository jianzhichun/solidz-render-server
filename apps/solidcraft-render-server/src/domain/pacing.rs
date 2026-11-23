//! When to run the next UI pass and whether a frame may go out: render on demand, at most
//! [`MAX_FRAMES_PER_SECOND`] passes a second, and frames only against credits (design §1.6).

use std::time::{Duration, Instant};

use super::wire::{INITIAL_CREDITS, MAX_FRAMES_PER_SECOND};

/// Frames the host may still take before it acknowledges one.
#[derive(Debug)]
pub struct Credits {
    left: u32,
}

impl Default for Credits {
    fn default() -> Self {
        Credits { left: INITIAL_CREDITS }
    }
}

impl Credits {
    pub fn available(&self) -> bool {
        self.left > 0
    }

    /// Spend one on a frame sent.
    pub fn spend(&mut self) {
        self.left = self.left.saturating_sub(1);
    }

    /// One back for an `ack`; never more than the start (a confused host cannot flood itself).
    pub fn give_back(&mut self) {
        self.left = (self.left + 1).min(INITIAL_CREDITS);
    }
}

/// The pass schedule: what asked for one, and the earliest the next may run.
#[derive(Debug, Default)]
pub struct Schedule {
    last_pass: Option<Instant>,
    /// The UI asked to be drawn again at this time (its repaint delay).
    repaint_at: Option<Instant>,
    /// Something only a pass can answer is waiting: input, a resize, a policy, an answer.
    wanted: bool,
}

impl Schedule {
    pub fn min_interval() -> Duration {
        Duration::from_secs(1) / MAX_FRAMES_PER_SECOND
    }

    /// Input or a host message arrived: a pass is due (paced).
    pub fn want(&mut self) {
        self.wanted = true;
    }

    /// The UI asked to be drawn again after `delay` (`Duration::MAX`: not at all).
    pub fn repaint_after(&mut self, now: Instant, delay: Duration) {
        if let Some(at) = now.checked_add(delay) {
            self.repaint_at = Some(self.repaint_at.map_or(at, |old| old.min(at)));
        }
    }

    /// A pass ran now; `delay` is the repaint delay it ended with.
    pub fn ran(&mut self, now: Instant, delay: Duration) {
        self.last_pass = Some(now);
        self.wanted = false;
        self.repaint_at = None;
        self.repaint_after(now, delay);
    }

    /// When the next pass should run: `None` while nothing asks for one.
    pub fn next_pass(&self) -> Option<Instant> {
        let asked = if self.wanted { self.last_pass.or(Some(Instant::now())) } else { self.repaint_at };
        let asked = match (asked, self.repaint_at) {
            (Some(a), Some(r)) => Some(a.min(r)),
            (a, r) => a.or(r),
        }?;
        let earliest = self.last_pass.map_or(asked, |last| last + Self::min_interval());
        Some(asked.max(earliest))
    }

    /// Whether a pass is due at `now`.
    pub fn due(&self, now: Instant) -> bool {
        self.next_pass().is_some_and(|at| at <= now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credits_start_at_two_and_never_grow_past_it() {
        let mut c = Credits::default();
        c.spend();
        c.spend();
        assert!(!c.available());
        c.give_back();
        c.give_back();
        c.give_back();
        c.spend();
        c.spend();
        assert!(!c.available());
    }

    #[test]
    fn passes_run_on_demand_and_at_most_sixty_a_second() {
        let t0 = Instant::now();
        let mut s = Schedule::default();
        assert_eq!(s.next_pass(), None, "idle: no pass at all");
        s.ran(t0, Duration::MAX);
        assert_eq!(s.next_pass(), None, "a pass that asks for no repaint");
        s.want();
        assert_eq!(s.next_pass(), Some(t0 + Schedule::min_interval()), "input right after a pass waits for the pacing");
        s.ran(t0 + Duration::from_millis(20), Duration::from_millis(500));
        assert_eq!(s.next_pass(), Some(t0 + Duration::from_millis(520)), "the UI's own repaint delay");
        s.repaint_after(t0 + Duration::from_millis(30), Duration::ZERO);
        assert!(s.due(t0 + Duration::from_millis(37)) && !s.due(t0 + Duration::from_millis(36)));
    }
}
