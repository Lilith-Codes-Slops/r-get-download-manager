//! Delegate librqbit's default filesystem storage through a quiescence gate.
//! Not yet used by downloads. This does not alter piece picking or assembly.
use super::storage_gate::StorageGate;
use librqbit::{
    storage::{
        filesystem::FilesystemStorageFactory, BoxStorageFactory, StorageFactory, StorageFactoryExt,
        TorrentStorage,
    },
    ManagedTorrentShared, TorrentMetadata,
};
use std::{any::TypeId, io::IoSlice, path::Path};

#[derive(Clone)]
pub struct GatedFilesystemFactory {
    pub gate: StorageGate,
}
impl StorageFactory for GatedFilesystemFactory {
    type Storage = GatedStorage;
    fn create(
        &self,
        shared: &ManagedTorrentShared,
        metadata: &TorrentMetadata,
    ) -> anyhow::Result<Self::Storage> {
        let _guard = self.gate.enter()?;
        let inner = FilesystemStorageFactory::default().create(shared, metadata)?;
        Ok(GatedStorage {
            inner: Box::new(inner),
            gate: self.gate.clone(),
        })
    }
    fn is_type_id(&self, id: TypeId) -> bool {
        FilesystemStorageFactory::default().is_type_id(id)
    }
    fn clone_box(&self) -> BoxStorageFactory {
        self.clone().boxed()
    }
}

pub struct GatedStorage {
    inner: Box<dyn TorrentStorage>,
    gate: StorageGate,
}
impl TorrentStorage for GatedStorage {
    fn init(
        &mut self,
        shared: &ManagedTorrentShared,
        metadata: &TorrentMetadata,
    ) -> anyhow::Result<()> {
        let _guard = self.gate.enter()?;
        self.inner.init(shared, metadata)
    }
    fn pread_exact(&self, file_id: usize, offset: u64, buf: &mut [u8]) -> anyhow::Result<()> {
        let _guard = self.gate.enter()?;
        self.inner.pread_exact(file_id, offset, buf)
    }
    fn pwrite_all(&self, file_id: usize, offset: u64, buf: &[u8]) -> anyhow::Result<()> {
        let _guard = self.gate.enter()?;
        self.inner.pwrite_all(file_id, offset, buf)
    }
    fn pwrite_all_vectored(
        &self,
        file_id: usize,
        offset: u64,
        bufs: [IoSlice<'_>; 2],
    ) -> anyhow::Result<usize> {
        let _guard = self.gate.enter()?;
        self.inner.pwrite_all_vectored(file_id, offset, bufs)
    }
    fn remove_file(&self, file_id: usize, filename: &Path) -> anyhow::Result<()> {
        let _guard = self.gate.enter()?;
        self.inner.remove_file(file_id, filename)
    }
    fn remove_directory_if_empty(&self, path: &Path) -> anyhow::Result<()> {
        let _guard = self.gate.enter()?;
        self.inner.remove_directory_if_empty(path)
    }
    fn ensure_file_length(&self, file_id: usize, length: u64) -> anyhow::Result<()> {
        let _guard = self.gate.enter()?;
        self.inner.ensure_file_length(file_id, length)
    }
    fn take(&self) -> anyhow::Result<Box<dyn TorrentStorage>> {
        let _guard = self.gate.enter()?;
        Ok(Box::new(Self {
            inner: self.inner.take()?,
            gate: self.gate.clone(),
        }))
    }
    // FilesystemStorage's on_piece_completed is the trait's no-op default.
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    struct Fake {
        calls: Arc<AtomicUsize>,
    }
    impl TorrentStorage for Fake {
        fn init(&mut self, _: &ManagedTorrentShared, _: &TorrentMetadata) -> anyhow::Result<()> {
            Ok(())
        }
        fn pread_exact(&self, _: usize, _: u64, _: &mut [u8]) -> anyhow::Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn pwrite_all(&self, _: usize, _: u64, _: &[u8]) -> anyhow::Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn remove_file(&self, _: usize, _: &Path) -> anyhow::Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn remove_directory_if_empty(&self, _: &Path) -> anyhow::Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn ensure_file_length(&self, _: usize, _: u64) -> anyhow::Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn take(&self) -> anyhow::Result<Box<dyn TorrentStorage>> {
            Ok(Box::new(Fake {
                calls: self.calls.clone(),
            }))
        }
    }
    #[test]
    fn transferred_storage_shares_the_closed_gate() {
        let gate = StorageGate::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let storage = GatedStorage {
            inner: Box::new(Fake {
                calls: calls.clone(),
            }),
            gate: gate.clone(),
        };
        storage.pwrite_all(0, 0, b"a").unwrap();
        let taken = storage.take().unwrap();
        gate.close_and_wait().unwrap();
        assert!(storage.pwrite_all(0, 0, b"a").is_err());
        assert!(taken.pwrite_all(0, 0, b"a").is_err());
        assert!(taken.ensure_file_length(0, 1).is_err());
        assert!(taken.remove_file(0, Path::new("a")).is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}

/// Test-only late write against the same gate used by the live session. The
/// delegated sentinel must not be invoked after the barrier closes.
#[cfg(feature = "torrent-test-hooks")]
pub(crate) fn probe_late_write(gate: StorageGate) -> bool {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    struct Probe(Arc<AtomicBool>);
    impl TorrentStorage for Probe {
        fn init(&mut self, _: &ManagedTorrentShared, _: &TorrentMetadata) -> anyhow::Result<()> {
            Ok(())
        }
        fn pread_exact(&self, _: usize, _: u64, _: &mut [u8]) -> anyhow::Result<()> {
            self.0.store(true, Ordering::SeqCst);
            Ok(())
        }
        fn pwrite_all(&self, _: usize, _: u64, _: &[u8]) -> anyhow::Result<()> {
            self.0.store(true, Ordering::SeqCst);
            Ok(())
        }
        fn remove_file(&self, _: usize, _: &Path) -> anyhow::Result<()> {
            self.0.store(true, Ordering::SeqCst);
            Ok(())
        }
        fn remove_directory_if_empty(&self, _: &Path) -> anyhow::Result<()> {
            self.0.store(true, Ordering::SeqCst);
            Ok(())
        }
        fn ensure_file_length(&self, _: usize, _: u64) -> anyhow::Result<()> {
            self.0.store(true, Ordering::SeqCst);
            Ok(())
        }
        fn take(&self) -> anyhow::Result<Box<dyn TorrentStorage>> {
            Ok(Box::new(Probe(self.0.clone())))
        }
    }
    let touched = Arc::new(AtomicBool::new(false));
    let storage = GatedStorage {
        inner: Box::new(Probe(touched.clone())),
        gate,
    };
    storage.pwrite_all(0, 0, b"late").is_err() && !touched.load(Ordering::SeqCst)
}
