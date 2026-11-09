//! Persist normal priorities without implying that librqbit applies them.
use downloader_ipc::TorrentFile;

pub const NORMAL: u8 = 1;

/// `name` must already have passed metadata path validation.
pub fn single_file(name: &str, length: u64) -> Vec<TorrentFile> {
    vec![TorrentFile { path: name.to_owned(), length, priority: NORMAL }]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normal_priority_and_length_are_recorded() {
        let files = single_file("file.bin", 123);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "file.bin");
        assert_eq!(files[0].length, 123);
        assert_eq!(files[0].priority, 1);
    }
}
