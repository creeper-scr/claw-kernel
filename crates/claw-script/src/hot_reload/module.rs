//! Versioned script module management for hot-reloading.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

use arc_swap::ArcSwap;

use crate::types::{EngineType, Script};

/// Metadata for a loaded script entry.
#[derive(Debug, Clone)]
pub struct ScriptEntry {
    /// Script name (usually derived from file stem).
    pub name: String,
    /// Engine type (Lua, JavaScript, etc.).
    pub engine: EngineType,
    /// Source code content.
    pub source: String,
    /// Source file path.
    pub path: PathBuf,
    /// Version number (increments on each reload).
    pub version: u64,
    /// When the script was loaded.
    pub loaded_at: SystemTime,
}

impl ScriptEntry {
    /// Create a new script entry.
    pub fn new(
        name: impl Into<String>,
        engine: EngineType,
        source: impl Into<String>,
        path: PathBuf,
    ) -> Self {
        Self {
            name: name.into(),
            engine,
            source: source.into(),
            path,
            version: 1,
            loaded_at: SystemTime::now(),
        }
    }

    /// Create a new version of this entry.
    pub fn with_source(&self, new_source: impl Into<String>) -> Self {
        Self {
            name: self.name.clone(),
            engine: self.engine,
            source: new_source.into(),
            path: self.path.clone(),
            version: self.version + 1,
            loaded_at: SystemTime::now(),
        }
    }

    /// Convert to a Script for execution.
    pub fn to_script(&self) -> Script {
        match self.engine {
            EngineType::Lua => Script::lua(&self.name, &self.source),
            #[cfg(feature = "engine-v8")]
            EngineType::JavaScript => Script::javascript(&self.name, &self.source),
            #[cfg(feature = "engine-v8")]
            EngineType::TypeScript => Script::typescript(&self.name, &self.source),
        }
    }

    /// Calculate a simple content hash for cache invalidation.
    pub fn content_hash(&self) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        self.source.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }
}

/// A versioned script module that supports atomic hot-swapping.
///
/// This provides lock-free concurrent access to scripts with
/// version history for rollback capabilities.
pub struct ScriptModule {
    /// Current script data (atomically swappable).
    current: ArcSwap<ScriptEntry>,
    /// Version history for rollback.
    history: ArcSwap<Vec<Arc<ScriptEntry>>>,
    /// Maximum number of versions to keep.
    max_history: usize,
    /// Global version counter.
    version_counter: AtomicU64,
}

impl std::fmt::Debug for ScriptModule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let current = self.current.load();
        f.debug_struct("ScriptModule")
            .field("name", &current.name)
            .field("version", &current.version)
            .field("engine", &current.engine)
            .finish()
    }
}

impl ScriptModule {
    /// Create a new script module with initial content.
    pub fn new(entry: ScriptEntry, max_history: usize) -> Self {
        let version = entry.version;
        Self {
            current: ArcSwap::new(Arc::new(entry)),
            history: ArcSwap::new(Arc::new(Vec::new())),
            max_history,
            version_counter: AtomicU64::new(version),
        }
    }

    /// Get the current script entry (lock-free).
    pub fn current(&self) -> Arc<ScriptEntry> {
        self.current.load_full()
    }

    /// Get the current version number.
    pub fn version(&self) -> u64 {
        self.current.load().version
    }

    /// Atomically swap to a new script entry.
    ///
    /// Returns the new version number. The old version is added to history.
    pub fn swap(&self, mut new_entry: ScriptEntry) -> u64 {
        let new_version = self.version_counter.fetch_add(1, Ordering::SeqCst) + 1;
        new_entry.version = new_version;

        let new_arc = Arc::new(new_entry);
        let old = self.current.swap(new_arc.clone());

        // Add to history
        let mut history: Vec<Arc<ScriptEntry>> = (*self.history.load_full()).clone();
        history.push(old);
        
        // Trim history if needed
        if history.len() > self.max_history {
            history.remove(0);
        }
        
        self.history.store(Arc::new(history));

        new_version
    }

    /// Rollback to a specific version.
    ///
    /// Returns true if successful, false if version not found in history.
    pub fn rollback_to(&self, target_version: u64) -> bool {
        let history = self.history.load();
        
        if let Some(entry) = history.iter().find(|e| e.version == target_version) {
            let new_entry = ScriptEntry {
                name: entry.name.clone(),
                engine: entry.engine,
                source: entry.source.clone(),
                path: entry.path.clone(),
                version: self.version_counter.fetch_add(1, Ordering::SeqCst) + 1,
                loaded_at: SystemTime::now(),
            };
            
            let new_arc = Arc::new(new_entry);
            let old = self.current.swap(new_arc);
            
            // Add current to history before rollback
            let mut new_history: Vec<Arc<ScriptEntry>> = history.to_vec();
            new_history.push(old);
            self.history.store(Arc::new(new_history));
            
            return true;
        }
        
        false
    }

    /// Rollback to the previous version.
    ///
    /// Returns true if successful, false if no previous version.
    pub fn rollback(&self) -> bool {
        let history = self.history.load();
        if let Some(entry) = history.last() {
            self.rollback_to(entry.version)
        } else {
            false
        }
    }

    /// Get version history.
    pub fn history_versions(&self) -> Vec<(u64, SystemTime)> {
        self.history
            .load()
            .iter()
            .map(|e| (e.version, e.loaded_at))
            .collect()
    }

    /// Get the number of versions in history.
    pub fn history_len(&self) -> usize {
        self.history.load().len()
    }
}

/// Thread-safe registry for managing multiple script modules.
pub struct ScriptRegistry {
    modules: ArcSwap<HashMap<String, Arc<ScriptModule>>>,
    max_history: usize,
}

impl std::fmt::Debug for ScriptRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let modules = self.modules.load();
        f.debug_struct("ScriptRegistry")
            .field("module_count", &modules.len())
            .field("max_history", &self.max_history)
            .finish()
    }
}

impl ScriptRegistry {
    /// Create a new empty registry.
    pub fn new(max_history: usize) -> Self {
        Self {
            modules: ArcSwap::new(Arc::new(HashMap::new())),
            max_history,
        }
    }

    /// Get or create a module for the given script name.
    pub fn get_or_create(&self, name: &str) -> Option<Arc<ScriptModule>> {
        self.modules.load().get(name).cloned()
    }

    /// Register a new script entry.
    ///
    /// If a script with the same name exists, it will be updated.
    pub fn register(&self, entry: ScriptEntry) -> Arc<ScriptModule> {
        let name = entry.name.clone();
        let module = Arc::new(ScriptModule::new(entry, self.max_history));

        let mut new_modules = (*self.modules.load_full()).clone();
        new_modules.insert(name, module.clone());
        self.modules.store(Arc::new(new_modules));

        module
    }

    /// Update an existing script with new source.
    ///
    /// Returns the new version number if the script exists.
    pub fn update(&self, name: &str, source: impl Into<String>) -> Option<u64> {
        self.modules
            .load()
            .get(name)
            .map(|module| {
                let current = module.current();
                let new_entry = current.with_source(source);
                module.swap(new_entry)
            })
    }

    /// Unregister a script.
    ///
    /// Returns true if the script was found and removed.
    pub fn unregister(&self, name: &str) -> bool {
        let mut new_modules = (*self.modules.load_full()).clone();
        let removed = new_modules.remove(name).is_some();
        if removed {
            self.modules.store(Arc::new(new_modules));
        }
        removed
    }

    /// Get a script entry by name.
    pub fn get(&self, name: &str) -> Option<Arc<ScriptEntry>> {
        self.modules
            .load()
            .get(name)
            .map(|module| module.current())
    }

    /// Get all script names.
    pub fn names(&self) -> Vec<String> {
        self.modules.load().keys().cloned().collect()
    }

    /// Check if a script exists.
    pub fn contains(&self, name: &str) -> bool {
        self.modules.load().contains_key(name)
    }

    /// Get the number of scripts.
    pub fn len(&self) -> usize {
        self.modules.load().len()
    }

    /// Check if the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.modules.load().is_empty()
    }

    /// Get all script entries.
    pub fn entries(&self) -> Vec<Arc<ScriptEntry>> {
        self.modules
            .load()
            .values()
            .map(|m| m.current())
            .collect()
    }

    /// Rollback a specific script to a previous version.
    pub fn rollback(&self, name: &str) -> bool {
        self.modules
            .load()
            .get(name)
            .map(|module| module.rollback())
            .unwrap_or(false)
    }

    /// Clear all scripts.
    pub fn clear(&self) {
        self.modules.store(Arc::new(HashMap::new()));
    }
}

impl Default for ScriptRegistry {
    fn default() -> Self {
        Self::new(5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_entry(name: &str, source: &str) -> ScriptEntry {
        ScriptEntry::new(
            name,
            EngineType::Lua,
            source,
            PathBuf::from(format!("{}.lua", name)),
        )
    }

    #[test]
    fn test_script_entry_creation() {
        let entry = test_entry("test", "return 1");
        assert_eq!(entry.name, "test");
        assert_eq!(entry.engine, EngineType::Lua);
        assert_eq!(entry.source, "return 1");
        assert_eq!(entry.version, 1);
    }

    #[test]
    fn test_script_entry_with_source() {
        let entry = test_entry("test", "return 1");
        let new_entry = entry.with_source("return 2");
        
        assert_eq!(new_entry.name, "test");
        assert_eq!(new_entry.version, 2);
        assert_eq!(new_entry.source, "return 2");
    }

    #[test]
    fn test_script_entry_to_script() {
        let entry = test_entry("test", "return 42");
        let script = entry.to_script();
        
        assert_eq!(script.name, "test");
        assert_eq!(script.source, "return 42");
        assert_eq!(script.engine, EngineType::Lua);
    }

    #[test]
    fn test_script_entry_content_hash() {
        let entry1 = test_entry("test", "return 1");
        let entry2 = test_entry("test", "return 1");
        let entry3 = test_entry("test", "return 2");
        
        assert_eq!(entry1.content_hash(), entry2.content_hash());
        assert_ne!(entry1.content_hash(), entry3.content_hash());
    }

    #[test]
    fn test_script_module_creation() {
        let entry = test_entry("test", "return 1");
        let module = ScriptModule::new(entry, 5);
        
        assert_eq!(module.version(), 1);
        assert_eq!(module.current().source, "return 1");
    }

    #[test]
    fn test_script_module_swap() {
        let entry = test_entry("test", "return 1");
        let module = ScriptModule::new(entry, 5);
        
        let new_version = module.swap(test_entry("test", "return 2"));
        assert_eq!(new_version, 2);
        assert_eq!(module.current().source, "return 2");
        assert_eq!(module.history_len(), 1);
    }

    #[test]
    fn test_script_module_rollback() {
        let entry = test_entry("test", "return 1");
        let module = ScriptModule::new(entry, 5);
        
        module.swap(test_entry("test", "return 2"));
        module.swap(test_entry("test", "return 3"));
        
        assert_eq!(module.version(), 3);
        
        // Rollback to previous
        assert!(module.rollback());
        // After rollback, version should be 4 (new version), content should be from v2
        assert_eq!(module.version(), 4);
    }

    #[test]
    fn test_script_registry() {
        let registry = ScriptRegistry::new(5);
        
        // Register
        let entry = test_entry("test", "return 1");
        registry.register(entry);
        
        assert!(registry.contains("test"));
        assert_eq!(registry.len(), 1);
        
        // Get
        let retrieved = registry.get("test").unwrap();
        assert_eq!(retrieved.source, "return 1");
        
        // Update
        let new_version = registry.update("test", "return 2").unwrap();
        assert_eq!(new_version, 2);
        
        let updated = registry.get("test").unwrap();
        assert_eq!(updated.source, "return 2");
        
        // Unregister
        assert!(registry.unregister("test"));
        assert!(!registry.contains("test"));
    }

    #[test]
    fn test_script_registry_rollback() {
        let registry = ScriptRegistry::new(5);
        
        registry.register(test_entry("test", "return 1"));
        registry.update("test", "return 2");
        
        assert!(registry.rollback("test"));
        
        let entry = registry.get("test").unwrap();
        // Version should be 3 after rollback, but content reverted
        assert_eq!(entry.version, 3);
    }

    #[test]
    fn test_script_registry_clear() {
        let registry = ScriptRegistry::new(5);
        
        registry.register(test_entry("test1", "return 1"));
        registry.register(test_entry("test2", "return 2"));
        
        assert_eq!(registry.len(), 2);
        
        registry.clear();
        
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn test_script_module_history_limit() {
        let entry = test_entry("test", "return 1");
        let module = ScriptModule::new(entry, 2); // Max 2 history entries
        
        module.swap(test_entry("test", "return 2"));
        module.swap(test_entry("test", "return 3"));
        module.swap(test_entry("test", "return 4"));
        
        // Should only have 2 history entries
        assert_eq!(module.history_len(), 2);
    }
}
