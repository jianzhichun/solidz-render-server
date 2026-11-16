//! Rules with no I/O and no GPU: the wire's messages and framing, input translation, frame
//! geometry, what changed, how it is encoded, and pacing. Unit-tested without a device.

pub mod codec;
pub mod damage;
pub mod envelope;
pub mod geometry;
pub mod input;
pub mod pacing;
pub mod wire;
