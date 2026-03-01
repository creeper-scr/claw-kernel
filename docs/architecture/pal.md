---
title: Platform Abstraction Layer (PAL)
description: PAL architecture, core traits, and platform-specific implementations
status: implemented
version: "0.1.0"
last_updated: "2026-03-01"
language: en
---

[中文版 →](pal.zh.md)

status: implemented
version: "0.1.0"
last_updated: "2026-03-01"
language: en
---

[中文版 →](pal.zh.md)

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
| **IPC Performance** (relative to Linux UDS baseline) | 100% | 95% | 90% |
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

