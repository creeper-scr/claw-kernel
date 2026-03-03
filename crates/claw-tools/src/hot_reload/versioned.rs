//! Versioned module with atomic hot-swap capabilities.
//!
//! Provides lock-free (or low-lock) atomic swapping of module versions,
//! enabling safe hot-reloading with zero-downtime updates and rollback support.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    VersionedModule<T>                       │
//! ├─────────────────────────────────────────────────────────────┤
//! │  current: ArcSwap<T>        ──► Lock-free atomic pointer   │
//! │  versions: RwLock<Vec<_>>    ──► Version history (lock)    │
//! │  version_counter: AtomicU64  ──► Monotonic version IDs     │
//! └─────────────────────────────────────────────────────────────┘
//!                          │
//!           ┌──────────────┼──────────────┐
//!           ▼              ▼              ▼
//!     ┌──────────┐   ┌──────────┐   ┌──────────┐
//!     │  Read    │   │  Update  │   │ Rollback │
//!     │ load()   │   │ swap()   │   │  to vN   │
//!     │ (lock-free)│  │ (atomic) │   │          │
//!     └──────────┘   └──────────┘   └──────────┘
//! ```
//!
//! # Example
//!
//! ```rust
//! use std::sync::Arc;
//! use claw_tools::hot_reload::VersionedModule;
//!
//! # fn example() {
//! let module: VersionedModule<String> = VersionedModule::new(Arc::new("v1".to_string()));
//!
//! // Lock-free read
//! let current = module.load();
//! assert_eq!(current.as_ref(), "v1");
//!
//! // Atomic swap to new version
//! let new_version = module.swap(Arc::new("v2".to_string()));
//! assert_eq!(new_version, 2);
//!
//! // Old Arc is still valid until dropped
//! assert_eq!(current.as_ref(), "v1"); // Still valid!
//! # }
//! ```

use arc_swap::ArcSwap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Instant;

/// A versioned snapshot of a module.
#[derive(Debug)]
pub struct ModuleVersion<T> {
    /// Monotonic version number (starts at 1).
    pub version: u64,
    /// The module data wrapped in Arc.
    pub module: Arc<T>,
    /// When this version was loaded.
    pub loaded_at: Instant,
}

impl<T: Clone> Clone for ModuleVersion<T> {
    fn clone(&self) -> Self {
        Self {
            version: self.version,
            module: Arc::new((*self.module).clone()),
            loaded_at: self.loaded_at,
        }
    }
}

impl<T> ModuleVersion<T> {
    /// Create a new module version.
    fn new(version: u64, module: Arc<T>) -> Self {
        Self {
            version,
            module,
            loaded_at: Instant::now(),
        }
    }
}

/// Versioned module supporting atomic hot-swaps and rollback.
///
/// Uses `arc-swap` for lock-free reads and atomic updates, while maintaining
/// a version history for rollback capabilities.
///
/// # Thread Safety
///
/// - `load()` is lock-free and highly concurrent
/// - `swap()` is wait-free for readers (old version stays alive via Arc)
/// - `versions()` requires a read lock on history
/// - `rollback()` requires a write lock on history
///
/// # Type Parameters
///
/// - `T`: The module type. Must be Send + Sync for thread safety.
pub struct VersionedModule<T> {
    /// Current active version - uses arc-swap for lock-free atomic updates.
    current: ArcSwap<T>,
    /// Version history for rollback (protected by RwLock).
    versions: RwLock<Vec<ModuleVersion<T>>>,
    /// Monotonic version counter.
    version_counter: AtomicU64,
    /// Maximum number of versions to keep in history.
    max_history: AtomicUsize,
}

impl<T: Send + Sync + std::fmt::Debug> std::fmt::Debug for VersionedModule<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VersionedModule")
            .field("current_version", &self.current_version())
            .field("history_len", &self.history_len())
            .field("max_history", &self.max_history())
            .finish()
    }
}

impl<T: Send + Sync> VersionedModule<T> {
    /// Create a new versioned module with the given initial value.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::sync::Arc;
    /// use claw_tools::hot_reload::VersionedModule;
    ///
    /// let module = VersionedModule::new(Arc::new("initial".to_string()));
    /// ```
    pub fn new(initial: Arc<T>) -> Self {
        let current = ArcSwap::new(initial.clone());
        let versions = RwLock::new(vec![ModuleVersion::new(1, initial)]);

        Self {
            current,
            versions,
            version_counter: AtomicU64::new(1),
            max_history: AtomicUsize::new(10), // Default: keep last 10 versions
        }
    }

    /// Create a new versioned module with custom history size.
    ///
    /// # Arguments
    ///
    /// * `initial` - The initial module value
    /// * `max_history` - Maximum number of versions to retain (min 1)
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::sync::Arc;
    /// use claw_tools::hot_reload::VersionedModule;
    ///
    /// let module = VersionedModule::with_capacity(Arc::new(42), 5);
    /// ```
    pub fn with_capacity(initial: Arc<T>, max_history: usize) -> Self {
        let max_history = max_history.max(1); // At least 1 version
        let current = ArcSwap::new(initial.clone());
        let versions = RwLock::new(vec![ModuleVersion::new(1, initial)]);

        Self {
            current,
            versions,
            version_counter: AtomicU64::new(1),
            max_history: AtomicUsize::new(max_history),
        }
    }

    /// Load the current module version.
    ///
    /// This operation is lock-free and highly efficient. The returned
    /// `Arc<T>` is a snapshot that remains valid even if the module
    /// is hot-swapped after this call.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::sync::Arc;
    /// use claw_tools::hot_reload::VersionedModule;
    ///
    /// # fn example() {
    /// let module = VersionedModule::new(Arc::new("data".to_string()));
    /// let snapshot: Arc<String> = module.load();
    /// println!("Current: {}", snapshot);
    /// # }
    /// ```
    pub fn load(&self) -> Arc<T> {
        self.current.load().clone()
    }

    /// Atomically swap to a new module version.
    ///
    /// Returns the new version number. Old readers continue to see
    /// the previous version until they drop their Arc.
    ///
    /// # Arguments
    ///
    /// * `new_module` - The new module value to swap in
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::sync::Arc;
    /// use claw_tools::hot_reload::VersionedModule;
    ///
    /// # fn example() {
    /// let module = VersionedModule::new(Arc::new(1));
    /// let new_version = module.swap(Arc::new(2));
    /// assert_eq!(new_version, 2);
    /// # }
    /// ```
    pub fn swap(&self, new_module: Arc<T>) -> u64 {
        let new_version = self.version_counter.fetch_add(1, Ordering::SeqCst) + 1;

        // Update history first (under write lock)
        {
            let mut versions = self.versions.write().expect("version lock poisoned");
            versions.push(ModuleVersion::new(new_version, new_module.clone()));

            // Trim history if exceeding max_history
            let max_history = self.max_history.load(Ordering::Relaxed);
            if versions.len() > max_history {
                versions.remove(0);
            }
        }

        // Atomic swap (wait-free for readers)
        self.current.store(new_module);

        new_version
    }

    /// Get the current version number.
    pub fn current_version(&self) -> u64 {
        self.version_counter.load(Ordering::SeqCst)
    }

    /// Get the number of versions in history.
    pub fn history_len(&self) -> usize {
        self.versions.read().expect("version lock poisoned").len()
    }

    /// Get a snapshot of all versions in history.
    ///
    /// Returns a vector of all stored versions, ordered from oldest to newest.
    pub fn versions(&self) -> Vec<ModuleVersion<T>>
    where
        T: Clone,
    {
        self.versions.read().expect("version lock poisoned").clone()
    }

    /// Rollback to a specific version.
    ///
    /// Returns true if rollback succeeded, false if version not found.
    ///
    /// # Arguments
    ///
    /// * `target_version` - The version number to rollback to
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::sync::Arc;
    /// use claw_tools::hot_reload::VersionedModule;
    ///
    /// # fn example() {
    /// let module = VersionedModule::new(Arc::new("v1".to_string()));
    /// module.swap(Arc::new("v2".to_string()));
    /// module.swap(Arc::new("v3".to_string()));
    ///
    /// // Rollback to version 2
    /// let success = module.rollback(2);
    /// assert!(success);
    /// assert_eq!(module.load().as_ref(), "v2");
    /// # }
    /// ```
    pub fn rollback(&self, target_version: u64) -> bool {
        let versions = self.versions.read().expect("version lock poisoned");

        // Find the target version
        if let Some(target) = versions.iter().find(|v| v.version == target_version) {
            let target_module = target.module.clone();
            drop(versions); // Release read lock before acquiring write lock

            // Perform the rollback
            let new_version = self.version_counter.fetch_add(1, Ordering::SeqCst) + 1;

            let mut versions = self.versions.write().expect("version lock poisoned");
            versions.push(ModuleVersion::new(new_version, target_module.clone()));

            // Trim history if needed
            let max_history = self.max_history.load(Ordering::Relaxed);
            if versions.len() > max_history {
                versions.remove(0);
            }

            // Atomic swap
            self.current.store(target_module);

            return true;
        }

        false
    }

    /// Rollback to the previous version.
    ///
    /// Returns true if rollback succeeded, false if no previous version.
    pub fn rollback_previous(&self) -> bool {
        let versions = self.versions.read().expect("version lock poisoned");

        if versions.len() >= 2 {
            // Get the second-to-last version
            let prev_idx = versions.len() - 2;
            let prev_module = versions[prev_idx].module.clone();
            drop(versions);

            // Create new version entry for the rollback
            let new_version = self.version_counter.fetch_add(1, Ordering::SeqCst) + 1;

            let mut versions = self.versions.write().expect("version lock poisoned");
            versions.push(ModuleVersion::new(new_version, prev_module.clone()));

            let max_history = self.max_history.load(Ordering::Relaxed);
            if versions.len() > max_history {
                versions.remove(0);
            }

            self.current.store(prev_module);

            return true;
        }

        false
    }

    /// Get the maximum history size.
    pub fn max_history(&self) -> usize {
        self.max_history.load(Ordering::Relaxed)
    }

    /// Update the maximum history size.
    ///
    /// If the new size is smaller than current history, old versions are trimmed.
    pub fn set_max_history(&self, max_history: usize) {
        let max_history = max_history.max(1);

        let mut versions = self.versions.write().expect("version lock poisoned");
        while versions.len() > max_history {
            versions.remove(0);
        }
        drop(versions);

        self.max_history.store(max_history, Ordering::Relaxed);
    }
}

impl<T> Default for VersionedModule<T>
where
    T: Default + Send + Sync,
{
    fn default() -> Self {
        Self::new(Arc::new(T::default()))
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_versioned_module_new() {
        let module = VersionedModule::new(Arc::new(42));
        assert_eq!(module.current_version(), 1);
        assert_eq!(module.history_len(), 1);
        assert_eq!(*module.load(), 42);
    }

    #[test]
    fn test_versioned_module_swap() {
        let module = VersionedModule::new(Arc::new("v1".to_string()));

        let v2 = module.swap(Arc::new("v2".to_string()));
        assert_eq!(v2, 2);
        assert_eq!(module.current_version(), 2);
        assert_eq!(module.history_len(), 2);
        assert_eq!(module.load().as_ref(), "v2");

        let v3 = module.swap(Arc::new("v3".to_string()));
        assert_eq!(v3, 3);
        assert_eq!(module.history_len(), 3);
    }

    #[test]
    fn test_versioned_module_snapshot_isolation() {
        let module = VersionedModule::new(Arc::new(1));

        // Take a snapshot
        let snapshot = module.load();
        assert_eq!(*snapshot, 1);

        // Swap to new version
        module.swap(Arc::new(2));

        // Snapshot still sees old value
        assert_eq!(*snapshot, 1);
        // But new loads see new value
        assert_eq!(*module.load(), 2);
    }

    #[test]
    fn test_versioned_module_rollback() {
        let module = VersionedModule::new(Arc::new("v1".to_string()));
        module.swap(Arc::new("v2".to_string()));
        module.swap(Arc::new("v3".to_string()));

        assert_eq!(module.load().as_ref(), "v3");

        // Rollback to v2
        assert!(module.rollback(2));
        assert_eq!(module.load().as_ref(), "v2");
        assert_eq!(module.current_version(), 4); // New version number

        // Rollback to v1
        assert!(module.rollback(1));
        assert_eq!(module.load().as_ref(), "v1");

        // Rollback to non-existent version
        assert!(!module.rollback(99));
    }

    #[test]
    fn test_versioned_module_rollback_previous() {
        let module = VersionedModule::new(Arc::new("v1".to_string()));
        module.swap(Arc::new("v2".to_string()));

        assert_eq!(module.load().as_ref(), "v2");
        assert!(module.rollback_previous());
        assert_eq!(module.load().as_ref(), "v1");

        // After rollback to v1, we have [v1, v2, v1(v3)]
        // Another rollback goes back to v2 (the previous content)
        // This is expected behavior - rollback creates new versions
        assert!(module.rollback_previous());
        assert_eq!(module.load().as_ref(), "v2");
    }

    #[test]
    fn test_versioned_module_concurrent_reads() {
        let module = Arc::new(VersionedModule::new(Arc::new(0)));
        let mut handles = vec![];

        // Spawn many readers
        for _ in 0..100 {
            let m = Arc::clone(&module);
            handles.push(thread::spawn(move || {
                for _ in 0..1000 {
                    let _ = m.load();
                }
            }));
        }

        // Concurrent swaps
        for i in 1..=10 {
            module.swap(Arc::new(i));
        }

        for h in handles {
            h.join().unwrap();
        }

        // Initial version 1 + 10 swaps = version 11
        assert_eq!(module.current_version(), 11);
    }

    #[test]
    fn test_versioned_module_history_limit() {
        let module = VersionedModule::with_capacity(Arc::new(0), 3);

        for i in 1..=5 {
            module.swap(Arc::new(i));
        }

        // Should only keep last 3 versions (plus initial = 3 max, so versions 3,4,5)
        assert_eq!(module.history_len(), 3);
        // Initial version 1 + 5 swaps = version 6
        assert_eq!(module.current_version(), 6);
    }

    #[test]
    fn test_versioned_module_versions_list() {
        let module = VersionedModule::new(Arc::new("a".to_string()));
        module.swap(Arc::new("b".to_string()));
        module.swap(Arc::new("c".to_string()));

        let versions = module.versions();
        assert_eq!(versions.len(), 3);
        assert_eq!(versions[0].version, 1);
        assert_eq!(versions[1].version, 2);
        assert_eq!(versions[2].version, 3);
    }

    #[test]
    fn test_versioned_module_default() {
        let module: VersionedModule<i32> = Default::default();
        assert_eq!(*module.load(), 0);
        assert_eq!(module.current_version(), 1);
    }
}
