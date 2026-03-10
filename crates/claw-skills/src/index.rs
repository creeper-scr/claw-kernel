use std::path::PathBuf;

/// A single entry in the skill index.
#[derive(Debug, Clone)]
pub struct SkillIndexEntry {
    /// Skill name (from frontmatter).
    pub name: String,

    /// One-line description.
    pub description: String,

    /// Tags for filtering and discovery.
    pub tags: Vec<String>,

    /// Absolute path to the skill directory (for `load_full`).
    pub path: PathBuf,
}

/// Compact index of all discovered skills.
///
/// Suitable for injection into LLM system prompts — contains only
/// name, description, and path; not the full skill body.
#[derive(Debug, Clone, Default)]
pub struct SkillIndex {
    pub entries: Vec<SkillIndexEntry>,
}

impl SkillIndex {
    /// Create a new empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Generate a compact text block for injection into a system prompt.
    ///
    /// Format: one line per skill — `[name] (path): description`
    ///
    /// # Example output
    ///
    /// ```text
    /// ## Available Skills
    /// Use the `load_skill` tool to get full instructions for any skill.
    ///
    /// [web-search] (/home/user/skills/web-search): Search the web and return summarized results
    /// [code-review] (/home/user/skills/code-review): Review code for quality and security issues
    /// ```
    pub fn to_prompt_block(&self) -> String {
        if self.entries.is_empty() {
            return String::new();
        }

        let mut lines = vec![
            "## Available Skills".to_string(),
            "Use the `load_skill` tool to get full instructions for any skill.".to_string(),
            String::new(),
        ];

        for entry in &self.entries {
            lines.push(format!(
                "[{}] ({}): {}",
                entry.name,
                entry.path.display(),
                entry.description
            ));
        }

        lines.join("\n")
    }

    /// Number of skills in the index.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the index is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
