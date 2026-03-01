---
title: claw-pal
description: Platform Abstraction Layer (sandbox, IPC, process)
status: implemented
version: "0.1.0"
last_updated: "2026-03-01"
language: en
---

[中文版 →](claw-pal.zh.md)


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
use claw_pal::{SandboxBackend, SandboxConfig, ExecutionMode};

// Create safe mode sandbox
let config = SandboxConfig {
    mode: ExecutionMode::Safe,
    filesystem_allowlist: vec![PathBuf::from("/data")],
    network_rules: vec![NetRule::Allow { 
        domains: vec!["api.example.com"],
        ports: vec![443],
    }],
};

let sandbox = claw_pal::create_sandbox(config)?;
sandbox.apply()?;
```

---

## Modules

### `sandbox`

Cross-platform sandboxing:

| Platform | Implementation |
|----------|---------------|
| Linux | seccomp-bpf + namespaces |
| macOS | sandbox(7) profile |
| Windows | AppContainer + Job Objects |

```rust
use claw_pal::sandbox::SandboxBackend;

pub trait SandboxBackend {
    fn create(config: SandboxConfig) -> Result<Self, SandboxError> where Self: Sized;
    fn restrict_filesystem(&mut self, allowlist: &[PathBuf]) -> &mut Self;
    fn restrict_network(&mut self, rules: &[NetRule]) -> &mut Self;
    fn restrict_syscalls(&mut self, policy: SyscallPolicy) -> &mut Self;
    fn restrict_resources(&mut self, limits: ResourceLimits) -> &mut Self;
    fn apply(self) -> Result<SandboxHandle, SandboxError>;
}
```

### `ipc`

Inter-process communication:

```rust
use claw_pal::ipc::IpcTransport;

// Server
let listener = IpcTransport::listen("/tmp/my-socket").await?;
let conn = listener.accept().await?;

// Client  
let conn = IpcTransport::connect("/tmp/my-socket").await?;
conn.send(b"hello").await?;
```

### `process`

Process management:

```rust
use claw_pal::process::{ProcessManager, ProcessConfig};

let manager = ProcessManager::new();
let handle = manager.spawn(ProcessConfig {
    command: "worker".to_string(),
    args: vec!["--task".to_string(), "1".to_string()],
    sandbox: Some(Box::new(sandbox)),
    ..Default::default()
}).await?;

manager.terminate(handle, Duration::from_secs(5)).await?;
```

### `dirs`

Configuration directories:

```rust
use claw_pal::dirs;

let config = dirs::config_dir();   // ~/.config/claw-kernel/
let data = dirs::data_dir();       // ~/.local/share/claw-kernel/
let cache = dirs::cache_dir();     // ~/.cache/claw-kernel/
```

---

## Platform Support

| Feature | Linux | macOS | Windows |
|---------|:-----:|:-----:|:-------:|
| Sandbox | Yes Strong | Yes Medium | Yes Medium |
| IPC | Yes UDS | Yes UDS | Yes Named Pipe |
| Process | Yes Full | Yes Full | Yes Full |

---

## See Also

- [PAL Architecture](../architecture/pal.md)
- [Platform Guides](../platform/)

---
