use thiserror::Error;

#[derive(Debug, Error)]
pub enum SkillError {
    #[error("IO error reading skill at {path}: {source}")]
    Io {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Missing SKILL.md in directory: {0}")]
    MissingSkillFile(std::path::PathBuf),

    #[error("Failed to parse YAML frontmatter in {path}: {source}")]
    FrontmatterParse {
        path: std::path::PathBuf,
        #[source]
        source: serde_yaml::Error,
    },

    #[error("Missing required frontmatter field '{field}' in {path}")]
    MissingField {
        field: &'static str,
        path: std::path::PathBuf,
    },

    #[error("Invalid semver version '{version}' in {path}: {source}")]
    InvalidVersion {
        version: String,
        path: std::path::PathBuf,
        #[source]
        source: semver::Error,
    },

    #[error("Skill '{0}' not found in any search directory")]
    NotFound(String),
}
