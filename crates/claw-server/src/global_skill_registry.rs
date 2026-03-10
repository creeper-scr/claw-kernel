//! Global server-level skill registry.
//!
//! Wraps `claw_skills::SkillLoader` with thread-safe access and exposes
//! skill management to the IPC server.

use std::path::PathBuf;
use tokio::sync::RwLock;
use claw_skills::{SkillLoader, SkillIndex, SkillManifest, SkillError};

/// Thread-safe global registry of loaded skills.
///
/// Internally tracks search directories and rebuilds the `SkillLoader`
/// whenever a new directory is added (since `SkillLoader::add_dir` takes
/// ownership via builder pattern).
pub struct GlobalSkillRegistry {
    /// Ordered list of search directories (lowest to highest priority).
    dirs: RwLock<Vec<PathBuf>>,
}

impl GlobalSkillRegistry {
    /// Creates a new, empty GlobalSkillRegistry (no default search dirs).
    pub fn new() -> Self {
        Self {
            dirs: RwLock::new(Vec::new()),
        }
    }

    /// Builds a `SkillLoader` from the current directory list.
    async fn build_loader(&self) -> SkillLoader {
        let dirs = self.dirs.read().await;
        let mut loader = SkillLoader::empty();
        for dir in dirs.iter() {
            loader = loader.add_dir(dir.clone());
        }
        loader
    }

    /// Adds a directory to the skill loader and scans it.
    /// Returns the number of skills found after scanning all dirs.
    pub async fn load_dir(&self, path: impl Into<PathBuf>) -> Result<usize, SkillError> {
        let path = path.into();
        {
            let mut dirs = self.dirs.write().await;
            if !dirs.contains(&path) {
                dirs.push(path);
            }
        }
        let loader = self.build_loader().await;
        let count = loader.build_index()
            .map(|idx| idx.entries.len())
            .unwrap_or(0);
        Ok(count)
    }

    /// Returns all loaded skill manifests.
    pub async fn list(&self) -> Vec<SkillManifest> {
        let loader = self.build_loader().await;
        loader.resolve_priority()
    }

    /// Loads the full content of a skill by name.
    pub async fn get_full(&self, name: &str) -> Result<String, SkillError> {
        let loader = self.build_loader().await;
        loader.load_full(name)
    }

    /// Builds a SkillIndex for injection into system prompts.
    pub async fn build_index(&self) -> Result<SkillIndex, SkillError> {
        let loader = self.build_loader().await;
        loader.build_index()
    }

    /// Returns true if no skills are loaded.
    pub async fn is_empty(&self) -> bool {
        let dirs = self.dirs.read().await;
        if dirs.is_empty() {
            return true;
        }
        let loader = self.build_loader().await;
        loader.resolve_priority().is_empty()
    }
}

impl Default for GlobalSkillRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for GlobalSkillRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GlobalSkillRegistry").finish_non_exhaustive()
    }
}
