---
title: Linux Platform Guide
description: Linux platform guide (seccomp-bpf + Namespaces)
status: implemented
version: "0.1.0"
last_updated: "2026-03-08"
language: en
---


# Linux Platform Guide

Linux provides the strongest sandboxing capabilities for claw-kernel through seccomp-bpf and namespaces.

---

## Architecture Position

This document describes the **Layer 0.5: Platform Abstraction Layer (PAL)** implementation for Linux.

claw-kernel uses a 5-layer architecture:
- **Layer 0**: Rust Hard Core — Platform-agnostic trust root
- **Layer 0.5**: Platform Abstraction Layer (PAL) — Platform-specific code (this document)
- **Layer 1-3**: System Runtime / Agent Kernel Protocol / Script Runtime — Platform-agnostic, use PAL via traits

> **Zero Platform Assumptions**: All code at Layer 0-3 is platform-agnostic. Only PAL (Layer 0.5) contains platform-specific implementations. Linux-specific sandbox, IPC, and configuration directory code is isolated in the `claw-pal` crate's Linux module.

---

## 架构位置

本文档描述 **Layer 0.5: Platform Abstraction Layer (PAL)** 的 Linux 实现。

claw-kernel 采用五层架构：
- **Layer 0**: Rust Hard Core — 平台无关的信任根
- **Layer 0.5**: Platform Abstraction Layer (PAL) — 平台特定代码（本文档）
- **Layer 1-3**: System Runtime / Agent Kernel Protocol / Script Runtime — 平台无关，通过 PAL trait 使用平台功能

> **Zero Platform Assumptions**: Layer 0-3 的所有代码都是平台无关的。只有 PAL (Layer 0.5) 包含平台特定实现。Linux 特定的沙箱、IPC 和配置目录代码都隔离在 `claw-pal` crate 的 Linux 模块中。

---

## Requirements

- Linux kernel 4.15+ (5.0+ recommended)
- Rust toolchain (stable)
- `libseccomp-dev` (for seccomp development)

---

## Installation

```bash
# Ubuntu/Debian
sudo apt-get install libseccomp-dev pkg-config

# Fedora/RHEL
sudo dnf install libseccomp-devel

# Arch
sudo pacman -S libseccomp
```

---

## Sandbox Implementation

Linux uses **seccomp-bpf** + **namespaces**:

```rust
// Internal implementation
create_namespaces()?;      // mount, pid, network
setup_seccomp_filter()?;   // syscall filtering
pivot_root()?;             // filesystem isolation
```

### seccomp Filter

Default blocked syscalls in Safe Mode:

```rust
const DANGEROUS_SYSCALLS: &[&str] = &[
    "execve",       // No new processes
    "execveat",
    "ptrace",       // No debugging
    "process_vm_readv",
    "process_vm_writev",
    "mount",        // No filesystem changes
    "umount2",
    "pivot_root",
    "chroot",
    "reboot",       // No system control
    "kexec_load",
    "init_module",  // No kernel modules
    "finit_module",
    "delete_module",
];

const NETWORK_SYSCALLS: &[&str] = &[
    "socket", "connect", "bind", "listen", "accept", "accept4"
];

const EXEC_SYSCALLS: &[&str] = &["execve", "execveat"];
```

Uses `SCMP_ACT_ERRNO(EPERM)` instead of `SCMP_ACT_KILL` to prevent Rust panics
when thread join detects a killed thread.

### Namespaces

| Namespace | Purpose |
|-----------|---------|
| Mount | Filesystem isolation |
| PID | Process isolation |
| Network | Network isolation |
| User | UID/GID mapping (optional) |

---

## Capabilities

### Filesystem

```rust
use claw_pal::{SandboxBackend, SandboxConfig};

let config = SandboxConfig::safe_default();
let mut sandbox = LinuxSandbox::create(config).unwrap();

sandbox.restrict_filesystem(&[
    PathBuf::from("/home/user/data"),
    PathBuf::from("/home/user/output"),
]);
```

### Network

```rust
use claw_pal::types::NetRule;

sandbox.restrict_network(&[
    NetRule::allow_port("api.openai.com".to_string(), 443),
    NetRule::allow("example.com".to_string()),
]);
```

---

## IPC Transport

Linux uses **Unix Domain Sockets (UDS)** for inter-process communication (Layer 0.5 PAL).

```rust
use claw_pal::IpcTransport;

// Create listener
let listener = LocalSocketListener::bind("/tmp/claw-kernel/agent.sock")?;

// Connect
let stream = LocalSocketStream::connect("/tmp/claw-kernel/agent.sock")?;
```

**Characteristics:**
- Path-based: `/tmp/claw-kernel/` or `~/.local/share/claw-kernel/sockets/`
- Performance: ~100% (baseline)
- Security: Filesystem permissions

---

## Configuration Directories

Following the **XDG Base Directory Specification**:

| Type | Environment Variable | Default Path |
|------|---------------------|--------------|
| Config | `XDG_CONFIG_HOME` | `~/.config/claw-kernel/` |
| Data | `XDG_DATA_HOME` | `~/.local/share/claw-kernel/` |
| Cache | `XDG_CACHE_HOME` | `~/.cache/claw-kernel/` |

**Subdirectories:**
- `~/.config/claw-kernel/` — Configuration files
- `~/.local/share/claw-kernel/tools/` — Hot-loaded tool scripts
- `~/.local/share/claw-kernel/scripts/` — Runtime extension scripts
- `~/.local/share/claw-kernel/agents/` — Agent runtime data
- `~/.cache/claw-kernel/` — Temporary cache

---

## Testing

### Run Tests

```bash
# All tests
cargo test --workspace

# Sandbox-specific tests
cargo test --features sandbox-tests

# With user namespace disabled
unshare -U cargo test
```

### Verify Sandbox

```bash
# Check seccomp is active
cat /proc/self/status | grep Seccomp
# Should show: Seccomp: 2

# Check namespaces
ls -la /proc/self/ns/
```

---

## Troubleshooting

### "seccomp load failed"

Your kernel may not support seccomp-bpf:

```bash
# Check kernel support
cat /boot/config-$(uname -r) | grep CONFIG_SECCOMP
# Should show: CONFIG_SECCOMP=y
```

### "namespace setup failed"

User namespaces may be disabled:

```bash
# Check if enabled
sysctl kernel.unprivileged_userns_clone
# If 0, enable with:
sudo sysctl kernel.unprivileged_userns_clone=1
```

### AppArmor/SELinux Conflicts

If sandbox behaves unexpectedly:

```bash
# Check for denials
sudo dmesg | grep -i apparmor
sudo ausearch -m avc -ts recent  # SELinux

# Temporarily disable (testing only)
sudo aa-disable claw-kernel  # AppArmor
setenforce 0                  # SELinux
```

---

## Performance

Linux provides the best performance:

| Metric | Value |
|--------|-------|
| Sandbox overhead | <1ms |
| IPC latency | TBD (UDS) |
| Context switch | Fastest (native) |

---

## Docker Integration

Run claw-kernel in Docker for additional isolation:

```dockerfile
FROM rust:1.83-slim

RUN apt-get update && apt-get install -y libseccomp-dev

COPY . /app
WORKDIR /app
RUN cargo build --release

ENTRYPOINT ["./target/release/my-agent"]
```

```bash
docker run --security-opt seccomp=unconfined my-agent
```

---

## See Also

- [PAL Architecture](../architecture/pal.md)
- [macOS Guide](macos.md)
- [Windows Guide](windows.md)

---
