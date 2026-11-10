//! Application-facing hooks established from the locked librqbit source.
#[derive(Debug, PartialEq, Eq)]
pub struct PickerCapabilities {
    pub dynamic_queue_depth: Option<bool>,
    pub fast_extension_control: Option<bool>,
    pub piece_priority: Option<bool>,
    pub endgame_control: Option<bool>,
    pub first_last_boost: Option<bool>,
}

pub fn current() -> PickerCapabilities {
    let known = librqbit::version() == "9.1.0";
    PickerCapabilities {
        dynamic_queue_depth: known.then_some(false),
        fast_extension_control: None,
        piece_priority: known.then_some(false),
        endgame_control: known.then_some(false),
        first_last_boost: known.then_some(false),
    }
}

pub fn log() {
    tracing::info!(version = librqbit::version(), capabilities = ?current(),
        "torrent picker capabilities (unknown values are not claims of support); first/last preview boost is not enabled");
}

#[cfg(test)]
mod tests {
    #[test]
    fn no_unverified_fast_extension_control_claim() {
        let caps = super::current();
        assert_eq!(caps.fast_extension_control, None);
        if librqbit::version() == "9.1.0" {
            assert_eq!(caps.dynamic_queue_depth, Some(false));
            assert_eq!(caps.first_last_boost, Some(false));
        }
    }
}
