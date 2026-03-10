use std::path::{Path, PathBuf};

use tracing::{debug, warn};

use crate::{
    error::SkillError,
    index::{SkillIndex, SkillIndexEntry},
    manifest::SkillManifest,
    priority::default_search_dirs,
};

/// Scans directories for skill files and provides lazy-loading of full content.
///
/// Search directories are ordered by priority — later-added directories take
/// precedence. When the same skill name appears in multiple directories,
/// the highest-priority version wins.
///
/// # Example
///
/// ```rust,no_run
/// use claw_skills::SkillLoader;
/// use std::path::PathBuf;
///
/// let loader = SkillLoader::new()
///     .add_dir(PathBuf::from("/usr/share/claw/skills"))   // low priority
///     .add_dir(PathBuf::from("~/.claw/skills"))           // medium priority
///     .add_dir(PathBuf::from("./skills"));               // highest priority
///
/// let index = loader.build_index().unwrap();
/// let full = loader.load_full("web-search").unwrap();
/// ```
pub struct SkillLoader {
    /// Search directories ordered from lowest to highest priority.
    search_dirs: Vec<PathBuf>,
}

impl SkillLoader {
    /// Create a new loader with default search directories.
    ///
    /// Default directories (lowest to highest priority):
    /// 1. Builtin skills directory (claw install dir)
    /// 2. User global skills (`~/.claw/skills`)
    /// 3. Workspace skills (`./skills`)
    pub fn new() -> Self {
        Self {
            search_dirs: default_search_dirs(),
        }
    }

    /// Create a new loader with no search directories.
    pub fn empty() -> Self {
        Self {
            search_dirs: Vec::new(),
        }
    }

    /// Add a search directory. Later-added directories have higher priority.
    pub fn add_dir(mut self, path: PathBuf) -> Self {
        self.search_dirs.push(path);
        self
    }

    /// Scan all directories and build a `SkillIndex`.
    ///
    /// Only metadata is loaded (no full body). Duplicate skill names are
    /// resolved by priority (last dir wins).
    pub fn build_index(&self) -> Result<SkillIndex, SkillError> {
        let manifests = self.resolve_priority();
        let entries = manifests
            .into_iter()
            .map(|m| SkillIndexEntry {
                name: m.name,
                description: m.description,
                path: m.path,
            })
            .collect();
        Ok(SkillIndex { entries })
    }

    /// Load the full content of a skill by name.
    ///
    /// Returns the complete `SKILL.md` content (frontmatter + body).
    /// Use this when an LLM requests full skill instructions.
    pub fn load_full(&self, name: &str) -> Result<String, SkillError> {
        let manifests = self.resolve_priority();
        let manifest = manifests
            .into_iter()
            .find(|m| m.name == name)
            .ok_or_else(|| SkillError::NotFound(name.to_string()))?;

        let skill_file = manifest.path.join("SKILL.md");
        std::fs::read_to_string(&skill_file).map_err(|e| SkillError::Io {
            path: skill_file,
            source: e,
        })
    }

    /// Resolve all manifests with priority merging.
    ///
    /// Scans all search directories (lowest priority first), then returns
    /// a deduplicated list where higher-priority directories win on name conflicts.
    pub fn resolve_priority(&self) -> Vec<SkillManifest> {
        // name → manifest, last write wins (higher priority)
        let mut seen: std::collections::HashMap<String, SkillManifest> =
            std::collections::HashMap::new();

        for dir in &self.search_dirs {
            if !dir.exists() {
                debug!(path = %dir.display(), "Skill search dir does not exist, skipping");
                continue;
            }

            match scan_dir(dir) {
                Ok(manifests) => {
                    for m in manifests {
                        debug!(name = %m.name, dir = %dir.display(), "Found skill");
                        seen.insert(m.name.clone(), m);
                    }
                }
                Err(e) => {
                    warn!(dir = %dir.display(), error = %e, "Failed to scan skill directory");
                }
            }
        }

        let mut result: Vec<SkillManifest> = seen.into_values().collect();
        result.sort_by(|a, b| a.name.cmp(&b.name));
        result
    }
}

impl Default for SkillLoader {
    fn default() -> Self {
        Self::new()
    }
}

/// Scan a single directory for skill subdirectories.
fn scan_dir(dir: &Path) -> Result<Vec<SkillManifest>, SkillError> {
    let mut manifests = Vec::new();

    let entries = std::fs::read_dir(dir).map_err(|e| SkillError::Io {
        path: dir.to_path_buf(),
        source: e,
    })?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            match SkillManifest::from_dir(&path) {
                Ok(m) => manifests.push(m),
                Err(SkillError::MissingSkillFile(_)) => {
                    // Not a skill directory — silently skip
                    debug!(path = %path.display(), "Skipping non-skill directory");
                }
                Err(e) => {
                    warn!(path = %path.display(), error = %e, "Failed to parse skill manifest");
                }
            }
        }
    }

    Ok(manifests)
}
