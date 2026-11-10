use super::protocol_contract::ProtocolHandler;
use std::sync::Arc;

pub struct ProtocolRegistry {
    handlers: Vec<Arc<dyn ProtocolHandler>>,
}

impl ProtocolRegistry {
    pub fn new() -> Self {
        Self {
            handlers: Vec::new(),
        }
    }

    pub fn register(&mut self, handler: Arc<dyn ProtocolHandler>) {
        self.handlers.push(handler);
    }

    pub async fn find(&self, url: &str) -> Option<Arc<dyn ProtocolHandler>> {
        for h in &self.handlers {
            if h.can_handle(url).await {
                return Some(Arc::clone(h));
            }
        }
        None
    }
}
