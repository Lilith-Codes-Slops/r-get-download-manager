//! Explicit lifecycle independent of the upstream session's internal states.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TorrentState {
    FetchingMetadata,
    LookingForPeers,
    Connecting,
    Downloading,
    CheckingFiles,
    Finished,
    Completed,
    Failed(String),
}

impl TorrentState {
    /// Whether this lifecycle may advance. Completed means verified, published
    /// and stopped, not just that librqbit has received the last piece.
    pub fn can_transition_to(&self, next: &Self) -> bool {
        use TorrentState::*;
        if matches!(next, Failed(_)) {
            return !matches!(self, Completed | Failed(_));
        }
        matches!((self, next),
            (FetchingMetadata, LookingForPeers)
            | (LookingForPeers, Connecting)
            | (Connecting, Downloading)
            | (Downloading, CheckingFiles | Finished)
            | (CheckingFiles, Finished)
            | (Finished, Completed))
    }

    /// The wire-facing spelling for a queue entry; failures retain their cause.
    pub fn queue_state(&self) -> String {
        match self {
            Self::FetchingMetadata => "metadata".into(),
            Self::LookingForPeers => "peers".into(),
            Self::Connecting => "connecting".into(),
            Self::Downloading => "downloading".into(),
            Self::CheckingFiles => "checking files".into(),
            Self::Finished => "finished".into(),
            Self::Completed => "completed".into(),
            Self::Failed(reason) => format!("failed: {reason}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TorrentState::*;

    #[test]
    fn lifecycle_does_not_mark_unpublished_data_completed() {
        let sequence = [FetchingMetadata, LookingForPeers, Connecting, Downloading, Finished, Completed];
        for pair in sequence.windows(2) { assert!(pair[0].can_transition_to(&pair[1])); }
        assert!(!Downloading.can_transition_to(&Completed));
        assert!(!Finished.can_transition_to(&Downloading));
        assert!(!Completed.can_transition_to(&Failed("late".into())));
        assert!(Downloading.can_transition_to(&Failed("io".into())));
        assert_eq!(Failed("io".into()).queue_state(), "failed: io");
    }
}
