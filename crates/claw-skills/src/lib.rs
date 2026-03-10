//! Skill engine — discover, index, and lazy-load SKILL.md files for AI agents.
//!
//! # Overview
//!
//! Skills are Markdown files with YAML frontmatter (`SKILL.md`) stored in
//! directories. The skill engine scans directories, builds a compact index
//! (suitable for injection into LLM system prompts), and loads full skill
//! content on demand.
//!
//! # Example
//!
//! ```rust,no_run
//! use claw_skills::SkillLoader;
//! use std::path::PathBuf;
//!
//! let loader = SkillLoader::new()
//!     .add_dir(PathBuf::from("./skills"));
//!
//! let index = loader.build_index().unwrap();
//! println!("{}", index.to_prompt_block());
//! ```

pub mod error;
pub mod index;
pub mod loader;
pub mod manifest;
pub mod priority;

pub use error::SkillError;
pub use index::{SkillIndex, SkillIndexEntry};
pub use loader::SkillLoader;
pub use manifest::SkillManifest;
pub use priority::default_search_dirs;
