//! What the render server shares with its tests: [`host`], the part of this process that makes
//! upstream's editing UI the host product's editor (no GPU, no I/O). The binary (`main.rs`) is
//! the rest.

pub mod host;
