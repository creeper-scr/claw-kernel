//! macOS sandbox implementation using sandbox(7) FFI.
//!
//! Implements [`SandboxBackend`] for macOS using:
//! - **sandbox_init()** C API for process-level sandboxing via unsafe FFI
//! - **Apple Sandbox Profile Language (SBPL)** S-expression format for policy definition
//!
//! # Design Decisions
//!
//! - Uses `sandbox_init()` with raw profile strings (flags = 0), NOT named profiles.
//! - The sandbox profile is generated during `apply()`, not during configuration methods,
//!   so the `MacOSSandbox` struct remains `Send + Sync`.
//! - Safe mode uses `(deny default)` with explicit allow rules for essential operations.
//! - Power mode skips sandbox application entirely (returns handle immediately).
//! - `SyscallPolicy` is mapped to SBPL operation categories since macOS has no
//!   syscall-level filtering like Linux's seccomp.
//! - Resource limits are stored but NOT enforced via `sandbox_init()` — macOS sandbox
//!   profiles do not support resource constraints. Higher-level components may enforce these.
//!
//! # Sandbox Profile Format (SBPL)
//!
//! ```text
//! (version 1)
//! (deny default)
//! (allow file-read* (subpath "/allowed/path"))
//! (allow network-outbound (remote tcp "example.com:443"))
//! ```

use crate::error::SandboxError;
use crate::traits::sandbox::{
    ExecutionMode, PlatformHandle, SandboxBackend, SandboxConfig, SandboxHandle, SyscallPolicy,
};
use crate::types::{NetRule, ResourceLimits};

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::path::PathBuf;

// FFI declarations for Apple's sandbox(7) C API.
//
// These functions are provided by libsystem_sandbox.dylib on macOS.
// sandbox_init() applies an irreversible sandbox to the calling process.
// sandbox_free_error() frees the error buffer allocated by sandbox_init().
extern "C" {
    fn sandbox_init(profile: *const c_char, flags: u64, errorbuf: *mut *mut c_char) -> c_int;

    fn sandbox_free_error(errorbuf: *mut c_char);
}

/// macOS sandbox implementation using sandbox(7) profile system.
///
/// Provides process-level isolation through:
/// - **Sandbox profiles**: SBPL (Apple Sandbox Profile Language) S-expressions
/// - **File access control**: Path-based allow/deny rules via `file-read*`/`file-write*`
/// - **Network restriction**: Domain/port-based outbound control via `network-outbound`
/// - **Subprocess blocking**: `process-exec` deny rules in Safe mode
///
/// # Resource Limit Note
///
/// macOS `sandbox_init()` does not support resource limits (memory, FDs, etc.).
/// Resource limits are stored for higher-level enforcement (e.g., via `launchd` or
/// application-level checks).
///
/// # Filesystem Filtering
///
/// Unlike Linux's seccomp (which cannot do path-based filtering), macOS sandbox
/// profiles natively support path-based filesystem restrictions using `subpath`,
/// `literal`, and `regex` filters.
///
/// # Example
///
/// ```rust,ignore
/// // Internal implementation example - platform types are not public API
/// use claw_pal::SandboxBackend;
/// use claw_pal::{SandboxConfig, SyscallPolicy, ResourceLimits};
///
/// let config = SandboxConfig::safe_default();
/// let mut sandbox = MacOSSandbox::create(config).unwrap();
///
/// sandbox
///     .restrict_syscalls(SyscallPolicy::DenyAll)
///     .restrict_resources(ResourceLimits::restrictive());
///
/// let handle = sandbox.apply().unwrap();
/// // Sandbox is now active — restricted operations return EPERM
/// ```
pub struct MacOSSandbox {
    /// Sandbox configuration (mode, subprocess policy).
    config: SandboxConfig,
    /// Filesystem whitelist paths.
    filesystem_rules: Vec<PathBuf>,
    /// Network access rules.
    network_rules: Vec<NetRule>,
    /// Syscall filtering policy (mapped to SBPL operation categories).
    syscall_policy: Option<SyscallPolicy>,
    /// Resource limits (stored for higher-level enforcement; not enforced by sandbox_init).
    resource_limits: Option<ResourceLimits>,
}

impl MacOSSandbox {
    /// Generate an Apple Sandbox Profile Language (SBPL) string.
    ///
    /// Translates the configured restrictions into S-expression format:
    /// - Power mode: `(allow default)` — no restrictions
    /// - Safe mode + `AllowAll`: `(allow default)` with explicit deny rules
    /// - Safe mode + `DenyAll` or no policy: `(deny default)` with essential allows
    ///
    /// # Profile Structure
    ///
    /// ```text
    /// (version 1)
    /// (deny default)
    /// ;; Essential system operations
    /// (allow sysctl-read)
    /// (allow mach-lookup)
    /// ;; Configured file rules
    /// (allow file-read* (subpath "/path"))
    /// ;; Configured network rules
    /// (allow network-outbound (remote tcp "host:port"))
    /// ```
    pub(crate) fn generate_profile(&self) -> String {
        let mut profile = String::new();
        profile.push_str("(version 1)\n");

        // Power mode: allow everything
        if self.config.mode == ExecutionMode::Power {
            profile.push_str("(allow default)\n");
            return profile;
        }

        // Safe mode with AllowAll syscall policy: permissive base with explicit denies
        if self.syscall_policy == Some(SyscallPolicy::AllowAll) {
            profile.push_str("(allow default)\n");
            self.append_deny_rules(&mut profile);
            return profile;
        }

        // Safe mode (DenyAll, Allowlist, or no policy): deny-by-default
        profile.push_str("(deny default)\n");
        self.append_essential_allows(&mut profile);
        self.append_filesystem_rules(&mut profile);
        self.append_network_rules(&mut profile);
        self.append_subprocess_rules(&mut profile);

        profile
    }

    /// Append essential system operation allows for deny-default profiles.
    ///
    /// These are the minimum operations required for a Rust process to not crash
    /// immediately after sandbox application.
    fn append_essential_allows(&self, profile: &mut String) {
        // System operations required by libSystem / Rust runtime
        profile.push_str("(allow sysctl-read)\n");
        profile.push_str("(allow mach-lookup)\n");
        profile.push_str("(allow signal (target self))\n");
        profile.push_str("(allow process-fork)\n");
        profile.push_str("(allow system-socket)\n");

        // Essential system paths for dynamic libraries and system frameworks
        profile.push_str("(allow file-read*\n");
        profile.push_str("    (subpath \"/usr/lib\")\n");
        profile.push_str("    (subpath \"/System\")\n");
        profile.push_str("    (subpath \"/dev\")\n");
        profile.push_str("    (subpath \"/private/var/db\")\n");
        profile.push_str("    (literal \"/etc/resolv.conf\")\n");
        profile.push_str("    (literal \"/private/etc/resolv.conf\"))\n");
    }

    /// Append filesystem access rules from configuration.
    fn append_filesystem_rules(&self, profile: &mut String) {
        for path in &self.filesystem_rules {
            let escaped = escape_sbpl_string(&path.to_string_lossy());
            profile.push_str(&format!("(allow file-read* (subpath \"{}\"))\n", escaped));
            profile.push_str(&format!("(allow file-write* (subpath \"{}\"))\n", escaped));
        }
    }

    /// Append network access rules from configuration.
    fn append_network_rules(&self, profile: &mut String) {
        let has_allow_rules = self.network_rules.iter().any(|r| r.allow);
        if has_allow_rules {
            for rule in &self.network_rules {
                if rule.allow {
                    let remote = match rule.port {
                        Some(port) => format!("\"{}:{}\"", escape_sbpl_string(&rule.host), port),
                        None => format!("\"{}:*\"", escape_sbpl_string(&rule.host)),
                    };
                    profile.push_str(&format!(
                        "(allow network-outbound (remote tcp {}))\n",
                        remote
                    ));
                }
            }
        }
        // Deny rules are implicit due to (deny default)
    }

    /// Append subprocess execution rules.
    fn append_subprocess_rules(&self, profile: &mut String) {
        if self.config.allow_subprocess {
            profile.push_str("(allow process-exec)\n");
        }
        // When !allow_subprocess, (deny default) already blocks process-exec
    }

    /// Append explicit deny rules for AllowAll base profiles.
    ///
    /// When using `(allow default)`, we still restrict:
    /// - Network access if no explicit allow rules exist
    /// - Subprocess execution if not allowed
    fn append_deny_rules(&self, profile: &mut String) {
        // Block network if no explicit allow rules
        let has_allow_rules = self.network_rules.iter().any(|r| r.allow);
        if !has_allow_rules && self.config.mode == ExecutionMode::Safe {
            profile.push_str("(deny network-outbound)\n");
        }

        // Block subprocess if not allowed
        if !self.config.allow_subprocess {
            profile.push_str("(deny process-exec)\n");
        }
    }

    /// Apply the sandbox profile using `sandbox_init()` FFI.
    ///
    /// # Safety Considerations
    ///
    /// This calls the C `sandbox_init()` function which:
    /// - Is irreversible once applied
    /// - Affects the entire calling process
    /// - Must be called with a valid null-terminated C string profile
    /// - Uses `flags = 0` for raw profile strings (not named profiles)
    fn apply_sandbox_profile(profile: &str) -> Result<(), SandboxError> {
        let c_profile = CString::new(profile).map_err(|e| {
            SandboxError::CreationFailed(format!("profile contains null byte: {}", e))
        })?;

        let mut errorbuf: *mut c_char = std::ptr::null_mut();

        // SAFETY: sandbox_init is a well-defined C API from libsystem_sandbox.dylib.
        // - c_profile is a valid CString (null-terminated, no interior null bytes)
        // - flags = 0 means raw profile string (not a named profile)
        // - errorbuf is a valid pointer that receives the error message on failure
        let result = unsafe { sandbox_init(c_profile.as_ptr(), 0, &mut errorbuf) };

        if result != 0 {
            let error_msg = if !errorbuf.is_null() {
                // SAFETY: errorbuf was set by sandbox_init to a valid C string
                let msg = unsafe { CStr::from_ptr(errorbuf) }
                    .to_string_lossy()
                    .into_owned();
                // SAFETY: errorbuf was allocated by sandbox_init, must be freed
                unsafe { sandbox_free_error(errorbuf) };
                msg
            } else {
                "unknown sandbox_init error".to_string()
            };

            Err(SandboxError::RestrictFailed(format!(
                "sandbox_init failed: {}",
                error_msg
            )))
        } else {
            Ok(())
        }
    }
}

/// Escape a string for use in SBPL double-quoted string literals.
///
/// Escapes backslashes and double quotes to prevent SBPL injection.
fn escape_sbpl_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

impl SandboxBackend for MacOSSandbox {
    /// Create a new macOS sandbox backend.
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

    /// Store filesystem whitelist for sandbox profile generation.
    ///
    /// Paths are translated to `(allow file-read* (subpath ...))` and
    /// `(allow file-write* (subpath ...))` rules during [`apply()`](SandboxBackend::apply).
    fn restrict_filesystem(&mut self, whitelist: &[PathBuf]) -> &mut Self {
        self.filesystem_rules = whitelist.to_vec();
        self
    }

    /// Configure network access rules.
    ///
    /// Allow rules are translated to `(allow network-outbound (remote tcp ...))`.
    /// Deny rules are implicit when using `(deny default)` base profile.
    fn restrict_network(&mut self, rules: &[NetRule]) -> &mut Self {
        self.network_rules = rules.to_vec();
        self
    }

    /// Set syscall filtering policy.
    ///
    /// macOS does not have syscall-level filtering like Linux's seccomp.
    /// The policy is mapped to SBPL operation categories:
    /// - `AllowAll`: Uses `(allow default)` base profile
    /// - `DenyAll`: Uses `(deny default)` base with essential allows
    /// - `Allowlist(vec)`: Uses `(deny default)` base (syscall names are not directly mapped)
    fn restrict_syscalls(&mut self, policy: SyscallPolicy) -> &mut Self {
        self.syscall_policy = Some(policy);
        self
    }

    /// Set resource limits (stored only, not enforced by sandbox_init).
    ///
    /// macOS `sandbox_init()` does not support resource limits. These are stored
    /// for use by higher-level components (e.g., `launchd` plist limits or
    /// application-level enforcement).
    fn restrict_resources(&mut self, limits: ResourceLimits) -> &mut Self {
        self.resource_limits = Some(limits);
        self
    }

    /// Apply all configured restrictions and return a sandbox handle.
    ///
    /// This method:
    /// 1. In Power mode: skips sandbox entirely, returns handle immediately
    /// 2. In Safe mode: generates SBPL profile and calls `sandbox_init()` FFI
    ///
    /// # Important
    ///
    /// `sandbox_init()` is **irreversible** — once applied, the sandbox cannot be
    /// removed or relaxed for the lifetime of the process. The profile MUST be
    /// applied as the FIRST security operation before any untrusted code runs.
    ///
    /// # Errors
    ///
    /// Returns `SandboxError::RestrictFailed` if `sandbox_init()` fails (e.g.,
    /// invalid profile syntax, sandbox already applied, or system error).
    /// Returns `SandboxError::CreationFailed` if the profile string contains
    /// null bytes (cannot be converted to C string).
    fn apply(self) -> Result<SandboxHandle, SandboxError> {
        // In Power mode, skip all restrictions
        if self.config.mode == ExecutionMode::Power {
            return Ok(SandboxHandle {
                platform_handle: PlatformHandle::MacOs("power-mode".to_string()),
            });
        }

        // Generate SBPL profile from configuration
        let profile = self.generate_profile();

        // Apply via sandbox_init() FFI — this is IRREVERSIBLE
        Self::apply_sandbox_profile(&profile)?;

        Ok(SandboxHandle {
            platform_handle: PlatformHandle::MacOs("safe-mode-sandboxed".to_string()),
        })
    }
}

#[cfg(test)]
#[cfg(target_os = "macos")]
mod tests {
    use super::*;
    use crate::types::ResourceLimits;

    // ===== Creation Tests =====

    #[test]
    fn test_macos_sandbox_create_safe() {
        let config = SandboxConfig::safe_default();
        let sandbox = MacOSSandbox::create(config).unwrap();
        assert_eq!(sandbox.config.mode, ExecutionMode::Safe);
        assert!(!sandbox.config.allow_subprocess);
        assert!(sandbox.filesystem_rules.is_empty());
        assert!(sandbox.network_rules.is_empty());
        assert!(sandbox.syscall_policy.is_none());
        assert!(sandbox.resource_limits.is_none());
    }

    #[test]
    fn test_macos_sandbox_create_power() {
        let config = SandboxConfig::power_mode();
        let sandbox = MacOSSandbox::create(config).unwrap();
        assert_eq!(sandbox.config.mode, ExecutionMode::Power);
        assert!(sandbox.config.allow_subprocess);
    }

    // ===== Configuration Tests =====

    #[test]
    fn test_macos_sandbox_restrict_filesystem() {
        let config = SandboxConfig::safe_default();
        let mut sandbox = MacOSSandbox::create(config).unwrap();

        let whitelist = vec![PathBuf::from("/tmp"), PathBuf::from("/Users/test/data")];
        sandbox.restrict_filesystem(&whitelist);

        assert_eq!(sandbox.filesystem_rules.len(), 2);
        assert_eq!(sandbox.filesystem_rules[0], PathBuf::from("/tmp"));
        assert_eq!(
            sandbox.filesystem_rules[1],
            PathBuf::from("/Users/test/data")
        );
    }

    #[test]
    fn test_macos_sandbox_restrict_network() {
        let config = SandboxConfig::safe_default();
        let mut sandbox = MacOSSandbox::create(config).unwrap();

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
    fn test_macos_sandbox_restrict_syscalls() {
        let config = SandboxConfig::safe_default();
        let mut sandbox = MacOSSandbox::create(config).unwrap();

        sandbox.restrict_syscalls(SyscallPolicy::DenyAll);
        assert_eq!(sandbox.syscall_policy, Some(SyscallPolicy::DenyAll));
    }

    #[test]
    fn test_macos_sandbox_restrict_syscalls_allowlist() {
        let config = SandboxConfig::safe_default();
        let mut sandbox = MacOSSandbox::create(config).unwrap();

        let allowed = vec!["read".to_string(), "write".to_string(), "close".to_string()];
        sandbox.restrict_syscalls(SyscallPolicy::Allowlist(allowed.clone()));
        assert_eq!(
            sandbox.syscall_policy,
            Some(SyscallPolicy::Allowlist(allowed))
        );
    }

    #[test]
    fn test_macos_sandbox_restrict_resources() {
        let config = SandboxConfig::safe_default();
        let mut sandbox = MacOSSandbox::create(config).unwrap();

        let limits = ResourceLimits::restrictive();
        sandbox.restrict_resources(limits.clone());
        assert_eq!(sandbox.resource_limits, Some(limits));
    }

    // ===== Builder Pattern Tests =====

    #[test]
    fn test_macos_sandbox_method_chaining() {
        let config = SandboxConfig::safe_default();
        let mut sandbox = MacOSSandbox::create(config).unwrap();

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
    fn test_macos_sandbox_overwrite_rules() {
        let config = SandboxConfig::safe_default();
        let mut sandbox = MacOSSandbox::create(config).unwrap();

        sandbox.restrict_filesystem(&[PathBuf::from("/a"), PathBuf::from("/b")]);
        assert_eq!(sandbox.filesystem_rules.len(), 2);

        sandbox.restrict_filesystem(&[PathBuf::from("/c")]);
        assert_eq!(sandbox.filesystem_rules.len(), 1);
        assert_eq!(sandbox.filesystem_rules[0], PathBuf::from("/c"));
    }

    #[test]
    fn test_macos_sandbox_empty_restrictions() {
        let config = SandboxConfig::safe_default();
        let mut sandbox = MacOSSandbox::create(config).unwrap();

        sandbox.restrict_filesystem(&[]);
        sandbox.restrict_network(&[]);

        assert!(sandbox.filesystem_rules.is_empty());
        assert!(sandbox.network_rules.is_empty());
    }

    // ===== Thread Safety Tests =====

    #[test]
    fn test_macos_sandbox_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<MacOSSandbox>();
    }

    // ===== Profile Generation Tests =====

    #[test]
    fn test_macos_sandbox_generate_profile_power_mode() {
        let config = SandboxConfig::power_mode();
        let sandbox = MacOSSandbox::create(config).unwrap();
        let profile = sandbox.generate_profile();

        assert!(profile.contains("(version 1)"));
        assert!(profile.contains("(allow default)"));
        assert!(!profile.contains("(deny default)"));
    }

    #[test]
    fn test_macos_sandbox_generate_profile_safe_default() {
        let config = SandboxConfig::safe_default();
        let sandbox = MacOSSandbox::create(config).unwrap();
        let profile = sandbox.generate_profile();

        assert!(profile.contains("(version 1)"));
        assert!(profile.contains("(deny default)"));
        // Essential system operations
        assert!(profile.contains("(allow sysctl-read)"));
        assert!(profile.contains("(allow mach-lookup)"));
        assert!(profile.contains("(allow signal (target self))"));
        assert!(profile.contains("(allow process-fork)"));
        // Essential system paths
        assert!(profile.contains("(subpath \"/usr/lib\")"));
        assert!(profile.contains("(subpath \"/System\")"));
        assert!(profile.contains("(subpath \"/dev\")"));
        // No subprocess execution allowed by default
        assert!(!profile.contains("(allow process-exec)"));
    }

    #[test]
    fn test_macos_sandbox_generate_profile_with_filesystem() {
        let config = SandboxConfig::safe_default();
        let mut sandbox = MacOSSandbox::create(config).unwrap();
        sandbox.restrict_filesystem(&[
            PathBuf::from("/tmp/sandbox-test"),
            PathBuf::from("/Users/test/data"),
        ]);
        let profile = sandbox.generate_profile();

        assert!(profile.contains("(allow file-read* (subpath \"/tmp/sandbox-test\"))"));
        assert!(profile.contains("(allow file-write* (subpath \"/tmp/sandbox-test\"))"));
        assert!(profile.contains("(allow file-read* (subpath \"/Users/test/data\"))"));
        assert!(profile.contains("(allow file-write* (subpath \"/Users/test/data\"))"));
    }

    #[test]
    fn test_macos_sandbox_generate_profile_with_network() {
        let config = SandboxConfig::safe_default();
        let mut sandbox = MacOSSandbox::create(config).unwrap();
        sandbox.restrict_network(&[
            NetRule::allow_port("api.openai.com".to_string(), 443),
            NetRule::allow("example.com".to_string()),
            NetRule::deny("malicious.com".to_string()),
        ]);
        let profile = sandbox.generate_profile();

        assert!(profile.contains("(allow network-outbound (remote tcp \"api.openai.com:443\"))"));
        assert!(profile.contains("(allow network-outbound (remote tcp \"example.com:*\"))"));
        // Deny rules are implicit via (deny default)
        assert!(!profile.contains("malicious.com"));
    }

    #[test]
    fn test_macos_sandbox_generate_profile_with_subprocess() {
        let mut config = SandboxConfig::safe_default();
        config.allow_subprocess = true;
        let sandbox = MacOSSandbox::create(config).unwrap();
        let profile = sandbox.generate_profile();

        assert!(profile.contains("(allow process-exec)"));
    }

    #[test]
    fn test_macos_sandbox_generate_profile_deny_all_policy() {
        let config = SandboxConfig::safe_default();
        let mut sandbox = MacOSSandbox::create(config).unwrap();
        sandbox.restrict_syscalls(SyscallPolicy::DenyAll);
        let profile = sandbox.generate_profile();

        // DenyAll uses deny-default base
        assert!(profile.contains("(deny default)"));
        assert!(profile.contains("(allow sysctl-read)"));
    }

    #[test]
    fn test_macos_sandbox_generate_profile_allow_all_policy() {
        let config = SandboxConfig::safe_default();
        let mut sandbox = MacOSSandbox::create(config).unwrap();
        sandbox.restrict_syscalls(SyscallPolicy::AllowAll);
        let profile = sandbox.generate_profile();

        // AllowAll uses allow-default base with explicit denies
        assert!(profile.contains("(allow default)"));
        assert!(!profile.contains("(deny default)"));
        // Should still deny network (no explicit allow rules)
        assert!(profile.contains("(deny network-outbound)"));
        // Should deny subprocess (safe_default has allow_subprocess = false)
        assert!(profile.contains("(deny process-exec)"));
    }

    #[test]
    fn test_macos_sandbox_generate_profile_allow_all_with_net_rules() {
        let config = SandboxConfig::safe_default();
        let mut sandbox = MacOSSandbox::create(config).unwrap();
        sandbox.restrict_syscalls(SyscallPolicy::AllowAll);
        sandbox.restrict_network(&[NetRule::allow("example.com".to_string())]);
        let profile = sandbox.generate_profile();

        // With explicit allow rules, should NOT deny network
        assert!(!profile.contains("(deny network-outbound)"));
    }

    // ===== SBPL Escaping Tests =====

    #[test]
    fn test_macos_sandbox_escape_sbpl_string() {
        assert_eq!(escape_sbpl_string("simple"), "simple");
        assert_eq!(escape_sbpl_string("has\"quote"), "has\\\"quote");
        assert_eq!(escape_sbpl_string("has\\slash"), "has\\\\slash");
        assert_eq!(escape_sbpl_string("both\"and\\here"), "both\\\"and\\\\here");
    }

    #[test]
    fn test_macos_sandbox_generate_profile_escapes_paths() {
        let config = SandboxConfig::safe_default();
        let mut sandbox = MacOSSandbox::create(config).unwrap();
        sandbox.restrict_filesystem(&[PathBuf::from("/path/with spaces/data")]);
        let profile = sandbox.generate_profile();

        // Spaces are allowed in SBPL strings, no escaping needed
        assert!(profile.contains("(subpath \"/path/with spaces/data\")"));
    }

    // ===== Apply Tests =====

    #[test]
    fn test_macos_sandbox_apply_power_mode() {
        let config = SandboxConfig::power_mode();
        let sandbox = MacOSSandbox::create(config).unwrap();
        let handle = sandbox.apply().unwrap();
        assert!(matches!(handle.platform_handle, PlatformHandle::MacOs(_)));

        if let PlatformHandle::MacOs(ref id) = handle.platform_handle {
            assert_eq!(id, "power-mode");
        }
    }

    // NOTE: We intentionally do NOT test apply() in Safe mode in unit tests.
    // sandbox_init() is IRREVERSIBLE — applying it would sandbox the test runner
    // process, causing all subsequent tests and file operations to fail.
    // Integration tests for real sandbox application should use process isolation.
}
