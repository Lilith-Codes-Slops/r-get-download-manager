//! Phase 1 torrent building blocks. This module is opt-in with `torrent`.
//!
//! The daemon does not yet dispatch torrent tasks. The upstream v1 engine
//! writes blocks to storage before checking each completed piece.

pub mod concurrency_queue;
pub mod diagnostics;
pub mod directory_publish;
pub mod file_priorities;
pub mod multi_file_layout;
pub mod transport_options;
pub mod webseed_support;
pub mod metadata;
pub mod metrics;
pub mod session;
pub mod settings;
pub mod source;
pub mod state;
pub mod stall_detector;
pub mod picker_capabilities;
pub mod task_identity;
