//! Durable reservation and inode identity for confirmed HTTP partials.
use super::{
    control::{Tasks, TransferControl},
    persistence::{self, Persistence},
};
use rusqlite::{params, Connection};
use std::{
    collections::HashMap,
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};
use tokio::sync::watch;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone, Debug)]
pub(super) struct Owner {
    pub name: String,
    pub device: u64,
    pub inode: u64,
}

pub(super) fn schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS rget_http_partial_owner_v1 (
        task_id TEXT PRIMARY KEY NOT NULL,
        name TEXT NOT NULL UNIQUE,
        device TEXT,
        inode TEXT,
        CHECK ((device IS NULL AND inode IS NULL) OR (device IS NOT NULL AND inode IS NOT NULL))
    );",
    )?;
    Ok(())
}

fn reserve(database: &Path, task: &downloader_ipc::DownloadInfo, name: &str) -> Result<()> {
    let mut conn = persistence::open(database)?;
    schema(&conn)?;
    let tx = conn.transaction()?;
    // No replacement: a stale attempt cannot claim an existing reservation.
    tx.execute(
        "INSERT INTO rget_http_partial_owner_v1(task_id,name) VALUES(?1,?2)",
        params![task.url, name],
    )?;
    let changed = tx.execute(
        "UPDATE rget_queue_v1 SET info=?1 WHERE url=?2",
        params![serde_json::to_string(task)?, task.url],
    )?;
    if changed != 1 {
        return Err("confirmed task is not durable".into());
    }
    tx.commit()?;
    Ok(())
}

fn confirm(database: &Path, id: &str, name: &str, device: u64, inode: u64) -> Result<()> {
    let conn = persistence::open(database)?;
    schema(&conn)?;
    let changed = conn.execute("UPDATE rget_http_partial_owner_v1 SET device=?1,inode=?2 WHERE task_id=?3 AND name=?4 AND device IS NULL AND inode IS NULL", params![device.to_string(), inode.to_string(), id, name])?;
    if changed != 1 {
        return Err("partial reservation changed".into());
    }
    Ok(())
}

pub(super) fn load(conn: &Connection) -> Result<HashMap<String, Owner>> {
    schema(conn)?;
    let mut stmt = conn.prepare("SELECT task_id,name,device,inode FROM rget_http_partial_owner_v1 WHERE device IS NOT NULL AND inode IS NOT NULL")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    let mut owners = HashMap::new();
    for row in rows {
        let (id, name, device, inode) = row?;
        owners.insert(
            id,
            Owner {
                name,
                device: device.parse()?,
                inode: inode.parse()?,
            },
        );
    }
    Ok(owners)
}

pub(super) fn verify(
    owner: Option<&Owner>,
    name: &str,
    file: &std::fs::File,
) -> std::result::Result<(), String> {
    let owner = owner.ok_or("cannot resume automatically: partial ownership not established")?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if owner.name != name
        || !metadata.is_file()
        || metadata.dev() != owner.device
        || metadata.ino() != owner.inode
    {
        return Err("cannot resume automatically: partial ownership changed".into());
    }
    Ok(())
}

/// Reserve in SQLite before creating the file, then record its inode before
/// returning it to a worker. Ambiguous reserved files are never adopted.
pub(super) async fn create(
    dest: &Path,
    id: &str,
    tasks: &Tasks,
    store: &Persistence,
    control: &watch::Receiver<TransferControl>,
) -> Result<(PathBuf, tokio::fs::File)> {
    let mut random = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
    let name = format!(
        ".rget-{}.part",
        random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    let path = dest
        .parent()
        .ok_or("destination has no parent")?
        .join(&name);
    let gate = store.gate.clone().lock_owned().await;
    let mut guard = tasks.clone().lock_owned().await;
    if *control.borrow() == TransferControl::Canceled {
        return Err("download canceled".into());
    }
    let task = guard.get_mut(id).ok_or("task was removed")?;
    if task.confirmed_filename.is_none() || task.partial_name.is_some() {
        return Err("confirmed task cannot reserve another partial".into());
    }
    task.partial_name = Some(name.clone());
    let snapshot = task.clone();
    let database = store.database_path();
    let id = id.to_owned();
    let output = path.clone();
    let control = control.clone();
    // Move both guards into the blocking closure: aborting the async waiter
    // must not unlock a transaction that can still commit later.
    let opened = tokio::task::spawn_blocking(move || -> Result<std::fs::File> {
        let _gate = gate;
        if let Err(error) = reserve(&database, &snapshot, &name) {
            if let Some(task) = guard.get_mut(&id) {
                task.partial_name = None;
            }
            return Err(error);
        }
        if *control.borrow() == TransferControl::Canceled {
            return Err("download canceled".into());
        }
        let legacy =
            PathBuf::from(std::env::var_os("HOME").ok_or("HOME is not set")?).join("Downloads");
        let (bound_root, opened_root) =
            super::task_directory::open_verified(&database, &id, &legacy)?;
        if output.parent() != Some(bound_root.as_path()) {
            return Err("partial destination is outside the bound root".into());
        }
        let file = create_at(&opened_root, &name)?;
        let metadata = file.metadata()?;
        // Failure leaves an empty, durably reserved file. It is not owned for
        // resume or deletion until inode confirmation commits.
        confirm(&database, &id, &name, metadata.dev(), metadata.ino())?;
        // If the pathname moved while allocating, retain the associated file
        // but never hand it to a writer under the replacement path.
        super::task_directory::open_verified(&database, &id, &legacy)?;
        Ok(file)
    })
    .await??;
    Ok((path, tokio::fs::File::from_std(opened)))
}

/// Create exclusively under the already verified directory handle. A rename or
/// replacement of its path cannot redirect this operation into another root.
fn create_at(root: &std::fs::File, name: &str) -> Result<std::fs::File> {
    use nix::{
        fcntl::{openat, OFlag},
        sys::stat::Mode,
    };
    if !name.starts_with(".rget-")
        || !name.ends_with(".part")
        || name.contains('/')
        || name.contains('\\')
    {
        return Err("unsafe reserved partial name".into());
    }
    let fd = openat(
        root,
        name,
        OFlag::O_RDWR | OFlag::O_CREAT | OFlag::O_EXCL | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
        Mode::from_bits_truncate(0o600),
    )?;
    Ok(std::fs::File::from(fd))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_relative_create_never_targets_replacement_directory() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let lab =
            std::env::temp_dir().join(format!("rget-http-anchored-{}-{stamp}", std::process::id()));
        std::fs::create_dir(&lab).unwrap();
        let root = lab.join("root");
        let old = lab.join("original");
        std::fs::create_dir(&root).unwrap();
        let descriptor = std::fs::File::open(&root).unwrap();
        std::fs::rename(&root, &old).unwrap();
        std::fs::create_dir(&root).unwrap();
        let file = create_at(&descriptor, ".rget-bound.part").unwrap();
        assert!(old.join(".rget-bound.part").exists());
        assert!(!root.join(".rget-bound.part").exists());
        assert!(create_at(&descriptor, ".rget-bound.part").is_err());
        drop(file);
        std::fs::remove_dir_all(lab).unwrap();
    }

    #[test]
    fn reservation_survives_creation_and_only_confirmed_inode_is_owned() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("rget-http-owner-{}-{stamp}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let db = root.join("queue.sqlite3");
        let name = ".rget-test.part";
        let id = "https://example.test/file";
        let task: downloader_ipc::DownloadInfo = serde_json::from_str(&format!(r#"{{"url":"{id}","state":"downloading","filename":"file.bin","downloaded":0,"total":12,"speed_bps":0,"final_path":null,"confirmed_filename":"file.bin","partial_name":"{name}"}}"#)).unwrap();
        let conn = persistence::open(&db).unwrap();
        let mut before_reservation = task.clone();
        before_reservation.partial_name = None;
        conn.execute(
            "INSERT INTO rget_queue_v1(url,info) VALUES(?1,?2)",
            params![id, serde_json::to_string(&before_reservation).unwrap()],
        )
        .unwrap();
        drop(conn);
        reserve(&db, &task, name).unwrap();
        // Simulate termination after create but before inode confirmation.
        let path = root.join(name);
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        let conn = persistence::open(&db).unwrap();
        let stored: String = conn
            .query_row("SELECT info FROM rget_queue_v1 WHERE url=?1", [id], |row| {
                row.get(0)
            })
            .unwrap();
        let restored: downloader_ipc::DownloadInfo = serde_json::from_str(&stored).unwrap();
        assert_eq!(restored.partial_name.as_deref(), Some(name));
        let owners = load(&conn).unwrap();
        assert!(verify(owners.get(id), name, &file).is_err());
        drop(conn);
        let meta = file.metadata().unwrap();
        confirm(&db, id, name, meta.dev(), meta.ino()).unwrap();
        let conn = persistence::open(&db).unwrap();
        let owners = load(&conn).unwrap();
        assert!(verify(owners.get(id), name, &file).is_ok());
        std::fs::rename(&path, root.join("old.part")).unwrap();
        let replacement = std::fs::File::create(&path).unwrap();
        assert!(verify(owners.get(id), name, &replacement).is_err());
        drop(conn);
        drop(file);
        drop(replacement);
        std::fs::remove_dir_all(root).unwrap();
    }
}
