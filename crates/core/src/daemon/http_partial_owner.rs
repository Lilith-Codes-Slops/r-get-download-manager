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

/// Reserve the task, create an unnamed inode, commit its identity, then link it.
/// No named file exists until its ownership is durable.
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
        // An unnamed inode disappears if the process crashes before linking.
        // Never fall back to named creation on filesystems without O_TMPFILE.
        let file = create_unnamed(&opened_root)?;
        file.sync_all()?;
        let metadata = file.metadata()?;
        confirm(&database, &id, &name, metadata.dev(), metadata.ino())?;
        // If the process crashes before this link, no named partial remains.
        // If it crashes after, the previously committed inode identifies it.
        link_unnamed(&opened_root, &file, &name)?;
        opened_root.sync_all()?;
        // If the pathname moved while allocating, retain the associated file
        // but never hand it to a writer under the replacement path.
        super::task_directory::open_verified(&database, &id, &legacy)?;
        Ok(file)
    })
    .await??;
    Ok((path, tokio::fs::File::from_std(opened)))
}

/// Create an inode without a name under the pinned directory.
fn create_unnamed(root: &std::fs::File) -> Result<std::fs::File> {
    use nix::{
        fcntl::{openat, OFlag},
        sys::stat::Mode,
    };
    let fd = openat(
        root,
        ".",
        OFlag::O_TMPFILE | OFlag::O_RDWR | OFlag::O_CLOEXEC,
        Mode::from_bits_truncate(0o600),
    )?;
    Ok(std::fs::File::from(fd))
}

/// Link only the inode held open by `file`; never adopt an existing name.
fn link_unnamed(root: &std::fs::File, file: &std::fs::File, name: &str) -> Result<()> {
    use nix::{
        fcntl::{AtFlags, AT_FDCWD},
        unistd::linkat,
    };
    use std::os::fd::AsRawFd;
    if !name.starts_with(".rget-")
        || !name.ends_with(".part")
        || name.contains('/')
        || name.contains('\\')
    {
        return Err("unsafe reserved partial name".into());
    }
    // This procfs link identifies the open inode, not the replaceable root.
    let source = format!("/proc/self/fd/{}", file.as_raw_fd());
    linkat(AT_FDCWD, source.as_str(), root, name, AtFlags::AT_SYMLINK_FOLLOW)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Invoked by the parent test in a separate test process. process::exit
    // bypasses Rust destructors and closes process descriptors on exit.
    #[test]
    fn reservation_crash_child() {
        let Ok(root) = std::env::var("RGET_HTTP_CRASH_ROOT") else { return };
        let stage = std::env::var("RGET_HTTP_CRASH_STAGE").unwrap();
        let root = PathBuf::from(root);
        let db = root.join("queue.sqlite3");
        let id = "https://example.test/crash";
        let name = ".rget-crash.part";
        let task: downloader_ipc::DownloadInfo = serde_json::from_str(&format!(r#"{{"url":"{id}","state":"downloading","filename":"file.bin","downloaded":0,"total":12,"speed_bps":0,"final_path":null,"confirmed_filename":"file.bin","partial_name":"{name}"}}"#)).unwrap();
        reserve(&db, &task, name).unwrap();
        let dir = std::fs::File::open(&root).unwrap();
        let file = create_unnamed(&dir).unwrap();
        file.sync_all().unwrap();
        if stage != "before-confirm" {
            let meta = file.metadata().unwrap();
            confirm(&db, id, name, meta.dev(), meta.ino()).unwrap();
        }
        if stage == "after-link" {
            link_unnamed(&dir, &file, name).unwrap();
            dir.sync_all().unwrap();
        }
        std::process::exit(0);
    }

    #[test]
    fn reservation_crash_never_leaves_unassociated_named_file() {
        for stage in ["before-confirm", "after-confirm", "after-link"] {
            let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
            let root = std::env::temp_dir().join(format!("rget-http-crash-{}-{stamp}-{stage}", std::process::id()));
            std::fs::create_dir(&root).unwrap();
            let db = root.join("queue.sqlite3");
            let conn = persistence::open(&db).unwrap();
            conn.execute("INSERT INTO rget_queue_v1(url,info) VALUES(?1,?2)",
                ["https://example.test/crash", "{}"] ).unwrap();
            drop(conn);
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "daemon::http_partial_owner::tests::reservation_crash_child"])
                .env("RGET_HTTP_CRASH_ROOT", &root)
                .env("RGET_HTTP_CRASH_STAGE", stage)
                .status().unwrap();
            assert!(status.success(), "crash child failed at {stage}");
            let path = root.join(".rget-crash.part");
            let conn = persistence::open(&db).unwrap();
            let stored: String = conn.query_row(
                "SELECT info FROM rget_queue_v1 WHERE url=?1",
                ["https://example.test/crash"], |row| row.get(0),
            ).unwrap();
            let task: downloader_ipc::DownloadInfo = serde_json::from_str(&stored).unwrap();
            assert_eq!(task.partial_name.as_deref(), Some(".rget-crash.part"));
            let owners = load(&conn).unwrap();
            if stage == "after-link" {
                let file = std::fs::File::open(&path).unwrap();
                assert!(verify(owners.get("https://example.test/crash"), ".rget-crash.part", &file).is_ok());
            } else {
                assert!(!path.exists(), "unexpected named partial after {stage}");
            }
            drop(conn);
            std::fs::remove_dir_all(root).unwrap();
        }
    }

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
        let file = create_unnamed(&descriptor).unwrap();
        link_unnamed(&descriptor, &file, ".rget-bound.part").unwrap();
        assert!(old.join(".rget-bound.part").exists());
        assert!(!root.join(".rget-bound.part").exists());
        assert!(link_unnamed(&descriptor, &file, ".rget-bound.part").is_err());
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
        // Simulate termination after unnamed creation but before confirmation.
        let path = root.join(name);
        let descriptor = std::fs::File::open(&root).unwrap();
        let file = create_unnamed(&descriptor).unwrap();
        assert!(!path.exists());
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
        // Dropping the only descriptor simulates process termination: no
        // directory entry exists which could be mistaken for a partial.
        drop(file);
        assert!(!path.exists());
        let file = create_unnamed(&descriptor).unwrap();
        let meta = file.metadata().unwrap();
        confirm(&db, id, name, meta.dev(), meta.ino()).unwrap();
        // A crash after confirmation but before linking is also unambiguous.
        assert!(!path.exists());
        link_unnamed(&descriptor, &file, name).unwrap();
        descriptor.sync_all().unwrap();
        let conn = persistence::open(&db).unwrap();
        let owners = load(&conn).unwrap();
        let reopened = std::fs::File::open(&path).unwrap();
        assert!(verify(owners.get(id), name, &reopened).is_ok());
        drop(reopened);
        std::fs::rename(&path, root.join("old.part")).unwrap();
        let replacement = std::fs::File::create(&path).unwrap();
        assert!(verify(owners.get(id), name, &replacement).is_err());
        drop(conn);
        drop(file);
        drop(replacement);
        std::fs::remove_dir_all(root).unwrap();
    }
}
