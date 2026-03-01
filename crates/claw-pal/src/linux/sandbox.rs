//! Linux sandbox implementation using seccomp-bpf and setrlimit.
//!
//! Implements [`SandboxBackend`] for Linux using:
//! - **libseccomp** for syscall filtering with `SCMP_ACT_ERRNO(EPERM)`
//! - **nix::sys::resource::setrlimit** for resource limits (memory, FDs, processes)
//! - **nix::sched::unshare** for optional namespace isolation
//!
//! # Design Decisions
//!
//! - Uses `SCMP_ACT_ERRNO(EPERM)` instead of `SCMP_ACT_KILL` to prevent Rust panics
//!   when thread join detects a killed thread.
//! - seccomp cannot do path-based filesystem filtering; the filesystem whitelist is stored
//!   for higher-level enforcement (e.g., Landlock LSM or mount namespaces).
//! - The seccomp filter is built during `apply()`, not during configuration methods,
//!   so the `LinuxSandbox` struct remains `Send + Sync`.

use crate::error::SandboxError;
use crate::traits::sandbox::{
    ExecutionMode, PlatformHandle, SandboxBackend, SandboxConfig, SandboxHandle, SyscallPolicy,
};
use crate::types::{NetRule, ResourceLimits};

use libseccomp::{ScmpAction, ScmpFilterContext, ScmpSyscall};
use nix::sys::resource::{setrlimit, Resource};

use std::path::PathBuf;

/// EPERM errno value (Permission denied) for seccomp ERRNO action.
/// This is POSIX-standardized as 1 on all Linux architectures.
const EPERM: i32 = 1;

/// Syscalls considered dangerous and blocked in `DenyAll` policy.
const DANGEROUS_SYSCALLS: &[&str] = &[
    "execve",
    "execveat",
    "ptrace",
    "process_vm_readv",
    "process_vm_writev",
    "mount",
    "umount2",
    "pivot_root",
    "chroot",
    "reboot",
    "kexec_load",
    "init_module",
    "finit_module",
    "delete_module",
];

/// Network-related syscalls blocked in Safe mode without explicit allow rules.
const NETWORK_SYSCALLS: &[&str] = &["socket", "connect", "bind", "listen", "accept", "accept4"];

/// Exec-family syscalls blocked when subprocess spawning is disabled.
const EXEC_SYSCALLS: &[&str] = &["execve", "execveat"];

/// Linux sandbox implementation using seccomp-bpf and setrlimit.
///
/// Provides process-level isolation through:
/// - **Syscall filtering**: seccomp-bpf with configurable policies
/// - **Network restriction**: Blocks socket-related syscalls in Safe mode
/// - **Resource limits**: Uses `setrlimit` for memory, FDs, and process counts
/// - **Subprocess blocking**: Blocks `execve`/`execveat` in Safe mode
///
/// # Filesystem Restriction Note
///
/// seccomp operates at the syscall level and cannot perform path-based filesystem
/// filtering. The filesystem whitelist is stored for use by higher-level components.
/// For path-level enforcement, consider:
/// - **Landlock LSM** (Linux 5.13+) for unprivileged path-based restrictions
/// - **Mount namespaces** with `CAP_SYS_ADMIN` for filesystem isolation
///
/// # Example
///
/// ```rust,no_run
/// # #[cfg(target_os = "linux")]
/// # {
/// use claw_pal::linux::LinuxSandbox;
/// use claw_pal::traits::sandbox::{SandboxBackend, SandboxConfig, SyscallPolicy};
/// use claw_pal::types::ResourceLimits;
///
/// let config = SandboxConfig::safe_default();
/// let mut sandbox = LinuxSandbox::create(config).unwrap();
///
/// sandbox
///     .restrict_syscalls(SyscallPolicy::DenyAll)
///     .restrict_resources(ResourceLimits::restrictive());
///
/// let handle = sandbox.apply().unwrap();
/// // Sandbox is now active — dangerous syscalls return EPERM
/// # }
/// ```
pub struct LinuxSandbox {
    /// Sandbox configuration (mode, subprocess policy).
    config: SandboxConfig,
    /// Filesystem whitelist (stored for higher-level enforcement).
    filesystem_rules: Vec<PathBuf>,
    /// Network access rules.
    network_rules: Vec<NetRule>,
    /// Syscall filtering policy.
    syscall_policy: Option<SyscallPolicy>,
    /// Resource limits to apply via setrlimit.
    resource_limits: Option<ResourceLimits>,
}

impl LinuxSandbox {
    /// Apply resource limits using `setrlimit(2)`.
    ///
    /// Sets hard and soft limits to the same value for:
    /// - `RLIMIT_AS`: Maximum virtual memory (address space)
    /// - `RLIMIT_NOFILE`: Maximum number of open file descriptors
    /// - `RLIMIT_NPROC`: Maximum number of processes for the real user ID
    ///
    /// # Note
    ///
    /// Resource limits are persistent and cannot be raised once lowered
    /// (without `CAP_SYS_RESOURCE`). This is called before seccomp filter
    /// loading so that limit-related failures are reported early.
    fn apply_resource_limits(limits: &ResourceLimits) -> Result<(), SandboxError> {
        if let Some(max_memory) = limits.max_memory_bytes {
            setrlimit(Resource::RLIMIT_AS, max_memory, max_memory).map_err(|e| {
                SandboxError::RestrictFailed(format!("failed to set memory limit: {}", e))
            })?;
        }

        if let Some(max_fds) = limits.max_file_descriptors {
            setrlimit(
                Resource::RLIMIT_NOFILE,
                u64::from(max_fds),
                u64::from(max_fds),
            )
            .map_err(|e| {
                SandboxError::RestrictFailed(format!("failed to set file descriptor limit: {}", e))
            })?;
        }

        if let Some(max_procs) = limits.max_processes {
            setrlimit(
                Resource::RLIMIT_NPROC,
                u64::from(max_procs),
                u64::from(max_procs),
            )
            .map_err(|e| {
                SandboxError::RestrictFailed(format!("failed to set process limit: {}", e))
            })?;
        }

        Ok(())
    }

    /// Build a seccomp-bpf filter based on the configured restrictions.
    ///
    /// The filter uses `SCMP_ACT_ERRNO(EPERM)` as the deny action to avoid
    /// thread-level kills that cause Rust panics on `thread::join()`.
    ///
    /// # Filter Construction Strategy
    ///
    /// - `AllowAll` / no policy: default ALLOW, add specific deny rules
    /// - `DenyAll`: default ALLOW, deny known dangerous syscalls
    /// - `Allowlist`: default ERRNO(EPERM), allow only listed syscalls
    fn build_seccomp_filter(&self) -> Result<ScmpFilterContext, SandboxError> {
        let deny = ScmpAction::Errno(EPERM);

        // Determine default action based on syscall policy
        let (default_action, is_allowlist) = match &self.syscall_policy {
            Some(SyscallPolicy::Allowlist(_)) => (deny, true),
            _ => (ScmpAction::Allow, false),
        };

        let mut ctx = ScmpFilterContext::new_filter(default_action).map_err(|e| {
            SandboxError::CreationFailed(format!("seccomp filter creation failed: {}", e))
        })?;

        // Apply syscall policy
        match &self.syscall_policy {
            Some(SyscallPolicy::Allowlist(allowed)) => {
                // Default is ERRNO(EPERM); explicitly allow listed syscalls
                for name in allowed {
                    if let Ok(syscall) = ScmpSyscall::from_name(name) {
                        ctx.add_rule(ScmpAction::Allow, syscall).map_err(|e| {
                            SandboxError::RestrictFailed(format!(
                                "failed to allow syscall '{}': {}",
                                name, e
                            ))
                        })?;
                    }
                }
            }
            Some(SyscallPolicy::DenyAll) => {
                // Default is ALLOW; block known dangerous syscalls
                for name in DANGEROUS_SYSCALLS {
                    if let Ok(syscall) = ScmpSyscall::from_name(name) {
                        ctx.add_rule(deny, syscall).map_err(|e| {
                            SandboxError::RestrictFailed(format!(
                                "failed to deny syscall '{}': {}",
                                name, e
                            ))
                        })?;
                    }
                }
            }
            Some(SyscallPolicy::AllowAll) | None => {
                // Default is ALLOW; specific restrictions added below
            }
        }

        // Block network syscalls in Safe mode when no explicit allow rules exist.
        // In allowlist mode, network syscalls are already blocked unless explicitly listed.
        if self.config.mode == ExecutionMode::Safe && !is_allowlist {
            let has_allow_rules = self.network_rules.iter().any(|r| r.allow);
            if !has_allow_rules {
                for name in NETWORK_SYSCALLS {
                    if let Ok(syscall) = ScmpSyscall::from_name(name) {
                        // Ignore errors for duplicate rules (e.g., if DenyAll already blocked these)
                        let _ = ctx.add_rule(deny, syscall);
                    }
                }
            }
        }

        // Block subprocess spawning if not allowed.
        // In allowlist mode, exec syscalls are already blocked unless explicitly listed.
        if !self.config.allow_subprocess && !is_allowlist {
            for name in EXEC_SYSCALLS {
                if let Ok(syscall) = ScmpSyscall::from_name(name) {
                    // Ignore errors for duplicate rules
                    let _ = ctx.add_rule(deny, syscall);
                }
            }
        }

        Ok(ctx)
    }

    /// Try to isolate mount namespace using `unshare(CLONE_NEWNS)`.
    ///
    /// This requires `CAP_SYS_ADMIN` or an unprivileged user namespace.
    /// Failure is non-fatal — the sandbox still provides seccomp + rlimit protection.
    fn try_unshare_mount_ns() -> Result<(), SandboxError> {
        use nix::sched::{unshare, CloneFlags};
        unshare(CloneFlags::CLONE_NEWNS).map_err(|e| {
            SandboxError::RestrictFailed(format!(
                "mount namespace isolation failed (non-fatal, requires CAP_SYS_ADMIN): {}",
                e
            ))
        })
    }
}

impl SandboxBackend for LinuxSandbox {
    /// Create a new Linux sandbox backend.
    ///
    /// This only initializes the configuration; no system calls are made
    /// until [`apply()`](SandboxBackend::apply) is called.
    fn create(config: SandboxConfig) -> Result<Self, SandboxError> {
        Ok(Self {
            config,
            filesystem_rules: Vec::new(),
            network_rules: Vec::new(),
            syscall_policy: None,
            resource_limits: None,
        })
    }

    /// Store filesystem whitelist for higher-level enforcement.
    ///
    /// Note: seccomp cannot perform path-based filtering. These rules are stored
    /// and can be queried by higher-level components (e.g., Landlock, script engine).
    fn restrict_filesystem(&mut self, whitelist: &[PathBuf]) -> &mut Self {
        self.filesystem_rules = whitelist.to_vec();
        self
    }

    /// Configure network access rules.
    ///
    /// In Safe mode with no allow rules, all socket-related syscalls are blocked
    /// via seccomp when [`apply()`](SandboxBackend::apply) is called.
    fn restrict_network(&mut self, rules: &[NetRule]) -> &mut Self {
        self.network_rules = rules.to_vec();
        self
    }

    /// Set syscall filtering policy.
    ///
    /// - `AllowAll`: No syscall restrictions (default if not called)
    /// - `DenyAll`: Block known dangerous syscalls (execve, ptrace, mount, etc.)
    /// - `Allowlist(vec)`: Only allow listed syscalls, deny everything else
    fn restrict_syscalls(&mut self, policy: SyscallPolicy) -> &mut Self {
        self.syscall_policy = Some(policy);
        self
    }

    /// Set resource limits to apply via `setrlimit(2)`.
    ///
    /// Limits are applied before seccomp filter loading during [`apply()`](SandboxBackend::apply).
    fn restrict_resources(&mut self, limits: ResourceLimits) -> &mut Self {
        self.resource_limits = Some(limits);
        self
    }

    /// Apply all configured restrictions and return a sandbox handle.
    ///
    /// This method:
    /// 1. In Power mode: skips all restrictions, returns handle immediately
    /// 2. Optionally attempts mount namespace isolation (non-fatal if fails)
    /// 3. Applies resource limits via `setrlimit(2)` (persistent, irreversible)
    /// 4. Builds and loads seccomp-bpf filter (irreversible once loaded)
    ///
    /// # Errors
    ///
    /// Returns `SandboxError::RestrictFailed` if resource limits or seccomp
    /// filter loading fails. Note that partially applied restrictions (e.g.,
    /// resource limits set before a seccomp failure) cannot be rolled back.
    fn apply(self) -> Result<SandboxHandle, SandboxError> {
        // In Power mode, skip all restrictions
        if self.config.mode == ExecutionMode::Power {
            return Ok(SandboxHandle {
                platform_handle: PlatformHandle::Linux(std::process::id() as i32),
            });
        }

        // Attempt mount namespace isolation (best-effort, non-fatal).
        // This helps with filesystem isolation when CAP_SYS_ADMIN is available.
        if !self.filesystem_rules.is_empty() {
            let _ = Self::try_unshare_mount_ns();
        }

        // Apply resource limits before seccomp (so limit failures are caught early)
        if let Some(ref limits) = self.resource_limits {
            Self::apply_resource_limits(limits)?;
        }

        // Build and load the seccomp-bpf filter
        let ctx = self.build_seccomp_filter()?;
        ctx.load().map_err(|e| {
            SandboxError::RestrictFailed(format!("failed to load seccomp filter: {}", e))
        })?;

        Ok(SandboxHandle {
            platform_handle: PlatformHandle::Linux(std::process::id() as i32),
        })
    }
}

#[cfg(test)]
#[cfg(target_os = "linux")]
mod tests {
    use super::*;
    use crate::types::ResourceLimits;

    #[test]
    fn test_linux_sandbox_create_safe() {
        let config = SandboxConfig::safe_default();
        let sandbox = LinuxSandbox::create(config).unwrap();
        assert_eq!(sandbox.config.mode, ExecutionMode::Safe);
        assert!(!sandbox.config.allow_subprocess);
        assert!(sandbox.filesystem_rules.is_empty());
        assert!(sandbox.network_rules.is_empty());
        assert!(sandbox.syscall_policy.is_none());
        assert!(sandbox.resource_limits.is_none());
    }

    #[test]
    fn test_linux_sandbox_create_power() {
        let config = SandboxConfig::power_mode();
        let sandbox = LinuxSandbox::create(config).unwrap();
        assert_eq!(sandbox.config.mode, ExecutionMode::Power);
        assert!(sandbox.config.allow_subprocess);
    }

    #[test]
    fn test_linux_sandbox_restrict_filesystem() {
        let config = SandboxConfig::safe_default();
        let mut sandbox = LinuxSandbox::create(config).unwrap();

        let whitelist = vec![PathBuf::from("/tmp"), PathBuf::from("/home/user/data")];
        sandbox.restrict_filesystem(&whitelist);

        assert_eq!(sandbox.filesystem_rules.len(), 2);
        assert_eq!(sandbox.filesystem_rules[0], PathBuf::from("/tmp"));
        assert_eq!(
            sandbox.filesystem_rules[1],
            PathBuf::from("/home/user/data")
        );
    }

    #[test]
    fn test_linux_sandbox_restrict_network() {
        let config = SandboxConfig::safe_default();
        let mut sandbox = LinuxSandbox::create(config).unwrap();

        let rules = vec![
            NetRule::allow("api.example.com".to_string()),
            NetRule::deny("malicious.com".to_string()),
        ];
        sandbox.restrict_network(&rules);

        assert_eq!(sandbox.network_rules.len(), 2);
        assert!(sandbox.network_rules[0].allow);
        assert!(!sandbox.network_rules[1].allow);
    }

    #[test]
    fn test_linux_sandbox_restrict_syscalls() {
        let config = SandboxConfig::safe_default();
        let mut sandbox = LinuxSandbox::create(config).unwrap();

        sandbox.restrict_syscalls(SyscallPolicy::DenyAll);
        assert_eq!(sandbox.syscall_policy, Some(SyscallPolicy::DenyAll));
    }

    #[test]
    fn test_linux_sandbox_restrict_syscalls_allowlist() {
        let config = SandboxConfig::safe_default();
        let mut sandbox = LinuxSandbox::create(config).unwrap();

        let allowed = vec!["read".to_string(), "write".to_string(), "close".to_string()];
        sandbox.restrict_syscalls(SyscallPolicy::Allowlist(allowed.clone()));
        assert_eq!(
            sandbox.syscall_policy,
            Some(SyscallPolicy::Allowlist(allowed))
        );
    }

    #[test]
    fn test_linux_sandbox_restrict_resources() {
        let config = SandboxConfig::safe_default();
        let mut sandbox = LinuxSandbox::create(config).unwrap();

        let limits = ResourceLimits::restrictive();
        sandbox.restrict_resources(limits.clone());
        assert_eq!(sandbox.resource_limits, Some(limits));
    }

    #[test]
    fn test_linux_sandbox_method_chaining() {
        let config = SandboxConfig::safe_default();
        let mut sandbox = LinuxSandbox::create(config).unwrap();

        let whitelist = vec![PathBuf::from("/tmp")];
        let rules = vec![NetRule::allow("example.com".to_string())];

        sandbox
            .restrict_filesystem(&whitelist)
            .restrict_network(&rules)
            .restrict_syscalls(SyscallPolicy::DenyAll)
            .restrict_resources(ResourceLimits::restrictive());

        assert_eq!(sandbox.filesystem_rules.len(), 1);
        assert_eq!(sandbox.network_rules.len(), 1);
        assert_eq!(sandbox.syscall_policy, Some(SyscallPolicy::DenyAll));
        assert!(sandbox.resource_limits.is_some());
    }

    #[test]
    fn test_linux_sandbox_overwrite_rules() {
        let config = SandboxConfig::safe_default();
        let mut sandbox = LinuxSandbox::create(config).unwrap();

        sandbox.restrict_filesystem(&[PathBuf::from("/a"), PathBuf::from("/b")]);
        assert_eq!(sandbox.filesystem_rules.len(), 2);

        sandbox.restrict_filesystem(&[PathBuf::from("/c")]);
        assert_eq!(sandbox.filesystem_rules.len(), 1);
        assert_eq!(sandbox.filesystem_rules[0], PathBuf::from("/c"));
    }

    #[test]
    fn test_linux_sandbox_empty_restrictions() {
        let config = SandboxConfig::safe_default();
        let mut sandbox = LinuxSandbox::create(config).unwrap();

        sandbox.restrict_filesystem(&[]);
        sandbox.restrict_network(&[]);

        assert!(sandbox.filesystem_rules.is_empty());
        assert!(sandbox.network_rules.is_empty());
    }

    #[test]
    fn test_linux_sandbox_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<LinuxSandbox>();
    }

    #[test]
    fn test_linux_sandbox_build_filter_default() {
        let config = SandboxConfig::safe_default();
        let sandbox = LinuxSandbox::create(config).unwrap();
        let ctx = sandbox.build_seccomp_filter();
        assert!(ctx.is_ok(), "Failed to build default seccomp filter");
    }

    #[test]
    fn test_linux_sandbox_build_filter_deny_all() {
        let config = SandboxConfig::safe_default();
        let mut sandbox = LinuxSandbox::create(config).unwrap();
        sandbox.restrict_syscalls(SyscallPolicy::DenyAll);
        let ctx = sandbox.build_seccomp_filter();
        assert!(ctx.is_ok(), "Failed to build DenyAll seccomp filter");
    }

    #[test]
    fn test_linux_sandbox_build_filter_allowlist() {
        let config = SandboxConfig::safe_default();
        let mut sandbox = LinuxSandbox::create(config).unwrap();
        sandbox.restrict_syscalls(SyscallPolicy::Allowlist(vec![
            "read".to_string(),
            "write".to_string(),
            "exit".to_string(),
            "exit_group".to_string(),
        ]));
        let ctx = sandbox.build_seccomp_filter();
        assert!(ctx.is_ok(), "Failed to build Allowlist seccomp filter");
    }

    #[test]
    fn test_linux_sandbox_build_filter_allow_all() {
        let config = SandboxConfig::safe_default();
        let mut sandbox = LinuxSandbox::create(config).unwrap();
        sandbox.restrict_syscalls(SyscallPolicy::AllowAll);
        let ctx = sandbox.build_seccomp_filter();
        assert!(ctx.is_ok(), "Failed to build AllowAll seccomp filter");
    }

    #[test]
    fn test_linux_sandbox_build_filter_with_network_rules() {
        let config = SandboxConfig::safe_default();
        let mut sandbox = LinuxSandbox::create(config).unwrap();
        sandbox.restrict_network(&[NetRule::allow("example.com".to_string())]);
        let ctx = sandbox.build_seccomp_filter();
        assert!(
            ctx.is_ok(),
            "Failed to build seccomp filter with network rules"
        );
    }

    #[test]
    fn test_linux_sandbox_build_filter_power_mode_subprocess() {
        let config = SandboxConfig::power_mode();
        let sandbox = LinuxSandbox::create(config).unwrap();
        let ctx = sandbox.build_seccomp_filter();
        assert!(
            ctx.is_ok(),
            "Failed to build seccomp filter for power mode config"
        );
    }

    #[test]
    fn test_linux_sandbox_apply_power_mode() {
        let config = SandboxConfig::power_mode();
        let sandbox = LinuxSandbox::create(config).unwrap();
        let handle = sandbox.apply().unwrap();
        assert!(matches!(handle.platform_handle, PlatformHandle::Linux(_)));

        if let PlatformHandle::Linux(pid) = handle.platform_handle {
            assert_eq!(pid, std::process::id() as i32);
        }
    }
}
