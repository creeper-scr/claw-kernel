[English](#english) | [中文](#chinese)

<a name="english"></a>
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

> **Zero Platform Assumptions**: Layer 0-3 的所有代码都是平台无关的。只有 PAL (Layer 0.5) 包含平台特定实现。Linux 特定的沙盒、IPC 和配置目录代码都隔离在 `claw-pal` crate 的 Linux 模块中。

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
let filter = seccomp::Filter::new()
    .block(Sysno::execve)      // No new processes
    .block(Sysno::execveat)
    .block(Sysno::ptrace)       // No debugging
    .block(Sysno::process_vm_writev)
    .block(Sysno::open_by_handle_at)
    .allow(Sysno::read)         // Allowed with FD checks
    .allow(Sysno::write)
    // ... etc
    .build()?;
```

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
SandboxConfig::safe_mode()
    .allow_directory(PathBuf::from("/home/user/data"))
    .allow_directory_rw(PathBuf::from("/home/user/output"))
```

### Network

```rust
SandboxConfig::safe_mode()
    .allow_endpoint("api.openai.com", 443)
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

<a name="chinese"></a>
# Linux 平台指南

Linux 通过 seccomp-bpf 和命名空间为 claw-kernel 提供了最强大的沙盒功能。

---

## 系统要求

- Linux 内核 4.15+（推荐 5.0+）
- Rust 工具链（稳定版）
- `libseccomp-dev`（用于 seccomp 开发）

---

## 安装

```bash
# Ubuntu/Debian
sudo apt-get install libseccomp-dev pkg-config

# Fedora/RHEL
sudo dnf install libseccomp-devel

# Arch
sudo pacman -S libseccomp
```

---

## 沙盒实现

Linux 使用 **seccomp-bpf** + **命名空间**：

```rust
// 内部实现
create_namespaces()?;      // 挂载、PID、网络命名空间
setup_seccomp_filter()?;   // 系统调用过滤
pivot_root()?;             // 文件系统隔离
```

### seccomp 过滤器

安全模式下默认阻止的系统调用：

```rust
let filter = seccomp::Filter::new()
    .block(Sysno::execve)      // 禁止创建新进程
    .block(Sysno::execveat)
    .block(Sysno::ptrace)       // 禁止调试
    .block(Sysno::process_vm_writev)
    .block(Sysno::open_by_handle_at)
    .allow(Sysno::read)         // 允许带文件描述符检查的读取
    .allow(Sysno::write)
    // ... 等等
    .build()?;
```

### 命名空间

| 命名空间 | 用途 |
|---------|------|
| Mount | 文件系统隔离 |
| PID | 进程隔离 |
| Network | 网络隔离 |
| User | UID/GID 映射（可选） |

---

## 功能

### 文件系统

```rust
SandboxConfig::safe_mode()
    .allow_directory(PathBuf::from("/home/user/data"))
    .allow_directory_rw(PathBuf::from("/home/user/output"))
```

### 网络

```rust
SandboxConfig::safe_mode()
    .allow_endpoint("api.openai.com", 443)
```

---

## 测试

### 运行测试

```bash
# 运行所有测试
cargo test --workspace

# 沙盒特定测试
cargo test --features sandbox-tests

# 禁用用户命名空间时测试
unshare -U cargo test
```

### 验证沙盒

```bash
# 检查 seccomp 是否激活
cat /proc/self/status | grep Seccomp
# 应显示：Seccomp: 2

# 检查命名空间
ls -la /proc/self/ns/
```

---

## 故障排除

### "seccomp load failed"（seccomp 加载失败）

您的内核可能不支持 seccomp-bpf：

```bash
# 检查内核支持
cat /boot/config-$(uname -r) | grep CONFIG_SECCOMP
# 应显示：CONFIG_SECCOMP=y
```

### "namespace setup failed"（命名空间设置失败）

用户命名空间可能被禁用：

```bash
# 检查是否启用
sysctl kernel.unprivileged_userns_clone
# 如果为 0，使用以下命令启用：
sudo sysctl kernel.unprivileged_userns_clone=1
```

### AppArmor/SELinux 冲突

如果沙盒行为异常：

```bash
# 检查拒绝日志
sudo dmesg | grep -i apparmor
sudo ausearch -m avc -ts recent  # SELinux

# 临时禁用（仅测试用）
sudo aa-disable claw-kernel  # AppArmor
setenforce 0                  # SELinux
```

---

## 性能

Linux 提供最佳性能：

| 指标 | 数值 |
|-----|------|
| 沙盒开销 | <1毫秒 |
| IPC 延迟 | ~10微秒（UDS） |
| 上下文切换 | 最快（原生） |

---

## Docker 集成

在 Docker 中运行 claw-kernel 以获得额外隔离：

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

## 另请参阅

- [PAL 架构](../architecture/pal.md)
- [macOS 指南](macos.md)
- [Windows 指南](windows.md)
