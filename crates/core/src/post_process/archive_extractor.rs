use std::path::Path;
use std::process::Command;

pub struct ArchiveExtractor;

impl ArchiveExtractor {
    pub fn extract(path: &Path, dest: &Path) -> Result<(), String> {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        match ext {
            "zip" | "tar" | "gz" | "zst" | "7z" => {
                let _ = Command::new("tar")
                    .arg("-xf")
                    .arg(path)
                    .arg("-C")
                    .arg(dest)
                    .status();
                Ok(())
            }
            _ => Ok(()),
        }
    }
}
