//! Daemon-owned default and user-selected download directory.
//!
//! Transfer routing and recovery use the root bound to each task.
use super::persistence::{self, Persistence};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::path::{Component, Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub(super) fn ensure_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS rget_download_directory_v1 (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            default_path TEXT NOT NULL,
            override_path TEXT
        );",
    )?;
    Ok(())
}

fn valid_path(path: &Path) -> bool {
    path.is_absolute()
        && !path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
        && path.to_str().is_some()
}

// XDG user-dirs files contain a shell-quoted absolute path or $HOME path.
// Parse the supported form without executing a shell or expanding arbitrary variables.
fn parse_xdg_download_dir(contents: &str, home: &Path) -> Option<PathBuf> {
    for line in contents.lines() {
        let line = line.trim();
        let Some(value) = line.strip_prefix("XDG_DOWNLOAD_DIR=") else {
            continue;
        };
        let value = value.trim();
        let value = value.strip_prefix('"')?;
        let mut parsed = String::new();
        let mut chars = value.chars().peekable();
        let mut closed = false;
        while let Some(ch) = chars.next() {
            match ch {
                '"' => {
                    closed = true;
                    break;
                }
                '\\' => {
                    let next = chars.next()?;
                    if matches!(next, '\\' | '"' | '$' | '`') {
                        parsed.push(next);
                    } else {
                        parsed.push('\\');
                        parsed.push(next);
                    }
                }
                '$' if parsed.is_empty() => {
                    for expected in ['H', 'O', 'M', 'E'] {
                        if chars.next()? != expected {
                            return None;
                        }
                    }
                    if !matches!(chars.peek(), Some('/') | Some('"')) {
                        return None;
                    }
                    parsed.push_str(home.to_str()?);
                }
                '$' | '`' => return None,
                other => parsed.push(other),
            }
        }
        if !closed {
            return None;
        }
        let remainder: String = chars.collect();
        let remainder = remainder.trim();
        if !remainder.is_empty() && !remainder.starts_with('#') {
            return None;
        }
        let path = PathBuf::from(parsed);
        return valid_path(&path).then_some(path);
    }
    None
}

fn detect_default() -> Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .filter(|s| !s.is_empty())
        .ok_or("HOME is not set")?;
    let home = PathBuf::from(home);
    if !valid_path(&home) {
        return Err("HOME is not an absolute UTF-8 path".into());
    }
    let config_dir = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|dir| Path::new(dir).is_absolute())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    let path = std::fs::read_to_string(config_dir.join("user-dirs.dirs"))
        .ok()
        .and_then(|contents| parse_xdg_download_dir(&contents, &home))
        .unwrap_or_else(|| home.join("Downloads"));
    Ok(path)
}

fn load(path: &Path) -> Result<PathBuf> {
    let mut conn = persistence::open(path)?;
    ensure_schema(&conn)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing: Option<String> = tx.query_row(
        "SELECT COALESCE(override_path, default_path) FROM rget_download_directory_v1 WHERE id = 1",
        [],
        |row| row.get(0),
    ).optional()?;
    let effective = if let Some(existing) = existing {
        PathBuf::from(existing)
    } else {
        let default = detect_default()?;
        let default_text = default.to_str().ok_or("download directory is not UTF-8")?;
        tx.execute(
            "INSERT INTO rget_download_directory_v1 (id, default_path) VALUES (1, ?1)",
            [default_text],
        )?;
        default
    };
    if !valid_path(&effective) {
        return Err("invalid stored download directory".into());
    }
    tx.commit()?;
    Ok(effective)
}

fn set(path: &Path, selected: &Path) -> Result<PathBuf> {
    if !valid_path(selected) {
        return Err(
            "download directory must be an absolute UTF-8 path without parent traversal".into(),
        );
    }
    let selected = std::fs::canonicalize(selected)?;
    if !selected.is_dir() || !valid_path(&selected) {
        return Err("download directory must be an existing UTF-8 directory".into());
    }
    let _ = load(path)?;
    let conn = persistence::open(path)?;
    ensure_schema(&conn)?;
    let selected_text = selected.to_str().ok_or("download directory is not UTF-8")?;
    conn.execute(
        "UPDATE rget_download_directory_v1 SET override_path = CASE WHEN default_path = ?1 THEN NULL ELSE ?1 END WHERE id = 1",
        params![selected_text],
    )?;
    Ok(selected)
}

impl Persistence {
    pub(super) async fn load_download_directory(&self) -> Result<PathBuf> {
        let path = self.database_path();
        tokio::task::spawn_blocking(move || load(&path)).await?
    }

    pub(super) async fn set_download_directory(&self, selected: PathBuf) -> Result<PathBuf> {
        let path = self.database_path();
        tokio::task::spawn_blocking(move || set(&path, &selected)).await?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn fixture() -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "rget-download-directory-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        (root.join("queue.sqlite3"), root)
    }

    #[test]
    fn parses_xdg_paths_without_shell_execution() {
        let home = Path::new("/home/test");
        assert_eq!(
            parse_xdg_download_dir("XDG_DOWNLOAD_DIR=\"$HOME/My Downloads\"\n", home),
            Some(home.join("My Downloads"))
        );
        assert_eq!(
            parse_xdg_download_dir("XDG_DOWNLOAD_DIR=\"/mnt/data/My \\\"Files\\\"\"", home),
            Some(PathBuf::from("/mnt/data/My \"Files\""))
        );
        for invalid in [
            "XDG_DOWNLOAD_DIR=\"$OTHER/Downloads\"",
            "XDG_DOWNLOAD_DIR=\"$(touch /tmp/x)\"",
            "XDG_DOWNLOAD_DIR=\"/tmp/../elsewhere\"",
            "XDG_DOWNLOAD_DIR=relative",
        ] {
            assert_eq!(parse_xdg_download_dir(invalid, home), None);
        }
    }

    #[test]
    fn selection_is_stored_separately_and_must_be_a_directory() {
        let (db, root) = fixture();
        let original = load(&db).unwrap();
        assert_eq!(load(&db).unwrap(), original);
        let chosen = root.join("chosen folder");
        std::fs::create_dir(&chosen).unwrap();
        let canonical = chosen.canonicalize().unwrap();
        assert_eq!(set(&db, &chosen).unwrap(), canonical);
        assert_eq!(load(&db).unwrap(), canonical);
        let file = root.join("file");
        std::fs::write(&file, b"kept").unwrap();
        assert!(set(&db, &file).is_err());
        assert!(set(&db, Path::new("relative")).is_err());
        assert_eq!(std::fs::read(&file).unwrap(), b"kept");
        assert_eq!(load(&db).unwrap(), canonical);
        let _ = std::fs::remove_dir_all(root);
    }
}
