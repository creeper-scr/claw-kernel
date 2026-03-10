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
/// precedence as a tiebreaker. When the same skill name appears in multiple
/// directories, the one with the **highest semver** wins; directory priority
/// only applies when versions are equal.
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
    /// resolved by semver (higher version wins); directory priority breaks ties.
    pub fn build_index(&self) -> Result<SkillIndex, SkillError> {
        let manifests = self.resolve_priority();
        let entries = manifests
            .into_iter()
            .map(|m| SkillIndexEntry {
                name: m.name,
                description: m.description,
                tags: m.tags,
                path: m.path,
            })
            .collect();
        Ok(SkillIndex { entries })
    }

    /// Scan all directories and build a `SkillIndex` filtered by tags.
    ///
    /// Only skills whose `tags` list contains **at least one** of the requested
    /// tags are included. Passing an empty slice is equivalent to
    /// [`build_index`] — all skills are returned.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use claw_skills::SkillLoader;
    ///
    /// let loader = SkillLoader::new();
    /// let web_skills = loader.build_index_filtered(&["web", "search"]).unwrap();
    /// ```
    pub fn build_index_filtered(&self, tags: &[&str]) -> Result<SkillIndex, SkillError> {
        let manifests = self.resolve_priority();
        let entries = manifests
            .into_iter()
            .filter(|m| tags.is_empty() || tags.iter().any(|t| m.tags.contains(&t.to_string())))
            .map(|m| SkillIndexEntry {
                name: m.name,
                description: m.description,
                tags: m.tags,
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
    /// Scans all search directories (lowest priority first), then returns a
    /// deduplicated list.  Conflict resolution for the same skill name:
    ///
    /// 1. **Higher semver wins** — a system-level `v1.2.0` is kept even when
    ///    a workspace-local `v0.1.0` is scanned afterwards.
    /// 2. **Directory priority as tiebreaker** — when versions are equal, the
    ///    later (higher-priority) directory overwrites the earlier one.
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
                        debug!(name = %m.name, version = %m.version, dir = %dir.display(), "Found skill");
                        match seen.get(&m.name) {
                            Some(existing) if existing.version > m.version => {
                                // Keep the newer version from a lower-priority dir rather than
                                // blindly overwriting with an older workspace copy.
                                debug!(
                                    name = %m.name,
                                    kept = %existing.version,
                                    skipped = %m.version,
                                    "Skipping lower-version skill"
                                );
                            }
                            _ => {
                                seen.insert(m.name.clone(), m);
                            }
                        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_skill(dir: &std::path::Path, skill_name: &str, version: &str) {
        let skill_dir = dir.join(skill_name);
        fs::create_dir_all(&skill_dir).unwrap();
        let content = format!(
            "---\nname: {skill_name}\ndescription: test skill\nversion: {version}\n---\n\nbody"
        );
        fs::write(skill_dir.join("SKILL.md"), content).unwrap();
    }

    fn write_skill_with_tags(
        dir: &std::path::Path,
        skill_name: &str,
        version: &str,
        tags: &[&str],
    ) {
        let skill_dir = dir.join(skill_name);
        fs::create_dir_all(&skill_dir).unwrap();
        let tags_yaml = tags
            .iter()
            .map(|t| format!("  - {t}"))
            .collect::<Vec<_>>()
            .join("\n");
        let content = format!(
            "---\nname: {skill_name}\ndescription: test skill\nversion: {version}\ntags:\n{tags_yaml}\n---\n\nbody"
        );
        fs::write(skill_dir.join("SKILL.md"), content).unwrap();
    }

    /// Higher-version system skill must survive when workspace installs an older copy.
    #[test]
    fn test_higher_version_wins_over_higher_priority_dir() {
        let system_dir = TempDir::new().unwrap();
        let workspace_dir = TempDir::new().unwrap();

        write_skill(system_dir.path(), "my-skill", "1.2.0");
        write_skill(workspace_dir.path(), "my-skill", "0.1.0");

        // workspace_dir added last → highest directory priority
        let loader = SkillLoader::empty()
            .add_dir(system_dir.path().to_path_buf())
            .add_dir(workspace_dir.path().to_path_buf());

        let manifests = loader.resolve_priority();
        assert_eq!(manifests.len(), 1);
        assert_eq!(
            manifests[0].version,
            semver::Version::parse("1.2.0").unwrap(),
            "system v1.2.0 must win over workspace v0.1.0"
        );
    }

    /// When versions are equal, higher-priority directory (workspace) should win.
    #[test]
    fn test_equal_version_higher_priority_dir_wins() {
        let system_dir = TempDir::new().unwrap();
        let workspace_dir = TempDir::new().unwrap();

        write_skill(system_dir.path(), "my-skill", "1.0.0");
        write_skill(workspace_dir.path(), "my-skill", "1.0.0");

        let loader = SkillLoader::empty()
            .add_dir(system_dir.path().to_path_buf())
            .add_dir(workspace_dir.path().to_path_buf());

        let manifests = loader.resolve_priority();
        assert_eq!(manifests.len(), 1);
        // Both are v1.0.0 — the workspace copy's path should be selected.
        // Canonicalize expected path to handle symlinks (e.g. /tmp → /private/tmp on macOS).
        let canonical_workspace = workspace_dir.path().canonicalize().unwrap();
        assert!(
            manifests[0].path.starts_with(&canonical_workspace),
            "workspace copy should win as tiebreaker"
        );
    }

    /// Workspace skill with a genuinely higher version should still win.
    #[test]
    fn test_workspace_newer_version_wins() {
        let system_dir = TempDir::new().unwrap();
        let workspace_dir = TempDir::new().unwrap();

        write_skill(system_dir.path(), "my-skill", "1.0.0");
        write_skill(workspace_dir.path(), "my-skill", "2.0.0");

        let loader = SkillLoader::empty()
            .add_dir(system_dir.path().to_path_buf())
            .add_dir(workspace_dir.path().to_path_buf());

        let manifests = loader.resolve_priority();
        assert_eq!(manifests.len(), 1);
        assert_eq!(
            manifests[0].version,
            semver::Version::parse("2.0.0").unwrap(),
            "workspace v2.0.0 must win over system v1.0.0"
        );
    }

    /// Tags are preserved in index entries.
    #[test]
    fn test_build_index_includes_tags() {
        let dir = TempDir::new().unwrap();
        write_skill_with_tags(dir.path(), "web-search", "1.0.0", &["web", "search"]);
        write_skill(dir.path(), "code-review", "1.0.0");

        let loader = SkillLoader::empty().add_dir(dir.path().to_path_buf());
        let index = loader.build_index().unwrap();

        let web = index.entries.iter().find(|e| e.name == "web-search").unwrap();
        assert_eq!(web.tags, vec!["web", "search"]);

        let review = index.entries.iter().find(|e| e.name == "code-review").unwrap();
        assert!(review.tags.is_empty());
    }

    /// Filtering by a matching tag returns only relevant skills.
    #[test]
    fn test_build_index_filtered_single_tag() {
        let dir = TempDir::new().unwrap();
        write_skill_with_tags(dir.path(), "web-search", "1.0.0", &["web", "search"]);
        write_skill_with_tags(dir.path(), "web-scraper", "1.0.0", &["web"]);
        write_skill(dir.path(), "code-review", "1.0.0");

        let loader = SkillLoader::empty().add_dir(dir.path().to_path_buf());
        let index = loader.build_index_filtered(&["web"]).unwrap();

        let names: Vec<&str> = index.entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"web-search"), "web-search must be included");
        assert!(names.contains(&"web-scraper"), "web-scraper must be included");
        assert!(!names.contains(&"code-review"), "code-review must be excluded");
    }

    /// Filtering with no tags returns all skills (same as build_index).
    #[test]
    fn test_build_index_filtered_empty_tags_returns_all() {
        let dir = TempDir::new().unwrap();
        write_skill_with_tags(dir.path(), "web-search", "1.0.0", &["web"]);
        write_skill(dir.path(), "code-review", "1.0.0");

        let loader = SkillLoader::empty().add_dir(dir.path().to_path_buf());
        let index = loader.build_index_filtered(&[]).unwrap();

        assert_eq!(index.len(), 2);
    }

    /// Filtering with a tag that no skill has returns an empty index.
    #[test]
    fn test_build_index_filtered_no_match_returns_empty() {
        let dir = TempDir::new().unwrap();
        write_skill_with_tags(dir.path(), "web-search", "1.0.0", &["web"]);

        let loader = SkillLoader::empty().add_dir(dir.path().to_path_buf());
        let index = loader.build_index_filtered(&["database"]).unwrap();

        assert!(index.is_empty());
    }
}
