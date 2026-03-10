---
title: Safe Mode Guide
description: Safe mode configuration and sandboxing guide
status: implemented
version: "0.1.0"
last_updated: "2026-03-08"
language: en
---

> **⚠️ Security Warning / 安全警告**
> 
> The Safe Mode sandbox implementation is **incomplete and platform-dependent**:
> - ✅ Linux: Full seccomp-bpf + namespaces implementation
> - ✅ macOS: sandbox profile implementation (limited syscall filtering)
> - ⚠️ Windows: Job Objects only — filesystem and network restrictions **not enforced** until v1.5.0
>
> **Contributions welcome!** We actively seek contributions for:
> - Windows AppContainer sandbox implementation
> - Security audit and penetration testing
> - Sandbox escape vulnerability reports
> 
> See [CONTRIBUTING.md](../../CONTRIBUTING.md) and [SECURITY.md](../../SECURITY.md) for details.

# Safe Mode Guide

Safe Mode is the kernel's sandbox feature (Layer 0.5). It provides sandboxed execution suitable for running LLM-generated code safely.

> **Note**: This guide describes the current PAL (Platform Abstraction Layer) implementation in `claw-pal` crate.

---

## What is Safe Mode?

Safe Mode restricts script capabilities through sandboxing:

| Capability | Safe Mode | Power Mode |
|------------|-----------|------------|
| **File System** | Allowlisted directories, read-only by default | Full access |
| **Network** | Allowed domains/ports only | Unrestricted |
| **Subprocesses** | Blocked | Allowed |
| **System Calls** | Filtered | Unrestricted |
| **Script Hot-Loading** | Allowed (subject to sandbox limits) | Allowed (global) |

---

## Two-Layer Permission Model

Safe Mode implements a **two-layer permission model**:

### Layer 1: Sandbox Permissions (Hard Constraints)
OS-level enforcement restricts what scripts *can* do:
- Filesystem allowlist
- Network domain/port rules  
- Subprocess blocking
- System call filtering

### Layer 2: Tool Declaration (Runtime Check)
Scripts declare permissions via `@permissions` annotation:
- Provides visibility to LLM (what the tool *may* do)
- Runtime validation against sandbox configuration
- **Static error if tool declares permissions beyond sandbox scope**

### Permission Resolution
```
Effective Permission = Tool Declaration ∩ Sandbox Configuration
```

| Scenario | Tool Declaration | Sandbox Config | Result |
|----------|------------------|----------------|--------|
| Yes Consistent | `fs.read` | `/home/user` readable | Works |
| Yes Tool more restrictive | `fs.read` (declares only) | `/home/user` readable | Works |
| No Tool exceeds sandbox | `fs.write` | Read-only | **Static error at registration** |

### Tool Registration Time Check

Permission validation happens **immediately when tool is registered** (not at call time):

```rust
// Tool permissions are checked at registration time
let mut tools = ToolRegistry::new();

// If any tool in ./tools declares permissions beyond sandbox config,
// load_from_directory fails immediately with PermissionError
tools.load_from_directory("./tools").await?;

// Once loaded, all tools are guaranteed to have valid permissions
// No runtime permission checks needed during execution
```

This ensures that permission mismatches are caught early during application startup, not during tool execution.

### Security Policy

| Layer | Responsibility |
|-------|---------------|
| **Kernel** | Sandbox isolation - restricts what scripts *can* do |
| **Application** | Permission decisions - determines what scripts *may* do |

The kernel provides the sandbox mechanism. The application decides which directories, network endpoints, and capabilities to allow.

---

## Permission Inheritance (is_subset)

Safe Mode uses `is_subset` checks to enforce permission inheritance — a tool can only request permissions that are a subset of what the context (execution environment) grants.

### How It Works

The `is_subset` check validates that every permission requested by a tool is contained within the permissions granted by the execution context:

```
Tool Permitted ⊆ Context Granted → Execution Allowed
Tool Permitted ⊄ Context Granted → Permission Denied
```

This is a **set containment check**, not a path prefix check. The tool's permission set must be entirely contained within the context's permission set.

### Example: Filesystem Permissions

```rust
use claw_tools::types::{PermissionSet, FsPermissions};

// Context grants read access to /data only
let ctx_permissions = PermissionSet {
    filesystem: FsPermissions::read_only(vec!["/data".to_string()]),
    ..PermissionSet::minimal()
};

// Tool A: Requests access to /data/subdir
// Result: DENIED — /data/subdir is not in the context's allowed set
let tool_a_permissions = PermissionSet {
    filesystem: FsPermissions::read_only(vec!["/data/subdir".to_string()]),
    ..PermissionSet::minimal()
};

// Tool B: Requests access to exactly /data
// Result: ALLOWED — tool's permission set is a subset of context's
let tool_b_permissions = PermissionSet {
    filesystem: FsPermissions::read_only(vec!["/data".to_string()]),
    ..PermissionSet::minimal()
};

// Tool C: Requests access to /data and /tmp
// Result: DENIED — /tmp is not in context's allowed set
let tool_c_permissions = PermissionSet {
    filesystem: FsPermissions::read_only(vec!["/data".to_string(), "/tmp".to_string()]),
    ..PermissionSet::minimal()
};
```

### Key Points

1. **Exact Match Required**: The context must explicitly list every path the tool wants to access. Subdirectories are **not** automatically allowed just because their parent is allowed.

2. **Set Containment, Not Path Prefix**: 
   - ❌ `/data` does **not** grant access to `/data/subdir`
   - ✅ `/data/subdir` must be explicitly added to the context's allowlist

3. **Applies to All Permission Types**: This same `is_subset` logic applies to:
   - Filesystem read/write paths
   - Network allowed domains
   - Subprocess policy

### Permission Resolution Flow

```
┌─────────────────┐     ┌──────────────────┐     ┌─────────────────┐
│  Tool declares  │────▶│  is_subset check │────▶│  Execute tool   │
│  permissions    │     │  against context │     │  if passed      │
└─────────────────┘     └──────────────────┘     └─────────────────┘
                               │
                               ▼ (if failed)
                        ┌──────────────────┐
                        │  PermissionDenied│
                        │  error returned  │
                        └──────────────────┘
```

### Why Subdirectories Are Not Inherited

While path prefix checks might seem intuitive, set-based containment provides:

- **Predictability**: No ambiguity about what is allowed
- **Explicit security**: Each path must be intentionally granted
- **Simpler auditing**: The allowed set is exactly what was configured
- **No symlink traversal issues**: Path prefix checks can be fooled by symlinks

---

## Default Allowlist

### File System

```
Linux/macOS:
  ~/.local/share/claw-kernel/      # Data directory
  ~/.cache/claw-kernel/            # Cache directory
  /tmp/                            # Temp files

Windows:
  %APPDATA%\claw-kernel\           # Data directory
  %LOCALAPPDATA%\claw-kernel\cache\ # Cache directory
  %TEMP%\                          # Temp files
```

### Network

```
Allowed domains (default):
  - api.openai.com:443
  - api.anthropic.com:443
  - api.gemini.google.com:443
  - localhost:11434 (Ollama default)
```

---

## Configuring Safe Mode

### Programmatic Configuration

```rust
use claw_pal::{SandboxBackend, SandboxConfig};
use claw_pal::types::{NetRule, ResourceLimits};
use claw_pal::traits::sandbox::SyscallPolicy;
use std::path::PathBuf;

// Create a safe mode configuration
let config = SandboxConfig::safe_default();

// Create platform-specific sandbox (Linux example)
#[cfg(target_os = "linux")]
use claw_pal::LinuxSandbox;
#[cfg(target_os = "linux")]
let mut sandbox = LinuxSandbox::create(config)?;

// Configure restrictions using builder pattern
sandbox
    .restrict_filesystem(&[
        PathBuf::from("/home/user/projects"),
        PathBuf::from("/home/user/output"),
    ])
    .restrict_network(&[
        NetRule::allow_port("api.example.com".to_string(), 443),
        NetRule::allow("internal.company.net".to_string()),
    ])
    .restrict_syscalls(SyscallPolicy::DenyAll)
    .restrict_resources(ResourceLimits::restrictive());

// Apply the sandbox
let handle = sandbox.apply()?;
```

### Configuration File

Create `~/.config/claw-kernel/sandbox.toml`:

```toml
[sandbox]
mode = "safe"

[[sandbox.filesystem]]
path = "/home/user/projects"
access = "read"

[[sandbox.filesystem]]
path = "/home/user/output"
access = "read-write"

[[sandbox.network]]
domain = "api.example.com"
ports = [443]

[[sandbox.network]]
domain = "internal.company.net"
ports = [80, 443]
```

---

## Platform-Specific Sandboxing

### Linux (seccomp + namespaces)

Strongest sandboxing:

```rust
// Automatically uses:
// - seccomp-bpf for syscall filtering
// - mount namespace for filesystem isolation
// - network namespace for network rules
// - pid namespace for process isolation
```

### macOS (sandbox profile)

Uses native macOS sandbox:

```rust
// Generates sandbox profile like:
// (version 1)
// (allow default)
// (deny network-outbound)
// (allow network-outbound (remote unix-socket))
// (allow file-read* (subpath "/allowed/path"))
```

### Windows (Job Objects — degraded)

> **v1.4.0**: Windows Safe mode uses **Job Objects** for partial isolation.
> Resource limits and subprocess blocking are enforced. Filesystem and network
> restrictions are **not enforced** until v1.5.0 (AppContainer).

```rust
// Windows Safe mode enforces via Job Object:
// - max_memory_bytes → JOBOBJECT_EXTENDED_LIMIT_INFORMATION::JobMemoryLimit
// - allow_subprocess=false → ActiveProcessLimit=1 (blocks CreateProcess)
// - max_processes → ActiveProcessLimit

// The following are stored but NOT enforced on Windows:
// sandbox.restrict_filesystem(&paths) // → no-op until v1.5.0
// sandbox.restrict_network(&rules)    // → no-op until v1.5.0
```

A non-suppressable `tracing::warn!` is emitted when Safe mode is applied on Windows,
clearly listing what is and is not enforced. For full filesystem and network isolation
on Windows, run the Linux version via WSL2.

---

## Testing Safe Mode

### Verify Restrictions

Create a test tool:

```lua
-- test_restrictions.lua
-- @name test_restrictions
-- @description Test sandbox restrictions
-- @permissions fs.read, net.http

function M.execute(params)
    local results = {}
    
    -- Test 1: Read allowed file
    local success = pcall(function()
        rust.fs.read("~/.local/share/claw-kernel/test.txt")
    end)
    table.insert(results, "Read allowed: " .. tostring(success))
    
    -- Test 2: Read disallowed file (should fail)
    success = pcall(function()
        rust.fs.read("/etc/passwd")
    end)
    table.insert(results, "Read disallowed: " .. tostring(not success))
    
    -- Test 3: Network to allowed domain
    success = pcall(function()
        rust.net.get("https://api.openai.com/v1/models")
    end)
    table.insert(results, "Net allowed: " .. tostring(success))
    
    -- Test 4: Network to disallowed domain (should fail)
    success = pcall(function()
        rust.net.get("https://evil.com/")
    end)
    table.insert(results, "Net disallowed: " .. tostring(not success))
    
    return {
        success = true,
        result = results
    }
end
```

### Expected Output

```
Read allowed: true
Read disallowed: true  (blocked)
Net allowed: true
Net disallowed: true   (blocked)
```

---

## Safe Mode Guarantees

The following are **security guarantees** in Safe Mode on **Linux and macOS**.
Violations are bugs. See the Windows caveat below.

1. **Filesystem Isolation**
   - Cannot read files outside allowlist
   - Cannot write files outside allowlist
   - Cannot escape via symlinks

2. **Network Restrictions**
   - Cannot connect to non-allowed domains
   - Cannot connect on non-allowed ports
   - DNS requests are filtered

3. **Process Restrictions**
   - Cannot spawn subprocesses
   - Cannot execute shell commands
   - Cannot load dynamic libraries outside system paths

4. **Kernel Protection**
   - Cannot modify claw-kernel configuration
   - Cannot access kernel credential storage
   - Cannot start in Power Mode without key

### ⚠️ Windows Safe Mode Caveats (v1.4.0)

On Windows, guarantees 1 and 2 are **NOT enforced**:

| Guarantee | Linux/macOS | Windows v1.4.0 | Windows v1.5.0 (planned) |
|-----------|-------------|----------------|--------------------------|
| Filesystem isolation | ✅ | ❌ not enforced | ✅ AppContainer |
| Network restrictions | ✅ | ❌ not enforced | ✅ AppContainer + WFP |
| Subprocess blocking | ✅ | ✅ Job Object | ✅ |
| Memory limits | ✅ | ✅ Job Object | ✅ |

Windows Safe mode emits a `WARN` log at activation time. If you require filesystem
or network isolation on Windows today, run the Linux version via WSL2.

---

## When Safe Mode Isn't Enough

Safe Mode intentionally restricts capabilities. If your agent needs:

- Installing system packages
- Modifying system configuration
- Accessing arbitrary files
- Running shell commands

Consider:

1. **Power Mode** — Explicit opt-in for full access
2. **Specific permissions** — Add only needed directories/endpoints
3. **Container deployment** — Run entire agent in Docker

---

## Best Practices

### 1. Start Restrictive, Relax as Needed

```rust
// Begin with minimal permissions
let config = SandboxConfig::safe_mode()
    .allow_directory_rw(dirs::data_dir().unwrap())
    .build();

// Add more as agent requires
```

### 2. Audit Tool Permissions

Review what permissions tools request:

```rust
.script_audit(|script_name, permissions| {
    println!("Script '{}' requests: {:?}", script_name, permissions);
    // Return false to block
    true
})
```

### 3. Use Read-Only Where Possible

```rust
// Prefer read-only unless write is necessary
.allow_directory(PathBuf::from("/data"))      // read-only
.allow_directory_rw(PathBuf::from("/output")) // read-write
```

### 4. Monitor Audit Logs

```bash
tail -f ~/.local/share/claw-kernel/logs/audit.log
```

---

## Troubleshooting

### "Permission denied" when reading allowed file

Check:
1. Path is exactly as allowlisted (no symlinks resolving outside)
2. Parent directories have execute permission
3. File exists and is readable

### Network requests blocked to allowed domain

Check:
1. Port is allowlisted (443 for HTTPS)
2. DNS resolution succeeds
3. No HTTPS interception breaking TLS

### Tool fails with cryptic error

Enable debug logging:

```bash
RUST_LOG=claw_pal=debug cargo run
```

---

## See Also

- [Power Mode Guide](power-mode.md) — For full system access
- [Security Policy](../../SECURITY.md) — Security model details
- [Platform-specific guides](../platform/) — OS-specific sandbox behavior

---
