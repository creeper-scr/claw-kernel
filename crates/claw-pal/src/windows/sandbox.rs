//! Windows sandbox implementation using Job Objects (degraded isolation).
//!
//! Provides **partial isolation** via Windows Job Objects, which is a significant
//! improvement over the previous stub, but weaker than Linux (seccomp) and macOS
//! (sandbox_init) due to Windows API limitations.
//!
//! # Isolation Comparison
//!
//! | Feature | Linux | macOS | Windows Job Object |
//! |---------|-------|-------|--------------------|
//! | Memory limits | ✅ setrlimit | ❌ not supported | ✅ JobMemoryLimit |
//! | Subprocess blocking | ✅ seccomp | ✅ SBPL | ✅ ActiveProcessLimit=1 |
//! | Process count limit | ✅ setrlimit NPROC | ❌ | ✅ ActiveProcessLimit |
//! | Network restrictions | ✅ seccomp socket | ✅ SBPL | ❌ NOT enforced |
//! | Filesystem restrictions | ⚠️ namespace | ✅ SBPL | ❌ NOT enforced |
//!
//! # What IS enforced in Safe mode
//!
//! - **Memory limit** (`max_memory_bytes`) via `JOBOBJECT_EXTENDED_LIMIT_INFORMATION::JobMemoryLimit`
//! - **Subprocess blocking** (`allow_subprocess=false`) via `ActiveProcessLimit=1`
//! - **Process count limit** (`max_processes`) via `ActiveProcessLimit`
//!
//! # What is NOT enforced (requires AppContainer — planned v1.5.0)
//!
//! - Filesystem access control (path-level read/write restrictions)
//! - Network access control (domain/port-based filtering)
//!
//! A `tracing::warn!` is emitted at Safe mode activation so operators are always
//! aware of the reduced isolation guarantees.
//!
//! # Job Object Lifecycle
//!
//! 1. Create anonymous Job Object (`CreateJobObjectW`)
//! 2. Configure limits (`SetInformationJobObject`)
//! 3. Assign current process (`AssignProcessToJobObject`)
//! 4. Close the Job Object handle
//!
//! After step 4, the kernel continues to enforce limits for the lifetime of the
//! assigned process. The Job Object is referenced by the process itself and will be
//! freed automatically when the process exits.
//!
//! # Full AppContainer Roadmap
//!
//! Full isolation (filesystem + network) requires:
//! - `CreateAppContainerProfile()` / `DeleteAppContainerProfile()`
//! - `CreateProcessAsUser()` with AppContainer SID
//! - Windows Filtering Platform (WFP) callout for network rules
//!
//! Tracked in v1.5.0 milestone.

use crate::error::SandboxError;
use crate::traits::sandbox::{
    ExecutionMode, PlatformHandle, SandboxBackend, SandboxConfig, SandboxHandle, SyscallPolicy,
};
use crate::types::{NetRule, ResourceLimits};

use std::path::PathBuf;

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
    JOB_OBJECT_LIMIT_JOB_MEMORY,
};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

/// Windows sandbox using Job Objects for resource isolation.
///
/// Provides partial isolation (resource limits + subprocess blocking) in Safe mode.
/// Filesystem and network restrictions are stored but not enforced until
/// AppContainer support is added in v1.5.0.
///
/// ⚠️ **Degraded isolation**: Safe mode on Windows does NOT restrict filesystem or
/// network access. A `tracing::warn!` is emitted when Safe mode is applied.
///
/// # Example
///
/// ```rust,ignore
/// // Internal implementation example - platform types are not public API
/// use claw_pal::SandboxBackend;
/// use claw_pal::{SandboxConfig, ResourceLimits};
///
/// let config = SandboxConfig::safe_default();
/// let mut sandbox = WindowsSandbox::create(config).unwrap();
///
/// sandbox.restrict_resources(ResourceLimits::restrictive());
///
/// // Applies Job Object: memory limits + subprocess blocking enforced.
/// // NOTE: filesystem/network rules are stored but NOT enforced.
/// let handle = sandbox.apply().unwrap();
/// ```
pub struct WindowsSandbox {
    /// Sandbox configuration (mode, subprocess policy).
    config: SandboxConfig,
    /// Filesystem whitelist paths (stored; not enforced — AppContainer pending).
    filesystem_rules: Vec<PathBuf>,
    /// Network access rules (stored; not enforced — AppContainer pending).
    network_rules: Vec<NetRule>,
    /// Syscall filtering policy (stored; not applicable on Windows).
    syscall_policy: Option<SyscallPolicy>,
    /// Resource limits to apply via Job Object.
    resource_limits: Option<ResourceLimits>,
}

impl WindowsSandbox {
    /// Apply Job Object limits to the current process.
    ///
    /// Creates an anonymous Job Object, sets resource limits, assigns the current
    /// process, then closes the handle. Limits remain kernel-enforced for the
    /// process lifetime even after the handle is closed.
    ///
    /// # Job Object Limits Applied
    ///
    /// - `JOB_OBJECT_LIMIT_ACTIVE_PROCESS` (1) when `allow_subprocess = false`
    /// - `JOB_OBJECT_LIMIT_JOB_MEMORY` when `max_memory_bytes` is set
    /// - `JOB_OBJECT_LIMIT_ACTIVE_PROCESS` from `max_processes` if set
    ///
    /// # Windows Version Note
    ///
    /// Nested Job Objects require Windows 8+ (build 9200+). On Windows 7, if the
    /// process is already in a job, `AssignProcessToJobObject` returns error 5
    /// (ACCESS_DENIED). Modern CI and development environments use Windows 10+.
    fn apply_job_limits(
        config: &SandboxConfig,
        resource_limits: Option<&ResourceLimits>,
    ) -> Result<(), SandboxError> {
        // SAFETY: CreateJobObjectW with null parameters creates a valid anonymous
        // job object with default security. Returns 0 on failure.
        let job_handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job_handle == 0 {
            // SAFETY: GetLastError is always safe to call after a failed Win32 API
            let err = unsafe { GetLastError() };
            return Err(SandboxError::CreationFailed(format!(
                "CreateJobObjectW failed: Windows error {}",
                err
            )));
        }

        // SAFETY: zeroed initialization is valid for JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        // which is a plain C struct with no invariants beyond numeric fields.
        let mut ext_info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        let mut limit_flags: u32 = 0;

        // Block subprocess spawning: limit active processes to 1 (current process only).
        // ActiveProcessLimit=1 prevents CreateProcess from succeeding inside the job.
        if !config.allow_subprocess {
            ext_info.BasicLimitInformation.ActiveProcessLimit = 1;
            limit_flags |= JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
        }

        // Apply resource limits
        if let Some(limits) = resource_limits {
            if let Some(max_memory) = limits.max_memory_bytes {
                ext_info.JobMemoryLimit = max_memory as usize;
                limit_flags |= JOB_OBJECT_LIMIT_JOB_MEMORY;
            }
            // max_processes overrides the allow_subprocess=false limit when set
            if let Some(max_procs) = limits.max_processes {
                ext_info.BasicLimitInformation.ActiveProcessLimit = max_procs;
                limit_flags |= JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
            }
        }

        ext_info.BasicLimitInformation.LimitFlags = limit_flags;

        // SAFETY: ext_info is properly zero-initialized and has the correct size.
        // JobObjectExtendedLimitInformation = 9 is the correct info class.
        let set_result = unsafe {
            SetInformationJobObject(
                job_handle,
                JobObjectExtendedLimitInformation,
                &ext_info as *const _ as *const core::ffi::c_void,
                core::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };

        if set_result == 0 {
            let err = unsafe { GetLastError() };
            // SAFETY: CloseHandle on a valid job handle is always safe
            unsafe { CloseHandle(job_handle) };
            return Err(SandboxError::RestrictFailed(format!(
                "SetInformationJobObject failed: Windows error {}",
                err
            )));
        }

        // SAFETY: GetCurrentProcess returns a pseudo-handle (-1), always valid,
        // does not need to be closed.
        let proc_handle = unsafe { GetCurrentProcess() };

        // SAFETY: AssignProcessToJobObject with valid job and process handles.
        let assign_result = unsafe { AssignProcessToJobObject(job_handle, proc_handle) };

        // Save error code BEFORE CloseHandle (which would overwrite GetLastError)
        let assign_err = if assign_result == 0 {
            unsafe { GetLastError() }
        } else {
            0
        };

        // Always close the job handle. If assignment succeeded, the kernel holds the
        // job alive via the process reference; limits remain enforced. If assignment
        // failed, the orphaned job (no handles, no processes) is freed immediately.
        // SAFETY: job_handle is a valid handle returned by CreateJobObjectW.
        unsafe { CloseHandle(job_handle) };

        if assign_result == 0 {
            return Err(SandboxError::RestrictFailed(format!(
                "AssignProcessToJobObject failed: Windows error {}. \
                 Error 5 (ACCESS_DENIED) on Windows 7 means the process is already \
                 in a non-nested job object. Windows 8+ supports nested job objects.",
                assign_err
            )));
        }

        // Limits are now enforced by the kernel for the process lifetime.
        Ok(())
    }

    /// Generate a human-readable description of configured restrictions.
    ///
    /// Shows what is and is not enforced on Windows.
    pub(crate) fn generate_profile(&self) -> String {
        let mut profile = String::new();
        profile.push_str("Windows Job Object Profile\n");

        if self.config.mode == ExecutionMode::Power {
            profile.push_str("Mode: Power (no restrictions)\n");
            return profile;
        }

        profile.push_str("Mode: Safe (Job Object — degraded isolation)\n");
        profile.push_str(&format!(
            "Subprocess blocked: {} [enforced]\n",
            !self.config.allow_subprocess
        ));
        profile.push_str(&format!(
            "Filesystem rules: {} [NOT enforced — AppContainer pending v1.5.0]\n",
            self.filesystem_rules.len()
        ));
        profile.push_str(&format!(
            "Network rules: {} [NOT enforced — AppContainer pending v1.5.0]\n",
            self.network_rules.len()
        ));
        if let Some(ref limits) = self.resource_limits {
            if let Some(mem) = limits.max_memory_bytes {
                profile.push_str(&format!("Memory limit: {} bytes [enforced]\n", mem));
            }
            if let Some(procs) = limits.max_processes {
                profile.push_str(&format!("Max processes: {} [enforced]\n", procs));
            }
        }
        profile
    }
}

impl SandboxBackend for WindowsSandbox {
    /// Create a new Windows sandbox backend.
    ///
    /// Only initializes configuration; no system calls are made until
    /// [`apply()`](SandboxBackend::apply) is called.
    fn create(config: SandboxConfig) -> Result<Self, SandboxError> {
        Ok(Self {
            config,
            filesystem_rules: Vec::new(),
            network_rules: Vec::new(),
            syscall_policy: None,
            resource_limits: None,
        })
    }

    /// Store filesystem whitelist.
    ///
    /// ⚠️ **Not enforced**: Paths are stored for future AppContainer implementation
    /// (v1.5.0). On Windows, filesystem access is currently unrestricted in Safe mode.
    fn restrict_filesystem(&mut self, whitelist: &[PathBuf]) -> &mut Self {
        self.filesystem_rules = whitelist.to_vec();
        self
    }

    /// Configure network access rules.
    ///
    /// ⚠️ **Not enforced**: Rules are stored for future AppContainer/WFP implementation
    /// (v1.5.0). On Windows, network access is currently unrestricted in Safe mode.
    fn restrict_network(&mut self, rules: &[NetRule]) -> &mut Self {
        self.network_rules = rules.to_vec();
        self
    }

    /// Set syscall filtering policy (stored only).
    ///
    /// Windows does not support syscall-level filtering like Linux seccomp.
    /// This policy is stored for API compatibility but has no effect.
    fn restrict_syscalls(&mut self, policy: SyscallPolicy) -> &mut Self {
        self.syscall_policy = Some(policy);
        self
    }

    /// Set resource limits.
    ///
    /// `max_memory_bytes` and `max_processes` are enforced via Job Object.
    /// `max_cpu_percent` and `max_file_descriptors` are stored but not enforced.
    fn restrict_resources(&mut self, limits: ResourceLimits) -> &mut Self {
        self.resource_limits = Some(limits);
        self
    }

    /// Apply configured restrictions and return a sandbox handle.
    ///
    /// - **Power mode**: skips all restrictions, returns handle immediately.
    /// - **Safe mode**: creates a Job Object, enforces resource limits and
    ///   subprocess blocking. Emits `tracing::warn!` about filesystem/network
    ///   restrictions not being enforced.
    ///
    /// # Errors
    ///
    /// Returns `SandboxError::CreationFailed` if `CreateJobObjectW` fails.
    /// Returns `SandboxError::RestrictFailed` if `SetInformationJobObject` or
    /// `AssignProcessToJobObject` fails (e.g., error 5 on Windows 7).
    fn apply(self) -> Result<SandboxHandle, SandboxError> {
        if self.config.mode == ExecutionMode::Power {
            return Ok(SandboxHandle {
                platform_handle: PlatformHandle::Windows(0),
            });
        }

        // Warn operators about degraded isolation guarantees
        tracing::warn!(
            platform = "windows",
            enforced = "memory_limit,subprocess_blocking,process_count",
            not_enforced = "filesystem_restrictions,network_restrictions",
            "Windows Safe mode uses Job Object isolation (degraded). \
             Resource limits and subprocess blocking are enforced via Job Object. \
             Filesystem and network restrictions are NOT enforced until v1.5.0 (AppContainer). \
             For full isolation, use WSL2 to run the Linux version."
        );

        Self::apply_job_limits(&self.config, self.resource_limits.as_ref())?;

        // Return sentinel value 1 to indicate safe mode restrictions applied.
        Ok(SandboxHandle {
            platform_handle: PlatformHandle::Windows(1),
        })
    }
}

#[cfg(test)]
#[cfg(target_os = "windows")]
mod tests {
    use super::*;
    use crate::types::ResourceLimits;

    // ===== Creation Tests =====

    #[test]
    fn test_windows_sandbox_create_safe() {
        let config = SandboxConfig::safe_default();
        let sandbox = WindowsSandbox::create(config).unwrap();
        assert_eq!(sandbox.config.mode, ExecutionMode::Safe);
        assert!(!sandbox.config.allow_subprocess);
        assert!(sandbox.filesystem_rules.is_empty());
        assert!(sandbox.network_rules.is_empty());
        assert!(sandbox.syscall_policy.is_none());
        assert!(sandbox.resource_limits.is_none());
    }

    #[test]
    fn test_windows_sandbox_create_power() {
        let config = SandboxConfig::power_mode();
        let sandbox = WindowsSandbox::create(config).unwrap();
        assert_eq!(sandbox.config.mode, ExecutionMode::Power);
        assert!(sandbox.config.allow_subprocess);
    }

    // ===== Configuration Tests =====

    #[test]
    fn test_windows_sandbox_restrict_filesystem() {
        let config = SandboxConfig::safe_default();
        let mut sandbox = WindowsSandbox::create(config).unwrap();

        let whitelist = vec![
            PathBuf::from("C:\\temp"),
            PathBuf::from("C:\\Users\\test\\data"),
        ];
        sandbox.restrict_filesystem(&whitelist);

        assert_eq!(sandbox.filesystem_rules.len(), 2);
        assert_eq!(sandbox.filesystem_rules[0], PathBuf::from("C:\\temp"));
        assert_eq!(
            sandbox.filesystem_rules[1],
            PathBuf::from("C:\\Users\\test\\data")
        );
    }

    #[test]
    fn test_windows_sandbox_restrict_network() {
        let config = SandboxConfig::safe_default();
        let mut sandbox = WindowsSandbox::create(config).unwrap();

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
    fn test_windows_sandbox_restrict_syscalls() {
        let config = SandboxConfig::safe_default();
        let mut sandbox = WindowsSandbox::create(config).unwrap();

        sandbox.restrict_syscalls(SyscallPolicy::DenyAll);
        assert_eq!(sandbox.syscall_policy, Some(SyscallPolicy::DenyAll));
    }

    #[test]
    fn test_windows_sandbox_restrict_resources() {
        let config = SandboxConfig::safe_default();
        let mut sandbox = WindowsSandbox::create(config).unwrap();

        let limits = ResourceLimits::restrictive();
        sandbox.restrict_resources(limits.clone());
        assert_eq!(sandbox.resource_limits, Some(limits));
    }

    // ===== Builder Pattern Tests =====

    #[test]
    fn test_windows_sandbox_method_chaining() {
        let config = SandboxConfig::safe_default();
        let mut sandbox = WindowsSandbox::create(config).unwrap();

        let whitelist = vec![PathBuf::from("C:\\temp")];
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

    // ===== Thread Safety Tests =====

    #[test]
    fn test_windows_sandbox_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<WindowsSandbox>();
    }

    // ===== Profile Generation Tests =====

    #[test]
    fn test_windows_sandbox_generate_profile_power_mode() {
        let config = SandboxConfig::power_mode();
        let sandbox = WindowsSandbox::create(config).unwrap();
        let profile = sandbox.generate_profile();

        assert!(profile.contains("Windows Job Object Profile"));
        assert!(profile.contains("Mode: Power (no restrictions)"));
    }

    #[test]
    fn test_windows_sandbox_generate_profile_safe_mode() {
        let config = SandboxConfig::safe_default();
        let sandbox = WindowsSandbox::create(config).unwrap();
        let profile = sandbox.generate_profile();

        assert!(profile.contains("Windows Job Object Profile"));
        assert!(profile.contains("Mode: Safe"));
        assert!(profile.contains("NOT enforced"));
        // Subprocess blocking should be listed
        assert!(profile.contains("Subprocess blocked: true"));
    }

    #[test]
    fn test_windows_sandbox_generate_profile_with_resources() {
        let config = SandboxConfig::safe_default();
        let mut sandbox = WindowsSandbox::create(config).unwrap();
        sandbox.restrict_resources(ResourceLimits {
            max_memory_bytes: Some(256 * 1024 * 1024),
            max_processes: Some(4),
            max_cpu_percent: None,
            max_file_descriptors: None,
        });
        let profile = sandbox.generate_profile();

        assert!(profile.contains("Memory limit: 268435456 bytes [enforced]"));
        assert!(profile.contains("Max processes: 4 [enforced]"));
    }

    // ===== Apply Tests =====

    #[test]
    fn test_windows_sandbox_apply_power_mode() {
        let config = SandboxConfig::power_mode();
        let sandbox = WindowsSandbox::create(config).unwrap();
        let handle = sandbox.apply().unwrap();
        assert!(matches!(handle.platform_handle, PlatformHandle::Windows(_)));

        if let PlatformHandle::Windows(id) = handle.platform_handle {
            assert_eq!(id, 0);
        }
    }

    // NOTE: test_windows_sandbox_apply_safe_mode calls the real Job Object API.
    // It requires Windows 8+ for nested job object support (if running inside CI
    // job container). On Windows 10/11 developer machines, this always succeeds.
    #[test]
    fn test_windows_sandbox_apply_safe_mode() {
        let config = SandboxConfig::safe_default();
        let sandbox = WindowsSandbox::create(config).unwrap();
        let handle = sandbox.apply().unwrap();
        assert!(matches!(handle.platform_handle, PlatformHandle::Windows(_)));

        if let PlatformHandle::Windows(id) = handle.platform_handle {
            assert_eq!(id, 1);
        }
    }
}
