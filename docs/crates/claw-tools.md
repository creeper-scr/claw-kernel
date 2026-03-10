---
title: claw-tools
description: Tool registry, hot-loading, schema generation
status: implemented
version: "0.1.0"
last_updated: "2026-03-09"
language: en
---



Tool registry and hot-loading for agent capabilities.

---

## Overview

`claw-tools` implements the tool-use protocol:
- Tool registration and discovery
- Schema generation and validation
- Hot-loading from scripts (placeholder, requires ScriptEngine integration)
- Permission management

---

## Usage

```toml
[dependencies]
claw-tools = "0.1"
```

```rust
use claw_tools::{ToolRegistry, Tool, ToolContext, PermissionSet};

let registry = ToolRegistry::new();

// Register a native tool
registry.register(Box::new(MyTool::new()))?;

// Execute tool
let result = registry.execute(
    "calculator",
    json!({"operation": "add", "a": 1, "b": 2}),
    ToolContext::new("agent-1", PermissionSet::minimal())
).await?;
```

---

## Core Components

### `Tool` Trait

The core abstraction for executable capabilities:

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    /// Unique tool name (snake_case).
    fn name(&self) -> &str;
    
    /// Human-readable description shown to the LLM.
    fn description(&self) -> &str;
    
    /// JSON Schema for input parameters.
    fn schema(&self) -> &ToolSchema;
    
    /// Permissions required by this tool.
    fn permissions(&self) -> &PermissionSet;
    
    /// Maximum execution time. Default: 30 seconds.
    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
    
    /// Execute the tool with the given JSON arguments.
    async fn execute(&self, args: serde_json::Value, ctx: &ToolContext) -> ToolResult;
}
```

### `ToolRegistry`

Central registry for tool discovery and execution:

```rust
pub struct ToolRegistry { /* ... */ }

impl ToolRegistry {
    pub fn new() -> Self;
    pub fn with_max_audit_entries(self, max: usize) -> Self;
    pub fn register(&self, tool: Box<dyn Tool>) -> Result<(), RegistryError>;
    pub fn unregister(&self, name: &str) -> Result<(), RegistryError>;
    pub fn update(&self, name: &str, tool: Arc<dyn Tool>) -> Result<(), RegistryError>;
    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>>;
    pub fn tool_names(&self) -> Vec<String>;
    pub fn tool_count(&self) -> usize;
    pub fn tool_meta(&self, name: &str) -> Option<ToolMeta>;
    pub async fn execute(&self, name: &str, args: serde_json::Value, ctx: ToolContext) 
        -> Result<ToolResult, RegistryError>;
    pub async fn recent_log(&self, n: usize) -> Vec<LogEntry>;
}
```

### Schema Generation

Tools declare their interface via JSON Schema:

```rust
use schemars::JsonSchema;
use serde::Deserialize;

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

impl PermissionSet {
    /// No permissions (read-only, no network, no subprocess).
    pub fn minimal() -> Self {
        Self {
            filesystem: FsPermissions::none(),
            network: NetworkPermissions::none(),
            subprocess: SubprocessPolicy::Denied,
        }
    }
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

## Error Types

```rust
use claw_tools::{LoadError, WatchError};

// Loading errors (from load_from_script/load_from_directory)
let err = LoadError::ParseError {
    path: "/tools/my_tool.lua".to_string(),
    message: "Invalid syntax at line 10".to_string(),
};

// Watch errors (from enable_hot_loading)
let err = WatchError::InvalidConfig("watch_dirs cannot be empty".to_string());
```

---

## Hot-Loading

> **⚠️ Note:** Hot-loading in `claw-tools` is currently a **placeholder implementation**. 
> The kernel provides the foundation APIs, but actual script compilation and execution 
> requires integration with a ScriptEngine (Layer 3 responsibility).
> 
> See [`claw-script`](claw-script.md) for the script engine integration.

The hot-reload system provides file watching, debouncing, and atomic hot-swapping capabilities.

### Architecture

```text
File System ──► FileWatcher ──► WatchEvent ──► HotReloadProcessor ──► ToolRegistry
                   │                              │
                   └─ debounce (50ms)             └─ compile & hot-swap
```

### Configuration

```rust
use claw_tools::types::HotLoadingConfig;

let config = HotLoadingConfig {
    watch_dirs: vec![PathBuf::from("./tools")],
    extensions: vec!["lua".to_string()],
    debounce_ms: 50,
    default_timeout_secs: 30,
    compile_timeout_secs: 10,
    keep_previous_secs: 300,
    auto_enable: true,
};

// Validate configuration
config.validate()?;
```

### Registry Hot-Loading API

```rust
impl ToolRegistry {
    /// Load a tool from a script file (metadata only, placeholder implementation).
    /// 
    /// Note: Actual script compilation requires ScriptEngine integration.
    pub async fn load_from_script(&self, path: &Path) -> Result<ToolMeta, LoadError>;
    
    /// Load all tools from a directory.
    pub async fn load_from_directory(&self, path: &Path) -> Result<Vec<ToolMeta>, LoadError>;
    
    /// Unload a script tool.
    pub fn unload(&self, name: &str) -> Result<(), RegistryError>;
    
    /// Enable hot-loading (placeholder, validates config only).
    /// 
    /// Note: Actual implementation requires ScriptEngine integration.
    pub async fn enable_hot_loading(&self, config: HotLoadingConfig) -> Result<(), WatchError>;
    
    /// Disable hot-loading.
    pub async fn disable_hot_loading(&self);
}
```

### Full Hot-Reload Setup (Application Layer)

For complete hot-reload functionality, the application layer needs to wire up the components:

```rust
use std::sync::Arc;
use claw_tools::hot_reload::{FileWatcher, HotReloadProcessor};
use claw_tools::{ToolRegistry, HotLoadingConfig};
use tokio::sync::mpsc;

let registry = Arc::new(ToolRegistry::new());
let config = HotLoadingConfig::default();

// Create watcher
let mut watcher = FileWatcher::new(&config)?;

// Channel for events
let (tx, rx) = mpsc::channel(32);

// Start processor
let processor = HotReloadProcessor::new(registry, config);
tokio::spawn(async move {
    processor.run(rx).await;
});

// Forward events
tokio::spawn(async move {
    while let Some(event) = watcher.recv().await {
        let _ = tx.send(event).await;
    }
});
```

### Key Components

| Component | Purpose |
|-----------|---------|
| `FileWatcher` | Watches directories for file changes with debouncing |
| `HotReloadProcessor` | Processes watch events and performs hot-swaps |
| `VersionedModule` | Manages versioned tool modules for atomic swaps |
| `VersionedToolSet` | Manages a collection of versioned tools |
| `ToolWatcher` | High-level validation watcher for tool scripts |

---

## Custom Tool (Rust)

```rust
use claw_tools::{Tool, ToolSchema, ToolContext, ToolResult, ToolError};
use claw_tools::{PermissionSet, FsPermissions, NetworkPermissions};
use async_trait::async_trait;
use std::time::Duration;

pub struct CalculatorTool {
    schema: ToolSchema,
    permissions: PermissionSet,
}

impl CalculatorTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema::new(
                "calculator",
                "Performs arithmetic operations",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "operation": { "type": "string", "enum": ["add", "subtract"] },
                        "a": { "type": "number" },
                        "b": { "type": "number" }
                    },
                    "required": ["operation", "a", "b"]
                }),
            ),
            permissions: PermissionSet::minimal(),
        }
    }
}

#[async_trait]
impl Tool for CalculatorTool {
    fn name(&self) -> &str {
        "calculator"
    }
    
    fn description(&self) -> &str {
        "Performs arithmetic operations"
    }
    
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
    
    fn permissions(&self) -> &PermissionSet {
        &self.permissions
    }
    
    async fn execute(&self, args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let start = Instant::now();
        
        let operation = args.get("operation").and_then(|v| v.as_str()).unwrap_or("");
        let a = args.get("a").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let b = args.get("b").and_then(|v| v.as_f64()).unwrap_or(0.0);
        
        let result = match operation {
            "add" => a + b,
            "subtract" => a - b,
            _ => {
                return ToolResult::err(
                    ToolError::invalid_args(format!("Unknown operation: {}", operation)),
                    start.elapsed().as_millis() as u64
                );
            }
        };
        
        ToolResult::ok(serde_json::json!(result), start.elapsed().as_millis() as u64)
    }
}
```

---

## Audit Logging

The registry maintains an in-memory audit log of all tool executions:

```rust
// Get recent log entries
let recent = registry.recent_log(100).await;

// Configure max audit entries
let registry = ToolRegistry::new().with_max_audit_entries(10_000);
```

---

## Features

```toml
[features]
default = []
```

> **Note:** Hot-loading support is built-in and does not require a feature flag. The `notify` crate is always included for file watching capabilities.
