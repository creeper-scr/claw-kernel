---
title: claw-kernel Project Guide for AI Agents
description: Comprehensive guide for AI coding agents working on claw-kernel
status: v0.1.0
version: "0.1.0"
last_updated: "2026-03-01"
language: en
---

# claw-kernel Project Guide for AI Agents

> The shared foundation for the Claw ecosystem — a cross-platform Agent Kernel built in Rust with embedded scripting and hot-loading capabilities.

**License:** MIT OR Apache-2.0 | **Status:** v0.1.0 — 9 crates implemented, 389 tests passing

---

## Project Overview

claw-kernel is a shared infrastructure library for the Claw ecosystem — a collection of AI agent implementations (OpenClaw, ZeroClaw, PicoClaw, etc.). It extracts common primitives (LLM provider HTTP calls, tool-use protocol, agent loop, memory system) into a single, well-tested, cross-platform Rust library.

### Key Design Principles

1. **Rust kernel, script logic** — The Rust core is stable and never hot-patched; all extensible logic lives in scripts
2. **Extensible by design** — Kernel provides hot-loading capabilities; applications can implement self-evolution on top
3. **Cross-platform first** — Linux, macOS, and Windows are equal first-class targets
4. **Two execution modes** — Safe Mode (sandboxed, default) and Power Mode (full system access, explicit opt-in)
5. **Minimal core, plugin ecosystem** — Unix philosophy: do one thing well, compose for the rest

---

## Technology Stack

| Component | Technology | Notes |
|-----------|------------|-------|
| Core Language | Rust (stable, **1.83+**) | MSRV driven by PyO3 0.28+ |
| Async Runtime | Tokio | For async I/O and concurrency |
| Script Engines | Lua (mlua), Deno/V8, PyO3 | Lua is default (zero deps), V8/Py optional |
| HTTP Client | reqwest | For LLM provider APIs |
| IPC | Unix Domain Socket / Named Pipe | Cross-platform via `interprocess` crate |
| Build Tool | Cargo | Standard Rust build system |
| JSON Schema | schemars | Automatic schema generation |
| File Watching | notify | For hot-reload functionality |

For pinned dependency versions, feature matrix, and build configuration examples, see [CONTRIBUTING.md](CONTRIBUTING.md#dependency-versions).

---

## Architecture

The project follows a 5-layer architecture:

```
┌─────────────────────────────────────────────────────────┐
│              Layer 3: Extension Foundation               │
│   Lua (default) · Deno/V8 · PyO3                        │
├─────────────────────────────────────────────────────────┤
│              Layer 2: Agent Kernel Protocol              │
│   Provider Trait · ToolRegistry · AgentLoop · History    │
├─────────────────────────────────────────────────────────┤
│              Layer 1: System Runtime                     │
│   Event Bus · IPC Transport · Process Daemon · Tokio     │
├═════════════════════════════════════════════════════════╡
│              Layer 0.5: Platform Abstraction (PAL)       │
│   Sandbox Backend · IPC Transport Primitives · Config Dirs │
├─────────────────────────────────────────────────────────┤
│              Layer 0: Rust Hard Core                     │
│   Memory Safety · OS Abstraction · Trust Root            │
└─────────────────────────────────────────────────────────┘
```

### Platform Support

| Platform | Sandbox Backend | Isolation Level |
|----------|-----------------|-----------------|
| Linux | seccomp-bpf + Namespaces | Strongest |
| macOS | sandbox(7) profile (Seatbelt) | Medium |
| Windows | AppContainer + Job Objects | Medium |

---

## Project Structure

```
claw-kernel/
├── crates/
│   ├── claw-pal/         # Layer 0.5: Platform Abstraction (IPC, sandbox, process)
│   ├── claw-runtime/     # Layer 1: Event bus, orchestrator, IPC router
│   ├── claw-provider/    # Layer 2: LLM providers (Anthropic/OpenAI/Ollama/DeepSeek/Moonshot)
│   ├── claw-tools/       # Layer 2: Tool registry, hot-loading
│   ├── claw-loop/        # Layer 2: Agent loop engine, history, stop conditions
│   ├── claw-memory/      # Layer 2: Memory store (SQLite + ngram embedder)
│   ├── claw-channel/     # Layer 2.5: Channel integrations (Discord/Webhook/Stdin)
│   ├── claw-script/      # Layer 3: Script engines (Lua via mlua)
│   └── claw-kernel/      # Meta-crate: re-exports all sub-crates + prelude
├── docs/
│   ├── architecture/     # Layer-by-layer architecture docs + crate map
│   ├── adr/              # Architecture Decision Records (ADR-001 through ADR-008)
│   ├── guides/           # User-facing how-to guides
│   ├── crates/           # Per-crate documentation
│   ├── platform/         # Platform-specific notes (linux.md, macos.md, windows.md)
│   └── design/           # Deep-dive design docs (agent-loop, channel-protocol)
├── examples/             # Runnable example agents
├── .github/              # CI workflows and issue templates
├── README.md             # Project overview (bilingual)
├── CONTRIBUTING.md       # Contribution guidelines + dependency versions + feature matrix
├── CHANGELOG.md          # Version history
├── ROADMAP.md            # Milestones and future work
├── SECURITY.md           # Security policy and vulnerability reporting
├── CODE_OF_CONDUCT.md    # Contributor Covenant
├── llm-index.md          # Machine-readable documentation index
└── AGENTS.md             # This file
```

---

## Crate Ecosystem

| Crate | Layer | Description |
|-------|-------|-------------|
| `claw-pal` | 0.5 | Platform Abstraction: sandbox, IPC (Unix socket, 4-byte LE frame), ProcessManager (DashMap+SIGTERM→SIGKILL) |
| `claw-runtime` | 1 | EventBus (broadcast cap 1024), AgentOrchestrator (DashMap), IpcRouter |
| `claw-provider` | 2 | 5 LLM providers + DefaultHttpTransport; 3-layer: MessageFormat → HttpTransport → LLMProvider |
| `claw-tools` | 2 | ToolRegistry (DashMap, timeout, audit), HotLoader (notify 6.1.1, 50ms debounce) |
| `claw-loop` | 2 | InMemoryHistory (overflow callback), MaxTurns/TokenBudget/NoToolCall stop conditions, AgentLoop + Builder |
| `claw-memory` | 2 | NgramEmbedder (64-dim bigram+trigram), SqliteMemoryStore (cosine sim), SecureMemoryStore (50MB quota) |
| `claw-channel` | 2.5 | Channel trait, ChannelMessage, Platform (Discord/Webhook/Stdin) |
| `claw-script` | 3 | ScriptEngine trait, LuaEngine (mlua via spawn_blocking), ToolsBridge |
| `claw-kernel` | meta | Re-exports all sub-crates + prelude module |

---

## Build and Development

For full build/test commands, code quality checks, and contributing workflow, see [CONTRIBUTING.md](CONTRIBUTING.md).

**Quick reference:**

```bash
cargo build                                        # default (Lua only)
cargo test --workspace                             # all tests
cargo clippy --workspace -- -D warnings            # lint (same as CI)
cargo fmt --all                                    # format
```

**Platform-specific setup:**
- Linux: `sudo apt-get install libseccomp-dev pkg-config`
- Windows: `rustup set default-host x86_64-pc-windows-msvc`

---

## Security Model

### Two Execution Modes

| Dimension | Safe Mode (default) | Power Mode (opt-in) |
|-----------|--------------------|---------------------|
| File System | Allowlist read-only | Full access |
| Network | Domain/port rules | Unrestricted |
| Subprocess | Blocked | Allowed |
| Script Self-Mod | Allowed (sandboxed) | Allowed (unrestricted)[^1] |
| Kernel Access | Blocked | Blocked (hard constraint) |

[^1]: Script self-modification means dynamically generating and loading new tools, not modifying the runtime engine.

### Mode Switching

```
┌─────────────┐      power-key + explicit flag      ┌─────────────┐
│  Safe Mode  │  ─────────────────────────────────► │  Power Mode │
│  (default)  │                                     │  (opt-in)   │
└─────────────┘                                     └─────────────┘
       ▲                                                    │
       │              restart or new process                │
       └────────────────────────────────────────────────────┘
```

**Important:** Power Mode → Safe Mode requires restart — a compromised agent cannot "downgrade" to hide evidence.

### Power Mode Activation

```bash
claw-kernel --set-power-key                        # set key (min 12 chars, 2 character classes)
claw-kernel --power-mode --power-key <key>         # activate via flag
export CLAW_KERNEL_POWER_KEY=<key>; claw-kernel --power-mode  # via env var
# or place key in ~/.config/claw-kernel/power.key
```

### Security Guarantees (Safe Mode)

- Scripts cannot access files outside allowlisted directories
- Scripts cannot spawn arbitrary subprocesses
- Scripts cannot make network requests to non-allowlisted endpoints
- Scripts cannot escalate to Power Mode without the correct credential
- The Rust hard core (Layer 0) cannot be modified by scripts

### Audit Logging

Location: `~/.local/share/claw-kernel/logs/audit.log`
Retention: 30 days (configurable). Levels: `minimal` (mode switches, permission changes, security events) / `verbose` (+ tool call args, network details, file paths).

---

## Extensibility Model

> **Key distinction:** Scripts/tools in Layer 3 are hot-loadable via ToolRegistry. The Rust runtime kernel (Layers 0–3) requires recompilation to change.

```
Application-level extensibility cycle:
  Detect gap → LLM generates tool script → Write to tools dir
  → ToolRegistry hot-loads (no restart) → Agent uses immediately
```

**Kernel provides:** hot-loading without restart, dynamic tool registration, script runtime (Lua/V8/Python), file watching (50ms debounce), sandboxed execution.

**Applications implement:** self-evolution logic, code generation, tool versioning, custom providers, workflows.

**CAN extend at runtime:** tool scripts, custom providers, memory strategies, stop conditions.
**CANNOT modify:** Rust kernel code, sandbox enforcement, mode-switching guards, credential storage.

---

## Key Documentation References

### Root-Level Documents

| Document | Purpose |
|----------|---------|
| [README.md](README.md) | Project overview and quick start (English) |
| [docs/README.zh.md](docs/README.zh.md) | 中文项目概览和快速开始 |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Dev setup, build commands, PR process, dependency versions, feature matrix |
| [SECURITY.md](SECURITY.md) | Vulnerability reporting and disclosure policy |
| [CHANGELOG.md](CHANGELOG.md) | Version history |
| [ROADMAP.md](ROADMAP.md) | Milestones and v0.2.0 plans |
| [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) | Contributor Covenant |

### Architecture & Design

| Document | Purpose |
|----------|---------|
| [docs/architecture/overview.md](docs/architecture/overview.md) | Full 5-layer architecture |
| [docs/architecture/crate-map.md](docs/architecture/crate-map.md) | Dependency graph of all crates |
| [docs/architecture/pal.md](docs/architecture/pal.md) | Platform Abstraction Layer deep dive |
| [docs/design/agent-loop-state-machine.md](docs/design/agent-loop-state-machine.md) | Agent loop execution algorithm and state transitions |
| [docs/design/channel-message-protocol.md](docs/design/channel-message-protocol.md) | ChannelMessage protocol, platform mappings |
| [docs/adr/](docs/adr/) | ADR-001 through ADR-008 (all Accepted) |

### Guides & Per-Crate Docs

| Document | Purpose |
|----------|---------|
| [docs/guides/getting-started.md](docs/guides/getting-started.md) | Build your first agent |
| [docs/guides/writing-tools.md](docs/guides/writing-tools.md) | Create custom tools with Lua scripts |
| [docs/guides/safe-mode.md](docs/guides/safe-mode.md) | Safe mode configuration and allowlists |
| [docs/guides/power-mode.md](docs/guides/power-mode.md) | Power mode activation and Power Key setup |
| [docs/guides/extension-capabilities.md](docs/guides/extension-capabilities.md) | Runtime extensibility and hot-loading |
| [docs/crates/](docs/crates/) | Per-crate API docs (claw-pal, -runtime, -provider, -tools, -loop, -script) |
| [docs/platform/](docs/platform/) | Platform-specific notes (linux.md, macos.md, windows.md) |

### Navigation by Use Case

| Goal | Start Here |
|------|------------|
| New user | README.md → docs/guides/getting-started.md |
| New contributor | CONTRIBUTING.md → AGENTS.md → docs/architecture/overview.md |
| Security review | SECURITY.md → AGENTS.md (Security Model) → docs/adr/003-security-model.md |
| Add a provider | docs/architecture/overview.md → docs/crates/claw-provider.md |
| Platform port | docs/architecture/pal.md → docs/platform/{linux,macos,windows}.md |
| Understand hot-loading | docs/adr/004-hot-loading-mechanism.md → docs/crates/claw-tools.md |

---

## Important Notes for AI Coding Agents

1. **Project Status:** v0.1.0 is fully implemented. All 9 crates exist in `crates/`. Read the actual source before proposing changes.

2. **Cross-Platform Critical:** Every change must consider Linux, macOS, and Windows. Platform-specific code belongs ONLY in `claw-pal`. Use `#[cfg(target_os = "...")]`.

3. **Critical Implementation Details:**
   - MSRV: Rust **1.83+**, edition 2021, ALL profiles `panic = "unwind"` (mlua requirement — do not change)
   - IPC: single reader thread + mpsc dispatch (NO concurrent split I/O — causes macOS panic)
   - `interprocess` feature name: `tokio_support` (not `tokio`)
   - mlua sync API must run in `tokio::task::spawn_blocking`
   - AgentId uses `AtomicU64` counter to prevent duplicates under fast parallel tests

4. **Feature Flags:** Default build has minimal dependencies. Lua is the only default script engine; V8 and Python are opt-in. See [CONTRIBUTING.md](CONTRIBUTING.md#feature-matrix) for the full matrix.

5. **Security First:** The Safe/Power mode distinction is fundamental. Never bypass sandbox restrictions in Safe Mode. Power Mode requires explicit user opt-in.

6. **ADR Process:** Significant changes (new public APIs, cross-platform behavior, new dependencies, security model) require an ADR in `docs/adr/`. Open a GitHub Discussion first.

7. **Testing:** Every crate needs unit + integration tests. `claw-pal` platform code requires tests on all three platforms. Run `cargo test --workspace` after changes.

8. **Worktrees:** Worktree agents start from the last committed state — commit your work before launching worktree agents.
