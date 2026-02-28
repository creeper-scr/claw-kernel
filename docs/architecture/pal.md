[English](#english) | [中文](#chinese)

<a name="english"></a>
# Platform Abstraction Layer (PAL)

The Platform Abstraction Layer (`claw-pal`) isolates all platform-specific code, enabling claw-kernel to run on Linux, macOS, and Windows with minimal platform-specific logic in the upper layers.

---

## Philosophy

**"Zero Platform Assumptions"**

- All code at Layers 0.5-2 must be platform-agnostic
- Platform differences are abstracted behind traits
- Each platform implementation is a module in `claw-pal`

---

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                    Upper Layers                         │
│         (claw-runtime, claw-script, etc.)               │
└───────────────────────┬─────────────────────────────────┘
                        │ uses
                        ▼
┌─────────────────────────────────────────────────────────┐
│              claw-pal (Platform Abstraction)            │
├─────────────────────────────────────────────────────────┤
│  Traits (platform-agnostic interface)                   │
│  ├── SandboxBackend                                     │
│  ├── IpcTransport                                       │
│  └── ProcessManager                                     │
├─────────────────────────────────────────────────────────┤
│  Platform Implementations                               │
│  ├── linux/   → seccomp, UDS, fork/exec                 │
│  ├── macos/   → sandbox(7), UDS, fork/exec              │
│  └── windows/ → AppContainer, Named Pipe, CreateProcess │
└─────────────────────────────────────────────────────────┘
```

---

## Core Traits

### `SandboxBackend`

Abstracts platform-specific sandboxing mechanisms.

```rust
pub trait SandboxBackend: Send + Sync {
    fn create(config: SandboxConfig) -> Result<Self, SandboxError> where Self: Sized;
    fn restrict_filesystem(&mut self, allowlist: &[PathBuf]) -> &mut Self;
    fn restrict_network(&mut self, rules: &[NetRule]) -> &mut Self;
    fn restrict_syscalls(&mut self, policy: SyscallPolicy) -> &mut Self;
    fn restrict_resources(&mut self, limits: ResourceLimits) -> &mut Self;
    fn apply(self) -> Result<SandboxHandle, SandboxError>;
}

pub struct SandboxConfig {
    pub name: String,
    pub mode: ExecutionMode,                  // Safe or Power
    pub working_dir: Option<PathBuf>,         // Working directory
}

pub struct SandboxHandle {
    pub id: Uuid,
    pub platform: PlatformHandle,
}

pub enum PlatformHandle {
    #[cfg(target_os = "linux")]
    Linux { seccomp_fd: RawFd },
    #[cfg(target_os = "macos")]
    MacOS { profile_ref: SandboxProfileRef },
    #[cfg(target_os = "windows")]
    Windows { token: HANDLE, job: HANDLE },
}

pub enum NetRule {
    Allow { domain: String, ports: Vec<u16> },  // Supports wildcards like "*.example.com"
    AllowAll,
    DenyAll,
}

pub enum SyscallPolicy {
    Default,                                   // Default policy
    Strict,                                    // Strict mode
    Custom(Vec<AllowedSyscall>),               // Custom allow list
}

pub struct AllowedSyscall {
    pub number: usize,
    pub args: Option<Vec<SeccompArg>>,
}

pub struct ResourceLimits {
    pub max_memory_mb: usize,
    pub max_cpu_percent: f32,
    pub max_file_descriptors: usize,
    pub max_processes: usize,
}

pub enum SandboxError {
    CreationFailed(String),
    AlreadyApplied,
    InvalidConfig(String),
    PlatformError(String),
}
```

### Platform Implementations

| Capability | Linux | macOS | Windows |
|------------|-------|-------|---------|
| Filesystem | seccomp + mount namespace | sandbox(7) profile | AppContainer capabilities |
| Network | network namespace | Socket filter (limited) | WFP (Windows Filtering Platform) |
| Syscalls | seccomp-bpf filter | Limited (ptrace) | Limited (API hooking) |
| Process | pid namespace | Standard | Job Objects |

**Linux Implementation:**
```rust
#[cfg(target_os = "linux")]
pub struct LinuxSandbox {
    config: SandboxConfig,
    seccomp_rules: Vec<SeccompRule>,
    mount_propagation: MountPropagation,
}

impl SandboxBackend for LinuxSandbox {
    fn apply(self) -> Result<SandboxHandle, SandboxError> {
        // 1. Create namespaces
        unshare(CloneFlags::NEW_NS | CloneFlags::NEW_PID | CloneFlags::NEW_NET)?;
        
        // 2. Setup seccomp-bpf
        let filter = self.build_seccomp_filter()?;
        seccomp_load(&filter)?;
        
        // 3. Pivot root if filesystem restricted
        if !self.fs_allowlist.is_empty() {
            self.setup_pivot_root()?;
        }
        
        Ok(SandboxHandle::Linux(...))
    }
}
```

**macOS Implementation:**
```rust
#[cfg(target_os = "macos")]
pub struct MacSandbox {
    config: SandboxConfig,
    profile: SandboxProfile,
}

impl SandboxBackend for MacSandbox {
    fn apply(self) -> Result<SandboxHandle, SandboxError> {
        // macOS uses declarative sandbox profiles
        let profile_str = self.generate_profile()?;
        
        // Compile and apply profile
        let sb_profile = sandbox_compile(profile_str)?;
        sandbox_apply(sb_profile)?;
        
        Ok(SandboxHandle::MacOS(...))
    }
}

// Example generated profile:
// (version 1)
// (allow default)
// (deny network-outbound)
// (allow network-outbound (remote unix-socket))
// (allow file-read* (subpath "/allowed/path"))
```

**Windows Implementation:**
```rust
#[cfg(target_os = "windows")]
pub struct WindowsSandbox {
    config: SandboxConfig,
    capabilities: Vec<AppContainerCapability>,
}

impl SandboxBackend for WindowsSandbox {
    fn apply(self) -> Result<SandboxHandle, SandboxError> {
        // 1. Create AppContainer SID
        let container_sid = create_app_container_sid(&self.config.name)?;
        
        // 2. Setup capabilities
        let capabilities = self.build_capability_sids()?;
        
        // 3. Create process with AppContainer token
        let token = create_app_container_token(container_sid, &capabilities)?;
        
        // 4. Apply Job Object limits
        let job = create_job_object()?;
        set_job_limits(job, &self.config)?;
        
        Ok(SandboxHandle::Windows { token, job })
    }
}
```

---

### `IpcTransport`

Cross-platform inter-process communication.

```rust
pub trait IpcTransport: Send + Sync {
    /// Connect to an IPC endpoint
    async fn connect(endpoint: &str) -> Result<IpcConnection, IpcError>;
    
    /// Listen for incoming connections
    async fn listen(endpoint: &str) -> Result<IpcListener, IpcError>;
    
    /// Send a message
    async fn send(&self, msg: &[u8]) -> Result<(), IpcError>;
    
    /// Receive a message
    async fn recv(&self) -> Result<Vec<u8>, IpcError>;
}

pub enum IpcConnection {
    #[cfg(unix)]
    Unix(UnixStream),
    #[cfg(windows)]
    Windows(NamedPipeClient),
    #[cfg(feature = "tcp-fallback")]
    Tcp(TcpStream),
}
```

**Implementation Strategy:**

We use the [`interprocess`](https://crates.io/crates/interprocess) crate which provides:
- **Unix:** Unix Domain Sockets (highest performance)
- **Windows:** Named Pipes (native equivalent)

```rust
// Platform-agnostic usage
use interprocess::local_socket::{LocalSocketStream, LocalSocketListener};

pub struct InterprocessTransport;

impl IpcTransport for InterprocessTransport {
    async fn connect(endpoint: &str) -> Result<IpcConnection, IpcError> {
        let stream = LocalSocketStream::connect(endpoint)?;
        Ok(IpcConnection::from(stream))
    }
    
    async fn listen(endpoint: &str) -> Result<IpcListener, IpcError> {
        let listener = LocalSocketListener::bind(endpoint)?;
        Ok(IpcListener::from(listener))
    }
}
```

**Fallback Strategy:**

If Named Pipe permissions are restricted on Windows, we automatically fall back to TCP loopback:

```rust
pub async fn connect_with_fallback(endpoint: &str) -> Result<Box<dyn IpcConnection>, IpcError> {
    // Try native IPC first
    match IpcTransport::connect(endpoint).await {
        Ok(conn) => return Ok(Box::new(conn)),
        Err(e) if is_permission_error(&e) => {
            log::warn!("Native IPC failed, falling back to TCP");
        }
        Err(e) => return Err(e),
    }
    
    // Fallback to TCP on loopback
    let tcp_conn = TcpTransport::connect("127.0.0.1:0").await?;
    Ok(Box::new(tcp_conn))
}
```

---

### `ProcessManager`

Cross-platform process lifecycle management.

```rust
pub trait ProcessManager: Send + Sync {
    async fn spawn(&self, config: ProcessConfig) -> Result<ProcessHandle, ProcessError>;
    async fn terminate(&self, handle: ProcessHandle, grace_period: Duration) -> Result<(), ProcessError>;
    async fn kill(&self, handle: ProcessHandle) -> Result<(), ProcessError>;
    async fn wait(&self, handle: ProcessHandle) -> Result<ExitStatus, ProcessError>;
    async fn signal(&self, handle: ProcessHandle, signal: ProcessSignal) -> Result<(), ProcessError>;
    fn list_children(&self) -> Vec<ProcessInfo>;
}

pub struct ProcessConfig {
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub env_inherit: bool,                    // Inherit parent env vars, default: true
    pub sandbox: Option<Box<dyn SandboxBackend>>,
    pub working_dir: Option<PathBuf>,
    pub stdin: StdioConfig,                   // Default: Null
    pub stdout: StdioRedirect,                // Default: Inherit
    pub stderr: StdioRedirect,                // Default: Inherit
    pub resource_limits: Option<ResourceLimits>,
}

pub enum StdioConfig {
    Null,
    Inherit,
    Piped,
    File(PathBuf),
}

pub enum StdioRedirect {
    Inherit,
    Piped,
    File { path: PathBuf, append: bool },
    Null,
}

pub struct ProcessHandle {
    pub pid: u32,
    pub sandbox: Option<SandboxHandle>,
    pub start_time: SystemTime,
}

pub struct ProcessInfo {
    pub handle: ProcessHandle,
    pub status: ProcessStatus,
    pub cpu_usage_percent: f32,
    pub memory_usage_mb: usize,
}

pub enum ProcessStatus {
    Running,
    Sleeping,
    Stopped,
    Zombie,
    Exited(ExitStatus),
}

pub struct ExitStatus {
    pub code: Option<i32>,
    pub signal: Option<i32>,              // Unix only
    pub success: bool,
}

pub enum ProcessSignal {
    Terminate,                            // Graceful termination
    Kill,                                 // Force kill
    Interrupt,                            // Ctrl+C
    #[cfg(unix)]
    User1,
    #[cfg(unix)]
    User2,
}

pub enum ProcessError {
    SpawnFailed(String),
    ProcessNotFound,
    AlreadyTerminated,
    PermissionDenied,
    Timeout,
}
```

**Signal Mapping:**

| Concept | Linux/macOS | Windows |
|---------|-------------|---------|
| Graceful stop | SIGTERM | Ctrl+C event → TerminateProcess |
| Force kill | SIGKILL | TerminateProcess |
| Custom signal | SIGUSR1/2 | Named Event |
| Status check | waitpid | GetExitCodeProcess |

**Implementation Notes:**

On Windows, we must avoid Unix-isms:

```rust
#[cfg(windows)]
impl ProcessManager for WindowsProcessManager {
    async fn terminate(&self, handle: ProcessHandle, grace_period: Duration) -> Result<()> {
        // Windows doesn't have signals; use Ctrl+C or graceful close
        if let Some(hwnd) = get_main_window(handle) {
            // Send WM_CLOSE to main window
            post_message(hwnd, WM_CLOSE, 0, 0)?;
            
            // Wait for graceful exit
            match timeout(grace_period, self.wait(handle)).await {
                Ok(_) => return Ok(()),
                Err(_) => {
                    log::warn!("Graceful termination timed out, forcing kill");
                    self.kill(handle).await
                }
            }
        } else {
            // No window, force kill
            self.kill(handle).await
        }
    }
}
```

---

## Configuration Directories

Cross-platform config/data/cache directory handling via the [`dirs`](https://crates.io/crates/dirs) crate.

```rust
pub mod dirs {
    use std::path::PathBuf;
    
    /// Configuration directory
    /// - Linux: ~/.config/claw-kernel/
    /// - macOS: ~/Library/Application Support/claw-kernel/
    /// - Windows: %APPDATA%\claw-kernel\
    pub fn config_dir() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("claw-kernel"))
    }
    
    /// Data directory (tools, scripts, persistent state)
    pub fn data_dir() -> Option<PathBuf> {
        dirs::data_dir().map(|d| d.join("claw-kernel"))
    }
    
    /// Cache directory
    pub fn cache_dir() -> Option<PathBuf> {
        dirs::cache_dir().map(|d| d.join("claw-kernel"))
    }
    
    /// Tools directory (hot-loaded scripts)
    pub fn tools_dir() -> Option<PathBuf> {
        data_dir().map(|d| d.join("tools"))
    }
    
    /// Runtime extension scripts
    pub fn scripts_dir() -> Option<PathBuf> {
        data_dir().map(|d| d.join("scripts"))
    }
}
```

---

## Platform Capability Matrix

| Feature | Linux | macOS | Windows |
|---------|:-----:|:-----:|:-------:|
| **Sandbox Strength** | ⭐⭐⭐ | ⭐⭐ | ⭐⭐ |
| Filesystem isolation | Strong | Medium | Medium |
| Network isolation | Strong | Limited | Limited |
| Syscall filtering | seccomp-bpf | Limited | Limited |
| **IPC Performance** | 100% | 95% | 90% |
| **Build Complexity** | Low | Low | Medium |
| MSVC required | No | No | Yes |
| Sandbox testing | Easy | Medium | Complex |

---

## Platform-Specific Considerations

### Linux

**Strengths:**
- Strongest sandboxing (seccomp + namespaces)
- Best performance
- Native containers

**Gotchas:**
- Different kernel versions have different seccomp features
- User namespaces may be disabled on some systems
- AppArmor/SELinux may conflict

**Testing:**
```bash
# Run with different seccomp profiles
cargo test --features sandbox-tests

# Test with user namespaces disabled
unshare -U cargo test
```

### macOS

**Strengths:**
- Official sandbox API
- Good developer experience

**Gotchas:**
- Sandbox profiles are declarative and limited
- No equivalent to seccomp for syscall filtering
- Code signing affects sandbox behavior

**Testing:**
```bash
# Sign test binaries for full sandbox testing
codesign -s "Developer ID" target/debug/deps/*
cargo test --features sandbox-tests
```

### Windows

**Strengths:**
- AppContainer is well-designed
- Job Objects for resource limits

**Gotchas:**
- MSVC toolchain required (not GNU)
- AppContainer setup is complex
- Named Pipes behave subtly differently from UDS
- Path separators (always use `std::path::Path`)
- No `fork()` (use `CreateProcess`)

**Testing:**
```powershell
# Must use MSVC toolchain
rustup default stable-x86_64-pc-windows-msvc

# Run tests as Administrator for AppContainer tests
cargo test --features sandbox-tests
```

---

## Adding a New Platform

To add support for a new platform (e.g., FreeBSD, Android):

1. **Create platform module:**
   ```
   claw-pal/src/
   └── freebsd/
       ├── mod.rs
       ├── sandbox.rs
       ├── ipc.rs
       └── process.rs
   ```

2. **Implement traits:**
   ```rust
   #[cfg(target_os = "freebsd")]
   mod freebsd;
   
   #[cfg(target_os = "freebsd")]
   pub use freebsd::*;
   ```

3. **Add platform to CI matrix**

4. **Document in `docs/platform/freebsd.md`**

---

## See Also

- [Linux Platform Guide](../platform/linux.md)
- [macOS Platform Guide](../platform/macos.md)
- [Windows Platform Guide](../platform/windows.md)
- [Architecture Overview](overview.md)

---

<a name="chinese"></a>
# 平台抽象层（PAL）

平台抽象层（`claw-pal`）隔离所有平台特定代码，使 claw-kernel 能够在 Linux、macOS 和 Windows 上运行，且上层只需最少的平台特定逻辑。

---

## 理念

**"零平台假设"**

- 第 0.5-2 层的所有代码必须是平台无关的
- 平台差异在特性背后抽象
- 每个平台实现是 `claw-pal` 中的一个模块

---

## 架构

```
┌─────────────────────────────────────────────────────────┐
│                    上层                                  │
│         (claw-runtime, claw-script, 等)                 │
└───────────────────────┬─────────────────────────────────┘
                        │ 使用
                        ▼
┌─────────────────────────────────────────────────────────┐
│              claw-pal（平台抽象）                        │
├─────────────────────────────────────────────────────────┤
│  特性（平台无关接口）                                     │
│  ├── SandboxBackend                                     │
│  ├── IpcTransport                                       │
│  └── ProcessManager                                     │
├─────────────────────────────────────────────────────────┤
│  平台实现                                               │
│  ├── linux/   → seccomp, UDS, fork/exec                 │
│  ├── macos/   → sandbox(7), UDS, fork/exec              │
│  └── windows/ → AppContainer, Named Pipe, CreateProcess │
└─────────────────────────────────────────────────────────┘
```

---

## 核心特性

### `SandboxBackend`

抽象平台特定的沙箱机制。

```rust
pub trait SandboxBackend: Send + Sync {
    /// 使用给定配置创建新沙箱
    fn create(config: SandboxConfig) -> Result<Self, SandboxError> where Self: Sized;
    
    /// 将文件系统访问限制到允许列表路径
    fn restrict_filesystem(&mut self, allowlist: &[PathBuf]) -> &mut Self;
    
    /// 按规则限制网络访问
    fn restrict_network(&mut self, rules: &[NetRule]) -> &mut Self;
    
    /// 限制可用的系统调用/API
    fn restrict_syscalls(&mut self, policy: SyscallPolicy) -> &mut Self;
    
    /// 限制资源使用
    fn restrict_resources(&mut self, limits: ResourceLimits) -> &mut Self;
    
    /// 应用所有限制（消费 self）
    fn apply(self) -> Result<SandboxHandle, SandboxError>;
}

pub struct SandboxConfig {
    pub name: String,
    pub mode: ExecutionMode,                  // 安全或强力
    pub working_dir: Option<PathBuf>,         // 工作目录
}

pub enum NetRule {
    Allow { domain: String, ports: Vec<u16> },
    AllowAll,
    DenyAll,
}
```

### 平台实现

| 能力 | Linux | macOS | Windows |
|------------|-------|-------|---------|
| 文件系统 | seccomp + 挂载命名空间 | sandbox(7) 配置文件 | AppContainer 能力 |
| 网络 | 网络命名空间 | 套接字过滤器（有限） | WFP（Windows 过滤平台） |
| 系统调用 | seccomp-bpf 过滤器 | 有限（ptrace） | 有限（API 钩子） |
| 进程 | pid 命名空间 | 标准 | 作业对象 |

**Linux 实现：**
```rust
#[cfg(target_os = "linux")]
pub struct LinuxSandbox {
    config: SandboxConfig,
    seccomp_rules: Vec<SeccompRule>,
    mount_propagation: MountPropagation,
}

impl SandboxBackend for LinuxSandbox {
    fn apply(self) -> Result<SandboxHandle, SandboxError> {
        // 1. 创建命名空间
        unshare(CloneFlags::NEW_NS | CloneFlags::NEW_PID | CloneFlags::NEW_NET)?;
        
        // 2. 设置 seccomp-bpf
        let filter = self.build_seccomp_filter()?;
        seccomp_load(&filter)?;
        
        // 3. 如果文件系统受限，则 pivot root
        if !self.fs_allowlist.is_empty() {
            self.setup_pivot_root()?;
        }
        
        Ok(SandboxHandle::Linux(...))
    }
}
```

**macOS 实现：**
```rust
#[cfg(target_os = "macos")]
pub struct MacSandbox {
    config: SandboxConfig,
    profile: SandboxProfile,
}

impl SandboxBackend for MacSandbox {
    fn apply(self) -> Result<SandboxHandle, SandboxError> {
        // macOS 使用声明式沙箱配置文件
        let profile_str = self.generate_profile()?;
        
        // 编译并应用配置文件
        let sb_profile = sandbox_compile(profile_str)?;
        sandbox_apply(sb_profile)?;
        
        Ok(SandboxHandle::MacOS(...))
    }
}

// 示例生成的配置文件：
// (version 1)
// (allow default)
// (deny network-outbound)
// (allow network-outbound (remote unix-socket))
// (allow file-read* (subpath "/allowed/path"))
```

**Windows 实现：**
```rust
#[cfg(target_os = "windows")]
pub struct WindowsSandbox {
    config: SandboxConfig,
    capabilities: Vec<AppContainerCapability>,
}

impl SandboxBackend for WindowsSandbox {
    fn apply(self) -> Result<SandboxHandle, SandboxError> {
        // 1. 创建 AppContainer SID
        let container_sid = create_app_container_sid(&self.config.name)?;
        
        // 2. 设置能力
        let capabilities = self.build_capability_sids()?;
        
        // 3. 使用 AppContainer 令牌创建进程
        let token = create_app_container_token(container_sid, &capabilities)?;
        
        // 4. 应用作业对象限制
        let job = create_job_object()?;
        set_job_limits(job, &self.config)?;
        
        Ok(SandboxHandle::Windows { token, job })
    }
}
```

---

### `IpcTransport`

跨平台进程间通信。

```rust
pub trait IpcTransport: Send + Sync {
    /// 连接到 IPC 端点
    async fn connect(endpoint: &str) -> Result<IpcConnection, IpcError>;
    
    /// 监听传入连接
    async fn listen(endpoint: &str) -> Result<IpcListener, IpcError>;
    
    /// 发送消息
    async fn send(&self, msg: &[u8]) -> Result<(), IpcError>;
    
    /// 接收消息
    async fn recv(&self) -> Result<Vec<u8>, IpcError>;
}

pub enum IpcConnection {
    #[cfg(unix)]
    Unix(UnixStream),
    #[cfg(windows)]
    Windows(NamedPipeClient),
    #[cfg(feature = "tcp-fallback")]
    Tcp(TcpStream),
}
```

**实现策略：**

我们使用 [`interprocess`](https://crates.io/crates/interprocess) crate，它提供：
- **Unix：** Unix 域套接字（最高性能）
- **Windows：** 命名管道（原生等效）

```rust
// 平台无关的使用
use interprocess::local_socket::{LocalSocketStream, LocalSocketListener};

pub struct InterprocessTransport;

impl IpcTransport for InterprocessTransport {
    async fn connect(endpoint: &str) -> Result<IpcConnection, IpcError> {
        let stream = LocalSocketStream::connect(endpoint)?;
        Ok(IpcConnection::from(stream))
    }
    
    async fn listen(endpoint: &str) -> Result<IpcListener, IpcError> {
        let listener = LocalSocketListener::bind(endpoint)?;
        Ok(IpcListener::from(listener))
    }
}
```

**回退策略：**

如果 Windows 上命名管道权限受限，我们自动回退到 TCP 环回：

```rust
pub async fn connect_with_fallback(endpoint: &str) -> Result<Box<dyn IpcConnection>, IpcError> {
    // 首先尝试原生 IPC
    match IpcTransport::connect(endpoint).await {
        Ok(conn) => return Ok(Box::new(conn)),
        Err(e) if is_permission_error(&e) => {
            log::warn!("原生 IPC 失败，回退到 TCP");
        }
        Err(e) => return Err(e),
    }
    
    // 回退到环回 TCP
    let tcp_conn = TcpTransport::connect("127.0.0.1:0").await?;
    Ok(Box::new(tcp_conn))
}
```

---

### `ProcessManager`

跨平台进程生命周期管理。

```rust
pub trait ProcessManager: Send + Sync {
    /// 生成新进程
    async fn spawn(&self, config: ProcessConfig) -> Result<ProcessHandle>;
    
    /// 优雅地终止进程
    async fn terminate(&self, handle: ProcessHandle, grace_period: Duration) -> Result<()>;
    
    /// 强制终止进程
    async fn kill(&self, handle: ProcessHandle) -> Result<()>;
    
    /// 等待进程退出
    async fn wait(&self, handle: ProcessHandle) -> Result<ExitStatus>;
    
    /// 向进程发送信号/消息
    async fn signal(&self, handle: ProcessHandle, signal: ProcessSignal) -> Result<(), ProcessError>;
    
    /// 列出子进程
    fn list_children(&self) -> Vec<ProcessInfo>;
}

pub struct ProcessConfig {
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub env_inherit: bool,                    // 继承父进程环境变量，默认: true
    pub sandbox: Option<Box<dyn SandboxBackend>>,
    pub working_dir: Option<PathBuf>,
    pub stdin: StdioConfig,                   // 默认: Null
    pub stdout: StdioRedirect,
    pub stderr: StdioRedirect,
    pub resource_limits: Option<ResourceLimits>,
}

pub enum StdioConfig {
    Null,
    Inherit,
    Piped,
    File(PathBuf),
}
```

**信号映射：**

| 概念 | Linux/macOS | Windows |
|---------|-------------|---------|
| 优雅停止 | SIGTERM | Ctrl+C 事件 → TerminateProcess |
| 强制终止 | SIGKILL | TerminateProcess |
| 自定义信号 | SIGUSR1/2 | 命名事件 |
| 状态检查 | waitpid | GetExitCodeProcess |

**实现注意事项：**

在 Windows 上，我们必须避免 Unix 特性：

```rust
#[cfg(windows)]
impl ProcessManager for WindowsProcessManager {
    async fn terminate(&self, handle: ProcessHandle, grace_period: Duration) -> Result<()> {
        // Windows 没有信号；使用 Ctrl+C 或优雅关闭
        if let Some(hwnd) = get_main_window(handle) {
            // 向主窗口发送 WM_CLOSE
            post_message(hwnd, WM_CLOSE, 0, 0)?;
            
            // 等待优雅退出
            match timeout(grace_period, self.wait(handle)).await {
                Ok(_) => return Ok(()),
                Err(_) => {
                    log::warn!("优雅终止超时，强制终止");
                    self.kill(handle).await
                }
            }
        } else {
            // 无窗口，强制终止
            self.kill(handle).await
        }
    }
}
```

---

## 配置目录

通过 [`dirs`](https://crates.io/crates/dirs) crate 进行跨平台配置/数据/缓存目录处理。

```rust
pub mod dirs {
    use std::path::PathBuf;
    
    /// 配置目录
    /// - Linux: ~/.config/claw-kernel/
    /// - macOS: ~/Library/Application Support/claw-kernel/
    /// - Windows: %APPDATA%\claw-kernel\
    pub fn config_dir() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("claw-kernel"))
    }
    
    /// 数据目录（工具、脚本、持久状态）
    pub fn data_dir() -> Option<PathBuf> {
        dirs::data_dir().map(|d| d.join("claw-kernel"))
    }
    
    /// 缓存目录
    pub fn cache_dir() -> Option<PathBuf> {
        dirs::cache_dir().map(|d| d.join("claw-kernel"))
    }
    
    /// 工具目录（热加载脚本）
    pub fn tools_dir() -> Option<PathBuf> {
        data_dir().map(|d| d.join("tools"))
    }
    
    /// 运行时扩展脚本
    pub fn scripts_dir() -> Option<PathBuf> {
        data_dir().map(|d| d.join("scripts"))
    }
}
```

---

## 平台能力矩阵

| 特性 | Linux | macOS | Windows |
|---------|:-----:|:-----:|:-------:|
| **沙箱强度** | ⭐⭐⭐ | ⭐⭐ | ⭐⭐ |
| 文件系统隔离 | 强 | 中 | 中 |
| 网络隔离 | 强 | 有限 | 有限 |
| 系统调用过滤 | seccomp-bpf | 有限 | 有限 |
| **IPC 性能** | 100% | 95% | 90% |
| **构建复杂度** | 低 | 低 | 中 |
| 需要 MSVC | 否 | 否 | 是 |
| 沙箱测试 | 简单 | 中等 | 复杂 |

---

## 平台特定注意事项

### Linux

**优势：**
- 最强的沙箱（seccomp + 命名空间）
- 最佳性能
- 原生容器

**注意事项：**
- 不同内核版本有不同的 seccomp 特性
- 某些系统可能禁用用户命名空间
- AppArmor/SELinux 可能冲突

**测试：**
```bash
# 使用不同的 seccomp 配置文件运行
cargo test --features sandbox-tests

# 在用户命名空间禁用时测试
unshare -U cargo test
```

### macOS

**优势：**
- 官方沙箱 API
- 良好的开发者体验

**注意事项：**
- 沙箱配置文件是声明式的且有限
- 没有等效于 seccomp 的系统调用过滤
- 代码签名影响沙箱行为

**测试：**
```bash
# 为完整沙箱测试签名测试二进制文件
codesign -s "Developer ID" target/debug/deps/*
cargo test --features sandbox-tests
```

### Windows

**优势：**
- AppContainer 设计良好
- 作业对象用于资源限制

**注意事项：**
- 需要 MSVC 工具链（不是 GNU）
- AppContainer 设置复杂
- 命名管道与 UDS 行为略有不同
- 路径分隔符（始终使用 `std::path::Path`）
- 没有 `fork()`（使用 `CreateProcess`）

**测试：**
```powershell
# 必须使用 MSVC 工具链
rustup default stable-x86_64-pc-windows-msvc

# 以管理员身份运行测试以进行 AppContainer 测试
cargo test --features sandbox-tests
```

---

## 添加新平台

要添加对新平台（如 FreeBSD、Android）的支持：

1. **创建平台模块：**
   ```
   claw-pal/src/
   └── freebsd/
       ├── mod.rs
       ├── sandbox.rs
       ├── ipc.rs
       └── process.rs
   ```

2. **实现特性：**
   ```rust
   #[cfg(target_os = "freebsd")]
   mod freebsd;
   
   #[cfg(target_os = "freebsd")]
   pub use freebsd::*;
   ```

3. **添加平台到 CI 矩阵**

4. **在 `docs/platform/freebsd.md` 中记录**

---

## 另请参阅

- [Linux 平台指南](../platform/linux.md)
- [macOS 平台指南](../platform/macos.md)
- [Windows 平台指南](../platform/windows.md)
- [架构概述](overview.md)
