---
title: claw-pal
description: "Platform Abstraction Layer (sandbox, IPC, process)"
status: design-phase
version: "0.1.0"
last_updated: "2026-02-28"
---

# claw-pal

> **Layer 0 & 0.5: Rust Hard Core + Platform Abstraction Layer (PAL)** — Trust root, cross-platform sandbox, IPC, and process management  
> Rust 硬核核心 + 平台抽象层 (第 0 & 0.5 层) — 信任根、跨平台沙箱、IPC 和进程管理

[English](#english) | [中文](#chinese)

<a name="english"></a>

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

<a name="chinese"></a>

# claw-pal

平台抽象层 — 跨平台沙箱、IPC 和进程管理。

---

## 概述

`claw-pal` 隔离所有平台特定代码，使 claw-kernel 能够在 Linux、macOS 和 Windows 上运行，而无需在整个代码库中散布平台条件判断。

---

## 用法

```toml
[dependencies]
claw-pal = "0.1"
```

```rust
use claw_pal::{SandboxBackend, SandboxConfig, ExecutionMode};

// 创建安全模式沙箱
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

## 模块

### `sandbox`

跨平台沙箱：

| 平台 | 实现 |
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

进程间通信：

```rust
use claw_pal::ipc::IpcTransport;

// 服务端
let listener = IpcTransport::listen("/tmp/my-socket").await?;
let conn = listener.accept().await?;

// 客户端  
let conn = IpcTransport::connect("/tmp/my-socket").await?;
conn.send(b"hello").await?;
```

### `process`

进程管理：

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

配置目录：

```rust
use claw_pal::dirs;

let config = dirs::config_dir();   // ~/.config/claw-kernel/
let data = dirs::data_dir();       // ~/.local/share/claw-kernel/
let cache = dirs::cache_dir();     // ~/.cache/claw-kernel/
```

---

## 平台支持

| 特性 | Linux | macOS | Windows |
|---------|:-----:|:-----:|:-------:|
| 沙箱 | Yes 强 | Yes 中等 | Yes 中等 |
| IPC | Yes UDS | Yes UDS | Yes 命名管道 |
| 进程 | Yes 完整 | Yes 完整 | Yes 完整 |

---

## 另请参阅

- [PAL 架构](../architecture/pal.md)
- [平台指南](../platform/)
