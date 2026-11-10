//! Global post-download action settings and daemon-side execution.
use super::persistence::Persistence;
use downloader_ipc::PostDownloadSettings;
use rusqlite::{params, Connection, OptionalExtension};
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

const TABLE: &str = "rget_post_download_settings_v1";

fn ensure_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS {TABLE} (id INTEGER PRIMARY KEY CHECK (id = 1), info TEXT NOT NULL);"
    ))?;
    Ok(())
}

fn load(path: &Path) -> Result<PostDownloadSettings> {
    let conn = super::persistence::open(path)?;
    ensure_schema(&conn)?;
    let json: Option<String> = conn
        .query_row(
            &format!("SELECT info FROM {TABLE} WHERE id = 1"),
            [],
            |row| row.get(0),
        )
        .optional()?;
    Ok(match json {
        Some(json) => serde_json::from_str(&json)?,
        None => PostDownloadSettings::default(),
    })
}

fn save(path: &Path, settings: &PostDownloadSettings) -> Result<()> {
    let conn = super::persistence::open(path)?;
    ensure_schema(&conn)?;
    conn.execute(
        &format!(
            "INSERT INTO {TABLE}(id, info) VALUES (1, ?1) ON CONFLICT(id) DO UPDATE SET info = excluded.info"
        ),
        params![serde_json::to_string(settings)?],
    )?;
    Ok(())
}

impl Persistence {
    pub(super) async fn load_post_download_settings(&self) -> Result<PostDownloadSettings> {
        let path = self.database_path();
        Ok(tokio::task::spawn_blocking(move || load(&path)).await??)
    }

    pub(super) async fn save_post_download_settings(
        &self,
        settings: PostDownloadSettings,
    ) -> Result<()> {
        let path = self.database_path();
        tokio::task::spawn_blocking(move || save(&path, &settings)).await??;
        Ok(())
    }
}

/// Shared daemon state; clones refer to the same setting.
#[derive(Clone, Default)]
pub(super) struct PostDownloadState(Arc<AtomicBool>);

impl PostDownloadState {
    pub(super) fn new(settings: PostDownloadSettings) -> Self {
        Self(Arc::new(AtomicBool::new(settings.open_containing_folder)))
    }

    pub(super) fn get(&self) -> PostDownloadSettings {
        PostDownloadSettings {
            open_containing_folder: self.0.load(Ordering::Relaxed),
        }
    }

    pub(super) fn set(&self, settings: PostDownloadSettings) {
        self.0
            .store(settings.open_containing_folder, Ordering::Relaxed);
    }
}

/// Open the folder that contains a completed download. For a single file this
/// is its parent; for a multi-file torrent root it is the published directory.
/// The path is canonicalized and confined to `$HOME/Downloads` before launch.
pub(super) fn open_containing_folder(final_path: &str) -> std::result::Result<(), String> {
    let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
    let target = resolve_open_target(final_path, Path::new(&home))?;
    if !graphical_session_available() {
        return Err("no graphical session environment is available".into());
    }
    Command::new("xdg-open")
        .arg(&target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not launch xdg-open: {e}"))?;
    Ok(())
}

fn resolve_open_target(final_path: &str, home: &Path) -> std::result::Result<PathBuf, String> {
    let downloads = home.join("Downloads");
    let downloads = downloads
        .canonicalize()
        .map_err(|e| format!("downloads directory unavailable: {e}"))?;
    let path = Path::new(final_path);
    if !path.is_absolute() {
        return Err("final path is not absolute".into());
    }
    let resolved = path
        .canonicalize()
        .map_err(|e| format!("final path unavailable: {e}"))?;
    if !resolved.starts_with(&downloads) {
        return Err("final path is outside Downloads".into());
    }
    let target = if resolved.is_dir() {
        resolved
    } else {
        resolved
            .parent()
            .ok_or("final path has no parent")?
            .to_path_buf()
    };
    if !target.is_dir() {
        return Err("containing folder is not a directory".into());
    }
    if !target.starts_with(&downloads) {
        return Err("containing folder is outside Downloads".into());
    }
    Ok(target)
}

fn graphical_session_available() -> bool {
    ["DISPLAY", "WAYLAND_DISPLAY", "DBUS_SESSION_BUS_ADDRESS"]
        .iter()
        .any(|key| std::env::var_os(*key).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn temp_home() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "rget-post-download-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, AtomicOrdering::Relaxed)
        ));
        std::fs::create_dir_all(path.join("Downloads")).unwrap();
        path
    }

    fn cleanup(path: &Path) {
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn settings_default_and_roundtrip() {
        let home = temp_home();
        let db = home.join("state.sqlite3");
        assert_eq!(load(&db).unwrap(), PostDownloadSettings::default());
        let settings = PostDownloadSettings {
            open_containing_folder: true,
        };
        save(&db, &settings).unwrap();
        assert_eq!(load(&db).unwrap(), settings);
        cleanup(&home);
    }

    #[test]
    fn invalid_row_is_reported() {
        let home = temp_home();
        let db = home.join("state.sqlite3");
        load(&db).unwrap();
        let conn = super::super::persistence::open(&db).unwrap();
        conn.execute(&format!("INSERT INTO {TABLE} VALUES (1, ?1)"), ["not json"])
            .unwrap();
        drop(conn);
        assert!(load(&db).is_err());
        cleanup(&home);
    }

    #[test]
    fn state_tracks_setting() {
        let state = PostDownloadState::new(PostDownloadSettings {
            open_containing_folder: true,
        });
        assert!(state.get().open_containing_folder);
        state.set(PostDownloadSettings::default());
        assert!(!state.get().open_containing_folder);
    }

    #[test]
    fn file_target_is_its_parent() {
        let home = temp_home();
        let downloads = home.join("Downloads");
        let file = downloads.join("file.bin");
        std::fs::write(&file, b"x").unwrap();
        let target = resolve_open_target(file.to_str().unwrap(), &home).unwrap();
        assert_eq!(target, downloads.canonicalize().unwrap());
        cleanup(&home);
    }

    #[test]
    fn directory_target_is_the_directory() {
        let home = temp_home();
        let dir = home.join("Downloads").join("bundle");
        std::fs::create_dir_all(&dir).unwrap();
        let target = resolve_open_target(dir.to_str().unwrap(), &home).unwrap();
        assert_eq!(target, dir.canonicalize().unwrap());
        cleanup(&home);
    }

    #[test]
    fn rejects_relative_missing_and_outside_paths() {
        let home = temp_home();
        let downloads = home.join("Downloads");
        let outside = home.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        assert!(resolve_open_target("Downloads/file", &home).is_err());
        assert!(resolve_open_target(downloads.join("missing").to_str().unwrap(), &home).is_err());
        assert!(resolve_open_target(outside.to_str().unwrap(), &home).is_err());
        cleanup(&home);
    }

    #[test]
    fn rejects_symlink_escape() {
        use std::os::unix::fs::symlink;
        let home = temp_home();
        let downloads = home.join("Downloads");
        let outside = home.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let link = downloads.join("escape");
        symlink(&outside, &link).unwrap();
        assert!(resolve_open_target(link.to_str().unwrap(), &home).is_err());
        cleanup(&home);
    }
}
