[English](#english) | [中文](#chinese)

<a name="english"></a>
# macOS Platform Guide

macOS provides good sandboxing through the native `sandbox(7)` system.

---

## Architecture Position

This document describes the **Layer 0.5: Platform Abstraction Layer (PAL)** implementation for macOS.

claw-kernel uses a 5-layer architecture:
- **Layer 0**: Rust Hard Core — Platform-agnostic trust root
- **Layer 0.5**: Platform Abstraction Layer (PAL) — Platform-specific code (this document)
- **Layer 1-3**: System Runtime / Agent Kernel Protocol / Script Runtime — Platform-agnostic, use PAL via traits

> **Zero Platform Assumptions**: All code at Layer 0-3 is platform-agnostic. Only PAL (Layer 0.5) contains platform-specific implementations. macOS-specific sandbox, IPC, and configuration directory code is isolated in the `claw-pal` crate's macOS module.

---

## 架构位置

本文档描述 **Layer 0.5: Platform Abstraction Layer (PAL)** 的 macOS 实现。

claw-kernel 采用五层架构：
- **Layer 0**: Rust Hard Core — 平台无关的信任根
- **Layer 0.5**: Platform Abstraction Layer (PAL) — 平台特定代码（本文档）
- **Layer 1-3**: System Runtime / Agent Kernel Protocol / Script Runtime — 平台无关，通过 PAL trait 使用平台功能

> **Zero Platform Assumptions**: Layer 0-3 的所有代码都是平台无关的。只有 PAL (Layer 0.5) 包含平台特定实现。macOS 特定的沙盒、IPC 和配置目录代码都隔离在 `claw-pal` crate 的 macOS 模块中。

---

## Requirements

- macOS 10.15+ (11.0+ recommended)
- Xcode Command Line Tools
- Rust toolchain (stable)

---

## Installation

```bash
# Install Xcode Command Line Tools
xcode-select --install

# Verify
clang --version
```

---

## Sandbox Implementation

macOS uses **sandbox profiles**:

```rust
// Generated profile example
(version 1)
(allow default)
(deny network-outbound)
(allow network-outbound 
    (remote unix-socket)
    (remote ip "api.openai.com:443"))
(allow file-read* 
    (subpath "/Users/user/Library/Application Support/claw-kernel"))
(allow file-write*
    (subpath "/Users/user/Library/Caches/claw-kernel"))
```

### Limitations

Compared to Linux:
- No equivalent to seccomp for syscall filtering
- Network filtering is more limited
- File system rules are path-based only

---

## Code Signing

For full sandbox testing, sign your binaries:

```bash
# Generate self-signed certificate (Keychain Access)
# Certificate Assistant → Create a Certificate...
# Name: "claw-kernel-dev", Type: Code Signing

# Sign binary
codesign -s "claw-kernel-dev" --force target/debug/my-agent

# Verify
codesign -dvv target/debug/my-agent
```

---

## Configuration

### Config Directory

```
~/Library/Application Support/claw-kernel/   # Data
~/Library/Caches/claw-kernel/                # Cache
```

### Example

```rust
use claw_kernel::pal::dirs;

let data_dir = dirs::data_dir();
// /Users/<user>/Library/Application Support/claw-kernel/
```

---

## IPC Transport

macOS uses **Unix Domain Sockets (UDS)** for inter-process communication (Layer 0.5 PAL).

```rust
use claw_pal::IpcTransport;

// Create listener
let listener = LocalSocketListener::bind("/tmp/claw-kernel/agent.sock")?;

// Connect
let stream = LocalSocketStream::connect("/tmp/claw-kernel/agent.sock")?;
```

**Characteristics:**
- Path-based: `/tmp/claw-kernel/` or `~/Library/Application Support/claw-kernel/sockets/`
- Performance: ~95% (comparable to Linux)
- Security: Filesystem permissions

---

## Configuration Directories

Following **macOS File System Guidelines**:

| Type | Path |
|------|------|
| Config | `~/Library/Preferences/claw-kernel/` |
| Data | `~/Library/Application Support/claw-kernel/` |
| Cache | `~/Library/Caches/claw-kernel/` |

**Subdirectories:**
- `~/Library/Preferences/claw-kernel/` — Configuration files (plist)
- `~/Library/Application Support/claw-kernel/tools/` — Hot-loaded tool scripts
- `~/Library/Application Support/claw-kernel/scripts/` — Runtime extension scripts
- `~/Library/Application Support/claw-kernel/agents/` — Agent runtime data
- `~/Library/Caches/claw-kernel/` — Temporary cache

```rust
let config_dir = dirs::config_dir();
// /Users/<user>/Library/Preferences/claw-kernel/

let data_dir = dirs::data_dir();
// /Users/<user>/Library/Application Support/claw-kernel/
```

---

## Testing

```bash
# Run tests
cargo test --workspace

# With sandbox tests (requires signed binary)
codesign -s "claw-kernel-dev" target/debug/deps/*
cargo test --features sandbox-tests
```

---

## Troubleshooting

### "sandbox_init failed"

Code signing issue:

```bash
# Check signature
codesign -dvv target/debug/my-agent

# Re-sign
codesign -s "claw-kernel-dev" --force target/debug/my-agent
```

### SIP Interference

System Integrity Protection can block some operations:

```bash
# Check SIP status
csrutil status

# SIP cannot be disabled on Apple Silicon easily
# Use entitlements instead
```

### Gatekeeper

If binary is quarantined:

```bash
# Remove quarantine attribute
xattr -d com.apple.quarantine target/debug/my-agent
```

---

## Performance

| Metric | Value |
|--------|-------|
| Sandbox overhead | ~1-2ms |
| IPC latency | TBD (UDS) |
| Context switch | Good (native) |

Slightly slower than Linux due to sandbox profile compilation.

---

## Notarization (Distribution)

For distributing your agent:

```bash
# Sign with Developer ID
codesign -s "Developer ID Application: Your Name" \
    --options runtime \
    --entitlements entitlements.plist \
    target/release/my-agent

# Create DMG
# ... 

# Notarize
xcrun notarytool submit my-agent.dmg --wait
```

---

## See Also

- [PAL Architecture](../architecture/pal.md)
- [Linux Guide](linux.md)
- [Windows Guide](windows.md)

---

<a name="chinese"></a>
# macOS 平台指南

macOS 通过原生 `sandbox(7)` 系统提供良好的沙盒功能。

---

## 系统要求

- macOS 10.15+（推荐 11.0+）
- Xcode 命令行工具
- Rust 工具链（稳定版）

---

## 安装

```bash
# 安装 Xcode 命令行工具
xcode-select --install

# 验证
clang --version
```

---

## 沙盒实现

macOS 使用**沙盒配置文件**：

```rust
// 生成的配置文件示例
(version 1)
(allow default)
(deny network-outbound)
(allow network-outbound 
    (remote unix-socket)
    (remote ip "api.openai.com:443"))
(allow file-read* 
    (subpath "/Users/user/Library/Application Support/claw-kernel"))
(allow file-write*
    (subpath "/Users/user/Library/Caches/claw-kernel"))
```

### 局限性

与 Linux 相比：
- 没有等同于 seccomp 的系统调用过滤功能
- 网络过滤更加受限
- 文件系统规则仅基于路径

---

## 代码签名

要进行完整的沙盒测试，需要为二进制文件签名：

```bash
# 生成自签名证书（钥匙串访问）
# 证书助理 → 创建证书...
# 名称："claw-kernel-dev"，类型：代码签名

# 为二进制文件签名
codesign -s "claw-kernel-dev" --force target/debug/my-agent

# 验证
codesign -dvv target/debug/my-agent
```

---

## 配置

### 配置目录

```
~/Library/Application Support/claw-kernel/   # 数据
~/Library/Caches/claw-kernel/                # 缓存
```

### 示例

```rust
use claw_kernel::pal::dirs;

let data_dir = dirs::data_dir();
// /Users/<user>/Library/Application Support/claw-kernel/
```

---

## 测试

```bash
# 运行测试
cargo test --workspace

# 带沙盒测试（需要已签名的二进制文件）
codesign -s "claw-kernel-dev" target/debug/deps/*
cargo test --features sandbox-tests
```

---

## 故障排除

### "sandbox_init failed"（沙盒初始化失败）

代码签名问题：

```bash
# 检查签名
codesign -dvv target/debug/my-agent

# 重新签名
codesign -s "claw-kernel-dev" --force target/debug/my-agent
```

### SIP 干扰

系统完整性保护可能会阻止某些操作：

```bash
# 检查 SIP 状态
csrutil status

# 在 Apple Silicon 上不容易禁用 SIP
# 请改用权限文件（entitlements）
```

### Gatekeeper

如果二进制文件被隔离：

```bash
# 移除隔离属性
xattr -d com.apple.quarantine target/debug/my-agent
```

---

## 性能

| 指标 | 数值 |
|-----|------|
| 沙盒开销 | ~1-2毫秒 |
| IPC 延迟 | ~15微秒（UDS） |
| 上下文切换 | 良好（原生） |

由于沙盒配置文件编译，比 Linux 稍慢。

---

## 公证（分发）

分发您的代理程序：

```bash
# 使用开发者 ID 签名
codesign -s "Developer ID Application: Your Name" \
    --options runtime \
    --entitlements entitlements.plist \
    target/release/my-agent

# 创建 DMG
# ... 

# 公证
xcrun notarytool submit my-agent.dmg --wait
```

---

## 另请参阅

- [PAL 架构](../architecture/pal.md)
- [Linux 指南](linux.md)
- [Windows 指南](windows.md)
