//! Hardcoded defaults for GUI settings and categories.
use crate::fake_backend::{Category, Settings, ThemeChoice};
use serde_json::Value;
use std::collections::BTreeMap;

pub fn default_settings() -> Settings {
    Settings {
        theme: ThemeChoice::System,
        compact: false,
        confirm_delete: true,
        alerts: true,
        max_concurrent: 5,
        seed_ratio: 2.0,
        speed_limit_kbps: None,
    }
}

pub fn default_categories() -> Vec<Category> {
    [
        (
            "images",
            "Images",
            "image",
            "jpg jpeg png gif webp bmp dng heic",
        ),
        ("videos", "Videos", "video", "mkv mp4 avi mov webm"),
        ("pdfs", "PDFs", "file", "pdf"),
        (
            "compressed",
            "Compressed",
            "archive",
            "zip tar gz 7z rar bz2 xz",
        ),
        ("documents", "Documents", "file", "docx doc txt xlsx pptx"),
        ("audio", "Audio", "music", "mp3 flac wav m4a ogg"),
        ("software", "Software", "disc", "iso exe msi dmg deb apk"),
    ]
    .into_iter()
    .map(|(id, name, icon, extensions)| Category {
        id: id.into(),
        name: name.into(),
        icon: icon.into(),
        extensions: extensions.split_whitespace().map(str::to_owned).collect(),
        builtin: true,
    })
    .collect()
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CategoryOverrides {
    updated: Vec<Category>,
    removed: Vec<String>,
}

/// Overlay stored values on the current defaults. A missing key always keeps
/// the hardcoded default, including when a new setting is added in the future.
pub fn apply_overrides(
    overrides: &BTreeMap<String, Value>,
) -> Result<(Settings, Vec<Category>), String> {
    let mut values = serde_json::to_value(default_settings()).map_err(|e| e.to_string())?;
    let object = values.as_object_mut().ok_or("invalid default settings")?;
    let mut categories = default_categories();
    for (key, value) in overrides {
        if key == "categories" {
            let changes: CategoryOverrides =
                serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
            categories.retain(|category| !changes.removed.contains(&category.id));
            for category in changes.updated {
                if let Some(existing) = categories
                    .iter_mut()
                    .find(|existing| existing.id == category.id)
                {
                    *existing = category;
                } else {
                    categories.push(category);
                }
            }
        } else if let Some(slot) = object.get_mut(key) {
            *slot = value.clone();
        } else {
            return Err(format!("unknown setting override: {key}"));
        }
    }
    let settings = serde_json::from_value(values).map_err(|e| e.to_string())?;
    Ok((settings, categories))
}

/// Produce only changed keys, removing overrides when a value returns to its default.
pub fn override_changes(
    previous: &Settings,
    previous_categories: &[Category],
    next: &Settings,
    next_categories: &[Category],
) -> Result<BTreeMap<String, Option<Value>>, String> {
    let previous = serde_json::to_value(previous).map_err(|e| e.to_string())?;
    let next = serde_json::to_value(next).map_err(|e| e.to_string())?;
    let defaults = serde_json::to_value(default_settings()).map_err(|e| e.to_string())?;
    let previous = previous.as_object().ok_or("invalid previous settings")?;
    let next = next.as_object().ok_or("invalid current settings")?;
    let defaults = defaults.as_object().ok_or("invalid default settings")?;
    let mut changes = BTreeMap::new();
    for (key, value) in next {
        if previous.get(key) != Some(value) {
            changes.insert(
                key.clone(),
                if defaults.get(key) == Some(value) {
                    None
                } else {
                    Some(value.clone())
                },
            );
        }
    }
    if previous_categories != next_categories {
        let defaults = default_categories();
        let category_changes = CategoryOverrides {
            updated: next_categories
                .iter()
                .filter(|category| {
                    defaults.iter().find(|item| item.id == category.id) != Some(*category)
                })
                .cloned()
                .collect(),
            removed: defaults
                .iter()
                .filter(|category| !next_categories.iter().any(|item| item.id == category.id))
                .map(|category| category.id.clone())
                .collect(),
        };
        changes.insert(
            "categories".into(),
            if category_changes.updated.is_empty() && category_changes.removed.is_empty() {
                None
            } else {
                Some(serde_json::to_value(category_changes).map_err(|e| e.to_string())?)
            },
        );
    }
    Ok(changes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_existing_gui() {
        let settings = default_settings();
        assert_eq!(settings.theme, ThemeChoice::System);
        assert!(!settings.compact);
        assert!(settings.confirm_delete);
        assert!(settings.alerts);
        assert_eq!(settings.max_concurrent, 5);
        assert_eq!(settings.seed_ratio, 2.0);
        assert_eq!(settings.speed_limit_kbps, None);
        let categories = default_categories();
        assert_eq!(categories.len(), 7);
        assert!(categories.iter().all(|category| category.builtin));
        assert_eq!(categories[0].id, "images");
        assert_eq!(
            categories[6].extensions,
            ["iso", "exe", "msi", "dmg", "deb", "apk"]
        );
    }

    #[test]
    fn override_diff_and_load_roundtrip() {
        let defaults = default_settings();
        let categories = default_categories();
        let mut changed = defaults.clone();
        changed.theme = ThemeChoice::Dark;
        changed.max_concurrent = 3;
        let changes = override_changes(&defaults, &categories, &changed, &categories).unwrap();
        assert_eq!(changes.len(), 2);
        let overrides = changes
            .into_iter()
            .map(|(key, value)| (key, value.unwrap()))
            .collect();
        assert_eq!(
            apply_overrides(&overrides).unwrap(),
            (changed.clone(), categories.clone())
        );
        let reset = override_changes(&changed, &categories, &defaults, &categories).unwrap();
        assert_eq!(reset.get("theme"), Some(&None));
        assert_eq!(reset.get("max_concurrent"), Some(&None));
    }

    #[test]
    fn categories_are_overlaid_and_removed_when_equal_to_defaults() {
        let settings = default_settings();
        let defaults = default_categories();
        let mut changed = defaults.clone();
        changed.pop();
        let changes = override_changes(&settings, &defaults, &settings, &changed).unwrap();
        let delta = changes.get("categories").unwrap().as_ref().unwrap();
        assert_eq!(delta["removed"], serde_json::json!(["software"]));
        assert_eq!(delta["updated"], serde_json::json!([]));
        let overrides = changes
            .into_iter()
            .map(|(key, value)| (key, value.unwrap()))
            .collect();
        assert_eq!(apply_overrides(&overrides).unwrap().1, changed);
        let reset = override_changes(&settings, &changed, &settings, &defaults).unwrap();
        assert_eq!(reset.get("categories"), Some(&None));
    }
}
