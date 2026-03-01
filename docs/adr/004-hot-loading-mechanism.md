---
title: ADR-004: Tool Hot-Loading
status: accepted
date: 2026-02-28
type: adr
last_updated: "2026-03-01"
language: en
---

[中文版 →](004-hot-loading-mechanism.zh.md)

# ADR 004: Tool Hot-Loading as Extension Infrastructure

**Status:** Accepted  
**Date:** 2024-02-01  
**Deciders:** claw-kernel core team

---

## Context

Hot-loading is a core kernel capability for runtime extensibility. It enables:
1. Loading new tools (scripts) without restart
2. Updating existing tools dynamically
3. Immediate availability after loading

This capability serves as infrastructure for higher-level features, but **the kernel itself does not dictate**:
- What content should be hot-loaded
- When hot-loading should occur
- Who decides to trigger hot-loading

These decisions belong to the **application layer** (e.g., a self-evolving system, a plugin manager, or a development tool).

---

## Decision

Implement **file-system based hot-loading** as a kernel service with the following flow:

```
Application decides to load tool
            │
            ▼
Write tool script ──► ~/.local/share/claw-kernel/tools/
            │                      │
            │                      ▼
            │               File system watcher (notify crate)
            │                      │
            ▼                      ▼
ToolRegistry validates ◄─── File change detected
            │
            ▼
ScriptEngine compiles
            │
            ▼
Tool registered & available immediately
```

**Kernel Responsibility Boundary:**
- Yes Provide hot-loading mechanism (watcher, validation, compilation)
- Yes Ensure safe execution (sandbox, permission audit)
- No Decide what/when to load (application layer decision)
- No Implement self-evolution logic (out of scope for kernel)

> **Note on Layer Boundary:** Hot-loading is a **Layer 3 (Extension Foundation)** kernel capability. How applications use this capability (e.g., for self-evolving agents, plugin systems) is an application-layer concern (Layers 4-5), not part of the kernel.

### Key Mechanisms

**1. Watcher-Based Discovery**

```rust
pub struct ToolWatcher {
    watcher: RecommendedWatcher,
    tools_dir: PathBuf,
}

impl ToolWatcher {
    pub async fn run(mut self, registry: Arc<ToolRegistry>) {
        while let Ok(event) = self.rx.recv().await {
            match event.kind {
                EventKind::Create(_) | EventKind::Modify(_) => {
                    for path in event.paths {
                        if path.extension() == Some("lua") {
                            registry.load_from_script(&path).await;
                        }
                    }
                }
                EventKind::Remove(_) => {
                    // Unload removed tools
                }
                _ => {}
            }
        }
    }
}
```

**2. Validation Pipeline**

Before loading, scripts must pass:

1. **Syntax check** — Engine-specific parsing
2. **Permission audit** — Verify declared permissions match Safe Mode policy
3. **Schema validation** — Tool schema must be valid JSON Schema
4. **Sandbox compilation** — Compile in isolated context first

```rust
pub async fn load_from_script(&self, path: &Path) -> Result<()> {
    // 1. Read
    let source = fs::read_to_string(path).await?;
    
    // 2. Syntax
    let ast = self.engine.parse(&source)?;
    
    // 3. Permission audit (Safe Mode only)
    if self.mode == ExecutionMode::Safe {
        let declared = extract_permissions(&ast)?;
        self.audit_permissions(&declared)?;
    }
    
    // 4. Schema validation
    let schema = extract_schema(&ast)?;
    validate_schema(&schema)?;
    
    // 5. Compile
    let compiled = self.engine.compile(&source)?;
    
    // 6. Register
    let tool = ScriptTool::new(compiled, self.bridge.clone());
    self.registry.register(tool)?;
    
    // 7. Emit event
    self.events.emit(Event::ToolLoaded { 
        name: tool.name(),
        source: path.to_path_buf(),
    });
    
    Ok(())
}
```

**3. Version Management**

Tools can be versioned for rollback:

```
~/.local/share/claw-kernel/tools/
├── file_search/
│   ├── v1/
│   │   └── tool.lua
│   ├── v2/
│   │   └── tool.lua
│   └── current -> v2/
└── web_scraper/
    └── ...
```

---

## Consequences

### Positive (Kernel Level)

- **Runtime extensibility:** No restart required for tool updates
- **Clean separation:** Kernel provides capability, application decides usage
- **Version control friendly:** Tools are just files in git
- **Debugging:** Edit script, save, immediate test
- **Flexible deployment:** Supports manual, automated, or AI-driven tool management (at application layer)

### Negative (Kernel Level)

- **File system dependency:** Requires writable directory
- **Race conditions:** Multiple processes writing simultaneously
- **Orphaned tools:** Removed from disk but still in memory

### Mitigations

- Lock files for concurrent writes
- TTL-based cleanup for orphaned tools
- Clear documentation of kernel/application boundaries

---

## Alternatives Considered

### Alternative 1: In-Memory Only

**Rejected:** Lost on restart, no persistence

### Alternative 2: Database Storage

**Rejected:** Adds dependency, harder to version control

### Alternative 3: Compile to Shared Library

**Rejected:** Platform-specific (.so/.dll/.dylib), complex build

---

## Implementation Details

### Tool Script Format (Lua Example)

```lua
-- file_search.lua
-- @name file_search
-- @description Search files by pattern
-- @permissions fs.read
-- @schema {
--   "type": "object",
--   "properties": {
--     "pattern": { "type": "string" },
--     "directory": { "type": "string" }
--   },
--   "required": ["pattern"]
-- }

local M = {}

function M.execute(params)
    local pattern = params.pattern
    local directory = params.directory or "."
    
    -- Use RustBridge for filesystem access
    local files = rust.fs.glob(directory, pattern)
    
    return {
        success = true,
        result = files
    }
end

return M
```

### Hot-Loading vs Cold-Start

| Aspect | Hot-Loading | Cold-Start |
|--------|-----------|------------|
| Latency | ~10-100ms | ~100-500ms |
| State preserved | Yes | No |
| Memory leaks possible | Yes (mitigated by TTL) | No |
| Use case | Development, dynamic updates | Production stability |

---

## Usage Example

```rust
// Application manually triggers hot-loading
let kernel = Kernel::new();
kernel.tools().load_from_path("./my_tool.lua").await?;
```

> **Note:** The kernel provides the `load_from_path` API as infrastructure. How applications decide *when* and *what* to load (e.g., implementing self-evolving systems, plugin managers) is outside the kernel scope.

---

## References

- [Writing Tools Guide](../guides/writing-tools.md)
- [claw-tools crate docs](../crates/claw-tools.md)

---
