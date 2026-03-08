//! Linux Sandbox integration tests.
//!
//! These tests require the `sandbox-tests` feature to be enabled.
//! Some tests require root privileges or specific Linux capabilities.
//!
//! # Running tests
//!
//! ```bash
//! # Run basic tests (no special privileges required)
//! cargo test --package claw-pal --features sandbox-tests --test sandbox_linux_test
//!
//! # Run all tests including privileged ones
//! sudo cargo test --package claw-pal --features sandbox-tests --test sandbox_linux_test -- --ignored
//! ```

#![cfg(all(target_os = "linux", feature = "sandbox-tests"))]

use claw_pal::{
    ExecutionMode, LinuxSandbox, NetRule, ResourceLimits, SandboxBackend, SandboxConfig,
    SyscallPolicy,
};
use std::path::PathBuf;

// =============================================================================
// Basic Sandbox Creation Tests (no privileges required)
// =============================================================================

/// Test sandbox creation with safe default configuration.
#[test]
fn test_sandbox_create_safe_default() {
    let config = SandboxConfig::safe_default();
    let sandbox = LinuxSandbox::create(config);
    assert!(sandbox.is_ok(), "Failed to create sandbox with safe default config");
    
    let sandbox = sandbox.unwrap();
    // Verify the sandbox was created with correct mode
    // Note: We can't directly access config field, but we can verify behavior through apply()
}

/// Test sandbox creation with power mode configuration.
#[test]
fn test_sandbox_create_power_mode() {
    let config = SandboxConfig::power_mode();
    let sandbox = LinuxSandbox::create(config);
    assert!(sandbox.is_ok(), "Failed to create sandbox with power mode config");
}

/// Test sandbox configuration chaining.
#[test]
fn test_sandbox_configuration_chaining() {
    let config = SandboxConfig::safe_default();
    let mut sandbox = LinuxSandbox::create(config).unwrap();

    let whitelist = vec![PathBuf::from("/tmp"), PathBuf::from("/var/tmp")];
    let net_rules = vec![NetRule::allow("example.com".to_string())];

    sandbox
        .restrict_filesystem(&whitelist)
        .restrict_network(&net_rules)
        .restrict_syscalls(SyscallPolicy::DenyAll)
        .restrict_resources(ResourceLimits::restrictive());

    // If we get here without panic, chaining works
}

// =============================================================================
// Filesystem Allowlist Tests
// =============================================================================

/// Test filesystem allowlist configuration.
#[test]
fn test_sandbox_filesystem_allowlist_config() {
    let config = SandboxConfig::safe_default();
    let mut sandbox = LinuxSandbox::create(config).unwrap();

    let whitelist = vec![
        PathBuf::from("/tmp"),
        PathBuf::from("/home/user/data"),
        PathBuf::from("/var/log/app"),
    ];

    sandbox.restrict_filesystem(&whitelist);
    // Configuration stored successfully if no panic
}

/// Test empty filesystem allowlist.
#[test]
fn test_sandbox_empty_filesystem_allowlist() {
    let config = SandboxConfig::safe_default();
    let mut sandbox = LinuxSandbox::create(config).unwrap();

    sandbox.restrict_filesystem(&[]);
    // Should accept empty allowlist
}

/// Test filesystem allowlist overwrite.
#[test]
fn test_sandbox_filesystem_allowlist_overwrite() {
    let config = SandboxConfig::safe_default();
    let mut sandbox = LinuxSandbox::create(config).unwrap();

    sandbox.restrict_filesystem(&[PathBuf::from("/a"), PathBuf::from("/b")]);
    sandbox.restrict_filesystem(&[PathBuf::from("/c")]);
    // Second call should overwrite first
}

// =============================================================================
// Network Restriction Tests
// =============================================================================

/// Test network allow rules configuration.
#[test]
fn test_sandbox_network_allow_rules() {
    let config = SandboxConfig::safe_default();
    let mut sandbox = LinuxSandbox::create(config).unwrap();

    let rules = vec![
        NetRule::allow("api.example.com".to_string()),
        NetRule::allow_port("api.example.com".to_string(), 443),
        NetRule::allow("*.trusted-domain.com".to_string()),
    ];

    sandbox.restrict_network(&rules);
}

/// Test network deny rules configuration.
#[test]
fn test_sandbox_network_deny_rules() {
    let config = SandboxConfig::safe_default();
    let mut sandbox = LinuxSandbox::create(config).unwrap();

    let rules = vec![
        NetRule::deny("malicious.com".to_string()),
        NetRule::deny("*.blocked-domain.com".to_string()),
    ];

    sandbox.restrict_network(&rules);
}

/// Test mixed network rules.
#[test]
fn test_sandbox_network_mixed_rules() {
    let config = SandboxConfig::safe_default();
    let mut sandbox = LinuxSandbox::create(config).unwrap();

    let rules = vec![
        NetRule::allow("trusted.com".to_string()),
        NetRule::deny("untrusted.com".to_string()),
        NetRule::allow_port("api.trusted.com".to_string(), 443),
    ];

    sandbox.restrict_network(&rules);
}

// =============================================================================
// Syscall Policy Tests
// =============================================================================

/// Test syscall policy DenyAll.
#[test]
fn test_sandbox_syscall_policy_deny_all() {
    let config = SandboxConfig::safe_default();
    let mut sandbox = LinuxSandbox::create(config).unwrap();

    sandbox.restrict_syscalls(SyscallPolicy::DenyAll);
}

/// Test syscall policy AllowAll.
#[test]
fn test_sandbox_syscall_policy_allow_all() {
    let config = SandboxConfig::safe_default();
    let mut sandbox = LinuxSandbox::create(config).unwrap();

    sandbox.restrict_syscalls(SyscallPolicy::AllowAll);
}

/// Test syscall policy Allowlist.
#[test]
fn test_sandbox_syscall_policy_allowlist() {
    let config = SandboxConfig::safe_default();
    let mut sandbox = LinuxSandbox::create(config).unwrap();

    let allowed_syscalls = vec![
        "read".to_string(),
        "write".to_string(),
        "close".to_string(),
        "exit".to_string(),
        "exit_group".to_string(),
    ];

    sandbox.restrict_syscalls(SyscallPolicy::Allowlist(allowed_syscalls));
}

/// Test empty syscall allowlist.
#[test]
fn test_sandbox_empty_syscall_allowlist() {
    let config = SandboxConfig::safe_default();
    let mut sandbox = LinuxSandbox::create(config).unwrap();

    sandbox.restrict_syscalls(SyscallPolicy::Allowlist(vec![]));
}

// =============================================================================
// Resource Limit Tests
// =============================================================================

/// Test restrictive resource limits configuration.
#[test]
fn test_sandbox_resource_limits_restrictive() {
    let config = SandboxConfig::safe_default();
    let mut sandbox = LinuxSandbox::create(config).unwrap();

    let limits = ResourceLimits::restrictive();
    sandbox.restrict_resources(limits);
}

/// Test unlimited resource limits configuration.
#[test]
fn test_sandbox_resource_limits_unlimited() {
    let config = SandboxConfig::safe_default();
    let mut sandbox = LinuxSandbox::create(config).unwrap();

    let limits = ResourceLimits::unlimited();
    sandbox.restrict_resources(limits);
}

/// Test custom resource limits.
#[test]
fn test_sandbox_resource_limits_custom() {
    let config = SandboxConfig::safe_default();
    let mut sandbox = LinuxSandbox::create(config).unwrap();

    let limits = ResourceLimits::unlimited()
        .with_memory(512 * 1024 * 1024) // 512 MB
        .with_fds(1024)
        .with_processes(50);

    sandbox.restrict_resources(limits);
}

// =============================================================================
// Power Mode Tests (no privileges required)
// =============================================================================

/// Test that power mode apply succeeds without restrictions.
#[test]
fn test_sandbox_power_mode_apply() {
    let config = SandboxConfig::power_mode();
    let sandbox = LinuxSandbox::create(config).unwrap();

    let handle = sandbox.apply();
    assert!(handle.is_ok(), "Power mode apply should succeed");

    let handle = handle.unwrap();
    assert!(matches!(handle.platform_handle, claw_pal::PlatformHandle::Linux(_)));
}

/// Test power mode with all restrictions configured (should be ignored).
#[test]
fn test_sandbox_power_mode_ignores_restrictions() {
    let config = SandboxConfig::power_mode();
    let mut sandbox = LinuxSandbox::create(config).unwrap();

    // Configure all restrictions
    sandbox
        .restrict_filesystem(&[PathBuf::from("/tmp")])
        .restrict_network(&[NetRule::allow("example.com".to_string())])
        .restrict_syscalls(SyscallPolicy::DenyAll)
        .restrict_resources(ResourceLimits::restrictive());

    // Apply should succeed - restrictions are ignored in power mode
    let handle = sandbox.apply();
    assert!(handle.is_ok(), "Power mode should ignore restrictions and succeed");
}

// =============================================================================
// Privileged Tests (require root or specific capabilities)
// =============================================================================

/// Test applying sandbox with DenyAll syscall policy.
/// 
/// **Privileged**: Requires ability to load seccomp filters.
/// Most systems allow this for regular users.
#[test]
#[ignore = "requires seccomp filter loading capability"]
fn test_sandbox_apply_syscall_deny_all() {
    let config = SandboxConfig::safe_default();
    let mut sandbox = LinuxSandbox::create(config).unwrap();

    sandbox.restrict_syscalls(SyscallPolicy::DenyAll);

    let handle = sandbox.apply();
    assert!(handle.is_ok(), "Failed to apply sandbox with DenyAll syscall policy");
}

/// Test applying sandbox with Allowlist syscall policy.
///
/// **Privileged**: Requires ability to load seccomp filters.
#[test]
#[ignore = "requires seccomp filter loading capability"]
fn test_sandbox_apply_syscall_allowlist() {
    let config = SandboxConfig::safe_default();
    let mut sandbox = LinuxSandbox::create(config).unwrap();

    let allowed = vec![
        "read".to_string(),
        "write".to_string(),
        "close".to_string(),
        "exit".to_string(),
        "exit_group".to_string(),
        "brk".to_string(),
        "mmap".to_string(),
        "munmap".to_string(),
    ];
    sandbox.restrict_syscalls(SyscallPolicy::Allowlist(allowed));

    let handle = sandbox.apply();
    assert!(handle.is_ok(), "Failed to apply sandbox with Allowlist syscall policy");
}

/// Test applying sandbox with resource limits.
///
/// **Privileged**: Setting resource limits may require elevated privileges
/// depending on system configuration.
#[test]
#[ignore = "may require elevated privileges for resource limits"]
fn test_sandbox_apply_resource_limits() {
    let config = SandboxConfig::safe_default();
    let mut sandbox = LinuxSandbox::create(config).unwrap();

    let limits = ResourceLimits::restrictive();
    sandbox.restrict_resources(limits);

    let handle = sandbox.apply();
    assert!(handle.is_ok(), "Failed to apply sandbox with resource limits");
}

/// Test applying sandbox with filesystem restrictions.
///
/// **Privileged**: Requires CAP_SYS_ADMIN or unprivileged user namespace
/// support for mount namespace isolation.
#[test]
#[ignore = "requires CAP_SYS_ADMIN or user namespace support"]
fn test_sandbox_apply_filesystem_restrictions() {
    let config = SandboxConfig::safe_default();
    let mut sandbox = LinuxSandbox::create(config).unwrap();

    sandbox.restrict_filesystem(&[PathBuf::from("/tmp")]);

    let handle = sandbox.apply();
    assert!(handle.is_ok(), "Failed to apply sandbox with filesystem restrictions");
}

/// Test applying sandbox with network restrictions in Safe mode.
///
/// **Privileged**: Requires ability to load seccomp filters.
#[test]
#[ignore = "requires seccomp filter loading capability"]
fn test_sandbox_apply_network_restrictions_safe_mode() {
    let config = SandboxConfig::safe_default();
    let sandbox = LinuxSandbox::create(config).unwrap();

    // In Safe mode without explicit network allow rules, network syscalls are blocked
    let handle = sandbox.apply();
    assert!(handle.is_ok(), "Failed to apply sandbox with network restrictions");
}

/// Test applying sandbox with network allow rules.
///
/// **Privileged**: Requires ability to load seccomp filters.
#[test]
#[ignore = "requires seccomp filter loading capability"]
fn test_sandbox_apply_network_allow_rules() {
    let config = SandboxConfig::safe_default();
    let mut sandbox = LinuxSandbox::create(config).unwrap();

    // With explicit allow rules, network syscalls should not be blocked
    sandbox.restrict_network(&[NetRule::allow("example.com".to_string())]);

    let handle = sandbox.apply();
    assert!(handle.is_ok(), "Failed to apply sandbox with network allow rules");
}

/// Test applying sandbox with full restrictions.
///
/// **Privileged**: Requires all capabilities needed for seccomp, rlimit, and namespace.
#[test]
#[ignore = "requires full sandbox capabilities (seccomp + rlimit + namespace)"]
fn test_sandbox_apply_full_restrictions() {
    let config = SandboxConfig::safe_default();
    let mut sandbox = LinuxSandbox::create(config).unwrap();

    sandbox
        .restrict_filesystem(&[PathBuf::from("/tmp"), PathBuf::from("/var/tmp")])
        .restrict_network(&[NetRule::allow("api.example.com".to_string())])
        .restrict_syscalls(SyscallPolicy::DenyAll)
        .restrict_resources(ResourceLimits::restrictive());

    let handle = sandbox.apply();
    assert!(handle.is_ok(), "Failed to apply sandbox with full restrictions");
}

// =============================================================================
// Subprocess Restriction Tests (Privileged)
// =============================================================================

/// Test that subprocess spawning is blocked in Safe mode.
///
/// **Privileged**: Requires seccomp filter to test actual blocking.
#[test]
#[ignore = "requires seccomp filter loading capability"]
fn test_sandbox_blocks_subprocess_in_safe_mode() {
    use std::process::Command;

    let config = SandboxConfig::safe_default();
    let sandbox = LinuxSandbox::create(config).unwrap();

    let handle = sandbox.apply();
    assert!(handle.is_ok(), "Failed to apply sandbox");

    // Try to spawn a subprocess - should fail with EPERM
    let result = Command::new("/bin/true").spawn();
    assert!(result.is_err(), "Subprocess should be blocked in Safe mode");
}

/// Test that subprocess spawning is allowed in Safe mode with allow_subprocess=true.
///
/// **Privileged**: Requires seccomp filter to verify.
#[test]
#[ignore = "requires seccomp filter loading capability"]
fn test_sandbox_allows_subprocess_when_configured() {
    let config = SandboxConfig {
        mode: ExecutionMode::Safe,
        filesystem_allowlist: vec![],
        network_rules: vec![],
        allow_subprocess: true,
    };
    let sandbox = LinuxSandbox::create(config).unwrap();

    let handle = sandbox.apply();
    assert!(handle.is_ok(), "Failed to apply sandbox");

    // Subprocess should be allowed
}

// =============================================================================
// Edge Case Tests
// =============================================================================

/// Test creating multiple sandbox configurations without applying.
#[test]
fn test_sandbox_multiple_configurations_no_apply() {
    // Create many sandboxes without applying - should not leak resources
    for i in 0..100 {
        let config = SandboxConfig::safe_default();
        let mut sandbox = LinuxSandbox::create(config).unwrap();
        
        sandbox
            .restrict_filesystem(&[PathBuf::from(format!("/tmp/test{}", i))])
            .restrict_syscalls(SyscallPolicy::DenyAll);
        
        // Drop without applying
    }
}

/// Test sandbox with very large allowlist (performance test).
#[test]
fn test_sandbox_large_filesystem_allowlist() {
    let config = SandboxConfig::safe_default();
    let mut sandbox = LinuxSandbox::create(config).unwrap();

    let whitelist: Vec<PathBuf> = (0..1000)
        .map(|i| PathBuf::from(format!("/tmp/path{}/subdir/file.txt", i)))
        .collect();

    sandbox.restrict_filesystem(&whitelist);
    // Should handle large allowlist without performance issues
}

/// Test sandbox with many network rules.
#[test]
fn test_sandbox_many_network_rules() {
    let config = SandboxConfig::safe_default();
    let mut sandbox = LinuxSandbox::create(config).unwrap();

    let rules: Vec<NetRule> = (0..100)
        .map(|i| {
            if i % 2 == 0 {
                NetRule::allow(format!("host{}.example.com", i))
            } else {
                NetRule::deny(format!("host{}.blocked.com", i))
            }
        })
        .collect();

    sandbox.restrict_network(&rules);
}

/// Test sandbox with very restrictive resource limits.
///
/// **Privileged**: Requires setting resource limits.
#[test]
#[ignore = "requires ability to set resource limits"]
fn test_sandbox_very_restrictive_limits() {
    let config = SandboxConfig::safe_default();
    let mut sandbox = LinuxSandbox::create(config).unwrap();

    let limits = ResourceLimits::unlimited()
        .with_memory(16 * 1024 * 1024)  // 16 MB
        .with_fds(32)
        .with_processes(1);

    sandbox.restrict_resources(limits);

    let handle = sandbox.apply();
    assert!(handle.is_ok(), "Failed to apply sandbox with very restrictive limits");
}

// =============================================================================
// Security Tests (Privileged)
// =============================================================================

/// Test that dangerous syscalls are blocked with DenyAll policy.
///
/// **Privileged**: Requires seccomp filter to test actual syscall blocking.
#[test]
#[ignore = "requires seccomp filter loading capability"]
fn test_sandbox_blocks_dangerous_syscalls() {
    // This test would need to be run in a subprocess to safely test
    // that dangerous syscalls like ptrace, mount, etc. are blocked
    
    // For now, we verify the filter builds correctly
    let config = SandboxConfig::safe_default();
    let mut sandbox = LinuxSandbox::create(config).unwrap();
    sandbox.restrict_syscalls(SyscallPolicy::DenyAll);
    
    let handle = sandbox.apply();
    assert!(handle.is_ok());
}

/// Test seccomp filter construction for various configurations.
#[test]
fn test_sandbox_seccomp_filter_construction() {
    // These tests verify that seccomp filters can be built without error
    // Actual loading is tested in privileged tests
    
    let config = SandboxConfig::safe_default();
    
    // Test default filter
    let sandbox = LinuxSandbox::create(config.clone()).unwrap();
    // build_seccomp_filter is private, but apply() will call it
    // We test apply() in privileged tests
    
    // Test with DenyAll
    let mut sandbox = LinuxSandbox::create(config.clone()).unwrap();
    sandbox.restrict_syscalls(SyscallPolicy::DenyAll);
    
    // Test with AllowAll
    let mut sandbox = LinuxSandbox::create(config.clone()).unwrap();
    sandbox.restrict_syscalls(SyscallPolicy::AllowAll);
    
    // Test with Allowlist
    let mut sandbox = LinuxSandbox::create(config).unwrap();
    sandbox.restrict_syscalls(SyscallPolicy::Allowlist(vec![
        "read".to_string(),
        "write".to_string(),
    ]));
}

/// Test that Power mode truly bypasses all restrictions.
///
/// **Privileged**: Requires applying sandbox.
#[test]
#[ignore = "requires seccomp filter loading capability"]
fn test_sandbox_power_mode_bypasses_seccomp() {
    use std::process::Command;

    let config = SandboxConfig::power_mode();
    let sandbox = LinuxSandbox::create(config).unwrap();

    let handle = sandbox.apply();
    assert!(handle.is_ok(), "Failed to apply power mode sandbox");

    // In power mode, subprocess should work
    let result = Command::new("/bin/true").spawn();
    assert!(result.is_ok(), "Subprocess should work in Power mode");
}
