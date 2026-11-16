//! Rules with no I/O and no GPU: the wire's messages and framing, input translation, frame
//! geometry, what changed, how it is encoded, pacing, the scene's samples, and the device's memory
//! by label. Unit-tested without a device.

pub mod codec;
pub mod damage;
pub mod envelope;
pub mod geometry;
pub mod input;
pub mod memory;
pub mod pacing;
pub mod samples;
pub mod wire;
