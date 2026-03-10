---
title: Windows Platform Guide
description: Windows platform guide (Job Objects + AppContainer roadmap)
status: partial-implementation
version: "1.4.0"
last_updated: "2026-03-10"
language: en
---

> ⚠️ **Pre-release notice:** v0.4.0 is a beta and may be unstable. APIs are subject to change without notice.

> **⚠️ Critical Security Warning / 重要安全警告**
> 
> Windows sandbox is a **degraded implementation** with significant limitations:
> - ✅ Resource limits enforced via Job Objects
> - ✅ Subprocess blocking via ActiveProcessLimit
> - ❌ **Filesystem restrictions NOT enforced**
> - ❌ **Network restrictions NOT enforced**
> 
> This is **not a complete security solution**. For security-sensitive deployments:
> - Use WSL2 to run the Linux version for full sandboxing
> - Deploy in Windows containers or VMs
> - Apply additional OS-level security controls
>
> **We urgently need contributions for:**
> - AppContainer sandbox implementation (target: v1.5.0)
> - Windows Filtering Platform (WFP) integration for network rules
> - Security testing and audit of Job Object implementation
> 
> If you have Windows security expertise, please consider contributing!
> See [CONTRIBUTING.md](../../CONTRIBUTING.md) and [SECURITY.md](../../SECURITY.md).

# Windows Platform Guide

> ⚠️ **降级实现 (Degraded Implementation)**: Windows 沙箱使用 **Job Objects** 提供部分隔离。
> 资源限制和子进程阻断已通过 Job Object 实际执行，但**文件系统和网络限制暂不生效**。
> 完整 AppContainer 实现（含文件系统/网络隔离）计划在 **v1.5.0** 提供。

Windows IPC (Named Pipe) 功能完整，沙箱已从 Stub 升级为 Job Object 实现。

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

### 当前实现：Job Object（v1.4.0）

Windows Safe 模式通过 **Job Objects** 提供降级隔离。与 Linux（seccomp）和 macOS（sandbox_init）相比：

| 隔离能力 | Linux | macOS | Windows Job Object (v1.4.0) |
|---------|-------|-------|------------------------------|
| 内存限制 | ✅ setrlimit | ❌ 不支持 | ✅ `JobMemoryLimit` |
| 子进程阻断 | ✅ seccomp | ✅ SBPL | ✅ `ActiveProcessLimit=1` |
| 进程数限制 | ✅ setrlimit NPROC | ❌ | ✅ `ActiveProcessLimit` |
| 文件系统限制 | ⚠️ namespace | ✅ SBPL | ❌ **暂不生效** |
| 网络限制 | ✅ seccomp socket | ✅ SBPL | ❌ **暂不生效** |

**Safe 模式实际执行的限制：**

```rust
// v1.4.0 实现 — Job Object 实际执行以下限制
// 1. 内存上限：若配置了 max_memory_bytes，超出则进程被终止
// 2. 子进程阻断：allow_subprocess=false 时 ActiveProcessLimit=1，
//    CreateProcess 调用将返回 ERROR_NOT_ENOUGH_QUOTA
// 3. 进程数上限：若配置了 max_processes
//
// 以下限制存储但暂不执行（v1.5.0 AppContainer 实现后生效）：
// restrict_filesystem(&whitelist)  → 存储，未执行
// restrict_network(&rules)         → 存储，未执行
```

**Safe 模式激活时的运行时告警（不可关闭）：**

```
WARN claw_pal::windows::sandbox: Windows Safe mode uses Job Object isolation (degraded).
     Resource limits and subprocess blocking are enforced via Job Object.
     Filesystem and network restrictions are NOT enforced until v1.5.0 (AppContainer).
     For full isolation, use WSL2 to run the Linux version.
```

### Job Object 生命周期

```
CreateJobObjectW(null, null)           → 创建匿名 Job Object
SetInformationJobObject(...)           → 配置资源限制
AssignProcessToJobObject(job, self)    → 将当前进程加入 Job
CloseHandle(job)                       → 关闭句柄（内核继续执行限制）
                                        ↑ 进程仍在 Job 中，限制有效
```

Job Object 句柄关闭后，只要进程仍在 Job 中，内核就继续执行限制规则。进程退出时 Job Object 自动释放。

### 规划：AppContainer（v1.5.0）

完整 Windows 沙箱实现需要：

```rust
// 计划 v1.5.0 实现
CreateAppContainerProfile(name)?;         // 创建 AppContainer profile
CreateProcessAsUser(token, ...)?;         // 以 AppContainer 身份启动进程
// Windows Filtering Platform (WFP) callout for network rules
```

届时将实现：
- ✅ 文件系统路径级别的读写限制
- ✅ 网络域名/端口级别过滤
- ✅ 低完整性级别进程隔离

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

> ⚠️ **v0.1.0 Limitation**: Windows IPC (Named Pipe) is **not implemented**. 
> Operations return `IpcError::ConnectionRefused`.
> Full Named Pipe support is planned for v0.2.0.

Windows will use **Named Pipes** for inter-process communication (Layer 0.5 PAL) when implemented.

**Planned Characteristics:**
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

Windows 平台采用**功能降级**策略，分两阶段实现完整沙箱：

| 功能 | Linux/macOS | Windows v1.4.0（当前） | Windows v1.5.0（计划） |
|------|-------------|------------------------|------------------------|
| 系统调用过滤 | seccomp-bpf / Seatbelt | 不适用（Windows无seccomp等价物） | 不适用 |
| 内存限制 | setrlimit | ✅ Job Object JobMemoryLimit | ✅ Job Object |
| 进程数限制 | setrlimit NPROC | ✅ Job Object ActiveProcessLimit | ✅ Job Object |
| 子进程阻断 | seccomp execve | ✅ ActiveProcessLimit=1 | ✅ Job Object |
| 文件系统隔离 | namespace / SBPL | ❌ 暂不执行 | ✅ AppContainer |
| 网络限制 | seccomp / SBPL | ❌ 暂不执行 | ✅ AppContainer + WFP |

**降级说明（v1.4.0）：**

1. Safe 模式通过 Job Object 实际执行资源和进程限制
2. 文件系统和网络规则存储但不生效
3. 启动时输出不可关闭的 `WARN` 级别告警，明确告知隔离能力范围
4. 如需完整文件系统/网络隔离，建议使用 WSL2 运行 Linux 版本

> **与旧版 Stub 的区别**：v1.3.0 及之前，Windows Safe 模式直接返回 `Err(NotSupported)` 完全拒绝工作。v1.4.0 提供真实的 Job Object 隔离，虽然不完整，但已具备实际安全价值。

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
