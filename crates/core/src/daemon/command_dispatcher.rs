use downloader_ipc::ClientRequest;

pub struct CommandDispatcher;

impl CommandDispatcher {
    pub async fn dispatch(&self, req: ClientRequest) -> Result<(), String> {
        match req {
            ClientRequest::AddTask { .. } => Ok(()),
            ClientRequest::AddConfirmedTask { .. } => Err(
                "confirmed downloads are not available until publication confirmation is integrated"
                    .into(),
            ),
            ClientRequest::Pause { .. } => Ok(()),
            ClientRequest::Resume { .. } => Ok(()),
            ClientRequest::Retry { .. } | ClientRequest::RestartFromBeginning { .. } => {
                Err("safe retry and restart are not available yet".into())
            }
            ClientRequest::Cancel { .. } => Ok(()),
            ClientRequest::RefreshUrl { .. } => Ok(()),
            ClientRequest::SetLimit { .. } | ClientRequest::SetConnections { .. } => Ok(()),
            ClientRequest::RemoveTask { .. } => Ok(()),
            ClientRequest::ListQueue
            | ClientRequest::GetDetailedQueue
            | ClientRequest::SubscribeSegments
            | ClientRequest::SubscribeSegmentPolicies
            | ClientRequest::SubscribeProgress
            | ClientRequest::SubscribeTorrentEvents => Ok(()),
            ClientRequest::GetSnapshot { .. } => Ok(()),
            ClientRequest::AddMirror { .. } => Ok(()),
            ClientRequest::SelectMediaQuality { .. } => Ok(()),
            ClientRequest::VerifyChecksum { .. } => Ok(()),
            ClientRequest::GetGuiSettings
            | ClientRequest::SetGuiSetting { .. }
            | ClientRequest::UpdateGuiSettings { .. }
            | ClientRequest::ResetGuiSettings
            | ClientRequest::GetDownloadDirectory
            | ClientRequest::SetDownloadDirectory { .. }
            | ClientRequest::ValidateDownloadDirectory { .. }
            | ClientRequest::GetPostDownloadSettings
            | ClientRequest::SetPostDownloadSettings { .. } => Ok(()),
            ClientRequest::Shutdown => Ok(()),
        }
    }
}
