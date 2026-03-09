---
title: claw-pal
description: Platform Abstraction Layer (sandbox, IPC, process)
status: implemented
version: "0.1.0"
last_updated: "2026-03-09"
language: en
---



Platform Abstraction Layer — Cross-platform sandbox, IPC, and process management.

---

## Overview

`claw-pal` isolates all platform-specific code, enabling claw-kernel to run on Linux, macOS, and Windows without platform conditionals scattered throughout the codebase.

### Layer 0: Rust Hard Core

The **trust root** of the system — immutable, never hot-patched, no platform assumptions.

**Responsibilities:**
- Process lifecycle management
- Secure credential storage (in-memory encryption)
- Script engine initialization
- Mode switching guards (Safe ↔ Power)

**Key Constraint:** This layer cannot be modified by scripts. Ever.

### Layer 0.5: Platform Abstraction Layer (PAL)

Isolates platform-specific code from the rest of the system.

**Components:**
- `SandboxBackend` — Platform sandbox implementations
- `IpcTransport` — Cross-platform IPC
- `ProcessManager` — Process lifecycle

---

## Usage

```toml
[dependencies]
claw-pal = "0.1"
```

```rust
use claw_pal::{SandboxBackend, SandboxConfig, ExecutionMode, NetRule, ResourceLimits};
use claw_pal::traits::sandbox::SyscallPolicy;
use std::path::PathBuf;

// Create safe mode sandbox configuration
let config = SandboxConfig::safe_default();

// Platform-specific sandbox implementations
#[cfg(target_os = "linux")]
{
    use claw_pal::LinuxSandbox;
    let mut sandbox = LinuxSandbox::create(config).unwrap();
    
    // Configure restrictions
    sandbox
        .restrict_filesystem(&[PathBuf::from("/data")])
        .restrict_network(&[NetRule::allow_port("api.example.com".to_string(), 443)])
        .restrict_syscalls(SyscallPolicy::DenyAll)
        .restrict_resources(ResourceLimits::restrictive());
    
    // Apply the sandbox
    let handle = sandbox.apply()?;
}

#[cfg(target_os = "macos")]
{
    use claw_pal::MacOSSandbox;
    let mut sandbox = MacOSSandbox::create(config).unwrap();
    // ... configure and apply
}
```

---

## Modules

### `sandbox`

Cross-platform sandboxing:

| Platform | Implementation | Status |
|----------|---------------|--------|
| Linux | seccomp-bpf + namespaces | ✅ Implemented |
| macOS | sandbox(7) profile | ✅ Implemented |
| Windows | AppContainer + Job Objects | ⚠️ Stub (v0.2.0) |

```rust
use claw_pal::traits::SandboxBackend;
use claw_pal::types::{SandboxConfig, ExecutionMode, ResourceLimits};
use claw_pal::traits::sandbox::SyscallPolicy;

pub trait SandboxBackend: Send + Sync {
    fn create(config: SandboxConfig) -> Result<Self, SandboxError> where Self: Sized;
    fn restrict_filesystem(&mut self, whitelist: &[PathBuf]) -> &mut Self;
    fn restrict_network(&mut self, rules: &[NetRule]) -> &mut Self;
    fn restrict_syscalls(&mut self, policy: SyscallPolicy) -> &mut Self;
    fn restrict_resources(&mut self, limits: ResourceLimits) -> &mut Self;
    fn apply(self) -> Result<SandboxHandle, SandboxError>;
}

// SandboxConfig fields:
pub struct SandboxConfig {
    pub mode: ExecutionMode,                  // Safe or Power
    pub filesystem_allowlist: Vec<PathBuf>,   // Allowed paths
    pub network_rules: Vec<NetRule>,          // Network rules
    pub allow_subprocess: bool,               // Subprocess permission
}

// Network rule:
pub struct NetRule {
    pub host: String,                         // Hostname or IP
    pub port: Option<u16>,                    // Port (None = all)
    pub allow: bool,                          // Allow or deny
}

// Resource limits:
pub struct ResourceLimits {
    pub max_memory_bytes: Option<u64>,
    pub max_cpu_percent: Option<u8>,
    pub max_file_descriptors: Option<u32>,
    pub max_processes: Option<u32>,
}

// Syscall policies:
pub enum SyscallPolicy {
    AllowAll,
    DenyAll,
    Allowlist(Vec<String>),                   // Syscall names
}
```

### `ipc`

Inter-process communication:

> **v1.0.0**: Unix Domain Sockets (Linux/macOS) and Named Pipes (Windows) are fully implemented.

```rust
use claw_pal::IpcTransport;
use claw_pal::ipc::InterprocessTransport;

// Server
let transport = InterprocessTransport::new_server("/tmp/my-socket").await?;

// Client  
let transport = InterprocessTransport::new_client("/tmp/my-socket").await?;
transport.send(b"hello").await?;
let response = transport.recv().await?;
```

**IPC Framing Protocol:**
- 4-byte Big Endian length prefix
- Maximum payload: 16 MiB

**Testing with Mock Transport:**

```rust
use claw_pal::traits::ipc::MockIpcTransport;

let transport = MockIpcTransport::new("/tmp/test".to_string());
transport.send(b"test message").await?;
let msg = transport.recv().await?;
```

### `process`

Process management:

```rust
use claw_pal::{ProcessManager, TokioProcessManager};
use claw_pal::types::process::ProcessConfig;

let manager = TokioProcessManager::new();
let handle = manager.spawn(ProcessConfig {
    program: "worker".to_string(),
    args: vec!["--task".to_string(), "1".to_string()],
    env: HashMap::new(),
    working_dir: None,
}).await?;

// Graceful termination with timeout
manager.terminate(handle, Duration::from_secs(5)).await?;

// Force kill
manager.kill(handle).await?;

// Wait for exit
let status = manager.wait(handle).await?;
```

### `dirs`

Configuration directories:

```rust
use claw_pal::dirs;

let config = dirs::config_dir();   // ~/.config/claw-kernel/
let data = dirs::data_dir();       // ~/.local/share/claw-kernel/
let cache = dirs::cache_dir();     // ~/.cache/claw-kernel/
let tools = dirs::tools_dir();     // ~/.local/share/claw-kernel/tools/
let scripts = dirs::scripts_dir(); // ~/.local/share/claw-kernel/scripts/
let logs = dirs::logs_dir();       // ~/.local/share/claw-kernel/logs/
```

### `security`

Power Key management and mode transition guards:

```rust
use claw_pal::security::{PowerKeyValidator, PowerKeyHash, PowerKeyManager};

// Validate a power key
PowerKeyValidator::validate("SecureKey123!")?;

// Create a hashed power key
let hash = PowerKeyHash::new("SecureKey123!")?;

// Save to config file
PowerKeyManager::save_power_key("SecureKey123!")?;
```

---

## Error Types

```rust
// Sandbox errors
pub enum SandboxError {
    CreationFailed(String),
    RestrictFailed(String),
    AlreadyApplied,
    NotSupported,
}

// IPC errors
pub enum IpcError {
    ConnectionRefused,
    Timeout,
    BrokenPipe,
    InvalidMessage,
    PermissionDenied,
}

// Process errors
pub enum ProcessError {
    SpawnFailed(String),
    SignalFailed(String),
    NotFound(u32),
    PermissionDenied,
    InvalidSignal,
}
```

---

## Platform Support

| Feature | Linux | macOS | Windows |
|---------|:-----:|:-----:|:-------:|
| Sandbox | ✅ Strong | ✅ Medium | ⚠️ Stub |
| IPC (UDS/NamedPipe) | ✅ UDS | ✅ UDS | ✅ Named Pipes |
| Process | ✅ Full | ✅ Full | ✅ Full |

---

## See Also

- [PAL Architecture](../architecture/pal.md)
- [Platform Guides](../platform/)

---
