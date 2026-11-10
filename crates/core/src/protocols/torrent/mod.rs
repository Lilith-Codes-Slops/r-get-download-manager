//! Phase 1 torrent building blocks. This module is opt-in with `torrent`.
//!
//! The daemon does not yet dispatch torrent tasks. The upstream v1 engine
//! writes blocks to storage before checking each completed piece.

pub mod concurrency_queue;
pub mod diagnostics;
pub mod directory_publish;
pub mod file_priorities;
pub mod gated_storage;
pub mod metadata;
pub mod metrics;
pub mod multi_file_layout;
pub mod multi_file_metadata;
#[cfg(test)]
mod multi_file_metadata_tests;
pub mod multi_file_resolve;
#[cfg(test)]
mod multi_file_resolve_tests;
pub mod picker_capabilities;
#[cfg(target_os = "linux")]
pub mod published_cleanup;
#[cfg(all(test, target_os = "linux"))]
mod published_cleanup_tests;
pub mod recovery_metadata;
#[cfg(target_os = "linux")]
pub mod recovery_piece_verify;
#[cfg(all(test, target_os = "linux"))]
mod recovery_piece_verify_tests;
pub mod session;
pub mod settings;
pub mod source;
#[cfg(target_os = "linux")]
pub mod staging_cleanup;
#[cfg(all(test, target_os = "linux"))]
mod staging_cleanup_tests;
pub mod staging_manifest;
#[cfg(target_os = "linux")]
pub mod staging_tree;
#[cfg(all(test, target_os = "linux"))]
mod staging_tree_tests;
#[cfg(target_os = "linux")]
pub mod staging_verify;
#[cfg(all(test, target_os = "linux"))]
mod staging_verify_tests;
pub mod stall_detector;
pub mod state;
pub mod storage_gate;
pub mod task_identity;
pub mod transport_options;
pub mod webseed_support;
