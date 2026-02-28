# Learnings & Conventions

## 2026-02-28 Session Start

### Workspace Structure
- Root Cargo.toml has [workspace] with 9 members (already complete)
- Members: crates/claw-pal, crates/claw-runtime, crates/claw-provider, crates/claw-tools, crates/claw-loop, crates/claw-memory, crates/claw-channel, crates/claw-script, claw-kernel (meta at root)
- All workspace.dependencies already defined in root Cargo.toml
- Rust 1.83+ required (PyO3 constraint)

### Cargo.toml Issues to Fix (Task 2)
- Root Cargo.toml has [features] and [dev-dependencies] at workspace root level WITHOUT workspace prefix
- These need fixing: [features] → move to individual crates; [dev-dependencies] → convert to [workspace.dev-dependencies] OR move to crate level
- panic=unwind must be added to [profile.release] and [profile.dev] in root Cargo.toml

### Per-Crate Dependencies (from crate-map)
- claw-pal: async-trait, thiserror, serde, serde_json, tokio, interprocess, dirs; Linux only: libseccomp, nix; Task 7 adds: argon2
- claw-runtime: claw-pal (path), tokio, async-trait, thiserror, serde, serde_json, dashmap
- claw-provider: tokio, async-trait, thiserror, serde, serde_json, reqwest, futures
- claw-tools: serde, serde_json, schemars, notify, thiserror, async-trait, tokio
- claw-loop: claw-provider (path), claw-tools (path), tokio, async-trait, thiserror, serde, serde_json
- claw-script: optional mlua, tokio, async-trait, thiserror, serde, serde_json; depends on claw-tools bridge
- claw-memory: minimal - serde, serde_json, thiserror, async-trait
- claw-channel: minimal - async-trait, serde, serde_json, thiserror
- claw-kernel (meta): all crates as path deps

### Feature Flag Strategy
- Features must live in individual crate Cargo.toml, NOT in workspace root
- claw-script: engine-lua = ["dep:mlua"], engine-v8 = ["dep:deno_core"], engine-py = ["dep:pyo3"]
- claw-kernel meta: should re-export features from claw-script
- dashmap NOT in workspace Cargo.toml → need to add as workspace.dependency OR add directly to claw-runtime/claw-pal

### Key Technical Constraints (Metis findings)
1. CRITICAL: panic = "unwind" in ALL profiles (panic = "abort" + mlua = SIGABRT crash)
2. CRITICAL: seccomp use SCMP_ACT_ERRNO(EPERM) NOT SCMP_ACT_KILL (KILL + thread join = Rust panic)
3. CRITICAL: interprocess - single reader thread + channel dispatch (concurrent split I/O panics on macOS)
4. WARNING: Windows sandbox is skeleton only (AppContainer + Named Pipe DACL issues)
5. WARNING: deno_core API unstable, pin exact version 0.245.0
6. WARNING: PyO3 GIL/Tokio deadlock - use dedicated Python thread + channel bridge

### dashmap Missing from Workspace
- claw-runtime needs DashMap<AgentId, AgentHandle> per plan
- dashmap = "5.5.3" needs to be added to [workspace.dependencies]
- Same for futures = "0.3" needed by claw-provider (BoxStream)
