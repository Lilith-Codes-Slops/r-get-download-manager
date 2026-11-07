use std::fs::{File, OpenOptions};
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum LockError {
    #[error("lock failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("already running")]
    AlreadyRunning,
    #[error("single-instance locking is unsupported on this platform")]
    Unsupported,
}

pub struct SingleInstanceLock {
    _file: File,
}

impl SingleInstanceLock {
    pub fn acquire(path: &Path) -> Result<Self, LockError> {
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::AsRawFd;
            use std::os::unix::fs::OpenOptionsExt;
            let file = OpenOptions::new().read(true).write(true).create(true)
                .mode(0o600).custom_flags(libc::O_NOFOLLOW).open(path)?;
            #[allow(deprecated)]
            match nix::fcntl::flock(file.as_raw_fd(), nix::fcntl::FlockArg::LockExclusiveNonblock) {
                Ok(()) => Ok(Self { _file: file }),
                Err(nix::errno::Errno::EWOULDBLOCK) => Err(LockError::AlreadyRunning),
                Err(e) => Err(LockError::Io(std::io::Error::from_raw_os_error(e as i32))),
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = path;
            Err(LockError::Unsupported)
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::{LockError, SingleInstanceLock};

    #[test]
    fn prevents_second_instance_and_releases_on_drop() {
        let path = std::env::temp_dir().join(format!("rget-lock-test-{}", std::process::id()));
        let first = SingleInstanceLock::acquire(&path).unwrap();
        assert!(matches!(SingleInstanceLock::acquire(&path), Err(LockError::AlreadyRunning)));
        drop(first);
        let second = SingleInstanceLock::acquire(&path).unwrap();
        drop(second);
        std::fs::remove_file(path).unwrap();
    }
}
