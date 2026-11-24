//! How many samples the 3D scene is drawn with in a streamed frame. The desktop app draws 4×
//! MSAA; here the multisampled colour and depth targets are most of the device's memory (at
//! 2560×1600, 4× is 188 MiB of the scene's 203), and the frame is read back and JPEG-encoded
//! while it moves. At a dpr of 2 or more a device pixel is small enough that edges drawn
//! without MSAA do not show stairs at 1:1, so the scene is drawn with one sample; below that,
//! with 2 where the device has it (stairs on model edges show at dpr 1 without any).

/// `--msaa`: what the host asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Msaa {
    /// By the dpr: 1 at dpr ≥ 2, else 2.
    Auto,
    /// This many, or the most the device has below it.
    Samples(u32),
}

impl Msaa {
    pub fn parse(s: &str) -> Option<Msaa> {
        match s {
            "auto" => Some(Msaa::Auto),
            "1" | "2" | "4" | "8" => s.parse().ok().map(Msaa::Samples),
            _ => None,
        }
    }

    /// The samples to draw the scene with at `dpr` on a device that can draw `supported`: the
    /// most it has at or below what is wanted, at least 1 (a software adapter often has only 1
    /// and 4, so `Auto` draws it with 1: faster, and its 4× targets would be process memory).
    pub fn samples(self, dpr: f32, supported: &[u32]) -> u32 {
        let wanted = match self {
            Msaa::Auto if dpr >= 2.0 => 1,
            Msaa::Auto => 2,
            Msaa::Samples(n) => n,
        };
        supported.iter().copied().filter(|n| (1..=wanted).contains(n)).max().unwrap_or(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn by_dpr_within_what_the_device_has() {
        let metal = [1, 2, 4];
        let lavapipe = [1, 4];
        assert_eq!(Msaa::Auto.samples(2.0, &metal), 1);
        assert_eq!(Msaa::Auto.samples(1.0, &metal), 2);
        assert_eq!(Msaa::Auto.samples(1.25, &lavapipe), 1);
        assert_eq!(Msaa::Samples(4).samples(2.0, &lavapipe), 4);
        assert_eq!(Msaa::Samples(2).samples(1.0, &[]), 1);
        assert_eq!((Msaa::parse("auto"), Msaa::parse("2"), Msaa::parse("3")), (Some(Msaa::Auto), Some(Msaa::Samples(2)), None));
    }
}
