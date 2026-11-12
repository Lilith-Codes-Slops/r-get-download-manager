//! Daemon-owned storage for GUI setting overrides, separate from queue data.
use super::persistence::{self, Persistence};
use rusqlite::{params, Connection, TransactionBehavior};
use serde_json::Value;
use std::{collections::BTreeMap, path::Path};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
type Overrides = BTreeMap<String, Value>;

fn ensure_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS rget_gui_settings_v1 (
            key TEXT PRIMARY KEY NOT NULL,
            value TEXT NOT NULL
        );",
    )?;
    Ok(())
}

fn read(conn: &Connection) -> Result<Overrides> {
    let mut statement = conn.prepare("SELECT key, value FROM rget_gui_settings_v1 ORDER BY key")?;
    let mut rows = statement.query([])?;
    let mut overrides = Overrides::new();
    while let Some(row) = rows.next()? {
        let key: String = row.get(0)?;
        let value: String = row.get(1)?;
        overrides.insert(key, serde_json::from_str(&value)?);
    }
    Ok(overrides)
}

fn valid_value(key: &str, value: &Value) -> bool {
    match key {
        "theme" => matches!(value.as_str(), Some("system" | "dark" | "light")),
        "compact" | "confirm_delete" | "alerts" => value.is_boolean(),
        "max_concurrent" => value.as_u64().is_some_and(|v| (1..=10).contains(&v)),
        "seed_ratio" => value.as_f64().is_some_and(|v| v.is_finite() && v >= 0.1),
        "speed_limit_kbps" => value
            .as_f64()
            .is_some_and(|v| v.is_finite() && (10.0..=1_000_000.0).contains(&v)),
        "categories" => {
            value
                .get("removed")
                .and_then(Value::as_array)
                .is_some_and(|removed| removed.iter().all(Value::is_string))
                && value
                    .get("updated")
                    .and_then(Value::as_array)
                    .is_some_and(|categories| {
                        categories.iter().all(|category| {
                            category.get("id").and_then(Value::as_str).is_some()
                                && category.get("name").and_then(Value::as_str).is_some()
                                && category.get("icon").and_then(Value::as_str).is_some()
                                && category.get("builtin").and_then(Value::as_bool).is_some()
                                && category
                                    .get("extensions")
                                    .and_then(Value::as_array)
                                    .is_some_and(|extensions| {
                                        extensions.iter().all(Value::is_string)
                                    })
                        })
                    })
        }
        _ => false,
    }
}

fn known_key(key: &str) -> bool {
    matches!(
        key,
        "theme"
            | "compact"
            | "confirm_delete"
            | "alerts"
            | "max_concurrent"
            | "seed_ratio"
            | "speed_limit_kbps"
            | "categories"
    )
}

fn load(path: &Path) -> Result<Overrides> {
    let conn = persistence::open(path)?;
    ensure_schema(&conn)?;
    read(&conn)
}

fn set(path: &Path, key: &str, value: Option<Value>) -> Result<Overrides> {
    update(path, [(key.to_owned(), value)].into_iter().collect())
}

fn update(path: &Path, changes: BTreeMap<String, Option<Value>>) -> Result<Overrides> {
    let mut encoded = Vec::with_capacity(changes.len());
    for (key, value) in changes {
        if !known_key(&key)
            || value
                .as_ref()
                .is_some_and(|value| !valid_value(&key, value))
        {
            return Err("invalid GUI setting override".into());
        }
        let json = value.map(|v| serde_json::to_string(&v)).transpose()?;
        if json.as_ref().is_some_and(|v| v.len() > 1024 * 1024) {
            return Err("GUI setting override is too large".into());
        }
        encoded.push((key, json));
    }
    let mut conn = persistence::open(path)?;
    ensure_schema(&conn)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    for (key, json) in encoded {
        if let Some(json) = json {
            tx.execute(
                "INSERT INTO rget_gui_settings_v1 (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, json],
            )?;
        } else {
            tx.execute("DELETE FROM rget_gui_settings_v1 WHERE key = ?1", [key])?;
        }
    }
    let overrides = read(&tx)?;
    tx.commit()?;
    Ok(overrides)
}

fn reset(path: &Path) -> Result<Overrides> {
    let mut conn = persistence::open(path)?;
    ensure_schema(&conn)?;
    super::download_directory::ensure_schema(&conn)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute("DELETE FROM rget_gui_settings_v1", [])?;
    tx.execute(
        "UPDATE rget_download_directory_v1 SET override_path = NULL WHERE id = 1",
        [],
    )?;
    tx.commit()?;
    Ok(Overrides::new())
}

impl Persistence {
    pub(super) async fn load_gui_settings(&self) -> Result<Overrides> {
        let path = self.database_path();
        tokio::task::spawn_blocking(move || load(&path)).await?
    }

    pub(super) async fn set_gui_setting(
        &self,
        key: String,
        value: Option<Value>,
    ) -> Result<Overrides> {
        let path = self.database_path();
        tokio::task::spawn_blocking(move || set(&path, &key, value)).await?
    }

    pub(super) async fn update_gui_settings(
        &self,
        changes: BTreeMap<String, Option<Value>>,
    ) -> Result<Overrides> {
        let path = self.database_path();
        tokio::task::spawn_blocking(move || update(&path, changes)).await?
    }

    pub(super) async fn reset_gui_settings(&self) -> Result<Overrides> {
        let path = self.database_path();
        tokio::task::spawn_blocking(move || reset(&path)).await?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn test_path() -> std::path::PathBuf {
        std::env::temp_dir()
            .join(format!(
                "rget-gui-settings-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ))
            .join("queue.sqlite3")
    }

    #[test]
    fn overrides_round_trip_and_reset_preserves_queue() {
        let path = test_path();
        assert!(load(&path).unwrap().is_empty());
        let conn = persistence::open(&path).unwrap();
        conn.execute(
            "INSERT INTO rget_queue_v1 (url, info) VALUES ('test', 'history')",
            [],
        )
        .unwrap();
        super::super::download_directory::ensure_schema(&conn).unwrap();
        conn.execute("INSERT INTO rget_download_directory_v1 (id, default_path, override_path) VALUES (1, '/original', '/changed')", []).unwrap();
        drop(conn);
        super::super::task_directory::resolve(&path, "test", Path::new("/legacy")).unwrap();
        let conn = persistence::open(&path).unwrap();
        conn.execute(
            "INSERT INTO rget_task_directory_v1 (task_id, root) VALUES ('test', '/changed')",
            [],
        )
        .unwrap();
        drop(conn);
        set(&path, "theme", Some(json!("dark"))).unwrap();
        set(&path, "compact", Some(json!(true))).unwrap();
        let saved = load(&path).unwrap();
        assert_eq!(saved.get("theme"), Some(&json!("dark")));
        assert_eq!(saved.get("compact"), Some(&json!(true)));
        set(&path, "theme", None).unwrap();
        assert!(!load(&path).unwrap().contains_key("theme"));
        reset(&path).unwrap();
        assert!(load(&path).unwrap().is_empty());
        let conn = persistence::open(&path).unwrap();
        let history: String = conn
            .query_row(
                "SELECT info FROM rget_queue_v1 WHERE url = 'test'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(history, "history");
        let (default_path, override_path): (String, Option<String>) = conn
            .query_row(
                "SELECT default_path, override_path FROM rget_download_directory_v1 WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(default_path, "/original");
        assert_eq!(override_path, None);
        let task_root: String = conn
            .query_row(
                "SELECT root FROM rget_task_directory_v1 WHERE task_id='test'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(task_root, "/changed");
        drop(conn);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn batch_changes_are_atomic_and_leave_other_keys_untouched() {
        let path = test_path();
        set(&path, "theme", Some(json!("dark"))).unwrap();
        let bad = [
            ("compact".to_owned(), Some(json!(true))),
            ("max_concurrent".to_owned(), Some(json!(0))),
        ]
        .into_iter()
        .collect();
        assert!(update(&path, bad).is_err());
        assert_eq!(load(&path).unwrap().len(), 1);
        let good = [
            ("compact".to_owned(), Some(json!(true))),
            ("max_concurrent".to_owned(), Some(json!(3))),
        ]
        .into_iter()
        .collect();
        let saved = update(&path, good).unwrap();
        assert_eq!(saved.len(), 3);
        assert_eq!(saved.get("theme"), Some(&json!("dark")));
        let categories = json!({"updated": [{"id":"c1", "name":"Books", "icon":"file", "builtin":false, "extensions":["epub"]}], "removed": ["software"]});
        set(&path, "categories", Some(categories.clone())).unwrap();
        assert_eq!(load(&path).unwrap().get("categories"), Some(&categories));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn invalid_overrides_do_not_modify_the_database() {
        let path = test_path();
        set(&path, "theme", Some(json!("dark"))).unwrap();
        for (key, value) in [
            ("theme", json!("blue")),
            ("max_concurrent", json!(0)),
            ("speed_limit_kbps", json!(-1)),
            ("categories", json!([{"id": "broken"}])),
            ("unexpected", json!(true)),
        ] {
            assert!(set(&path, key, Some(value)).is_err());
        }
        assert_eq!(load(&path).unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
