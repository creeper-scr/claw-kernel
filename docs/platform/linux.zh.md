---
title: Linux 平台指南
description: Linux platform guide (seccomp-bpf + Namespaces)
status: design-phase
version: "0.1.0"
last_updated: "2026-03-01"
language: zh
---

[English →](linux.md)

# Linux 平台指南

Linux 通过 seccomp-bpf 和命名空间为 claw-kernel 提供了最强大的沙箱功能。

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

## 沙箱实现

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

# 沙箱特定测试
cargo test --features sandbox-tests

# 禁用用户命名空间时测试
unshare -U cargo test
```

### 验证沙箱

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

如果沙箱行为异常：

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
| 沙箱开销 | <1ms per syscall* |

*测试条件：Intel i7-1165G7, Ubuntu 22.04, seccomp-bpf 过滤模式
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
