---
title: claw-skills
description: SKILL.md scanner, skill manifest indexer, and lazy loader for the Claw skill ecosystem
status: implemented
version: "1.4.1"
last_updated: "2026-03-10"
language: en
---


# claw-skills

Skill discovery, indexing, and loading for the Claw skill ecosystem.

---

## Overview

`claw-skills` scans the filesystem for `SKILL.md` files, parses their YAML frontmatter metadata, and builds a searchable index of available skills. It enables agents and the `KernelServer` to lazily discover and load skills at runtime without pre-registration.

### What is a Skill?

A skill is a markdown file (`SKILL.md`) with a YAML frontmatter header that describes a reusable capability — a prompt template, a workflow pattern, or a domain-specific instruction set. Skills are discovered automatically from standard search directories.

---

## Architecture Position

```
Layer 3: claw-skills ← You are here
    ↓ used by
Layer 2.5: claw-server (GlobalSkillRegistry)
Layer 1:   claw-loop (optional `skills` feature)
```

`claw-skills` has **zero runtime dependencies** on other claw crates — it only uses `serde`, `serde_yaml`, `semver`, `dirs`, and `tracing`.

---

## Usage

```toml
[dependencies]
claw-skills = "1.0"
```

```rust
use claw_skills::{SkillLoader, SkillIndex};

// Create a loader with default search directories
let loader = SkillLoader::new();

// Scan all search paths and build an index
let index: SkillIndex = loader.scan().await?;

// Find a skill by name
if let Some(entry) = index.find("code-reviewer") {
    println!("Skill: {}", entry.manifest.name);
    println!("Version: {}", entry.manifest.version);
    println!("Path: {}", entry.path.display());

    // Load the full skill content
    let content = entry.load_content().await?;
    println!("Content: {} bytes", content.len());
}

// List all available skills
for entry in index.entries() {
    println!("- {} v{}", entry.manifest.name, entry.manifest.version);
}
```

---

## SkillManifest

The YAML frontmatter in a `SKILL.md` file defines the skill's metadata:

```yaml
---
name: code-reviewer
version: "1.0.0"
description: Expert code review specialist for quality, security, and maintainability.
tags: [code-review, security, quality]
min_kernel_version: "1.3.0"
---

# Code Reviewer

[Skill content here...]
```

```rust
pub struct SkillManifest {
    /// Skill name (must be unique within a namespace)
    pub name: String,
    /// SemVer version string
    pub version: semver::Version,
    /// Human-readable description
    pub description: Option<String>,
    /// Searchable tags
    pub tags: Vec<String>,
    /// Minimum claw-kernel version required
    pub min_kernel_version: Option<semver::VersionReq>,
}
```

---

## Search Directories

`SkillLoader` scans directories in priority order:

| Priority | Directory | Description |
|----------|-----------|-------------|
| 1 (highest) | `$PWD/.claude/skills/` | Project-local skills |
| 2 | `~/.config/claw/skills/` | User global skills |
| 3 | `/etc/claw/skills/` | System-wide skills (Linux/macOS) |
| 4 | `~/.claw/skills/` | Legacy location (backward compat) |

Higher-priority directories shadow lower-priority ones for skills with the same name.

```rust
use claw_skills::SkillLoader;
use std::path::PathBuf;

// Use default search paths
let loader = SkillLoader::new();

// Or specify custom search paths
let loader = SkillLoader::with_paths(vec![
    PathBuf::from("/custom/skills"),
    PathBuf::from("./project-skills"),
]);
```

---

## SkillIndex

The index is built by scanning all configured search directories:

```rust
pub struct SkillIndex {
    entries: Vec<SkillIndexEntry>,
}

pub struct SkillIndexEntry {
    /// Parsed manifest from YAML frontmatter
    pub manifest: SkillManifest,
    /// Absolute path to the SKILL.md file
    pub path: PathBuf,
    /// Search directory where this skill was found
    pub source_dir: PathBuf,
}

impl SkillIndex {
    /// Find a skill by exact name
    pub fn find(&self, name: &str) -> Option<&SkillIndexEntry>;

    /// Search skills by tag
    pub fn by_tag(&self, tag: &str) -> Vec<&SkillIndexEntry>;

    /// All index entries
    pub fn entries(&self) -> &[SkillIndexEntry];

    /// Count of loaded skills
    pub fn len(&self) -> usize;
}

impl SkillIndexEntry {
    /// Load the full SKILL.md content (excluding frontmatter)
    pub async fn load_content(&self) -> Result<String, SkillError>;
}
```

---

## GlobalSkillRegistry (claw-server integration)

In the `claw-server` context, `GlobalSkillRegistry` wraps `SkillIndex` and exposes it over IPC:

```jsonc
// IPC: skill.list
{ "method": "skill.list" }
// → [{ "name": "...", "version": "...", "description": "...", "tags": [...] }, ...]

// IPC: skill.get
{ "method": "skill.get", "params": { "name": "code-reviewer" } }
// → { "name": "...", "content": "..." }
```

---

## Error Types

```rust
pub enum SkillError {
    /// IO error reading a SKILL.md file
    IoError(std::io::Error),
    /// Failed to parse YAML frontmatter
    FrontmatterParseError { path: PathBuf, message: String },
    /// Skill not found in index
    NotFound(String),
    /// Manifest validation failed (e.g., invalid semver)
    ValidationError(String),
}
```

---

## See Also

- [claw-server](claw-server.md) — GlobalSkillRegistry IPC integration
- [claw-loop](claw-loop.md) — `skills` feature for loop-level skill injection
- [Writing Tools Guide](../guides/writing-tools.md) — Creating custom Lua tools
