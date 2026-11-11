//! Global post-download action settings shared by the GUI and daemon.
use serde::{Deserialize, Serialize};

/// A daemon-wide setting, not a per-task field. New actions can be added here
/// without changing the IPC framing version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PostDownloadSettings {
    /// Open the containing folder in the desktop file manager after completion.
    #[serde(default)]
    pub open_containing_folder: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ClientRequest;

    #[test]
    fn missing_field_defaults_to_disabled() {
        let settings: PostDownloadSettings = serde_json::from_str("{}").unwrap();
        assert!(!settings.open_containing_folder);
    }

    #[test]
    fn settings_roundtrip() {
        let settings = PostDownloadSettings {
            open_containing_folder: true,
        };
        let json = serde_json::to_string(&settings).unwrap();
        assert_eq!(
            serde_json::from_str::<PostDownloadSettings>(&json).unwrap(),
            settings
        );
    }

    #[test]
    fn set_request_roundtrip() {
        let request = ClientRequest::SetPostDownloadSettings {
            settings: PostDownloadSettings {
                open_containing_folder: true,
            },
        };
        let json = serde_json::to_string(&request).unwrap();
        assert_eq!(
            serde_json::from_str::<ClientRequest>(&json).unwrap(),
            request
        );
    }
}
