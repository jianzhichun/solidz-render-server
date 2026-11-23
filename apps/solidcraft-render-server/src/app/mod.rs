//! The two modes and what they share: the UI host (assembly, passes, paint), the frame stream,
//! the platform output, the host's pickers, read-only, and the loop's channel.

pub mod events;
pub mod frames;
pub mod output;
pub mod policy;
pub mod services;
pub mod snapshot;
pub mod stream;
pub mod ui_host;
