use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use tracing::debug;

use crate::error::SkillError;

/// Raw YAML frontmatter fields (deserialized from `---` block).
#[derive(Debug, Deserialize)]
struct RawFrontmatter {
    name: String,
    description: String,
    version: Option<String>,
    tags: Option<Vec<String>>,
    #[allow(dead_code)]
    #[serde(flatten)]
    extra: HashMap<String, serde_yaml::Value>,
}

/// Parsed skill manifest extracted from a `SKILL.md` file.
#[derive(Debug, Clone)]
pub struct SkillManifest {
    /// Skill name (from frontmatter `name` field).
    pub name: String,

    /// One-line summary used when building `SkillIndex`.
    pub description: String,

    /// Semantic version (defaults to `0.1.0` if omitted).
    pub version: semver::Version,

    /// Optional tags for filtering and discovery.
    pub tags: Vec<String>,

    /// Absolute path to the skill directory.
    pub path: PathBuf,
}

impl SkillManifest {
    /// Parse a `SkillManifest` from a skill directory.
    ///
    /// Expects the directory to contain a `SKILL.md` file with YAML frontmatter.
    ///
    /// # SKILL.md format
    ///
    /// ```markdown
    /// ---
    /// name: web-search
    /// description: Search the web and return summarized results
    /// version: 1.0.0
    /// tags: [search, web, research]
    /// ---
    ///
    /// # Web Search Skill
    /// ...body content...
    /// ```
    pub fn from_dir(path: &Path) -> Result<Self, SkillError> {
        let skill_file = path.join("SKILL.md");
        if !skill_file.exists() {
            return Err(SkillError::MissingSkillFile(path.to_path_buf()));
        }

        let content = std::fs::read_to_string(&skill_file).map_err(|e| SkillError::Io {
            path: skill_file.clone(),
            source: e,
        })?;

        let (frontmatter_str, _body) = parse_frontmatter(&content, &skill_file)?;
        let raw: RawFrontmatter =
            serde_yaml::from_str(&frontmatter_str).map_err(|e| SkillError::FrontmatterParse {
                path: skill_file.clone(),
                source: e,
            })?;

        let version_str = raw.version.as_deref().unwrap_or("0.1.0");
        let version =
            semver::Version::parse(version_str).map_err(|e| SkillError::InvalidVersion {
                version: version_str.to_string(),
                path: skill_file.clone(),
                source: e,
            })?;

        let abs_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        debug!(name = %raw.name, path = %abs_path.display(), "Parsed skill manifest");

        Ok(SkillManifest {
            name: raw.name,
            description: raw.description,
            version,
            tags: raw.tags.unwrap_or_default(),
            path: abs_path,
        })
    }
}

/// Split content into frontmatter + body.
/// Returns `(frontmatter_str, body_str)`.
fn parse_frontmatter<'a>(
    content: &'a str,
    path: &Path,
) -> Result<(String, &'a str), SkillError> {
    // Must start with "---\n" or "---\r\n"
    let rest = content
        .strip_prefix("---\n")
        .or_else(|| content.strip_prefix("---\r\n"))
        .ok_or_else(|| SkillError::MissingSkillFile(path.to_path_buf()))?;

    // Find closing "---"
    if let Some(end_idx) = rest.find("\n---\n").or_else(|| rest.find("\n---\r\n")) {
        let frontmatter = &rest[..end_idx];
        let after_end = end_idx + if rest[end_idx..].starts_with("\n---\r\n") { 6 } else { 5 };
        let body = &rest[after_end..];
        Ok((frontmatter.to_string(), body))
    } else if let Some(end_idx) = rest.rfind("\n---") {
        // End of file
        let frontmatter = &rest[..end_idx];
        Ok((frontmatter.to_string(), ""))
    } else {
        Err(SkillError::MissingSkillFile(path.to_path_buf()))
    }
}
