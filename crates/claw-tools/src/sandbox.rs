//! OS-level sandbox injection for [`ToolRegistry`].
//!
//! Defines the [`SandboxApplier`] trait that lets application code wire a real
//! platform sandbox (Linux seccomp-bpf / macOS sandbox_init) into the tool
//! registry **without creating a circular dependency** between `claw-tools` and
//! `claw-pal`.
//!
//! # Why a separate trait?
//!
//! `claw-pal` already has complete `LinuxSandbox` and `MacOSSandbox` backends, but
//! `claw-tools` cannot depend on `claw-pal` (it would create a cycle:
//! `claw-pal` → … → `claw-tools` → `claw-pal`).  The same pattern used by
//! [`PowerKeyVerify`][crate::registry::PowerKeyVerify] is applied here: define a
//! local object-safe abstraction in `claw-tools`, and let higher-level crates
//! (`claw-runtime`, `claw-kernel`) provide the concrete implementation backed by
//! `claw-pal`.
//!
//! # Security semantics
//!
//! `apply_safe_mode()` is called **at most once** per [`ToolRegistry`] instance,
//! on the first `Safe`-mode `execute()` call.  On Linux and macOS the underlying
//! mechanisms (seccomp-bpf, `sandbox_init`) are **irrevocable** — they restrict
//! the entire calling process for its remaining lifetime.
//!
//! This means:
//! - The registry (and its process) should be dedicated to sandboxed tool
//!   execution.  Injecting a real `SandboxApplier` into the main kernel process
//!   would restrict the whole daemon.
//! - The natural integration point is an **agent subprocess** spawned by
//!   `claw-runtime`'s `AgentOrchestrator`, which configures a `SandboxApplier`
//!   and calls `ToolRegistry::with_sandbox_applier()` before any tool is run.
//!
//! # Integration guide (claw-runtime / claw-kernel)
//!
//! ```rust,ignore
//! // In your agent subprocess startup (e.g., claw-runtime's agent process):
//! use claw_pal::linux::sandbox::{LinuxSandbox, SandboxBackend};
//! use claw_pal::{SandboxConfig, SyscallPolicy};
//! use claw_tools::sandbox::SandboxApplier;
//! use std::path::PathBuf;
//! use std::sync::Arc;
//!
//! struct PalLinuxSandboxApplier {
//!     fs_allowlist: Vec<PathBuf>,
//! }
//!
//! impl SandboxApplier for PalLinuxSandboxApplier {
//!     fn apply_safe_mode(&self) -> Result<(), String> {
//!         let config = SandboxConfig::safe_default();
//!         let mut sb = LinuxSandbox::create(config).map_err(|e| e.to_string())?;
//!         sb.restrict_filesystem(&self.fs_allowlist)
//!           .restrict_syscalls(SyscallPolicy::DenyAll);
//!         sb.apply().map(|_| ()).map_err(|e| e.to_string())
//!     }
//! }
//!
//! let registry = ToolRegistry::new()
//!     .with_sandbox_applier(Arc::new(PalLinuxSandboxApplier {
//!         fs_allowlist: vec!["/tmp".into()],
//!     }));
//! ```

use std::sync::Arc;

// ─── SandboxApplier ──────────────────────────────────────────────────────────

/// Object-safe abstraction over OS sandbox application.
///
/// Inject an implementation of this trait into [`ToolRegistry`][crate::registry::ToolRegistry]
/// via [`with_sandbox_applier()`][crate::registry::ToolRegistry::with_sandbox_applier] to
/// enable OS-level enforcement of the Safe execution mode.
///
/// # Contract
///
/// - `apply_safe_mode()` MAY be called from any thread.
/// - It will be called **at most once** per registry instance (guarded by
///   [`std::sync::OnceLock`] in the registry).
/// - Implementations MUST be idempotent: calling them twice MUST NOT make things
///   worse (even though the registry guarantees at-most-once).
/// - On failure, the registry returns [`RegistryError::ExecutionFailed`][crate::error::RegistryError]
///   and refuses to run any further tools (fail-closed).
///
/// # Thread safety
///
/// Implementations must be `Send + Sync`; the registry may call `apply_safe_mode`
/// from a `tokio` worker thread.
pub trait SandboxApplier: Send + Sync {
    /// Apply the pre-configured OS sandbox to the current process.
    ///
    /// The sandbox policy (filesystem allowlist, network rules, syscall policy,
    /// resource limits) should be baked into the implementation at construction
    /// time and derived from the agent's `PermissionSet` and deployment config.
    ///
    /// # Errors
    ///
    /// Return `Err(String)` with a human-readable message if the sandbox could
    /// not be applied (e.g., insufficient privileges, kernel too old, invalid
    /// policy).  The registry treats any error as fatal and will refuse all
    /// subsequent tool executions.
    fn apply_safe_mode(&self) -> Result<(), String>;
}

// ─── NoopSandboxApplier ──────────────────────────────────────────────────────

/// A no-op [`SandboxApplier`] that always succeeds without applying any OS restrictions.
///
/// Use this in:
/// - The main kernel/server process (where real sandboxing would break all daemons)
/// - Unit and integration tests
/// - Development environments where kernel sandbox support is unavailable
///
/// Note: Using `NoopSandboxApplier` means `ToolRegistry` relies only on its
/// Rust-level permission checks (glob matching), **not** OS-level enforcement.
/// This is the legacy behaviour prior to G-2 being fixed.
pub struct NoopSandboxApplier;

impl SandboxApplier for NoopSandboxApplier {
    fn apply_safe_mode(&self) -> Result<(), String> {
        tracing::debug!(
            "NoopSandboxApplier: OS-level sandbox skipped \
             (Rust-layer permission checks only)"
        );
        Ok(())
    }
}

impl NoopSandboxApplier {
    /// Wrap a `NoopSandboxApplier` in an `Arc` ready for injection.
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> Arc<dyn SandboxApplier> {
        Arc::new(NoopSandboxApplier)
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A counting applier used to verify at-most-once invocation semantics.
    struct CountingApplier {
        call_count: AtomicU32,
    }

    impl CountingApplier {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                call_count: AtomicU32::new(0),
            })
        }
    }

    impl SandboxApplier for CountingApplier {
        fn apply_safe_mode(&self) -> Result<(), String> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    /// A failing applier to test fail-closed behaviour.
    struct FailingApplier;

    impl SandboxApplier for FailingApplier {
        fn apply_safe_mode(&self) -> Result<(), String> {
            Err("simulated sandbox failure".to_string())
        }
    }

    #[test]
    fn test_noop_applier_succeeds() {
        let applier = NoopSandboxApplier;
        assert!(applier.apply_safe_mode().is_ok());
    }

    #[test]
    fn test_noop_applier_new_returns_arc() {
        let applier = NoopSandboxApplier::new();
        assert!(applier.apply_safe_mode().is_ok());
    }

    #[test]
    fn test_counting_applier_tracks_calls() {
        let applier = CountingApplier::new();
        assert_eq!(applier.call_count.load(Ordering::SeqCst), 0);
        applier.apply_safe_mode().unwrap();
        assert_eq!(applier.call_count.load(Ordering::SeqCst), 1);
        applier.apply_safe_mode().unwrap();
        assert_eq!(applier.call_count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn test_failing_applier_returns_error() {
        let applier = FailingApplier;
        let err = applier.apply_safe_mode().unwrap_err();
        assert!(err.contains("simulated sandbox failure"));
    }

    #[test]
    fn test_sandbox_applier_is_object_safe() {
        // Verify the trait can be used as a trait object (object-safe check).
        let _: Box<dyn SandboxApplier> = Box::new(NoopSandboxApplier);
        let _: Arc<dyn SandboxApplier> = NoopSandboxApplier::new();
    }

    #[test]
    fn test_sandbox_applier_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<NoopSandboxApplier>();
        assert_send_sync::<FailingApplier>();
    }
}
