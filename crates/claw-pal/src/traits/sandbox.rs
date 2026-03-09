//! SandboxBackend trait and related types.
//!
//! Defines the core trait for sandbox implementations and configuration types.

use crate::error::SandboxError;
use crate::types::{NetRule, ResourceLimits};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Execution mode for sandboxed operations.
///
/// Determines the level of system access allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ExecutionMode {
    /// Safe mode: restricted access (default)
    #[default]
    Safe,
    /// Power mode: full system access (opt-in)
    Power,
}

/// Syscall policy for sandbox restrictions.
///
/// Defines which syscalls are allowed or denied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyscallPolicy {
    /// Allow all syscalls
    AllowAll,
    /// Deny all syscalls
    DenyAll,
    /// Allow specific syscalls by name
    Allowlist(Vec<String>),
}

/// Platform-specific sandbox handle.
///
/// Represents a handle to an applied sandbox on the current platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformHandle {
    /// Linux seccomp-bpf filter ID
    Linux(i32),
    /// macOS sandbox profile identifier
    MacOs(String),
    /// Windows AppContainer SID
    Windows(u32),
    /// Unsupported platform
    Unsupported,
}

/// Sandbox configuration.
///
/// Specifies the restrictions and policies for a sandboxed environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxConfig {
    /// Execution mode (Safe or Power)
    pub mode: ExecutionMode,
    /// Filesystem allowlist (paths that can be accessed)
    pub filesystem_allowlist: Vec<PathBuf>,
    /// Network access rules
    pub network_rules: Vec<NetRule>,
    /// Whether to allow subprocess spawning
    pub allow_subprocess: bool,
}

impl SandboxConfig {
    /// Create a safe default configuration.
    ///
    /// Suitable for untrusted code execution with minimal privileges.
    pub fn safe_default() -> Self {
        Self {
            mode: ExecutionMode::Safe,
            filesystem_allowlist: vec![],
            network_rules: vec![],
            allow_subprocess: false,
        }
    }

    /// Create a power mode configuration.
    ///
    /// Grants full system access. Should only be used with explicit user consent.
    pub fn power_mode() -> Self {
        Self {
            mode: ExecutionMode::Power,
            filesystem_allowlist: vec![],
            network_rules: vec![],
            allow_subprocess: true,
        }
    }
}

/// Sandbox handle representing an applied sandbox.
///
/// This handle represents a sandbox that has been successfully applied to the system.
/// It cannot be cloned because it represents a unique system resource.
#[derive(Debug, PartialEq, Eq)]
pub struct SandboxHandle {
    /// Platform-specific handle
    pub platform_handle: PlatformHandle,
}

/// Core trait for sandbox backend implementations.
///
/// Defines the interface for creating and configuring sandboxes on different platforms.
/// Implementations should be platform-specific (Linux, macOS, Windows).
pub trait SandboxBackend: Send + Sync {
    /// Create a new sandbox backend with the given configuration.
    fn create(config: SandboxConfig) -> Result<Self, SandboxError>
    where
        Self: Sized;

    /// Restrict filesystem access to the given whitelist.
    ///
    /// Returns a mutable reference to self for method chaining.
    fn restrict_filesystem(&mut self, whitelist: &[PathBuf]) -> &mut Self;

    /// Apply network access rules.
    ///
    /// Returns a mutable reference to self for method chaining.
    fn restrict_network(&mut self, rules: &[NetRule]) -> &mut Self;

    /// Apply syscall restrictions.
    ///
    /// Returns a mutable reference to self for method chaining.
    fn restrict_syscalls(&mut self, policy: SyscallPolicy) -> &mut Self;

    /// Apply resource limits.
    ///
    /// Returns a mutable reference to self for method chaining.
    fn restrict_resources(&mut self, limits: ResourceLimits) -> &mut Self;

    /// Apply all configured restrictions and return a sandbox handle.
    ///
    /// This method consumes the backend and returns a handle to the applied sandbox.
    /// Once applied, the sandbox cannot be modified.
    fn apply(self) -> Result<SandboxHandle, SandboxError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mock sandbox backend for testing.
    struct MockSandboxBackend {
        config: SandboxConfig,
        filesystem_restricted: bool,
        network_restricted: bool,
        syscalls_restricted: bool,
        resources_restricted: bool,
    }

    impl MockSandboxBackend {
        fn new(config: SandboxConfig) -> Self {
            Self {
                config,
                filesystem_restricted: false,
                network_restricted: false,
                syscalls_restricted: false,
                resources_restricted: false,
            }
        }
    }

    impl SandboxBackend for MockSandboxBackend {
        fn create(config: SandboxConfig) -> Result<Self, SandboxError> {
            Ok(Self::new(config))
        }

        fn restrict_filesystem(&mut self, _whitelist: &[PathBuf]) -> &mut Self {
            self.filesystem_restricted = true;
            self
        }

        fn restrict_network(&mut self, _rules: &[NetRule]) -> &mut Self {
            self.network_restricted = true;
            self
        }

        fn restrict_syscalls(&mut self, _policy: SyscallPolicy) -> &mut Self {
            self.syscalls_restricted = true;
            self
        }

        fn restrict_resources(&mut self, _limits: ResourceLimits) -> &mut Self {
            self.resources_restricted = true;
            self
        }

        fn apply(self) -> Result<SandboxHandle, SandboxError> {
            Ok(SandboxHandle {
                platform_handle: PlatformHandle::Unsupported,
            })
        }
    }

    #[test]
    fn test_execution_mode_safe() {
        assert_eq!(ExecutionMode::Safe, ExecutionMode::Safe);
        assert_ne!(ExecutionMode::Safe, ExecutionMode::Power);
    }

    #[test]
    fn test_execution_mode_power() {
        assert_eq!(ExecutionMode::Power, ExecutionMode::Power);
        assert_ne!(ExecutionMode::Power, ExecutionMode::Safe);
    }

    #[test]
    fn test_syscall_policy_allow_all() {
        let policy = SyscallPolicy::AllowAll;
        assert_eq!(policy, SyscallPolicy::AllowAll);
    }

    #[test]
    fn test_syscall_policy_deny_all() {
        let policy = SyscallPolicy::DenyAll;
        assert_eq!(policy, SyscallPolicy::DenyAll);
    }

    #[test]
    fn test_syscall_policy_allowlist() {
        let policy = SyscallPolicy::Allowlist(vec!["read".to_string(), "write".to_string()]);
        assert_eq!(
            policy,
            SyscallPolicy::Allowlist(vec!["read".to_string(), "write".to_string()])
        );
    }

    #[test]
    fn test_platform_handle_linux() {
        let handle = PlatformHandle::Linux(42);
        assert_eq!(handle, PlatformHandle::Linux(42));
    }

    #[test]
    fn test_platform_handle_macos() {
        let handle = PlatformHandle::MacOs("sandbox.profile".to_string());
        assert_eq!(handle, PlatformHandle::MacOs("sandbox.profile".to_string()));
    }

    #[test]
    fn test_platform_handle_windows() {
        let handle = PlatformHandle::Windows(12345);
        assert_eq!(handle, PlatformHandle::Windows(12345));
    }

    #[test]
    fn test_platform_handle_unsupported() {
        let handle = PlatformHandle::Unsupported;
        assert_eq!(handle, PlatformHandle::Unsupported);
    }

    #[test]
    fn test_sandbox_config_safe_default() {
        let config = SandboxConfig::safe_default();
        assert_eq!(config.mode, ExecutionMode::Safe);
        assert!(config.filesystem_allowlist.is_empty());
        assert!(config.network_rules.is_empty());
        assert!(!config.allow_subprocess);
    }

    #[test]
    fn test_sandbox_config_power_mode() {
        let config = SandboxConfig::power_mode();
        assert_eq!(config.mode, ExecutionMode::Power);
        assert!(config.filesystem_allowlist.is_empty());
        assert!(config.network_rules.is_empty());
        assert!(config.allow_subprocess);
    }

    #[test]
    fn test_sandbox_config_clone() {
        let config = SandboxConfig::safe_default();
        let cloned = config.clone();
        assert_eq!(config, cloned);
    }

    #[test]
    fn test_sandbox_handle_creation() {
        let handle = SandboxHandle {
            platform_handle: PlatformHandle::Unsupported,
        };
        assert_eq!(handle.platform_handle, PlatformHandle::Unsupported);
    }

    #[test]
    fn test_sandbox_handle_equality() {
        let handle1 = SandboxHandle {
            platform_handle: PlatformHandle::Linux(42),
        };
        let handle2 = SandboxHandle {
            platform_handle: PlatformHandle::Linux(42),
        };
        assert_eq!(handle1, handle2);
    }

    #[test]
    fn test_mock_sandbox_backend_create() {
        let config = SandboxConfig::safe_default();
        let backend = MockSandboxBackend::create(config.clone()).unwrap();
        assert_eq!(backend.config, config);
    }

    #[test]
    fn test_mock_sandbox_backend_restrict_filesystem() {
        let config = SandboxConfig::safe_default();
        let mut backend = MockSandboxBackend::create(config).unwrap();
        let whitelist = vec![PathBuf::from("/tmp")];
        backend.restrict_filesystem(&whitelist);
        assert!(backend.filesystem_restricted);
    }

    #[test]
    fn test_mock_sandbox_backend_restrict_network() {
        let config = SandboxConfig::safe_default();
        let mut backend = MockSandboxBackend::create(config).unwrap();
        let rules = vec![NetRule::allow("example.com".to_string())];
        backend.restrict_network(&rules);
        assert!(backend.network_restricted);
    }

    #[test]
    fn test_mock_sandbox_backend_restrict_syscalls() {
        let config = SandboxConfig::safe_default();
        let mut backend = MockSandboxBackend::create(config).unwrap();
        backend.restrict_syscalls(SyscallPolicy::DenyAll);
        assert!(backend.syscalls_restricted);
    }

    #[test]
    fn test_mock_sandbox_backend_restrict_resources() {
        let config = SandboxConfig::safe_default();
        let mut backend = MockSandboxBackend::create(config).unwrap();
        let limits = ResourceLimits::restrictive();
        backend.restrict_resources(limits);
        assert!(backend.resources_restricted);
    }

    #[test]
    fn test_mock_sandbox_backend_apply() {
        let config = SandboxConfig::safe_default();
        let backend = MockSandboxBackend::create(config).unwrap();
        let handle = backend.apply().unwrap();
        assert_eq!(handle.platform_handle, PlatformHandle::Unsupported);
    }

    #[test]
    fn test_mock_sandbox_backend_method_chaining() {
        let config = SandboxConfig::safe_default();
        let mut backend = MockSandboxBackend::create(config).unwrap();
        let whitelist = vec![PathBuf::from("/tmp")];
        let rules = vec![NetRule::allow("example.com".to_string())];

        backend
            .restrict_filesystem(&whitelist)
            .restrict_network(&rules)
            .restrict_syscalls(SyscallPolicy::DenyAll)
            .restrict_resources(ResourceLimits::restrictive());

        assert!(backend.filesystem_restricted);
        assert!(backend.network_restricted);
        assert!(backend.syscalls_restricted);
        assert!(backend.resources_restricted);
    }
}
