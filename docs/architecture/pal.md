---
title: Platform Abstraction Layer (PAL)
description: PAL architecture, core traits, and platform-specific implementations
status: implemented
version: "0.1.0"
last_updated: "2026-03-08"
language: en
---


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
    fn restrict_filesystem(&mut self, whitelist: &[PathBuf]) -> &mut Self;
    fn restrict_network(&mut self, rules: &[NetRule]) -> &mut Self;
    fn restrict_syscalls(&mut self, policy: SyscallPolicy) -> &mut Self;
    fn restrict_resources(&mut self, limits: ResourceLimits) -> &mut Self;
    fn apply(self) -> Result<SandboxHandle, SandboxError>;
}

pub struct SandboxConfig {
    pub mode: ExecutionMode,                  // Safe or Power
    pub filesystem_allowlist: Vec<PathBuf>,   // Paths allowed for file access
    pub network_rules: Vec<NetRule>,          // Network access rules
    pub allow_subprocess: bool,               // Whether subprocess spawning is allowed
}

pub struct SandboxHandle {
    pub platform_handle: PlatformHandle,
}

pub enum PlatformHandle {
    #[cfg(target_os = "linux")]
    Linux(i32),                               // Process ID with seccomp applied
    #[cfg(target_os = "macos")]
    MacOs(String),                            // Sandbox profile identifier
    #[cfg(target_os = "windows")]
    Windows(u32),                             // AppContainer SID (stub in v0.1)
    Unsupported,                              // Fallback for unknown platforms
}
```

#### Network Rules

```rust
pub struct NetRule {
    pub host: String,                         // Hostname or IP address
    pub port: Option<u16>,                    // Port number (None = all ports)
    pub allow: bool,                          // Allow (true) or deny (false)
}

impl NetRule {
    pub fn allow(host: String) -> Self;
    pub fn allow_port(host: String, port: u16) -> Self;
    pub fn deny(host: String) -> Self;
}
```

#### Syscall Policy

```rust
pub enum SyscallPolicy {
    AllowAll,                                 // Allow all syscalls
    DenyAll,                                  // Deny dangerous syscalls
    Allowlist(Vec<String>),                   // Allow only specific syscalls by name
}
```

#### Resource Limits

```rust
pub struct ResourceLimits {
    pub max_memory_bytes: Option<u64>,        // Maximum memory in bytes
    pub max_cpu_percent: Option<u8>,          // Maximum CPU usage (0-100)
    pub max_file_descriptors: Option<u32>,    // Maximum number of open FDs
    pub max_processes: Option<u32>,           // Maximum number of processes
}

impl ResourceLimits {
    pub fn unlimited() -> Self;
    pub fn restrictive() -> Self;             // 256MB, 50% CPU, 256 FDs, 10 procs
}
```

### Platform Implementations

| Capability | Linux | macOS | Windows |
|------------|-------|-------|---------|
| Filesystem | seccomp + mount namespace | sandbox(7) profile | AppContainer capabilities |
| Network | network namespace | Socket filter (limited) | WFP (Windows Filtering Platform) |
| Syscalls | seccomp-bpf filter | Limited (SBPL operations) | Limited (API hooking) |
| Process | pid namespace | Standard | Job Objects |

**Linux Implementation:**
```rust
#[cfg(target_os = "linux")]
pub struct LinuxSandbox {
    config: SandboxConfig,
    filesystem_rules: Vec<PathBuf>,
    network_rules: Vec<NetRule>,
    syscall_policy: Option<SyscallPolicy>,
    resource_limits: Option<ResourceLimits>,
}

impl SandboxBackend for LinuxSandbox {
    fn apply(self) -> Result<SandboxHandle, SandboxError> {
        // 1. In Power mode: skip all restrictions
        // 2. Attempt mount namespace isolation (non-fatal)
        // 3. Apply resource limits via setrlimit(2)
        // 4. Build and load seccomp-bpf filter with SCMP_ACT_ERRNO(EPERM)
        
        Ok(SandboxHandle {
            platform_handle: PlatformHandle::Linux(std::process::id() as i32),
        })
    }
}
```

**macOS Implementation:**
```rust
#[cfg(target_os = "macos")]
pub struct MacOSSandbox {
    config: SandboxConfig,
    filesystem_rules: Vec<PathBuf>,
    network_rules: Vec<NetRule>,
    syscall_policy: Option<SyscallPolicy>,
    resource_limits: Option<ResourceLimits>,
}

impl SandboxBackend for MacOSSandbox {
    fn apply(self) -> Result<SandboxHandle, SandboxError> {
        // macOS uses declarative sandbox profiles (SBPL)
        let profile_str = self.generate_profile()?;
        
        // Apply via sandbox_init() FFI — this is IRREVERSIBLE
        Self::apply_sandbox_profile(&profile_str)?;
        
        Ok(SandboxHandle {
            platform_handle: PlatformHandle::MacOs("safe-mode-sandboxed".to_string()),
        })
    }
}

// Example generated SBPL profile:
// (version 1)
// (deny default)
// (allow sysctl-read)
// (allow mach-lookup)
// (allow file-read* (subpath "/allowed/path"))
// (allow network-outbound (remote tcp "example.com:443"))
```

**Windows Implementation (Stub for v0.1.0):**
```rust
#[cfg(target_os = "windows")]
pub struct WindowsSandbox {
    config: SandboxConfig,
    filesystem_rules: Vec<PathBuf>,
    network_rules: Vec<NetRule>,
    syscall_policy: Option<SyscallPolicy>,
    resource_limits: Option<ResourceLimits>,
}

impl SandboxBackend for WindowsSandbox {
    fn apply(self) -> Result<SandboxHandle, SandboxError> {
        // v0.1.0: Stub implementation
        // Returns handle without actual AppContainer creation
        // Full implementation planned for v0.2.0
        
        Ok(SandboxHandle {
            platform_handle: PlatformHandle::Windows(
                if self.config.mode == ExecutionMode::Power { 0 } else { 1 }
            ),
        })
    }
}
```

---

### `IpcTransport`

Cross-platform inter-process communication.

> **v0.1.0 Limitation:** IPC is currently only supported on Unix-like systems (Linux, macOS).
> Windows Named Pipe support is planned for v0.2.0. On Windows, IPC operations will return
> `IpcError::ConnectionRefused`.

```rust
#[async_trait]
pub trait IpcTransport: Send + Sync {
    /// Connect to an IPC endpoint (returns metadata only)
    async fn connect(endpoint: &str) -> Result<IpcConnection, IpcError>;
    
    /// Listen on an IPC endpoint (returns metadata only)
    async fn listen(endpoint: &str) -> Result<IpcListener, IpcError>;
    
    /// Send a message
    async fn send(&self, msg: &[u8]) -> Result<(), IpcError>;
    
    /// Receive a message
    async fn recv(&self) -> Result<Vec<u8>, IpcError>;
}

pub struct IpcConnection {
    pub endpoint: String,
}

pub struct IpcListener {
    pub endpoint: String,
}

pub enum IpcEndpoint {
    UnixSocket(PathBuf),
    NamedPipe(String),
}

pub struct IpcMessage {
    pub id: u64,
    pub payload: Vec<u8>,
    pub timestamp: u64,
}
```

**IPC Framing Protocol:**

Wire format: **4-byte Big Endian (BE)** length prefix followed by payload bytes.
Maximum frame payload size: 16 MiB (0x100_0000 bytes).

```rust
// Frame structure:
// [4 bytes: length in BE] [N bytes: payload]

pub async fn write_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    data: &[u8],
) -> Result<(), IpcError>;

pub async fn read_frame<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> Result<Vec<u8>, IpcError>;
```

**Implementation Strategy:**

We use the [`interprocess`](https://crates.io/crates/interprocess) crate which provides:
- **Unix:** Unix Domain Sockets (highest performance) — ✅ **Implemented in v0.1.0**
- **Windows:** Named Pipes (native equivalent) — 🚫 **Planned for v0.2.0**

```rust
// Unix implementation (v0.1.0)
pub struct InterprocessTransport {
    writer: Mutex<OwnedWriteHalf>,
    recv_rx: Mutex<mpsc::Receiver<Result<Vec<u8>, IpcError>>>,
    _reader_task: tokio::task::JoinHandle<()>,
}

impl InterprocessTransport {
    /// Connect as a client to the given endpoint path.
    pub async fn new_client(endpoint: &str) -> Result<Self, IpcError>;
    
    /// Bind a listener, accept exactly one incoming connection.
    pub async fn new_server(endpoint: &str) -> Result<Self, IpcError>;
}
```

**Design Note:** The implementation uses a single background reader task that continuously
reads frames from the socket and forwards them via an mpsc channel. This avoids concurrent
bi-directional split I/O on the same socket (which panics on macOS with interprocess 1.2.1).

---

### `ProcessManager`

Cross-platform process lifecycle management.

```rust
#[async_trait]
pub trait ProcessManager: Send + Sync {
    async fn spawn(&self, config: ProcessConfig) -> Result<ProcessHandle, ProcessError>;
    async fn terminate(&self, handle: ProcessHandle, grace_period: Duration) -> Result<(), ProcessError>;
    async fn kill(&self, handle: ProcessHandle) -> Result<(), ProcessError>;
    async fn wait(&self, handle: ProcessHandle) -> Result<ExitStatus, ProcessError>;
    async fn signal(&self, handle: ProcessHandle, signal: ProcessSignal) -> Result<(), ProcessError>;
}

pub struct ProcessConfig {
    pub program: String,                      // Program name or path
    pub args: Vec<String>,                    // Command-line arguments
    pub env: HashMap<String, String>,         // Environment variables
    pub working_dir: Option<PathBuf>,         // Working directory
}

pub struct ProcessHandle {
    pub pid: u32,
    pub name: String,
}

pub struct ExitStatus {
    pub code: Option<i32>,                    // Exit code (None if terminated by signal)
    pub success: bool,                        // Whether process exited successfully
}

pub enum ProcessSignal {
    Term,                                     // SIGTERM (Unix), TerminateProcess (Windows)
    Kill,                                     // SIGKILL (Unix), TerminateProcess (Windows)
    Interrupt,                                // SIGINT (Unix), Ctrl+C (Windows)
}
```

**Tokio-based Implementation:**

```rust
pub struct TokioProcessManager {
    children: Arc<DashMap<u32, Mutex<Child>>>,
}

impl ProcessManager for TokioProcessManager {
    async fn spawn(&self, config: ProcessConfig) -> Result<ProcessHandle, ProcessError> {
        // Uses tokio::process::Command
        // Stores Child in DashMap keyed by PID
    }
    
    async fn terminate(&self, handle: ProcessHandle, grace_period: Duration) -> Result<(), ProcessError> {
        // 1. Send SIGTERM (Unix) or skip (Windows)
        // 2. Wait up to grace_period
        // 3. If timeout, send SIGKILL/TerminateProcess
    }
    
    async fn kill(&self, handle: ProcessHandle) -> Result<(), ProcessError> {
        // Send SIGKILL (Unix) or TerminateProcess (Windows)
        // Reap the zombie via wait()
    }
    
    async fn wait(&self, handle: ProcessHandle) -> Result<ExitStatus, ProcessError> {
        // Block until process exits
        // Return ExitStatus with code and success flag
    }
    
    async fn signal(&self, handle: ProcessHandle, signal: ProcessSignal) -> Result<(), ProcessError> {
        // Map ProcessSignal to platform-specific signal
        // On Windows: Term/Interrupt fall back to kill()
    }
}
```

**Signal Mapping:**

| Concept | Linux/macOS | Windows |
|---------|-------------|---------|
| Graceful stop | SIGTERM | Ctrl+C event → TerminateProcess |
| Force kill | SIGKILL | TerminateProcess |
| Interrupt | SIGINT | TerminateProcess (fallback) |
| Status check | waitpid | GetExitCodeProcess |

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
    pub fn config_dir() -> Option<PathBuf>;
    
    /// Data directory (tools, scripts, persistent state)
    pub fn data_dir() -> Option<PathBuf>;
    
    /// Cache directory
    pub fn cache_dir() -> Option<PathBuf>;
    
    /// Tools directory (hot-loaded scripts)
    pub fn tools_dir() -> Option<PathBuf>;
    
    /// Runtime extension scripts directory
    pub fn scripts_dir() -> Option<PathBuf>;
    
    /// Logs directory
    pub fn logs_dir() -> Option<PathBuf>;
    
    /// Agents directory
    pub fn agents_dir() -> Option<PathBuf>;
    
    /// Power key file path
    pub fn power_key_path() -> Option<PathBuf>;
}

/// Kernel directory paths (convenience struct)
pub struct KernelDirs {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub log_dir: PathBuf,
    pub agents_dir: PathBuf,
    pub tools_dir: PathBuf,
}

impl KernelDirs {
    pub fn new() -> Result<Self, std::io::Error>;
    pub async fn ensure_all(&self) -> Result<(), std::io::Error>;
    pub fn ensure_all_sync(&self) -> Result<(), std::io::Error>;
}
```

---

## Security Module

Power Key validation, Argon2 hashing, and mode transition guards.

```rust
/// Validates Power Key strength requirements.
pub struct PowerKeyValidator;

impl PowerKeyValidator {
    /// Validate a Power Key against security requirements.
    /// Rules:
    /// - Length >= 12 characters
    /// - At least 2 distinct character types (uppercase, lowercase, digit, special)
    pub fn validate(key: &str) -> Result<(), SecurityError>;
}

/// Argon2 hashed Power Key for secure storage.
pub struct PowerKeyHash(String);

impl PowerKeyHash {
    pub fn new(key: &str) -> Result<Self, SecurityError>;
    pub fn verify(&self, candidate: &str) -> bool;
    pub fn from_string(hash: &str) -> Result<Self, SecurityError>;
}

/// SHA-256 based Power Key for verification (deterministic, no salt).
pub struct PowerKey {
    verification_hash: [u8; 32],
}

impl PowerKey {
    pub fn new(key: &str) -> Self;
    pub fn verify(&self, provided: &str) -> bool;
    pub fn load_from_file(path: &Path) -> Result<Self, SecurityError>;
    pub fn save_to_file(&self, path: &Path) -> Result<(), SecurityError>;
}

/// Manages Power Key persistence and retrieval.
pub struct PowerKeyManager;

impl PowerKeyManager {
    /// Save a Power Key to the config file (hashed with Argon2).
    pub fn save_power_key(key: &str) -> Result<(), SecurityError>;
    
    /// Load the stored Power Key hash from config file.
    pub fn load_stored_hash() -> Result<PowerKeyHash, SecurityError>;
    
    /// Check if a Power Key has been configured.
    pub fn is_configured() -> bool;
    
    /// Resolve the effective Power Key following priority order:
    /// 1. CLI argument (`--power-key`)
    /// 2. Environment variable (`CLAW_KERNEL_POWER_KEY`)
    /// 3. Config file (`~/.config/claw-kernel/power.key`)
    pub fn resolve_power_key(cli_key: Option<String>) -> Option<String>;
}

/// Guard for mode transitions between Safe and Power modes.
pub struct ModeTransitionGuard;

impl ModeTransitionGuard {
    /// Attempt to enter Power Mode from Safe Mode.
    /// Requires a valid Power Key that matches the stored hash.
    pub fn enter_power_mode(
        key: &str,
        stored_hash: &PowerKeyHash,
    ) -> Result<ExecutionMode, SecurityError>;
    
    /// Attempt to exit Power Mode (always denied).
    /// Per ADR-003: Power Mode → Safe Mode requires process restart.
    pub fn exit_power_mode() -> Result<ExecutionMode, SecurityError>;
}

/// Security-related errors.
pub enum SecurityError {
    KeyTooShort { len: usize, min: usize },
    InsufficientComplexity { found_types: usize, required: usize },
    InvalidPowerKey,
    ModeTransitionDenied { from: ExecutionMode, to: ExecutionMode },
    HashError(String),
}
```

---

## Error Types

```rust
/// Sandbox-related errors.
pub enum SandboxError {
    CreationFailed(String),
    RestrictFailed(String),
    AlreadyApplied,
    NotSupported,
}

/// IPC-related errors.
pub enum IpcError {
    ConnectionRefused,
    Timeout,
    BrokenPipe,
    InvalidMessage,
    PermissionDenied,
}

/// Process-related errors.
pub enum ProcessError {
    SpawnFailed(String),
    SignalFailed(String),
    NotFound(u32),
    PermissionDenied,
    InvalidSignal,
}

/// Unified error type for claw-pal operations.
pub enum PalError {
    Sandbox(SandboxError),
    Ipc(IpcError),
    Process(ProcessError),
    PermissionDenied(String),
    Io(String),
}
```

---

## Platform Capability Matrix

| Feature | Linux | macOS | Windows |
|---------|:-----:|:-----:|:-------:|
| **Sandbox Strength** | ⭐⭐⭐ | ⭐⭐ | ⭐⭐ (stub) |
| Filesystem isolation | Strong | Medium | Medium (planned) |
| Network isolation | Strong | Limited | Limited (planned) |
| Syscall filtering | seccomp-bpf | SBPL operations | Limited (planned) |
| **IPC Performance** (relative to Linux UDS baseline) | 100% | 95% | N/A (v0.2.0) |
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

**Status for v0.1.0:**
- AppContainer sandbox: **Stub implementation**
- Named Pipe IPC: **Not implemented** (returns `IpcError::ConnectionRefused`)
- Process management: **Full implementation** via Tokio

**Planned for v0.2.0:**
- Full AppContainer implementation with `CreateAppContainerProfile()`
- Named Pipe IPC support
- Job Objects for resource limits

**Gotchas:**
- MSVC toolchain required (not GNU)
- AppContainer setup is complex
- Named Pipes behave subtly differently from UDS
- Path separators (always use `std::path::Path`)
- No `fork()` (use `CreateProcess`)

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
