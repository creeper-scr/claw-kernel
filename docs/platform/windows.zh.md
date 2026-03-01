---
title: Windows 平台指南
description: Windows platform guide (AppContainer + Job Objects)
status: design-phase
version: "0.1.0"
last_updated: "2026-03-01"
language: zh
---

[English →](windows.md)

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
