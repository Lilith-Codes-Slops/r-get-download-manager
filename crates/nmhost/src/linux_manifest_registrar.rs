#![cfg(target_os = "linux")]

use std::fs;
use std::path::PathBuf;

pub struct LinuxManifestRegistrar;

impl LinuxManifestRegistrar {
    pub fn register(host_path: &str) -> Result<(), std::io::Error> {
        let manifest = format!(
            r#"{{"name":"rget","description":"rget native host","path":"{}","type":"stdio","allowed_origins":["chrome-extension://*"]}}"#,
            host_path
        );
        let home = std::env::var("HOME").unwrap_or_default();
        let path = PathBuf::from(home).join(".config/google-chrome/NativeMessagingHosts/rget.json");
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, manifest)?;
        Ok(())
    }
}
