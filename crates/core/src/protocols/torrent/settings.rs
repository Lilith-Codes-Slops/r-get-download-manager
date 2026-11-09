//! Persistable Phase 4 policy values. They do not enable upload or seeding.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TorrentSettings {
    pub upload_while_downloading: bool,
    pub download_time_upload_cap_bps: u64,
    pub seed_upload_cap_bps: u64,
    pub seed_by_default: bool,
    pub max_active_downloads: u32,
    pub max_active_checks: u32,
    pub max_active_seeds: u32,
}

impl Default for TorrentSettings {
    fn default() -> Self {
        Self { upload_while_downloading: true, download_time_upload_cap_bps: 2_000_000,
            seed_upload_cap_bps: 5_000_000, seed_by_default: false,
            max_active_downloads: 3, max_active_checks: 1, max_active_seeds: 1 }
    }
}

impl TorrentSettings {
    /// Reject configurations that the future engine integration cannot apply.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.download_time_upload_cap_bps == 0 || self.download_time_upload_cap_bps > u32::MAX as u64
            || self.seed_upload_cap_bps == 0 || self.seed_upload_cap_bps > u32::MAX as u64 {
            return Err("upload caps must be between 1 and 4294967295 bytes per second");
        }
        if !(1..=32).contains(&self.max_active_downloads) || !(1..=32).contains(&self.max_active_checks)
            || self.max_active_seeds > 32 {
            return Err("download/check limits must be 1–32 and the seed limit 0–32");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_and_partial_json_are_backward_compatible() {
        let settings: TorrentSettings = serde_json::from_str(r#"{"max_active_seeds":0}"#).unwrap();
        assert_eq!(settings.max_active_seeds, 0);
        assert_eq!(settings.max_active_downloads, 3);
        assert!(settings.validate().is_ok());
        assert_eq!(serde_json::from_str::<TorrentSettings>("{}").unwrap(), TorrentSettings::default());
    }
    #[test]
    fn invalid_limits_are_rejected() {
        let mut settings = TorrentSettings::default();
        settings.download_time_upload_cap_bps = 0; assert!(settings.validate().is_err());
        settings.download_time_upload_cap_bps = 1; settings.max_active_checks = 0;
        assert!(settings.validate().is_err());
        settings.max_active_checks = 1; settings.max_active_seeds = 33;
        assert!(settings.validate().is_err());
    }
}
