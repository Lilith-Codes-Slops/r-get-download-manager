use downloader_ipc::ClientRequest;

pub struct CommandDispatcher;

impl CommandDispatcher {
    pub async fn dispatch(&self, req: ClientRequest) -> Result<(), String> {
        match req {
            ClientRequest::AddTask { .. } => Ok(()),
            ClientRequest::Pause { .. } => Ok(()),
            ClientRequest::Resume { .. } => Ok(()),
            ClientRequest::RefreshUrl { .. } => Ok(()),
            ClientRequest::SetLimit { .. } => Ok(()),
            ClientRequest::RemoveTask { .. } => Ok(()),
            ClientRequest::ListQueue | ClientRequest::GetDetailedQueue => Ok(()),
            ClientRequest::GetSnapshot { .. } => Ok(()),
            ClientRequest::AddMirror { .. } => Ok(()),
            ClientRequest::SelectMediaQuality { .. } => Ok(()),
            ClientRequest::VerifyChecksum { .. } => Ok(()),
            ClientRequest::Shutdown => Ok(()),
        }
    }
}
