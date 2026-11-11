//! GUI-only preferences, persisted under `$XDG_CONFIG_HOME/rget/` or
//! `~/.config/rget/`. These settings never reach the daemon: they are local to
//! the egui frontend. `serde_json::Value` is used directly so the GUI crate
//! needs no new `serde` derive dependency and `Cargo.lock` stays untouched.

use super::{SettingsTab, Sort, Theme};
use serde_json::{json, Value};
use std::ffi::OsString;
use std::path::PathBuf;

const PREFS_VERSION: u64 = 1;
const PREFS_FILE: &str = "gui-preferences.json";

/// The subset of `AppState` that describes local appearance and behavior.
/// Unknown, missing or wrongly typed JSON fields fall back to these defaults,
/// so a config written by a newer or older build is never fatal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct GuiPreferences {
    pub(super) theme: Theme,
    pub(super) compact: bool,
    pub(super) confirm_delete: bool,
    pub(super) alerts: bool,
    pub(super) rail_pinned: bool,
    pub(super) sort: Sort,
    pub(super) descending: bool,
    pub(super) settings_tab: SettingsTab,
}

impl Default for GuiPreferences {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            compact: false,
            confirm_delete: true,
            alerts: true,
            rail_pinned: false,
            sort: Sort::Name,
            descending: false,
            settings_tab: SettingsTab::Appearance,
        }
    }
}

impl GuiPreferences {
    fn to_json(&self) -> Value {
        json!({
            "version": PREFS_VERSION,
            "theme": theme_key(self.theme),
            "compact": self.compact,
            "confirm_delete": self.confirm_delete,
            "alerts": self.alerts,
            "rail_pinned": self.rail_pinned,
            "sort": sort_key(self.sort),
            "descending": self.descending,
            "settings_tab": settings_tab_key(self.settings_tab),
        })
    }

    fn from_json(value: &Value) -> Self {
        let defaults = Self::default();
        if value.get("version").and_then(Value::as_u64) != Some(PREFS_VERSION) {
            return defaults;
        }
        Self {
            theme: value
                .get("theme")
                .and_then(Value::as_str)
                .and_then(theme_from_key)
                .unwrap_or(defaults.theme),
            compact: value
                .get("compact")
                .and_then(Value::as_bool)
                .unwrap_or(defaults.compact),
            confirm_delete: value
                .get("confirm_delete")
                .and_then(Value::as_bool)
                .unwrap_or(defaults.confirm_delete),
            alerts: value
                .get("alerts")
                .and_then(Value::as_bool)
                .unwrap_or(defaults.alerts),
            rail_pinned: value
                .get("rail_pinned")
                .and_then(Value::as_bool)
                .unwrap_or(defaults.rail_pinned),
            sort: value
                .get("sort")
                .and_then(Value::as_str)
                .and_then(sort_from_key)
                .unwrap_or(defaults.sort),
            descending: value
                .get("descending")
                .and_then(Value::as_bool)
                .unwrap_or(defaults.descending),
            settings_tab: value
                .get("settings_tab")
                .and_then(Value::as_str)
                .and_then(settings_tab_from_key)
                .unwrap_or(defaults.settings_tab),
        }
    }
}

fn theme_key(theme: Theme) -> &'static str {
    match theme {
        Theme::System => "system",
        Theme::Light => "light",
        Theme::Dark => "dark",
    }
}

fn theme_from_key(key: &str) -> Option<Theme> {
    match key {
        "system" => Some(Theme::System),
        "light" => Some(Theme::Light),
        "dark" => Some(Theme::Dark),
        _ => None,
    }
}

fn sort_key(sort: Sort) -> &'static str {
    match sort {
        Sort::Name => "name",
        Sort::Size => "size",
        Sort::Date => "date",
        Sort::Type => "type",
    }
}

fn sort_from_key(key: &str) -> Option<Sort> {
    match key {
        "name" => Some(Sort::Name),
        "size" => Some(Sort::Size),
        "date" => Some(Sort::Date),
        "type" => Some(Sort::Type),
        _ => None,
    }
}

fn settings_tab_key(tab: SettingsTab) -> &'static str {
    match tab {
        SettingsTab::Appearance => "appearance",
        SettingsTab::Behavior => "behavior",
        SettingsTab::Notifications => "notifications",
        SettingsTab::Downloads => "downloads",
        SettingsTab::Torrent => "torrent",
    }
}

fn settings_tab_from_key(key: &str) -> Option<SettingsTab> {
    match key {
        "appearance" => Some(SettingsTab::Appearance),
        "behavior" => Some(SettingsTab::Behavior),
        "notifications" => Some(SettingsTab::Notifications),
        "downloads" => Some(SettingsTab::Downloads),
        "torrent" => Some(SettingsTab::Torrent),
        _ => None,
    }
}

/// Pure path selection so it can be unit-tested without mutating process env.
fn config_dir_from(xdg: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let xdg = xdg.map(PathBuf::from).filter(|dir| dir.is_absolute());
    if let Some(dir) = xdg {
        return Some(dir.join("rget"));
    }
    home.map(|dir| PathBuf::from(dir).join(".config").join("rget"))
}

fn config_dir() -> Option<PathBuf> {
    let home_key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    config_dir_from(
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os(home_key),
    )
}

pub(super) fn path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join(PREFS_FILE))
}

/// A missing, unreadable or malformed config is not an error: the GUI starts
/// with defaults and rewrites the file the next time a preference changes.
pub(super) fn load() -> GuiPreferences {
    let Some(file) = path() else {
        return GuiPreferences::default();
    };
    let Ok(text) = std::fs::read_to_string(&file) else {
        return GuiPreferences::default();
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return GuiPreferences::default();
    };
    GuiPreferences::from_json(&value)
}

pub(super) fn save(prefs: &GuiPreferences) -> std::io::Result<()> {
    let Some(file) = path() else {
        return Ok(());
    };
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(&prefs.to_json()).map_err(std::io::Error::other)?;
    std::fs::write(file, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_through_json_preserves_every_preference() {
        let prefs = GuiPreferences {
            theme: Theme::Dark,
            compact: true,
            confirm_delete: false,
            alerts: false,
            rail_pinned: true,
            sort: Sort::Size,
            descending: true,
            settings_tab: SettingsTab::Notifications,
        };
        let text = serde_json::to_string(&prefs.to_json()).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(GuiPreferences::from_json(&value), prefs);
    }

    #[test]
    fn defaults_match_the_first_run_state() {
        let defaults = GuiPreferences::default();
        assert_eq!(defaults.theme, Theme::System);
        assert!(!defaults.compact);
        assert!(defaults.confirm_delete);
        assert!(defaults.alerts);
        assert!(!defaults.rail_pinned);
        assert_eq!(defaults.sort, Sort::Name);
        assert!(!defaults.descending);
        assert_eq!(defaults.settings_tab, SettingsTab::Appearance);
    }

    #[test]
    fn missing_unknown_and_wrongly_typed_fields_fall_back_to_defaults() {
        let defaults = GuiPreferences::default();
        assert_eq!(
            GuiPreferences::from_json(&json!({ "version": PREFS_VERSION })),
            defaults
        );
        assert_eq!(
            GuiPreferences::from_json(&json!({
                "version": PREFS_VERSION,
                "theme": "neon",
                "compact": "yes",
                "sort": 7,
                "settings_tab": null,
                "future_field": true,
            })),
            defaults
        );
    }

    #[test]
    fn partial_config_keeps_known_fields_and_defaults_the_rest() {
        let value = json!({ "version": PREFS_VERSION, "theme": "dark", "compact": true });
        let prefs = GuiPreferences::from_json(&value);
        assert_eq!(prefs.theme, Theme::Dark);
        assert!(prefs.compact);
        assert!(prefs.confirm_delete);
        assert_eq!(prefs.sort, Sort::Name);
    }

    #[test]
    fn a_newer_version_is_ignored_instead_of_misread() {
        assert_eq!(
            GuiPreferences::from_json(&json!({ "version": PREFS_VERSION + 1, "theme": "dark" })),
            GuiPreferences::default()
        );
    }

    #[test]
    fn xdg_config_home_wins_and_relative_values_are_ignored() {
        let absolute = if cfg!(windows) {
            OsString::from("C:\\Users\\xdg")
        } else {
            OsString::from("/tmp/xdg")
        };
        let home = OsString::from("home");
        assert_eq!(
            config_dir_from(Some(absolute.clone()), Some(home.clone())),
            Some(PathBuf::from(&absolute).join("rget"))
        );
        assert_eq!(
            config_dir_from(Some(OsString::from("relative/config")), Some(home.clone())),
            Some(PathBuf::from(&home).join(".config").join("rget"))
        );
        assert_eq!(
            config_dir_from(None, Some(home.clone())),
            Some(PathBuf::from(&home).join(".config").join("rget"))
        );
        assert_eq!(config_dir_from(None, None), None);
    }
}
