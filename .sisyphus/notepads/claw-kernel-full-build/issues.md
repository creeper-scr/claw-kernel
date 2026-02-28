# Issues & Gotchas

## 2026-02-28 Session Start

### Workspace Cargo.toml Issues Found
1. [features] at workspace root is NOT valid when no [package] section exists → need to move features to individual crates
2. [dev-dependencies] at workspace root (without workspace. prefix) → should be [workspace.dev-dependencies] or per-crate
3. Missing workspace.dependencies: dashmap (needed by claw-runtime, claw-pal), futures (needed by claw-provider for BoxStream)
4. Optional deps (mlua, deno_core, pyo3) defined in workspace.dependencies - individual crates must declare them with dep: syntax

### Platform-Specific Issues
- macOS: sandbox_init() has "init window" vulnerability → must apply sandbox FIRST in process before any user code
- Linux: SCMP_ACT_KILL with thread-level sandbox causes Rust panic → must use SCMP_ACT_ERRNO(EPERM)
- Windows: interprocess crate doesn't expose SECURITY_ATTRIBUTES for Named Pipe DACL → Windows sandbox stays as stub

### async-trait Note
- BUILD_PLAN.md shows IpcTransport trait with async methods directly
- In actual Rust, async in traits requires either async-trait crate OR Rust 1.75+ RPITIT
- Since Rust 1.83+ is required, we can use impl Trait in trait (RPITIT) OR async-trait - agent should decide
