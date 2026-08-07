use std::collections::HashMap;
use std::sync::Mutex;

use domain::ContentHash;
use wasmtime::Module;

/// In-process compiled Module cache, keyed by content hash.
pub struct ModuleCache {
    inner: Mutex<HashMap<ContentHash, Module>>,
}

impl ModuleCache {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    pub fn get(&self, hash: &ContentHash) -> Option<Module> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(hash)
            .cloned()
    }

    pub fn insert(&self, hash: ContentHash, module: Module) {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(hash, module);
    }
}

impl Default for ModuleCache {
    fn default() -> Self {
        Self::new()
    }
}
