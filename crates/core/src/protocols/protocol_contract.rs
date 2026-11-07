use async_trait::async_trait;
use downloader_ipc::TaskSnapshot;

#[async_trait]
pub trait ProtocolHandler: Send + Sync {
    async fn can_handle(&self, url: &str) -> bool;
    async fn start(&self, url: &str) -> Result<TaskSnapshot, String>;
}
