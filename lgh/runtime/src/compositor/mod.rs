//! Compositor process orchestration for sandbox worlds.
//!
//! Each world runs one compositor process. The X11 world uses Xvfb +
//! openbox (existing `xserver.rs` / `wm.rs`); the Wayland world uses a
//! single sway process (headless or nested). This module owns the sway
//! process lifecycle: spawn, socket discovery, teardown.

pub mod sway;
