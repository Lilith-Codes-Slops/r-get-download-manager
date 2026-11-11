use std::collections::HashMap;

pub struct MirrorPoolManager {
    mirrors: HashMap<String, String>,
}

impl MirrorPoolManager {
    pub fn new() -> Self {
        Self {
            mirrors: HashMap::new(),
        }
    }

    pub fn add(&mut self, id: String, url: String) {
        self.mirrors.insert(id, url);
    }

    pub fn get(&self, id: &str) -> Option<&str> {
        self.mirrors.get(id).map(|s| s.as_str())
    }

    pub fn all(&self) -> Vec<(&str, &str)> {
        self.mirrors
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect()
    }
}

impl Default for MirrorPoolManager {
    fn default() -> Self {
        Self::new()
    }
}
