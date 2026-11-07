pub mod atomic_file_finalizer;
pub mod disk_space_guard;
pub mod dlpart_file_handle;
pub mod range_ownership_guard;
pub mod sidecar_meta_reader;
pub mod sidecar_meta_writer;
pub mod sparse_preallocator;

#[cfg(target_os = "linux")]
pub mod btrfs_nocow_linux;
#[cfg(target_os = "linux")]
pub mod pwrite_linux;

#[cfg(target_os = "windows")]
pub mod seek_write_windows;

pub use atomic_file_finalizer::AtomicFileFinalizer;
pub use disk_space_guard::DiskSpaceGuard;
pub use dlpart_file_handle::DlpartFileHandle;
pub use range_ownership_guard::RangeOwnershipGuard;
pub use sidecar_meta_reader::SidecarMetaReader;
pub use sidecar_meta_writer::SidecarMetaWriter;
pub use sparse_preallocator::SparsePreallocator;

#[cfg(target_os = "linux")]
pub use btrfs_nocow_linux::BtrfsNocow;
#[cfg(target_os = "linux")]
pub use pwrite_linux::PwriteLinux;

#[cfg(target_os = "windows")]
pub use seek_write_windows::SeekWriteWindows;
