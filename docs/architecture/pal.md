---
title: Platform Abstraction Layer (PAL)
description: PAL architecture, core traits, and platform-specific implementations for cross-platform sandboxing, IPC, and process management
status: implemented
version: "1.0.0"
last_updated: "2026-03-09"
version: "1.0.0"
language: en
---

# Platform Abstraction Layer (PAL)

The Platform Abstraction Layer (`claw-pal`) isolates all platform-specific code, enabling claw-kernel to run on Linux, macOS, and Windows with minimal platform-specific logic in the upper layers.

---

## Table of Contents

- [Philosophy](#philosophy)
- [Architecture Overview](#architecture-overview)
- [Quick Start](#quick-start)
- [Core Traits](#core-traits)
  - [SandboxBackend](#sandboxbackend)
  - [IpcTransport](#ipctransport)
  - [ProcessManager](#processmanager)
- [Policy Types](#policy-types)
- [Configuration Directories](#configuration-directories)
- [Security Module](#security-module)
- [Error Handling](#error-handling)
- [Platform Capability Matrix](#platform-capability-matrix)
- [Platform-Specific Considerations](#platform-specific-considerations)
- [Adding a New Platform](#adding-a-new-platform)
- [See Also](#see-also)

---

## Philosophy

**"Zero Platform Assumptions"**

- All code at Layers 0.5-2 must be platform-agnostic
- Platform differences are abstracted behind traits
- Each platform implementation is a module in `claw-pal`

---

## Architecture Overview

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
│  Policy Types                                           │
│  ├── NetRule, PathRule, ResourceLimits                  │
│  └── ExecutionMode, SyscallPolicy                       │
├─────────────────────────────────────────────────────────┤
│  Platform Implementations                               │
│  ├── linux/   → seccomp-bpf, UDS, fork/exec             │
│  ├── macos/   → sandbox(7), UDS, fork/exec              │
│  └── windows/ → AppContainer, Named Pipe, CreateProcess │
└─────────────────────────────────────────────────────────┘
```

---

## Quick Start

### Creating a Sandboxed Environment

```rust
use claw_pal::{SandboxBackend, SandboxConfig, SyscallPolicy, ResourceLimits, NetRule};
use std::path::PathBuf;

// Create a safe default configuration
let config = SandboxConfig::safe_default();

// Platform-specific sandbox implementations are available via cfg flags
// On Linux:
#[cfg(target_os = "linux")]
{
    use claw_pal::LinuxSandbox;
    let sandbox = LinuxSandbox::create(config)?
        .restrict_filesystem(&[PathBuf::from("/data")])
        .restrict_network(&[NetRule::allow("api.example.com".to_string())])
        .restrict_syscalls(SyscallPolicy::DenyAll)
        .restrict_resources(ResourceLimits::restrictive())
        .apply()?;
}

// On macOS:
#[cfg(target_os = "macos")]
{
    use claw_pal::MacOSSandbox;
    let sandbox = MacOSSandbox::create(config)?
        .restrict_filesystem(&[PathBuf::from("/data")])
        .restrict_network(&[NetRule::allow("api.example.com".to_string())])
        .restrict_resources(ResourceLimits::restrictive())
        .apply()?;
}
```

### IPC Communication

```rust
use claw_pal::{IpcTransport, InterprocessTransport};

// Server side
let server = InterprocessTransport::new_server("/tmp/claw.sock").await?;
let msg = server.recv().await?;
server.send(b"response").await?;

// Client side
let client = InterprocessTransport::new_client("/tmp/claw.sock").await?;
client.send(b"hello").await?;
let response = client.recv().await?;
```

### Process Management

```rust
use claw_pal::{ProcessManager, TokioProcessManager, ProcessConfig};

let manager = TokioProcessManager::new();
let config = ProcessConfig::new("echo".to_string())
    .with_arg("hello".to_string())
    .with_env("KEY".to_string(), "value".to_string());

let handle = manager.spawn(config).await?;
let status = manager.wait(handle).await?;
```

---

## Core Traits

### `SandboxBackend`

Abstracts platform-specific sandboxing mechanisms. Implemented by `LinuxSandbox`, `MacOSSandbox`, and `WindowsSandbox`.

```rust
pub trait SandboxBackend: Send + Sync {
    fn create(config: SandboxConfig) -> Result<Self, SandboxError> where Self: Sized;
    fn restrict_filesystem(&mut self, whitelist: &[PathBuf]) -> &mut Self;
    fn restrict_network(&mut self, rules: &[NetRule]) -> &mut Self;
    fn restrict_syscalls(&mut self, policy: SyscallPolicy) -> &mut Self;
    fn restrict_resources(&mut self, limits: ResourceLimits) -> &mut Self;
    fn apply(self) -> Result<SandboxHandle, SandboxError>;
}
```

#### Configuration Types

**`SandboxConfig`** — Base configuration for sandbox creation:

```rust
pub struct SandboxConfig {
    pub mode: ExecutionMode,                  // Safe or Power
    pub filesystem_allowlist: Vec<PathBuf>,   // Paths allowed for file access
    pub network_rules: Vec<NetRule>,          // Network access rules
    pub allow_subprocess: bool,               // Whether subprocess spawning is allowed
}

impl SandboxConfig {
    pub fn safe_default() -> Self;    // Safe mode, no subprocess
    pub fn power_mode() -> Self;      // Power mode, all permissions
}
```

**`SandboxHandle`** — Opaque handle representing an applied sandbox:

```rust
pub struct SandboxHandle {
    pub platform_handle: PlatformHandle,
}

pub enum PlatformHandle {
    /// Linux seccomp-bpf filter ID
    Linux(i32),
    /// macOS sandbox profile identifier
    MacOs(String),
    /// Windows AppContainer SID
    Windows(u32),
    /// Unsupported platform fallback
    Unsupported,
}
```

---

### `IpcTransport`

Cross-platform inter-process communication. Fully implemented on Linux, macOS, and Windows.

```rust
#[async_trait]
pub trait IpcTransport: Send + Sync {
    async fn connect(endpoint: &str) -> Result<IpcConnection, IpcError>;
    async fn listen(endpoint: &str) -> Result<IpcListener, IpcError>;
    async fn send(&self, msg: &[u8]) -> Result<(), IpcError>;
    async fn recv(&self) -> Result<Vec<u8>, IpcError>;
}
```

#### Core Types

```rust
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

#### Implementation: `InterprocessTransport`

Platform-specific implementation backing the `IpcTransport` trait:

| Platform | Backend | Status | Notes |
|----------|---------|--------|-------|
| Linux | `interprocess` crate | ✅ v1.0.0 | Unix Domain Sockets |
| macOS | `interprocess` crate | ✅ v1.0.0 | Unix Domain Sockets |
| Windows | `tokio::net::windows::named_pipe` | ✅ v1.0.0 | Named Pipes with full bidirectional support |

```rust
pub struct InterprocessTransport {
    // Platform-specific writer: OwnedWriteHalf on Unix, PipeWriter enum on Windows
    writer: Mutex<WriterType>,
    // Channel for receiving frames from the background reader task
    recv_rx: Mutex<mpsc::Receiver<Result<Vec<u8>, IpcError>>>,
    // Keeps the reader task alive for the lifetime of this transport
    _reader_task: tokio::task::JoinHandle<()>,
}

impl InterprocessTransport {
    /// Connect as a client
    pub async fn new_client(endpoint: &str) -> Result<Self, IpcError>;
    
    /// Create server and accept one connection
    pub async fn new_server(endpoint: &str) -> Result<Self, IpcError>;
}
```

**Design Pattern:** Uses a single background reader task that continuously reads frames and forwards them via an mpsc channel. This avoids concurrent bi-directional split I/O issues (known to panic on macOS with `interprocess` 1.2.1).

#### IPC Framing Protocol

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

#### Testing: `MockIpcTransport`

For unit testing without actual IPC infrastructure:

```rust
use claw_pal::traits::ipc::MockIpcTransport;

let transport = MockIpcTransport::new("/tmp/test".to_string());
transport.send(b"test message").await?;
let msg = transport.recv().await?;
```

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
```

#### Configuration Types

```rust
pub struct ProcessConfig {
    pub program: String,                      // Program name or path
    pub args: Vec<String>,                    // Command-line arguments
    pub env: HashMap<String, String>,         // Environment variables
    pub working_dir: Option<PathBuf>,         // Working directory
}

impl ProcessConfig {
    pub fn new(program: String) -> Self;
    pub fn with_arg(self, arg: String) -> Self;
    pub fn with_args(self, args: Vec<String>) -> Self;
    pub fn with_env(self, key: String, value: String) -> Self;
    pub fn with_working_dir(self, dir: PathBuf) -> Self;
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

#### Implementation: `TokioProcessManager`

```rust
pub struct TokioProcessManager {
    children: Arc<DashMap<u32, Mutex<Child>>>,
}

impl TokioProcessManager {
    pub fn new() -> Self;
}

impl ProcessManager for TokioProcessManager {
    async fn spawn(&self, config: ProcessConfig) -> Result<ProcessHandle, ProcessError>;
    async fn terminate(&self, handle: ProcessHandle, grace_period: Duration) -> Result<(), ProcessError>;
    async fn kill(&self, handle: ProcessHandle) -> Result<(), ProcessError>;
    async fn wait(&self, handle: ProcessHandle) -> Result<ExitStatus, ProcessError>;
    async fn signal(&self, handle: ProcessHandle, signal: ProcessSignal) -> Result<(), ProcessError>;
}
```

**Signal Mapping:**

| Concept | Linux/macOS | Windows |
|---------|-------------|---------|
| Graceful stop | SIGTERM | Ctrl+C → TerminateProcess |
| Force kill | SIGKILL | TerminateProcess |
| Interrupt | SIGINT | TerminateProcess (fallback) |
| Status check | waitpid | GetExitCodeProcess |

---

## Policy Types

### Network Rules

```rust
pub struct NetRule {
    pub host: String,                         // Hostname or IP address
    pub port: Option<u16>,                    // Port number (None = all ports)
    pub allow: bool,                          // Allow (true) or deny (false)
}

impl NetRule {
    pub fn new(host: String, port: Option<u16>, allow: bool) -> Self;
    pub fn allow(host: String) -> Self;
    pub fn allow_port(host: String, port: u16) -> Self;
    pub fn deny(host: String) -> Self;
}
```

### Filesystem Rules

```rust
pub struct PathRule {
    pub path: PathBuf,
    pub read: bool,
    pub write: bool,
    pub execute: bool,
}

impl PathRule {
    pub fn new(path: PathBuf) -> Self;        // All permissions disabled
    pub fn with_read(self) -> Self;
    pub fn with_write(self) -> Self;
    pub fn with_execute(self) -> Self;
}
```

### Resource Limits

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
    
    // Builder methods
    pub fn with_memory(self, bytes: u64) -> Self;
    pub fn with_cpu(self, percent: u8) -> Self;    // clamped to 100
    pub fn with_fds(self, count: u32) -> Self;
    pub fn with_processes(self, count: u32) -> Self;
}
```

### Syscall Policy

```rust
pub enum SyscallPolicy {
    AllowAll,                                 // Allow all syscalls
    DenyAll,                                  // Deny dangerous syscalls
    Allowlist(Vec<String>),                   // Allow only specific syscalls by name
}
```

### Execution Mode

```rust
pub enum ExecutionMode {
    Safe,                                     // Restricted access (default)
    Power,                                    // Full system access (opt-in)
}
```

---

## Configuration Directories

Cross-platform config/data/cache directory handling via the [`dirs`](https://crates.io/crates/dirs) crate.

### Module Functions

```rust
pub mod dirs {
    use std::path::PathBuf;
    
    /// Configuration directory
    /// - Linux: ~/.config/claw-kernel/
    /// - macOS: ~/Library/Application Support/claw-kernel/
    /// - Windows: %APPDATA%\claw-kernel\
    pub fn config_dir() -> Option<PathBuf>;
    
    /// Data directory
    /// - Linux: ~/.local/share/claw-kernel/
    /// - macOS: ~/Library/Application Support/claw-kernel/
    /// - Windows: %APPDATA%\claw-kernel\
    pub fn data_dir() -> Option<PathBuf>;
    
    /// Cache directory
    /// - Linux: ~/.cache/claw-kernel/
    /// - macOS: ~/Library/Caches/claw-kernel/
    /// - Windows: %LOCALAPPDATA%\claw-kernel\Cache
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
```

### Convenience Struct

```rust
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

Power Key validation, Argon2 hashing, and mode transition guards. Implements the dual-mode security model described in [ADR-003](../adr/003-security-model.md).

### Power Key Validator

Validates Power Key strength requirements:
- Minimum 12 characters
- At least 2 distinct character types (uppercase, lowercase, digit, special)

```rust
pub struct PowerKeyValidator;

impl PowerKeyValidator {
    pub fn validate(key: &str) -> Result<(), SecurityError>;
}
```

### Power Key Hash (Argon2)

For secure persistent storage with Argon2 hashing:

```rust
pub struct PowerKeyHash(String);

impl PowerKeyHash {
    /// Create new hash from plaintext key (validates key strength first)
    pub fn new(key: &str) -> Result<Self, SecurityError>;
    
    /// Verify candidate key against stored hash (constant-time comparison)
    pub fn verify(&self, candidate: &str) -> bool;
    
    /// Load hash from previously stored string representation
    pub fn from_string(hash: &str) -> Result<Self, SecurityError>;
}

impl fmt::Display for PowerKeyHash {
    /// Returns the hash string for storage
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result;
}
```

### Power Key (SHA-256)

For deterministic verification (no salt, vulnerable to rainbow table attacks — use only for temporary/in-memory verification):

```rust
pub struct PowerKey {
    verification_hash: [u8; 32],
}

impl PowerKey {
    /// Create new PowerKey from plaintext key (computes SHA-256 hash)
    pub fn new(key: &str) -> Self;
    
    /// Verify provided key against stored hash (constant-time comparison)
    pub fn verify(&self, provided: &str) -> bool;
    
    /// Load from file containing hex-encoded hash
    pub fn load_from_file(path: &Path) -> Result<Self, SecurityError>;
    
    /// Save hash to file as hex-encoded string
    pub fn save_to_file(&self, path: &Path) -> Result<(), SecurityError>;
}
```

### Power Key Manager

```rust
pub struct PowerKeyManager;

impl PowerKeyManager {
    pub fn save_power_key(key: &str) -> Result<(), SecurityError>;
    pub fn load_stored_hash() -> Result<PowerKeyHash, SecurityError>;
    pub fn is_configured() -> bool;
    pub fn resolve_power_key(cli_key: Option<String>) -> Option<String>;
}
```

**Resolution Priority:**
1. CLI argument (`--power-key`)
2. Environment variable (`CLAW_KERNEL_POWER_KEY`)
3. Config file (`~/.config/claw-kernel/power.key`)

### Mode Transition Guard

```rust
pub struct ModeTransitionGuard;

impl ModeTransitionGuard {
    /// Enter Power Mode from Safe Mode (requires valid key)
    pub fn enter_power_mode(key: &str, stored_hash: &PowerKeyHash) -> Result<ExecutionMode, SecurityError>;
    
    /// Exit Power Mode - always returns Err(ModeTransitionDenied)
    /// Power Mode → Safe Mode requires process restart (per ADR-003)
    pub fn exit_power_mode() -> Result<ExecutionMode, SecurityError>;
}
```

**Important:** `exit_power_mode()` always returns `Err(SecurityError::ModeTransitionDenied)` because Power Mode → Safe Mode requires process restart. This prevents a compromised Power Mode agent from hiding evidence.

### Security Errors

```rust
pub enum SecurityError {
    KeyTooShort { len: usize, min: usize },
    InsufficientComplexity { found_types: usize, required: usize },
    InvalidPowerKey,
    ModeTransitionDenied { from: ExecutionMode, to: ExecutionMode },
    HashError(String),                        // Internal hashing error
}
```

---

## Error Handling

### Error Types

```rust
/// Sandbox-related errors
pub enum SandboxError {
    CreationFailed(String),
    RestrictFailed(String),
    AlreadyApplied,
    NotSupported,
}

/// IPC-related errors
#[non_exhaustive]
pub enum IpcError {
    ConnectionRefused,
    Timeout,
    BrokenPipe,
    InvalidMessage,
    PermissionDenied,
}

/// Process-related errors
pub enum ProcessError {
    SpawnFailed(String),
    SignalFailed(String),
    NotFound(u32),
    PermissionDenied,
    InvalidSignal,
}

/// Unified error type
pub enum PalError {
    Sandbox(#[from] SandboxError),
    Ipc(#[from] IpcError),
    Process(#[from] ProcessError),
    PermissionDenied(String),
    Io(String),
}
```

### Best Practices

1. **Use `PalError` for general operations** — Automatically converts from specific error types
2. **Handle `#[non_exhaustive]` IPC errors** — New variants may be added in future versions
3. **Check `SecurityError` variants** — Provide user-friendly messages for key validation failures

---

## Platform Capability Matrix

| Feature | Linux | macOS | Windows |
|---------|:-----:|:-----:|:-------:|
| **Sandbox Strength** | ⭐⭐⭐ | ⭐⭐ | ⭐⭐ (stub) |
| Filesystem isolation | Strong (seccomp + mount ns) | Medium (sandbox(7)) | Medium (planned) |
| Network isolation | Strong (seccomp-bpf) | Limited (SBPL) | Limited (planned) |
| Syscall filtering | seccomp-bpf | SBPL operations | Limited (planned) |
| Process isolation | pid namespace | Standard | Job Objects (planned) |
| **IPC** | UDS | UDS | Named Pipes |
| **IPC Performance** (relative to Linux) | 100% | 95% | 90% |
| **Build Complexity** | Low | Low | Medium |
| MSVC required | No | No | Yes |
| Sandbox testing | Easy | Medium | Complex |

### Implementation Status Summary

| Component | Linux | macOS | Windows |
|-----------|:-----:|:-----:|:-------:|
| `LinuxSandbox`/`MacOSSandbox`/`WindowsSandbox` | ✅ Full | ✅ Full | ⚠️ Stub |
| `InterprocessTransport` | ✅ UDS (Unix Domain Socket) | ✅ UDS | ✅ Named Pipes (v1.0.0) |
| `TokioProcessManager` | ✅ Full | ✅ Full | ✅ Full |

---

## Platform-Specific Considerations

### Linux

**Strengths:**
- Strongest sandboxing (seccomp-bpf + namespaces)
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
- `sandbox_init()` is **irreversible** — once applied, cannot be removed

**Testing:**
```bash
# Sign test binaries for full sandbox testing
codesign -s "Developer ID" target/debug/deps/*
cargo test --features sandbox-tests
```

### Windows

**Status for v1.0.0:**
- **Named Pipe IPC**: ✅ **Fully implemented** via `tokio::net::windows::named_pipe`
- **Process management**: ✅ **Full implementation** via Tokio
- **AppContainer sandbox**: ⚠️ **Stub implementation** — returns handle without actual sandbox enforcement

**Windows Sandbox Limitations:**
The Windows sandbox is currently a stub that stores configuration but does not enforce restrictions.
For production use on Windows, additional security measures are recommended:
- Use Power Mode only in trusted environments
- Consider third-party sandboxing solutions
- Run agents in Windows containers or VMs for isolation

**Planned for future versions:**
- Full AppContainer implementation with `CreateAppContainerProfile()`
- Job Objects for resource limits enforcement

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
- [ADR-003: Dual-Mode Security](../adr/003-security-model.md)

---
