//! Agent Discovery and Management
//!
//! Provides agent discovery, metadata loading, and agent directory management.
//! Agents are discovered from the filesystem at `~/.local/share/claw-kernel/agents/`.

use crate::agent_types::{AgentId, ExecutionMode};
use crate::error::RuntimeError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ─── AgentMeta ────────────────────────────────────────────────────────────────

/// Metadata for a discovered agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMeta {
    /// Unique agent identifier.
    pub id: AgentId,
    /// Human-readable agent name.
    pub name: String,
    /// Agent version (semver format recommended).
    pub version: String,
    /// Agent capabilities.
    #[serde(default)]
    pub capabilities: Vec<AgentCapability>,
    /// Execution mode (Safe or Power).
    #[serde(default)]
    pub sandbox_mode: ExecutionMode,
    /// Path to agent configuration file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_path: Option<PathBuf>,
    /// Additional metadata key-value pairs.
    #[serde(default)]
    pub metadata: HashMap<String, String>,
    /// Agent description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Agent author.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Agent entry point script/tool.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_point: Option<String>,
}

impl AgentMeta {
    /// Create new agent metadata with the given ID and name.
    pub fn new(id: impl Into<String>, name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            id: AgentId::new(id),
            name: name.into(),
            version: version.into(),
            capabilities: Vec::new(),
            sandbox_mode: ExecutionMode::Safe,
            config_path: None,
            metadata: HashMap::new(),
            description: None,
            author: None,
            entry_point: None,
        }
    }

    /// Set the sandbox mode.
    pub fn with_mode(mut self, mode: ExecutionMode) -> Self {
        self.sandbox_mode = mode;
        self
    }

    /// Set the config path.
    pub fn with_config_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.config_path = Some(path.into());
        self
    }

    /// Set the description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Set the author.
    pub fn with_author(mut self, author: impl Into<String>) -> Self {
        self.author = Some(author.into());
        self
    }

    /// Set the entry point.
    pub fn with_entry_point(mut self, entry: impl Into<String>) -> Self {
        self.entry_point = Some(entry.into());
        self
    }

    /// Add a capability.
    pub fn with_capability(mut self, cap: AgentCapability) -> Self {
        self.capabilities.push(cap);
        self
    }

    /// Add metadata.
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Save metadata to a file.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), RuntimeError> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| RuntimeError::IpcError(format!("serialize failed: {}", e)))?;
        std::fs::write(path, json)
            .map_err(|e| RuntimeError::IpcError(format!("write failed: {}", e)))?;
        Ok(())
    }

    /// Load metadata from a file.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, RuntimeError> {
        let content = std::fs::read_to_string(path.as_ref())
            .map_err(|e| RuntimeError::IpcError(format!("read failed: {}", e)))?;
        let mut meta: Self = serde_json::from_str(&content)
            .map_err(|e| RuntimeError::IpcError(format!("parse failed: {}", e)))?;
        meta.config_path = Some(path.as_ref().to_path_buf());
        Ok(meta)
    }
}

// ─── AgentCapability ──────────────────────────────────────────────────────────

/// A capability offered by an agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentCapability {
    /// Capability name (e.g., "file-read", "llm-chat").
    pub name: String,
    /// Human-readable description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Capability version.
    pub version: String,
    /// Input schema (JSON Schema format).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<serde_json::Value>,
    /// Output schema (JSON Schema format).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<serde_json::Value>,
}

impl AgentCapability {
    /// Create a new capability.
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: None,
            version: version.into(),
            input_schema: None,
            output_schema: None,
        }
    }

    /// Set the description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }
}

// ─── AgentDiscovery ───────────────────────────────────────────────────────────

/// Discovers and manages agents from the filesystem.
pub struct AgentDiscovery {
    /// Base directory for agent discovery.
    agents_dir: PathBuf,
}

impl AgentDiscovery {
    /// Create a new discovery instance with the default agents directory.
    pub fn new() -> Result<Self, RuntimeError> {
        let agents_dir = Self::default_agents_dir()?;
        Ok(Self { agents_dir })
    }

    /// Create a new discovery instance with a custom agents directory.
    pub fn with_dir(path: impl Into<PathBuf>) -> Self {
        Self {
            agents_dir: path.into(),
        }
    }

    /// Get the default agents directory path.
    pub fn default_agents_dir() -> Result<PathBuf, RuntimeError> {
        let home = dirs::home_dir()
            .ok_or_else(|| RuntimeError::IpcError("cannot determine home directory".to_string()))?;
        Ok(home.join(".local/share/claw-kernel/agents"))
    }

    /// Get the agents directory.
    pub fn agents_dir(&self) -> &Path {
        &self.agents_dir
    }

    /// Scan the agents directory and return metadata for all discovered agents.
    pub fn scan(&self) -> Result<Vec<AgentMeta>, RuntimeError> {
        let mut agents = Vec::new();

        if !self.agents_dir.exists() {
            return Ok(agents);
        }

        for entry in std::fs::read_dir(&self.agents_dir)
            .map_err(|e| RuntimeError::IpcError(format!("read dir failed: {}", e)))?
        {
            let entry =
                entry.map_err(|e| RuntimeError::IpcError(format!("dir entry failed: {}", e)))?;
            let path = entry.path();

            if !path.is_dir() {
                continue;
            }

            match self.load_agent_meta(&path) {
                Ok(meta) => agents.push(meta),
                Err(e) => {
                    tracing::warn!("Failed to load agent from {:?}: {}", path, e);
                    // Continue scanning other agents
                }
            }
        }

        Ok(agents)
    }

    /// Load metadata for a single agent from its directory.
    pub fn load_agent_meta(&self, agent_dir: impl AsRef<Path>) -> Result<AgentMeta, RuntimeError> {
        let agent_dir = agent_dir.as_ref();

        // Try to find the metadata file
        let meta_file = agent_dir.join("agent.json");
        if meta_file.exists() {
            return AgentMeta::load(&meta_file);
        }

        // Fallback: try config.toml
        let config_file = agent_dir.join("config.toml");
        if config_file.exists() {
            return Self::load_from_toml(&config_file);
        }

        // Fallback: try manifest.yaml
        let manifest_file = agent_dir.join("manifest.yaml");
        if manifest_file.exists() {
            return Self::load_from_yaml(&manifest_file);
        }

        // If no metadata file found, try to infer from directory name
        let dir_name = agent_dir
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| RuntimeError::IpcError("invalid agent directory name".to_string()))?;

        let meta = AgentMeta::new(dir_name, dir_name, "0.1.0")
            .with_config_path(agent_dir.join("agent.json"));

        Ok(meta)
    }

    /// Load agent metadata from a TOML config file.
    fn load_from_toml(path: impl AsRef<Path>) -> Result<AgentMeta, RuntimeError> {
        let content = std::fs::read_to_string(path.as_ref())
            .map_err(|e| RuntimeError::IpcError(format!("read toml failed: {}", e)))?;

        // Parse minimal TOML structure
        let parsed: toml::Value = content
            .parse()
            .map_err(|e| RuntimeError::IpcError(format!("parse toml failed: {}", e)))?;

        let agent_id = parsed
            .get("id")
            .and_then(|v| v.as_str())
            .or_else(|| parsed.get("name").and_then(|v| v.as_str()))
            .ok_or_else(|| RuntimeError::IpcError("missing agent id in toml".to_string()))?;

        let name = parsed
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(agent_id);

        let version = parsed
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("0.1.0");

        let mut meta = AgentMeta::new(agent_id, name, version).with_config_path(path.as_ref());

        if let Some(desc) = parsed.get("description").and_then(|v| v.as_str()) {
            meta.description = Some(desc.to_string());
        }

        if let Some(author) = parsed.get("author").and_then(|v| v.as_str()) {
            meta.author = Some(author.to_string());
        }

        if let Some(entry) = parsed.get("entry_point").and_then(|v| v.as_str()) {
            meta.entry_point = Some(entry.to_string());
        }

        // Parse mode
        if let Some(mode_str) = parsed.get("mode").and_then(|v| v.as_str()) {
            meta.sandbox_mode = match mode_str.to_lowercase().as_str() {
                "power" => ExecutionMode::Power,
                _ => ExecutionMode::Safe,
            };
        }

        Ok(meta)
    }

    /// Load agent metadata from a YAML manifest file.
    fn load_from_yaml(path: impl AsRef<Path>) -> Result<AgentMeta, RuntimeError> {
        let content = std::fs::read_to_string(path.as_ref())
            .map_err(|e| RuntimeError::IpcError(format!("read yaml failed: {}", e)))?;

        let parsed: serde_yaml::Value = serde_yaml::from_str(&content)
            .map_err(|e| RuntimeError::IpcError(format!("parse yaml failed: {}", e)))?;

        let agent_id = parsed
            .get("id")
            .and_then(|v| v.as_str())
            .or_else(|| parsed.get("name").and_then(|v| v.as_str()))
            .ok_or_else(|| RuntimeError::IpcError("missing agent id in yaml".to_string()))?;

        let name = parsed
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(agent_id);

        let version = parsed
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("0.1.0");

        let mut meta = AgentMeta::new(agent_id, name, version).with_config_path(path.as_ref());

        if let Some(desc) = parsed.get("description").and_then(|v| v.as_str()) {
            meta.description = Some(desc.to_string());
        }

        if let Some(author) = parsed.get("author").and_then(|v| v.as_str()) {
            meta.author = Some(author.to_string());
        }

        Ok(meta)
    }

    /// Create a new agent directory structure.
    pub fn create_agent(
        &self,
        agent_id: impl Into<String>,
        name: impl Into<String>,
        version: impl Into<String>,
    ) -> Result<PathBuf, RuntimeError> {
        let agent_id = agent_id.into();
        let agent_dir = self.agents_dir.join(&agent_id);

        // Create directory structure
        std::fs::create_dir_all(&agent_dir)
            .map_err(|e| RuntimeError::IpcError(format!("create dir failed: {}", e)))?;

        // Create subdirectories
        for subdir in &["tools", "scripts", "data", "logs"] {
            std::fs::create_dir_all(agent_dir.join(subdir))
                .map_err(|e| RuntimeError::IpcError(format!("create subdir failed: {}", e)))?;
        }

        // Create agent.json metadata file
        let meta =
            AgentMeta::new(&agent_id, name, version).with_config_path(agent_dir.join("agent.json"));
        meta.save(agent_dir.join("agent.json"))?;

        Ok(agent_dir)
    }

    /// Remove an agent directory.
    pub fn remove_agent(&self, agent_id: impl AsRef<str>) -> Result<(), RuntimeError> {
        let agent_dir = self.agents_dir.join(agent_id.as_ref());
        if agent_dir.exists() {
            std::fs::remove_dir_all(&agent_dir)
                .map_err(|e| RuntimeError::IpcError(format!("remove dir failed: {}", e)))?;
        }
        Ok(())
    }

    /// Check if an agent exists.
    pub fn agent_exists(&self, agent_id: impl AsRef<str>) -> bool {
        self.agents_dir.join(agent_id.as_ref()).exists()
    }

    /// Get the path to an agent's directory.
    pub fn agent_dir(&self, agent_id: impl AsRef<str>) -> PathBuf {
        self.agents_dir.join(agent_id.as_ref())
    }

    /// Ensure the agents directory exists.
    pub fn ensure_dir(&self) -> Result<(), RuntimeError> {
        std::fs::create_dir_all(&self.agents_dir)
            .map_err(|e| RuntimeError::IpcError(format!("create agents dir failed: {}", e)))?;
        Ok(())
    }
}

impl Default for AgentDiscovery {
    fn default() -> Self {
        Self::with_dir(
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join(".local/share/claw-kernel/agents"),
        )
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn create_test_discovery() -> (AgentDiscovery, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let discovery = AgentDiscovery::with_dir(temp_dir.path().join("agents"));
        (discovery, temp_dir)
    }

    // ── test_agent_meta_builder ────────────────────────────────────────────────
    #[test]
    fn test_agent_meta_builder() {
        let meta = AgentMeta::new("test-agent", "Test Agent", "1.0.0")
            .with_mode(ExecutionMode::Power)
            .with_description("A test agent")
            .with_author("Test Author")
            .with_capability(AgentCapability::new("echo", "1.0.0"))
            .with_metadata("key", "value");

        assert_eq!(meta.id.0, "test-agent");
        assert_eq!(meta.name, "Test Agent");
        assert_eq!(meta.version, "1.0.0");
        assert_eq!(meta.sandbox_mode, ExecutionMode::Power);
        assert_eq!(meta.description, Some("A test agent".to_string()));
        assert_eq!(meta.author, Some("Test Author".to_string()));
        assert_eq!(meta.capabilities.len(), 1);
        assert_eq!(meta.metadata.get("key"), Some(&"value".to_string()));
    }

    // ── test_agent_meta_save_load ──────────────────────────────────────────────
    #[test]
    fn test_agent_meta_save_load() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("agent.json");

        let meta =
            AgentMeta::new("save-test", "Save Test", "1.0.0").with_description("Test save/load");

        meta.save(&path).unwrap();

        let loaded = AgentMeta::load(&path).unwrap();
        assert_eq!(loaded.id, meta.id);
        assert_eq!(loaded.name, meta.name);
        assert_eq!(loaded.version, meta.version);
        assert_eq!(loaded.description, meta.description);
    }

    // ── test_discovery_scan_empty ──────────────────────────────────────────────
    #[test]
    fn test_discovery_scan_empty() {
        let (discovery, _temp) = create_test_discovery();

        let agents = discovery.scan().unwrap();
        assert!(agents.is_empty());
    }

    // ── test_discovery_create_and_scan ─────────────────────────────────────────
    #[test]
    fn test_discovery_create_and_scan() {
        let (discovery, _temp) = create_test_discovery();

        // Create an agent
        let agent_dir = discovery
            .create_agent("test-agent", "Test Agent", "1.0.0")
            .unwrap();
        assert!(agent_dir.exists());
        assert!(agent_dir.join("agent.json").exists());
        assert!(agent_dir.join("tools").exists());
        assert!(agent_dir.join("scripts").exists());

        // Scan should find it
        let agents = discovery.scan().unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].id.0, "test-agent");
    }

    // ── test_discovery_load_agent_meta ─────────────────────────────────────────
    #[test]
    fn test_discovery_load_agent_meta() {
        let (discovery, _temp) = create_test_discovery();

        // Create an agent first
        let _ = discovery.create_agent("meta-test", "Meta Test", "2.0.0");

        // Load its metadata
        let meta = discovery
            .load_agent_meta(discovery.agent_dir("meta-test"))
            .unwrap();
        assert_eq!(meta.id.0, "meta-test");
        assert_eq!(meta.name, "Meta Test");
        assert_eq!(meta.version, "2.0.0");
    }

    // ── test_discovery_load_from_toml ──────────────────────────────────────────
    #[test]
    fn test_discovery_load_from_toml() {
        let (discovery, temp) = create_test_discovery();

        // Create a TOML config file
        let agent_dir = temp.path().join("agents/tommy");
        std::fs::create_dir_all(&agent_dir).unwrap();

        let mut file = std::fs::File::create(agent_dir.join("config.toml")).unwrap();
        writeln!(
            file,
            r#"
id = "tommy"
name = "Tommy Agent"
version = "1.2.3"
description = "A TOML-configured agent"
author = "Test"
mode = "safe"
entry_point = "main.lua"
"#
        )
        .unwrap();

        let meta = discovery.load_agent_meta(&agent_dir).unwrap();
        assert_eq!(meta.id.0, "tommy");
        assert_eq!(meta.name, "Tommy Agent");
        assert_eq!(meta.version, "1.2.3");
        assert_eq!(meta.sandbox_mode, ExecutionMode::Safe);
        assert_eq!(meta.entry_point, Some("main.lua".to_string()));
    }

    // ── test_discovery_load_from_yaml ──────────────────────────────────────────
    #[test]
    fn test_discovery_load_from_yaml() {
        let (discovery, temp) = create_test_discovery();

        // Create a YAML manifest file
        let agent_dir = temp.path().join("agents/yammy");
        std::fs::create_dir_all(&agent_dir).unwrap();

        let mut file = std::fs::File::create(agent_dir.join("manifest.yaml")).unwrap();
        writeln!(
            file,
            r#"
id: yammy
name: Yammy Agent
version: 3.0.0
description: A YAML-configured agent
author: Test
"#
        )
        .unwrap();

        let meta = discovery.load_agent_meta(&agent_dir).unwrap();
        assert_eq!(meta.id.0, "yammy");
        assert_eq!(meta.name, "Yammy Agent");
        assert_eq!(meta.version, "3.0.0");
    }

    // ── test_discovery_remove_agent ────────────────────────────────────────────
    #[test]
    fn test_discovery_remove_agent() {
        let (discovery, _temp) = create_test_discovery();

        discovery
            .create_agent("delete-me", "Delete Me", "1.0.0")
            .unwrap();
        assert!(discovery.agent_exists("delete-me"));

        discovery.remove_agent("delete-me").unwrap();
        assert!(!discovery.agent_exists("delete-me"));
    }

    // ── test_discovery_agent_exists ────────────────────────────────────────────
    #[test]
    fn test_discovery_agent_exists() {
        let (discovery, _temp) = create_test_discovery();

        assert!(!discovery.agent_exists("nonexistent"));

        discovery.create_agent("exists", "Exists", "1.0.0").unwrap();
        assert!(discovery.agent_exists("exists"));
    }

    // ── test_discovery_ensure_dir ──────────────────────────────────────────────
    #[test]
    fn test_discovery_ensure_dir() {
        let (discovery, temp) = create_test_discovery();

        // The discovery creates a subdir "agents" under temp
        let agents_dir = temp.path().join("agents");
        assert!(!agents_dir.exists());

        discovery.ensure_dir().unwrap();
        assert!(agents_dir.exists());
    }

    // ── test_capability_builder ────────────────────────────────────────────────
    #[test]
    fn test_capability_builder() {
        let cap = AgentCapability::new("file-read", "1.0.0").with_description("Read files");

        assert_eq!(cap.name, "file-read");
        assert_eq!(cap.version, "1.0.0");
        assert_eq!(cap.description, Some("Read files".to_string()));
    }

    // ── test_discovery_scan_multiple ───────────────────────────────────────────
    #[test]
    fn test_discovery_scan_multiple() {
        let (discovery, _temp) = create_test_discovery();

        discovery
            .create_agent("agent-a", "Agent A", "1.0.0")
            .unwrap();
        discovery
            .create_agent("agent-b", "Agent B", "1.0.0")
            .unwrap();
        discovery
            .create_agent("agent-c", "Agent C", "1.0.0")
            .unwrap();

        let agents = discovery.scan().unwrap();
        assert_eq!(agents.len(), 3);

        let ids: Vec<_> = agents.into_iter().map(|a| a.id.0).collect();
        assert!(ids.contains(&"agent-a".to_string()));
        assert!(ids.contains(&"agent-b".to_string()));
        assert!(ids.contains(&"agent-c".to_string()));
    }

    // ── test_discovery_default_agents_dir ──────────────────────────────────────
    #[test]
    fn test_discovery_default_agents_dir() {
        // Just verify it returns a path containing the expected components
        let path = AgentDiscovery::default_agents_dir().unwrap();
        let path_str = path.to_string_lossy();
        assert!(path_str.contains("claw-kernel"));
        assert!(path_str.contains("agents"));
    }
}
