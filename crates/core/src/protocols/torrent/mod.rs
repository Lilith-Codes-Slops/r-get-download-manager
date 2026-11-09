//! Phase 1 torrent building blocks. This module is opt-in with `torrent`.
//!
//! The daemon does not yet dispatch torrent tasks. The upstream v1 engine
//! writes blocks to storage before checking each completed piece.

pub mod metadata;
pub mod session;
pub mod source;
pub mod state;
