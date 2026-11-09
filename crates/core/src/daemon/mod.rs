pub mod command_dispatcher;
pub mod event_broadcaster;
pub mod server_loop;
pub mod single_instance_lock;
pub mod tray_service;

pub use command_dispatcher::CommandDispatcher;
pub use event_broadcaster::EventBroadcaster;
pub use server_loop::ServerLoop;
pub use single_instance_lock::SingleInstanceLock;
pub use tray_service::TrayService;

mod transfer;
mod progress;
#[cfg(test)]
mod publication;
mod collision;
mod destination;
#[cfg(target_os = "linux")]
mod retry;
#[cfg(target_os = "linux")]
mod split;
#[cfg(target_os = "linux")]
mod segment_events;
#[cfg(target_os = "linux")]
mod progress_events;
mod bandwidth;
mod control;
mod task_removal;
mod range;
mod transfer_body;
#[cfg(test)]
mod range_tests;
mod persistence;
mod checksum;
mod checkpoint;
mod checkpoint_validation;
mod recovery;
mod recovered_transfer;
#[cfg(test)]
mod recovery_tests;

#[cfg(all(target_os = "linux", feature = "torrent"))]
mod torrent_transfer;

#[cfg(target_os = "linux")]
mod torrent_transfer_dispatch;

#[cfg(target_os = "linux")]
mod torrent_cleanup;

#[cfg(all(target_os = "linux", feature = "torrent"))]
mod torrent_events;

#[cfg(all(target_os = "linux", feature = "torrent"))]
mod torrent_queue;

#[cfg(all(target_os = "linux", feature = "torrent"))]
mod torrent_settings;

#[cfg(all(target_os = "linux", feature = "torrent"))]
mod torrent_stage_store;

#[cfg(all(target_os = "linux", feature = "torrent"))]
mod torrent_stage_recovery;
#[cfg(all(test, target_os = "linux", feature = "torrent"))]
mod torrent_stage_recovery_tests;

#[cfg(all(target_os = "linux", feature = "torrent"))]
mod torrent_publish_intent;
#[cfg(all(test, target_os = "linux", feature = "torrent"))]
mod torrent_publish_intent_tests;

#[cfg(all(target_os = "linux", feature = "torrent"))]
mod torrent_publish_commit;
#[cfg(all(test, target_os = "linux", feature = "torrent"))]
mod torrent_publish_commit_tests;

#[cfg(all(target_os = "linux", feature = "torrent"))]
mod torrent_manifest_store;
#[cfg(all(test, target_os = "linux", feature = "torrent"))]
mod torrent_manifest_store_tests;

#[cfg(all(target_os = "linux", feature = "torrent"))]
mod torrent_publish_inspection;
#[cfg(all(test, target_os = "linux", feature = "torrent"))]
mod torrent_publish_inspection_tests;

#[cfg(all(target_os = "linux", feature = "torrent"))]
mod torrent_publish_proof;
#[cfg(all(test, target_os = "linux", feature = "torrent"))]
mod torrent_publish_proof_tests;

#[cfg(all(target_os = "linux", feature = "torrent"))]
mod torrent_publish_recovery_evidence;

#[cfg(all(target_os = "linux", feature = "torrent"))]
mod torrent_lifecycle;

#[cfg(all(target_os = "linux", feature = "torrent"))]
mod torrent_stage_allocate;

#[cfg(all(target_os = "linux", feature = "torrent"))]
mod torrent_stage_publish;

#[cfg(all(target_os = "linux", feature = "torrent"))]
mod torrent_stage_complete;

#[cfg(all(test, target_os = "linux", feature = "torrent"))]
mod torrent_recovery_info_tests;

#[cfg(all(target_os = "linux", feature = "torrent"))]
mod torrent_recovery_content;

#[cfg(all(target_os = "linux", feature = "torrent"))]
mod torrent_publication_recovery;

#[cfg(all(test, target_os = "linux", target_env = "gnu", feature = "torrent"))]
mod torrent_publication_recovery_tests;

#[cfg(all(target_os = "linux", feature = "torrent"))]
mod torrent_published_store;

#[cfg(all(target_os = "linux", feature = "torrent"))]
mod torrent_published_remove;

#[cfg(all(target_os = "linux", feature = "torrent"))]
mod torrent_remove_completed;

#[cfg(all(target_os = "linux", feature = "torrent"))]
mod torrent_remove_recovery;

#[cfg(all(target_os = "linux", feature = "torrent"))]
mod torrent_remove_keep;

#[cfg(target_os = "linux")]
mod torrent_remove_dispatch;

#[cfg(all(target_os = "linux", feature = "torrent"))]
mod torrent_multi_transfer;

#[cfg(all(target_os = "linux", feature = "torrent"))]
mod torrent_remove_unfinished;

#[cfg(all(target_os = "linux", feature = "torrent"))]
mod torrent_stage_content;
