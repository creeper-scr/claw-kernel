# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
# Build (default: Lua engine only, ~3 min)
cargo build

# Build with optional engines
cargo build --features engine-v8   # Deno/V8, needs Node.js ≥ 20, ~30 min first build
cargo build --features engine-py   # Python, needs Python ≥ 3.10 and Rust 1.83+

# Test
cargo test --workspace
cargo test --workspace --features integration-tests
cargo test --workspace --features sandbox-tests  # Linux only

# Run a single test
cargo test -p claw-loop test_name
cargo test -p claw-provider -- --nocapture

# Quality checks (must pass before PR)
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo audit

# Platform setup (Linux only)
sudo apt-get install libseccomp-dev pkg-config
```

## Architecture

5-layer Rust workspace. Platform-specific code goes **only** in `claw-pal`.

```
Layer 3     claw-script   — ScriptEngine trait; LuaEngine (mlua via spawn_blocking)
Layer 2.5   claw-channel  — Channel trait: Discord / HTTP Webhook / Stdin
Layer 2     claw-provider — 5 LLM providers (Anthropic/OpenAI/Ollama/DeepSeek/Moonshot)
            claw-tools    — ToolRegistry (DashMap), HotLoader (notify, 50ms debounce)
            claw-loop     — AgentLoop, InMemoryHistory, stop conditions, Builder
            claw-memory   — NgramEmbedder (64-dim), SqliteMemoryStore, SecureMemoryStore (50MB)
Layer 1     claw-runtime  — EventBus (broadcast 1024), AgentOrchestrator, IpcRouter
Layer 0.5   claw-pal      — IPC (Unix socket/Named Pipe), ProcessManager, sandbox, dirs
Meta        claw-kernel/  — Re-exports all sub-crates + prelude module
```

### Key data flow

`AgentLoopBuilder` (claw-loop) composes a `Provider` + `ToolRegistry` + `History` + stop conditions → `AgentLoop::run()` → streams tokens from provider → dispatches tool calls through registry → writes back to history. The runtime's `EventBus` allows cross-agent signaling; `IpcRouter` handles inter-process routing via the PAL IPC transport.

### Provider structure (claw-provider/src/)

Each provider crate follows a 3-layer internal structure: `format.rs` (MessageFormat trait impl) → `transport.rs` (DefaultHttpTransport / reqwest) → `mod.rs` (LLMProvider trait impl). New providers go in their own subdirectory following this pattern.

## Critical Implementation Rules

1. **`panic = "unwind"` in all profiles** — required by mlua. Never change this in `Cargo.toml`.
2. **IPC: single reader thread + mpsc dispatch** — do NOT use concurrent split I/O on the socket; causes macOS panics.
3. **mlua sync API must run inside `tokio::task::spawn_blocking`** — never call blocking mlua code directly in an async context.
4. **`interprocess` feature name is `tokio_support`** — not `tokio`.
5. **`AgentId` uses `AtomicU64`** — do not use random IDs; the counter prevents duplicates under fast parallel tests.
6. **Platform-specific code belongs only in `claw-pal`** — use `#[cfg(target_os = "...")]`; no `fork()`, POSIX signals, or hardcoded `/` paths in shared code.
7. **Error handling convention** — `thiserror` for library crates, `anyhow` only in examples/application code.

## Security Model

Two execution modes:
- **Safe Mode** (default): filesystem allowlist, network rules, subprocess blocked.
- **Power Mode** (explicit opt-in): full access, requires a Power Key ≥ 12 chars, 2 character classes.

Power Mode → Safe Mode **requires restart** (a compromised agent cannot downgrade). Never add bypass paths in Safe Mode enforcement.

## Significant Change Process

Changes to public APIs, cross-platform behavior, new dependencies, or the security model require an **ADR** in `docs/adr/`. Open a GitHub Discussion first, then open a PR with the ADR (format: Context → Decision → Consequences).

## Feature Flags

| Flag | Engine | External requirement |
|------|--------|---------------------|
| `engine-lua` (default) | Lua 5.4 via mlua | none |
| `engine-v8` | Deno/V8 via deno_core | Node.js ≥ 20 |
| `engine-py` | CPython via PyO3 | Python ≥ 3.10, Rust 1.83+ |
| `sqlite` (default) | SQLite memory backend | none (bundled) |
| `sandbox-tests` | seccomp sandbox tests | Linux only |

Default build uses only `engine-lua` + `sqlite`. Keep the default build minimal.
