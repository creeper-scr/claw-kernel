# claw-kernel Project Guide for AI Agents

> The shared foundation for the Claw ecosystem — a cross-platform Agent Kernel built in Rust with embedded scripting and hot-loading capabilities.

**Language:** English (primary), Chinese (secondary in documentation)  
**License:** MIT OR Apache-2.0  
**Status:** Design/Planning Stage — `crates/` directory is empty, no implementation started yet

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
| Core Language | Rust (stable, **1.83+**) | Memory safety, async/await via Tokio。`engine-py` 需要 1.83+ |
| Async Runtime | Tokio | For async I/O and concurrency |
| Script Engines | Lua (mlua), Deno/V8, PyO3 | Lua is default (zero deps), V8/Py optional<br>`engine-v8` 特性使用 deno_core crate 提供 V8 引擎支持 |
| HTTP Client | reqwest | For LLM provider APIs |
| IPC | Unix Domain Socket / Named Pipe | Cross-platform via `interprocess` crate |
| Build Tool | Cargo | Standard Rust build system |
| JSON Schema | schemars | Automatic schema generation |
| File Watching | notify | For hot-reload functionality |

### Optional Dependencies (Feature-gated)

- **Deno/V8 engine (`engine-v8`):** Node.js ≥ 20, adds ~100MB to binary
  - *术语说明："Deno/V8" 用于用户文档表示 Deno 项目基于 V8 引擎；`engine-v8` 用于 Cargo 特性标志*
- **Python engine (`engine-py`):** Python ≥ 3.10, for ML ecosystem integration

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
│   Event Bus · IPC Transport · Process Daemon · Tokio │
├═════════════════════════════════════════════════════════╡
│              Layer 0.5: Platform Abstraction (PAL)       │
│   Sandbox Backend · IPC Transport Primitives · Config Directories │
├─────────────────────────────────────────────────────────┤
│              Layer 0: Rust Hard Core                     │
│   Memory Safety · OS Abstraction · Trust Root            │
└─────────────────────────────────────────────────────────┘

**分层说明**：
- Layer 0: Rust Hard Core（Rust硬核心）— 内存安全和操作系统抽象
- Layer 0.5: Platform Abstraction Layer（平台抽象层）— IPC传输原语和沙箱后端
- Layer 1: System Runtime（系统运行时）— IPC路由/事件总线和进程守护
- Layer 2: Agent Kernel Protocol（代理内核协议）— 工具注册、LLM提供者、代理循环
- Layer 3: Extension Foundation（扩展基础）— 提供脚本运行时和热加载能力
```

### Platform Support

| Platform | Sandbox Backend | Status |
|----------|-----------------|--------|
| Linux | seccomp-bpf + Namespaces | Strongest isolation (最强隔离) |
| macOS | sandbox(7) profile (Seatbelt) | Medium isolation (中等隔离) |
| Windows | AppContainer + Job Objects | Medium isolation (中等隔离) |

---

## Project Structure

```
claw-kernel/
├── crates/               # Workspace crates (CURRENTLY EMPTY)
│   ├── claw-pal/         # Platform Abstraction Layer
│   ├── claw-provider/    # LLM provider trait + implementations
│   ├── claw-tools/       # Tool registry and hot-loading
│   ├── claw-loop/        # Agent loop engine
│   ├── claw-runtime/     # Event bus and process management
│   └── claw-script/      # Embedded script engines
├── docs/
│   ├── architecture/     # Layer-by-layer architecture docs
│   │   ├── overview.md   # Full architecture description
│   │   ├── crate-map.md  # Dependency graph of all crates
│   │   └── pal.md        # Platform Abstraction Layer deep dive
│   ├── adr/              # Architecture Decision Records
│   │   ├── README.md     # ADR index
│   │   ├── 001-architecture-layers.md
│   │   ├── 002-script-engine-selection.md
│   │   ├── 003-security-model.md
│   │   ├── 004-hot-loading-mechanism.md
│   │   └── 005-ipc-multi-agent.md
│   ├── guides/           # User-facing how-to guides
│   │   ├── getting-started.md
│   │   ├── writing-tools.md
│   │   ├── extension.md
│   │   ├── safe-mode.md
│   │   └── power-mode.md
│   ├── crates/           # Per-crate documentation
│   ├── platform/         # Platform-specific notes (linux.md, macos.md, windows.md)
│   └── rfcs/             # RFCs directory (empty)
├── examples/             # Runnable example agents (CURRENTLY EMPTY)
│   ├── simple-agent/
│   ├── custom-tool/
│   └── self-evolving-agent/
├── .github/              # CI workflows and issue templates
│   ├── workflows/        # GitHub Actions (empty)
│   ├── ISSUE_TEMPLATE/   # Issue templates
│   └── pull_request_template.md
├── README.md             # Project overview (bilingual)
├── CONTRIBUTING.md       # Contribution guidelines (bilingual)
├── CHANGELOG.md          # Version history (bilingual)
├── CODE_OF_CONDUCT.md    # Contributor Covenant
├── SECURITY.md           # Security policy (bilingual)
├── BUILD_PLAN.md         # Detailed Chinese build roadmap
├── LICENSE-*             # Dual licensing (MIT/Apache-2.0)
└── AGENTS.md             # This file
```

---

## Crate Ecosystem

| Crate | Description | Key Dependencies |
|-------|-------------|------------------|
| `claw-pal` | Platform Abstraction Layer (sandbox, IPC, process) | None (core only) |
| `claw-provider` | LLM provider trait + Anthropic/OpenAI/Ollama implementations | reqwest, serde, async-trait |
| `claw-tools` | Tool-use protocol, registry, schema gen, hot-loading | serde_json, schemars, notify |
| `claw-loop` | Agent loop engine, history management, stop conditions | claw-provider, claw-tools |
| `claw-runtime` | Event bus, async runtime, multi-agent orchestration | claw-pal, tokio |
| `claw-script` | Embedded script engines (Lua default, Deno/V8, PyO3) | mlua, deno_core, pyo3 |
| `claw-kernel` | Meta-crate, re-exports all above crates | All of the above |

### Dependency Graph

```
                        ┌─────────────────┐
                        │  claw-kernel    │  ← Meta-crate
                        └────────┬────────┘
                                 │
        ┌────────────────────────┼────────────────┐
        │            │           │                │
        ▼            ▼           ▼                ▼
┌──────────┐ ┌──────────┐ ┌─────────┐      ┌──────────┐
│claw-pal  │ │claw-tools│ │claw-loop│      │claw-runtime│
└────┬─────┘ └────┬─────┘ └────┬────┘      └────┬─────┘
     │            │            │                │
     │       ┌────┴────┐       │                │
     │       ▼         ▼       │                │
     │  ┌────────┐ ┌────────┐  │                │
     │  │claw-   │ │claw-   │  │                │
     │  │provider│ │runtime │  │                │
     │  └────────┘ └────┬───┘  │                │
     │                  │      │                │
     └──────────────────┘      │                │
                               │                │
                               ▼                ▼
                          ┌──────────┐   ┌──────────┐
                          │  tokio   │   │  serde   │
                          └──────────┘   └──────────┘

┌──────────┐
│claw-script│  ← Independent, can depend on others via bridge
└──────────┘
```

*注：图中 claw-pal 作为基础依赖被所有其他 crate 间接使用*

---

## Build and Development Commands

### Prerequisites

| Tool | Version | Notes |
|------|---------|-------|
| Rust | stable (**1.83+**) | Via rustup。`engine-py` feature requires 1.83+ |
| cargo | bundled | — |
| Node.js | ≥ 20 (optional) | Only for `engine-v8` feature |
| Python | ≥ 3.10 (optional) | Only for `engine-py` feature |

### Platform-specific Requirements

**Linux:**
```bash
# Ubuntu/Debian
sudo apt-get install libseccomp-dev pkg-config

# Fedora/RHEL
sudo dnf install libseccomp-devel

# Arch
sudo pacman -S libseccomp
```

**Windows:**
- Use MSVC toolchain: `rustup set default-host x86_64-pc-windows-msvc`

### Build Commands

```bash
# Default build (Lua engine only, zero extra dependencies)
cargo build

# With Deno/V8 engine (downloads precompiled V8, may take a few minutes)
cargo build --features engine-v8

# With Python engine
cargo build --features engine-py

# Full build with all features
cargo build --all-features

# Release build
cargo build --release
```

### Testing Commands

```bash
# Run all tests across all crates
cargo test --workspace

# Include integration tests
cargo test --workspace --features integration-tests

# Platform-specific sandbox tests (Linux only)
cargo test --workspace --features sandbox-tests

# Run tests for specific crate
cargo test -p claw-pal
```

### Code Quality Commands

```bash
# Check for lint warnings
cargo clippy --workspace

# Check for lint warnings (treat as errors)
cargo clippy --workspace -- -D warnings

# Format code
cargo fmt --all

# Check formatting without modifying
cargo fmt --all -- --check

# Run cargo audit for security
cargo audit
```

---

## Code Style Guidelines

### General Principles

1. **Cross-platform first** — No Unix-isms, no Windows-isms in core code
2. **Platform-specific code** — Isolate in `claw-pal` using conditional compilation:
   ```rust
   #[cfg(target_os = "linux")]
   mod linux;
   #[cfg(target_os = "macos")]
   mod macos;
   #[cfg(target_os = "windows")]
   mod windows;
   ```
3. **Feature flags** — Use for:
   - Platform-specific functionality with heavy deps (e.g., `engine-v8`)
   - Optional storage backends (e.g., `sqlite`)
4. **Documentation** — All public APIs must have doc comments
5. **Error handling** — Use `thiserror` for library errors, `anyhow` for applications

### Testing Strategy by Crate

| Crate | Unit Tests | Integration | Platform Tests |
|-------|:----------:|:-----------:|:--------------:|
| claw-pal | ✅ | ✅ | Required per-platform |
| claw-provider | ✅ | ✅ (mock HTTP) | N/A |
| claw-tools | ✅ | ✅ | N/A |
| claw-loop | ✅ | ✅ | N/A |
| claw-runtime | ✅ | ✅ | Required |
| claw-script | ✅ | ✅ | Required per-engine |

---

## Security Model

### Two Execution Modes

| Dimension | Safe Mode (default) | Power Mode (opt-in) |
|-----------|--------------------|---------------------|
| File System | Allowlist read-only | Full access |
| Network | Domain/port rules | Unrestricted |
| Subprocess | Blocked | Allowed |
| Script Self-Mod | Allowed (sandboxed) | Allowed (global) [^1] |
| Kernel Access | Blocked | Blocked (hard constraint) |

[^1]: *脚本自修改指脚本代码动态生成和加载新工具，不涉及修改运行时引擎*

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

**Important:** Power Mode → Safe Mode requires restart. This is intentional — a compromised Power Mode agent cannot "downgrade" to hide evidence.

### Power Mode Activation

Power Key 由用户自定义设置，用于从 Safe Mode 切换到 Power Mode。

```bash
# 设置 Power Key（首次使用或重置）
claw-kernel --set-power-key
Enter new power key (min 8 chars): ********
Confirm power key: ********
Power key set successfully.

# 方式1：命令行参数
claw-kernel --power-mode --power-key <your-key>

# 方式2：环境变量
export CLAW_KERNEL_POWER_KEY=<your-key>
claw-kernel --power-mode

# 方式3：配置文件 (~/.config/claw-kernel/power.key)
claw-kernel --power-mode
```

**要求**：
- Power Key 最小长度：**8 位字符**
- 实际安全性依赖用户选择的密码复杂度
- 遗忘 Power Key 后，只能通过 `--reset-power-key` 重置，这将丢失 Power Mode 的访问权限

### Security Guarantees (Safe Mode)

- Scripts cannot access files outside allowlisted directories
- Scripts cannot spawn arbitrary subprocesses
- Scripts cannot make network requests to non-allowlisted endpoints
- Scripts cannot escalate to Power Mode without the correct credential
- The Rust hard core (Layer 0) cannot be modified by scripts
- Kernel secret storage is inaccessible to scripts

### Audit Logging

内核提供简化版审计日志，默认开启，记录内容：
- 工具调用事件
- 文件访问事件
- 模式切换事件
- 网络请求事件

日志位置：`~/.local/share/claw-kernel/logs/audit.log`
保留策略：按时间限制（默认 **30 天**，可通过配置调整）
日志级别：minimal（仅关键事件）、verbose（完整参数）

```rust
// 配置审计日志保留时间
let config = AuditConfig {
    retention_days: 30,  // 可配置
    log_level: LogLevel::Minimal,
};
```

---

## Extensibility Model

> **重要区分**：
> - **可热加载**：Layer 3 的脚本/工具代码（通过 ToolRegistry）
> - **不可修改**：Layer 0/0.5/1/2/3 运行时内核（需要重新编译）
> 
> Rust 核心代码 "never hot-patched" 指的是后者，与脚本热加载不矛盾。

The kernel provides hot-loading capabilities that enable applications to extend their functionality at runtime. Applications built on claw-kernel can implement self-evolution patterns using these primitives:

```
Extensibility Cycle (Application Layer):
    │
    ▼
┌─────────────┐
│ 1. Detect   │  Agent identifies capability gap
│    Gap      │
└──────┬──────┘
       ▼
┌─────────────┐
│ 2. Generate │  LLM generates tool script
│    Tool     │
└──────┬──────┘
       ▼
┌─────────────┐
│ 3. Write    │  Script saved to tools directory
│             │
└──────┬──────┘
       ▼
┌─────────────┐
│ 4. Hot Load │  ToolRegistry loads without restart (kernel feature)
│             │
└──────┬──────┘
       ▼
┌─────────────┐
│ 5. Use      │  Agent immediately uses new capability
│             │
└─────────────┘
```

### Kernel Capabilities for Extensibility

**Provided by Kernel:**
- Hot-loading of tool scripts without restart
- Dynamic tool registration via ToolRegistry
- Script runtime environment (Lua/V8/Python)
- File watching for automatic reload
- Sandboxed execution environment

**Implemented by Applications:**
- Self-evolution logic and decision making
- Code generation strategies
- Tool versioning and rollback
- Custom provider implementations
- Application-specific workflows

### Extensibility Boundaries

**CAN Extend at Runtime:**
- Tool scripts
- Custom providers
- Memory strategies
- Stop conditions

**CANNOT Modify:**
- Rust kernel code
- Sandbox enforcement
- Mode switching guards
- Credential storage

---

## Contributing Guidelines

### Pull Request Checklist

- [ ] Tests pass on your local platform (`cargo test --workspace`)
- [ ] No new `clippy` warnings (`cargo clippy --workspace`)
- [ ] Code formatted (`cargo fmt --all`)
- [ ] Documentation updated if behavior changed
- [ ] PR description explains *why*, not just *what*
- [ ] Platform impact noted (Linux only? All platforms?)
- [ ] CHANGELOG.md updated (if user-facing change — e.g., API changes, new features, behavior modifications)
- [ ] ADR added for significant architectural changes (if applicable — e.g., new public APIs, cross-platform behavior changes, new dependencies)

### Branch Naming

- Bug fixes: `fix/<short-description>`
- New features: `feat/<short-description>`
- Documentation: `docs/<short-description>`

### High-Priority Areas

- **Windows sandbox hardening** — AppContainer/Job Object coverage is weakest
- **New LLM provider implementations** — Gemini, Mistral, local GGUF models (llama.cpp-compatible quantized models)
- **Script bridge improvements** — Lua ↔ Rust FFI performance, Deno/V8 embedding stability
- **Platform-specific test coverage** — especially Windows CI edge cases
- **Documentation** — architecture explanations, guide corrections, translation

### Architecture Decisions

Major decisions are recorded as ADRs in `docs/adr/`. Before proposing significant changes:

1. Read existing ADRs
2. Open a discussion issue with the `adr` label
3. If consensus reached, open a PR adding a new ADR

ADR format: **Context → Decision → Consequences**

---

## Key Documentation References

| Document | Purpose |
|----------|---------|
| [Architecture Overview](docs/architecture/overview.md) | Full 5-layer architecture |
| [Crate Map](docs/architecture/crate-map.md) | Dependency graph of all crates |
| [Getting Started](docs/guides/getting-started.md) | Build your first agent |
| [Writing Tools](docs/guides/writing-tools.md) | Create custom tools with scripts |
| [Extension Capabilities](docs/guides/extension-capabilities.md) | Application extensibility guide |
| [Safe Mode](docs/guides/safe-mode.md) | Secure your agent |
| [Power Mode](docs/guides/power-mode.md) | Full system access guide |
| [Contributing](CONTRIBUTING.md) | Contribution guidelines |
| [Build Plan](BUILD_PLAN.md) | Detailed Chinese build roadmap |

---

## Important Notes for AI Coding Agents

1. **Project Stage:** This is a design-first project. The `crates/` directory is currently empty — no implementation has been started yet. Any implementation work should follow the architecture described in `docs/architecture/` and the build plan in `BUILD_PLAN.md`.

2. **Cross-Platform Critical:** Every change must consider Linux, macOS, and Windows. Platform-specific code belongs ONLY in `claw-pal`. Use conditional compilation with `#[cfg(target_os = "...")]`.

3. **Feature Flags:** Use feature flags liberally for optional functionality. The default build should have minimal dependencies. Lua is the only default script engine; V8 and Python are opt-in.

4. **Documentation:** Keep documentation in sync with code. Bilingual documentation (English/Chinese) is preferred for user-facing docs.

5. **Security First:** The Safe/Power mode distinction is fundamental. Never bypass sandbox restrictions in Safe Mode. Power Mode requires explicit user opt-in with a key.

6. **ADR Process:** For architectural changes, follow the ADR process documented in `docs/adr/README.md`. Create a discussion first, then submit an ADR PR if consensus is reached.

7. **Testing:** Every crate must have both unit tests and integration tests. Platform-specific code in `claw-pal` requires tests on all three platforms.

8. **Build Order:** Follow the build plan order:
   - Phase 1: claw-pal (Layer 0.5)
   - Phase 2: claw-runtime (Layer 1)
   - Phase 3: claw-provider + claw-tools (Layer 2, Part 1)
   - Phase 4: claw-loop (Layer 2, Part 2)
   - Phase 5: claw-script (Layer 3)
   - Phase 6: Examples and application patterns
   - Phase 7: Meta-crate claw-kernel

9. **Interface First:** Define traits before implementations. See BUILD_PLAN.md for the detailed trait specifications.

10. **No Assumptions:** This project is not a standard Rust project. Do not assume crates exist or that code is already written. Always check if files/directories exist before referencing them.
