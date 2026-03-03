---
title: claw-tools
description: Tool registry, hot-loading, schema generation
status: implemented
version: "0.1.0"
last_updated: "2026-03-01"
language: en
---

[中文版 →](claw-tools.zh.md)


Tool registry and hot-loading for agent capabilities.

---

## Overview

`claw-tools` implements the tool-use protocol:
- Tool registration and discovery
- Schema generation and validation
- Hot-loading from scripts
- Permission management

---

## Usage

```toml
[dependencies]
claw-tools = { version = "0.1", features = ["hot-loading"] }
```

```rust
use claw_tools::{ToolRegistry, Tool};

let mut registry = ToolRegistry::new();

// Load from directory
registry.load_from_directory("./tools").await?;

// Enable hot-loading
registry.enable_hot_loading().await?;

// Execute tool
let result = registry.execute("calculator", json!({
    "operation": "add",
    "a": 1,
    "b": 2
})).await?;
```

---

## Core Components

### `Tool` Trait

The core abstraction for executable capabilities:

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    /// Tool identifier
    fn name(&self) -> &str;
    
    /// Tool description for LLM
    fn description(&self) -> &str;
    
    /// Semantic version, e.g., "1.0.0"
    fn version(&self) -> &str;
    
    /// JSON Schema for parameter validation
    fn schema(&self) -> ToolSchema;
    
    /// Execute with given parameters
    async fn execute(&self, params: Value) -> Result<ToolResult, ToolError>;
    
    /// Required permissions
    fn permissions(&self) -> PermissionSet;
    
    /// Default timeout
    fn timeout(&self) -> Duration { Duration::from_secs(30) }
}
```

### `ToolRegistry`

Central registry for tool discovery and execution:

```rust
pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
    hot_loading: Option<HotLoadingWatcher>,
}

impl ToolRegistry {
    pub fn new() -> Self;
    pub fn register(&mut self, tool: Box<dyn Tool>);
    pub fn get(&self, name: &str) -> Option<&dyn Tool>;
    pub fn list(&self) -> Vec<&ToolMeta>;
    
    // Hot-loading support (requires "hot-loading" feature)
    pub async fn load_from_script(&mut self, path: &Path) -> Result<ToolMeta, LoadError>;
    pub fn unload(&mut self, name: &str);
    
    // Directory loading and auto-reload
    pub async fn load_from_directory(&mut self, path: &Path) -> Result<()>;
    pub async fn enable_hot_loading(&mut self) -> Result<()>;
}
```

### Schema Generation

Tools declare their interface via JSON Schema:

```rust
#[derive(JsonSchema, Deserialize)]
struct SearchParams {
    query: String,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize { 10 }
```

### Permission System

```rust
pub struct PermissionSet {
    pub filesystem: FsPermissions,
    pub network: NetworkPermissions,
    pub subprocess: SubprocessPolicy,
}

/// Filesystem permission for a tool.
pub struct FsPermissions {
    /// Allowed read paths (glob patterns or absolute paths).
    pub read_paths: HashSet<String>,
    /// Allowed write paths.
    pub write_paths: HashSet<String>,
}

impl FsPermissions {
    pub fn none() -> Self {
        Self {
            read_paths: HashSet::new(),
            write_paths: HashSet::new(),
        }
    }
    pub fn read_only(paths: impl IntoIterator<Item = String>) -> Self {
        Self {
            read_paths: paths.into_iter().collect(),
            write_paths: HashSet::new(),
        }
    }
}

/// Network permissions for a tool.
pub struct NetworkPermissions {
    /// Allowed domains (e.g., "api.example.com"). Empty = no network.
    pub allowed_domains: HashSet<String>,
    /// Allowed ports (applies to all domains). Default: [443, 80].
    pub allowed_ports: Vec<u16>,
    /// Allow localhost connections. Default: true.
    pub allow_localhost: bool,
    /// Allow private IP ranges. Default: false.
    pub allow_private_ips: bool,
}

impl Default for NetworkPermissions {
    fn default() -> Self {
        Self {
            allowed_domains: HashSet::new(),
            allowed_ports: vec![443, 80],      // Default: HTTPS and HTTP
            allow_localhost: true,
            allow_private_ips: false,
        }
    }
}

/// Subprocess policy for a tool.
pub enum SubprocessPolicy {
    Denied,
    Allowed,
}
```

Available permissions:
- `filesystem.read_paths` / `filesystem.write_paths` — File system access (HashSet of path patterns)
- `network.allowed_domains` — HTTP requests to allowed domains
- `network.allowed_ports` — Allowed ports (default: [443, 80])
- `network.allow_localhost` — Allow localhost connections (default: true)
- `network.allow_private_ips` — Allow private IP ranges (default: false)
- `subprocess` — Subprocess spawning policy (Power Mode only)

---

## Hot-Loading

```rust
// Watch for file changes and auto-reload
registry.enable_hot_loading().await?;

// Or manually trigger
registry.load_from_script("./new_tool.lua").await?;
```

---

## Custom Tool (Rust)

```rust
use claw_tools::{Tool, ToolResult};
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(JsonSchema, Deserialize)]
struct CalculatorParams {
    a: f64,
    b: f64,
    operation: String,
}

pub struct CalculatorTool;

#[async_trait]
impl Tool for CalculatorTool {
    fn name(&self) -> &str {
        "calculator"
    }
    
    fn schema(&self) -> Value {
        serde_json::to_value(CalculatorParams::schema()).unwrap()
    }
    
    fn permissions(&self) -> PermissionSet {
        PermissionSet::empty()
    }
    
    async fn execute(&self, params: Value) -> Result<ToolResult, ToolError> {
        let params: CalculatorParams = serde_json::from_value(params)?;
        
        let start = Instant::now();
        let result = match params.operation.as_str() {
            "add" => params.a + params.b,
            "subtract" => params.a - params.b,
            _ => return Err(ToolError::invalid_operation(&params.operation)),
        };
        
        Ok(ToolResult {
            output: Some(json!(result)),
            error: None,
            logs: vec![],
            execution_time_ms: start.elapsed().as_millis() as u64,
        })
    }
}
```

---

## Features

```toml
[features]
default = ["hot-loading"]
hot-loading = ["notify"]  # File watching (50ms debounce)
schema-gen = ["schemars"]
```

---
