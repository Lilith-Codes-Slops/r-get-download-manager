#![cfg(target_os = "linux")]

use std::fs::File;
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum BtrfsError {
    #[error("not a btrfs filesystem")]
    NotBtrfs,
    #[error("ioctl failed: {0}")]
    Ioctl(#[from] std::io::Error),
}

pub struct BtrfsNocow;

impl BtrfsNocow {
    pub fn is_btrfs(_path: &Path) -> bool {
        false
    }

    pub fn set_nocow(_file: &File) -> Result<(), BtrfsError> {
        Ok(())
    }
}