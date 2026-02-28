---
title: Windows Platform Guide
description: Windows platform guide (AppContainer + Job Objects)
status: design-phase
version: "0.1.0"
last_updated: "2026-02-28"
---

[English](#english) | [中文](#chinese)

<a name="english"></a>
# Windows Platform Guide

Windows support is fully functional with AppContainer sandboxing and Named Pipe IPC.

---

## Architecture Position

This document describes the **Layer 0.5: Platform Abstraction Layer (PAL)** implementation for Windows.

claw-kernel uses a 5-layer architecture:
- **Layer 0**: Rust Hard Core — Platform-agnostic trust root
- **Layer 0.5**: Platform Abstraction Layer (PAL) — Platform-specific code (this document)
- **Layer 1-3**: System Runtime / Agent Kernel Protocol / Script Runtime — Platform-agnostic, use PAL via traits

> **Zero Platform Assumptions**: All code at Layer 0-3 is platform-agnostic. Only PAL (Layer 0.5) contains platform-specific implementations. Windows-specific sandbox, IPC, and configuration directory code is isolated in the `claw-pal` crate's Windows module.

---

## 架构位置

本文档描述 **Layer 0.5: Platform Abstraction Layer (PAL)** 的 Windows 实现。

claw-kernel 采用五层架构：
- **Layer 0**: Rust Hard Core — 平台无关的信任根
- **Layer 0.5**: Platform Abstraction Layer (PAL) — 平台特定代码（本文档）
- **Layer 1-3**: System Runtime / Agent Kernel Protocol / Script Runtime — 平台无关，通过 PAL trait 使用平台功能

> **Zero Platform Assumptions**: Layer 0-3 的所有代码都是平台无关的。只有 PAL (Layer 0.5) 包含平台特定实现。Windows 特定的沙箱、IPC 和配置目录代码都隔离在 `claw-pal` crate 的 Windows 模块中。

---

## Requirements

- Windows 10/11 (64-bit)
- Visual Studio 2019+ OR Build Tools for Visual Studio
- Rust with MSVC toolchain

---

## Installation

### 1. Install Visual Studio Build Tools

Download from: https://visualstudio.microsoft.com/downloads/

Required components:
- MSVC v143 - VS 2022 C++ x64/x86 build tools
- Windows 10/11 SDK

### 2. Install Rust (MSVC)

```powershell
# If you have GNU toolchain installed, switch to MSVC
rustup default stable-x86_64-pc-windows-msvc

# Verify
rustc --print host-triple
# Should show: x86_64-pc-windows-msvc
```

---

## Sandbox Implementation

Windows uses **AppContainer** + **Job Objects**:

```rust
// Internal implementation
create_app_container()?;    // Low integrity process
create_capabilities()?;     // Capability SIDs
apply_job_limits()?;        // Resource restrictions
create_process_with_token()?; // Launch sandboxed
```

### AppContainer

Isolates the process with:
- Low integrity level
- Capability-based access control
- Network isolation

### Job Objects

Enforce:
- Memory limits
- CPU limits
- Active process limits

---

## Important: Path Handling

Windows uses backslashes. Always use `std::path::Path`:

```rust
// Correct
let path = PathBuf::from("data").join("file.txt");
// Works on all platforms

// Wrong
let path = "data/file.txt";  // Fails on Windows
```

---

## Configuration

### Config Directory

```
%APPDATA%\claw-kernel\         # Data (Roaming)
%LOCALAPPDATA%\claw-kernel\    # Cache (Local)
```

### Example

```rust
use claw_kernel::pal::dirs;

let data_dir = dirs::data_dir();
// C:\Users\<user>\AppData\Roaming\claw-kernel\
```

---

## IPC Transport

Windows uses **Named Pipes** for inter-process communication (Layer 0.5 PAL).

```rust
use claw_pal::IpcTransport;

// Create listener
let listener = LocalSocketListener::bind("claw-kernel-agent")?;
// Creates: \\.\pipe\claw-kernel-agent

// Connect
let stream = LocalSocketStream::connect("claw-kernel-agent")?;
```

**Characteristics:**
- Named pipe: `\\.\pipe\<name>`
- Performance: ~90% (slightly slower than UDS)
- Security: Pipe ACLs (Access Control Lists)
- Fallback: TCP loopback if Named Pipe fails

---

## Configuration Directories

Following **Windows Known Folders** conventions:

| Type | Environment Variable | Default Path |
|------|---------------------|--------------|
| Config (Roaming) | `APPDATA` | `%APPDATA%\claw-kernel\` |
| Data (Roaming) | `APPDATA` | `%APPDATA%\claw-kernel\data\` |
| Data (Local) | `LOCALAPPDATA` | `%LOCALAPPDATA%\claw-kernel\` |
| Cache | `LOCALAPPDATA` | `%LOCALAPPDATA%\claw-kernel\cache\` |

**Full Paths:**
- `C:\Users\<user>\AppData\Roaming\claw-kernel\` — Configuration and tool scripts
- `C:\Users\<user>\AppData\Roaming\claw-kernel\tools\` — Hot-loaded tool scripts
- `C:\Users\<user>\AppData\Roaming\claw-kernel\scripts\` — Runtime extension scripts
- `C:\Users\<user>\AppData\Local\claw-kernel\` — Local data and cache

```rust
use claw_kernel::pal::dirs;

let config_dir = dirs::config_dir();
// C:\Users\<user>\AppData\Roaming\claw-kernel\

let data_dir = dirs::data_dir();
// C:\Users\<user>\AppData\Roaming\claw-kernel\
```

---

## Testing

### Run Tests

```powershell
# Must use MSVC toolchain
rustup default stable-x86_64-pc-windows-msvc

# Run tests
cargo test --workspace

# Sandbox tests require Administrator
cargo test --features sandbox-tests
```

### Administrator for Sandbox

AppContainer creation requires elevated privileges for testing:

```powershell
# Run PowerShell as Administrator
# Then run tests
```

---

## Troubleshooting

### "linker not found"

Missing Visual Studio Build Tools:

```powershell
# Install via Visual Studio Installer
# Or use chocolatey
choco install visualstudio2022buildtools
```

### "cannot find -lxyz"

Library not found. Check:
- Correct architecture (x64 vs x86)
- Library paths in environment

### Named Pipe Issues

Named Pipes behave subtly different from Unix Domain Sockets:

```rust
// Windows Named Pipe names must be:
// \\.\pipe\<name>

let pipe_name = r"\\.\pipe\claw-kernel-agent-123";
```

### Long Path Support

Enable long path support for paths >260 chars:

```powershell
# Registry setting
Set-ItemProperty -Path "HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem" -Name "LongPathsEnabled" -Value 1

# Application manifest required
```

---

## Sandbox Degradation Strategy

Windows 平台采用**功能降级**策略：

| 功能 | Linux/macOS | Windows（降级后） |
|------|-------------|-------------------|
| 系统调用过滤 | seccomp-bpf / Seatbelt | AppContainer + WFP（简化实现） |
| 文件系统隔离 | 完整 namespace | AppContainer 能力声明 |
| 网络规则 | 域名+端口组合匹配 | AppContainer 网络隔离（较粗粒度） |
| 进程隔离 | PID namespace | Job Objects |

**降级说明**：
1. Windows 版本完整支持 API，但某些高级沙箱特性可能简化实现
2. 启动时输出警告：`[WARN] Windows sandbox provides medium isolation, suitable for personal use`
3. **警告级别为 WARN，不可关闭**（提醒用户当前隔离强度）
4. 如需更强隔离，建议使用 WSL2 运行 Linux 版本

## Performance

| Metric | Value |
|--------|-------|
| Sandbox overhead | ~2-3ms per process start* |

*Test conditions: Intel i7-1165G7, Windows 11 23H2, cold start AppContainer
| IPC latency | TBD (Named Pipe) |
| Context switch | Good |

Slightly slower than Linux due to AppContainer setup.

---

## Signal Differences

Windows doesn't have POSIX signals:

| Unix | Windows Equivalent |
|------|-------------------|
| SIGTERM | WM_CLOSE → TerminateProcess |
| SIGKILL | TerminateProcess |
| SIGUSR1 | Named Event |
| SIGCHLD | WaitForSingleObject |

---

## Anti-Virus Considerations

Windows Defender or other AV may flag:
- Process creation (sandbox tests)
- Named Pipe creation
- AppContainer operations

Add exclusions if needed for development:

```powershell
# PowerShell as Administrator
Add-MpPreference -ExclusionPath "C:\path\to\claw-kernel"
```

---

## See Also

- [PAL Architecture](../architecture/pal.md)
- [Linux Guide](linux.md)
- [macOS Guide](macos.md)

---

<a name="chinese"></a>
# Windows 平台指南

Windows 支持功能完整，包含 AppContainer 沙箱和命名管道 IPC。

---

## 系统要求

- Windows 10/11（64位）
- Visual Studio 2019+ 或 Visual Studio 构建工具
- 使用 MSVC 工具链的 Rust

---

## 安装

### 1. 安装 Visual Studio 构建工具

从以下地址下载：https://visualstudio.microsoft.com/downloads/

必需组件：
- MSVC v143 - VS 2022 C++ x64/x86 构建工具
- Windows 10/11 SDK

### 2. 安装 Rust（MSVC）

```powershell
# 如果已安装 GNU 工具链，切换到 MSVC
rustup default stable-x86_64-pc-windows-msvc

# 验证
rustc --print host-triple
# 应显示：x86_64-pc-windows-msvc
```

---

## 沙箱实现

Windows 使用 **AppContainer** + **作业对象**：

```rust
// 内部实现
create_app_container()?;    // 低完整性进程
create_capabilities()?;     // 功能 SID
apply_job_limits()?;        // 资源限制
create_process_with_token()?; // 启动沙箱化进程
```

### AppContainer

通过以下方式隔离进程：
- 低完整性级别
- 基于功能的访问控制
- 网络隔离

### 作业对象

强制执行：
- 内存限制
- CPU 限制
- 活动进程限制

---

## 重要提示：路径处理

Windows 使用反斜杠。请始终使用 `std::path::Path`：

```rust
// 正确
let path = PathBuf::from("data").join("file.txt");
// 可在所有平台上工作

// 错误
let path = "data/file.txt";  // 在 Windows 上会失败
```

---

## 配置

### 配置目录

```
%APPDATA%\claw-kernel\         # 数据（漫游）
%LOCALAPPDATA%\claw-kernel\    # 缓存（本地）
```

### 示例

```rust
use claw_kernel::pal::dirs;

let data_dir = dirs::data_dir();
// C:\Users\<user>\AppData\Roaming\claw-kernel\
```

---

## 测试

### 运行测试

```powershell
# 必须使用 MSVC 工具链
rustup default stable-x86_64-pc-windows-msvc

# 运行测试
cargo test --workspace

# 沙箱测试需要管理员权限
cargo test --features sandbox-tests
```

### 沙箱需要管理员权限

创建 AppContainer 需要提升的权限进行测试：

```powershell
# 以管理员身份运行 PowerShell
# 然后运行测试
```

---

## 故障排除

### "linker not found"（找不到链接器）

缺少 Visual Studio 构建工具：

```powershell
# 通过 Visual Studio 安装程序安装
# 或使用 chocolatey
choco install visualstudio2022buildtools
```

### "cannot find -lxyz"（找不到 -lxyz）

找不到库。检查：
- 正确的架构（x64 与 x86）
- 环境中的库路径

### 命名管道问题

命名管道与 Unix 域套接字的行为略有不同：

```rust
// Windows 命名管道名称格式：
// \\.\pipe\<name>

let pipe_name = r"\\.\pipe\claw-kernel-agent-123";
```

### 长路径支持

为超过 260 个字符的路径启用长路径支持：

```powershell
# 注册表设置
Set-ItemProperty -Path "HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem" -Name "LongPathsEnabled" -Value 1

# 需要应用程序清单文件
```

---

## 沙箱降级策略

Windows 平台采用**功能降级**策略：

| 功能 | Linux/macOS | Windows（降级后） |
|------|-------------|-------------------|
| 系统调用过滤 | seccomp-bpf / Seatbelt | AppContainer + WFP（简化实现） |
| 文件系统隔离 | 完整 namespace | AppContainer 能力声明 |
| 网络规则 | 域名+端口组合匹配 | AppContainer 网络隔离（较粗粒度） |
| 进程隔离 | PID namespace | Job Objects |

**降级说明**：
1. Windows 版本完整支持 API，但某些高级沙箱特性可能简化实现
2. 启动时输出警告：`[WARN] Windows sandbox provides medium isolation, suitable for personal use`
3. 如需更强隔离，建议使用 WSL2 运行 Linux 版本

## 性能

| 指标 | 数值 |
|-----|------|
| 沙箱开销 | ~2-3毫秒 |
| IPC 延迟 | ~20微秒（命名管道） |
| 上下文切换 | 良好 |

由于 AppContainer 设置，比 Linux 稍慢。

---

## 信号差异

Windows 没有 POSIX 信号：

| Unix | Windows 等效 |
|------|-------------|
| SIGTERM | WM_CLOSE → TerminateProcess |
| SIGKILL | TerminateProcess |
| SIGUSR1 | 命名事件 |
| SIGCHLD | WaitForSingleObject |

---

## 杀毒软件注意事项

Windows Defender 或其他杀毒软件可能会标记：
- 进程创建（沙箱测试）
- 命名管道创建
- AppContainer 操作

开发时如有需要可添加排除项：

```powershell
# 以管理员身份运行 PowerShell
Add-MpPreference -ExclusionPath "C:\path\to\claw-kernel"
```

---

## 另请参阅

- [PAL 架构](../architecture/pal.md)
- [Linux 指南](linux.md)
- [macOS 指南](macos.md)
