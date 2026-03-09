# claw-kernel

> The shared Rust foundation for the Claw agent ecosystem — cross-platform, sandboxed, hot-loadable.

[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
[![Rust](https://img.shields.io/badge/rust-1.83%2B-orange.svg)](https://www.rust-lang.org)
[![Platform](https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows-lightgrey.svg)](docs/platform/)
[![Tests](https://img.shields.io/badge/tests-389+%20passing-brightgreen.svg)](#)
[![Version](https://img.shields.io/badge/version-0.1.0-blue.svg)](CHANGELOG.md)

---

Every project in the Claw ecosystem independently reimplements the same primitives: LLM provider HTTP calls, tool-use protocol, agent loop, memory system, channel integrations. **claw-kernel** extracts these into a single, well-tested, cross-platform Rust library.

It is a **shared infrastructure library**, not a standalone agent — think of it as the Linux kernel to your agent's userspace.

## Architecture

```
┌─────────────────────────────────────────────────────┐
│              Layer 3: Extension Foundation          │
│    Lua (default) · Deno/V8 · PyO3                   │
├─────────────────────────────────────────────────────┤
│              Layer 2: Agent Kernel Protocol         │
│    Provider · ToolRegistry · AgentLoop · Memory     │
│    Channel (Discord · HTTP Webhook · Stdin)         │
├─────────────────────────────────────────────────────┤
│              Layer 1: System Runtime                │
│    EventBus · AgentOrchestrator · IpcRouter         │
├═════════════════════════════════════════════════════╡
│              Layer 0.5: Platform Abstraction (PAL)  │
│    Sandbox · IPC (Unix socket / Named Pipe)         │
├─────────────────────────────────────────────────────┤
│              Layer 0: Rust Hard Core                │
│    Memory Safety · OS Abstraction · Trust Root      │
└─────────────────────────────────────────────────────┘
```

## Quick Start

```toml
[dependencies]
claw-kernel = { git = "https://github.com/claw-project/claw-kernel", features = ["engine-lua"] }
```

```rust
use std::sync::Arc;
use claw_kernel::prelude::*;
// 或明确使用:
// use claw_kernel::provider::AnthropicProvider;
// use claw_kernel::tools::ToolRegistry;
// use claw_kernel::loop_builder::AgentLoopBuilder;

#[tokio::main]
async fn main() {
    let agent = AgentLoopBuilder::new()
        .with_provider(Arc::new(AnthropicProvider::from_env().unwrap()))
        .with_tools(Arc::new(ToolRegistry::new()))
        .with_max_turns(10)
        .build()
        .unwrap();
    agent.run("Hello, world!").await.unwrap();
}
```

See [`examples/`](examples/) for `simple-agent`, `custom-tool`, and `self-evolving-agent`.

## Crates

| Crate | Layer | Description |
|-------|-------|-------------|
| [`claw-pal`](docs/crates/claw-pal.md) | 0.5 | Platform Abstraction: sandbox, IPC, process management |
| [`claw-runtime`](docs/crates/claw-runtime.md) | 1 | EventBus (broadcast 1024), AgentOrchestrator, IpcRouter |
| [`claw-provider`](docs/crates/claw-provider.md) | 2 | LLM providers: Anthropic, OpenAI, Ollama, DeepSeek, Moonshot |
| [`claw-tools`](docs/crates/claw-tools.md) | 2 | Tool registry, JSON Schema gen, hot-loading (50ms debounce) |
| [`claw-loop`](docs/crates/claw-loop.md) | 2 | Agent loop engine, history, stop conditions |
| [`claw-memory`](docs/crates/claw-memory.md) | 2 | Ngram embedder, SQLite store, SecureMemoryStore (50 MB) |
| [`claw-channel`](docs/crates/claw-channel.md) | 2 | Channel trait: Discord, HTTP Webhook, Stdin |
| [`claw-script`](docs/crates/claw-script.md) | 3 | Script engines: Lua (default), Deno/V8, PyO3 |
| `claw-kernel` | meta | Re-exports all of the above + prelude |

## Platform Support

| Platform | Sandbox | Isolation | IPC Support |
|----------|---------|-----------|-------------|
| Linux | seccomp-bpf + Namespaces | Strongest | ✅ Unix Domain Socket |
| macOS | sandbox(7) / Seatbelt | Medium | ✅ Unix Domain Socket |
| Windows | AppContainer + Job Objects | Medium | 🚧 Skeleton included, fully available in v0.2.0 |

> **Note:** Windows IPC foundation is included in v0.1.0 with basic skeleton structure. Full Named Pipe implementation will be available in v0.2.0 **(High Priority)**.
>
> **注意**: IPC 远程消息投递当前未实现，仅支持本地进程内通信。Windows IPC 计划在 v0.2.0 中实现。

Platform guides: [Linux](docs/platform/linux.md) · [macOS](docs/platform/macos.md) · [Windows](docs/platform/windows.md)

## Build

**Requirements:** Rust 1.83+, `libseccomp-dev` on Linux.

```bash
git clone https://github.com/claw-project/claw-kernel.git
cd claw-kernel
cargo build                          # default (Lua only)
cargo test --workspace               # 389+ tests
cargo clippy --workspace -- -D warnings
```

Optional features: `engine-v8` (Node.js ≥ 20), `engine-py` (Python ≥ 3.10, Rust 1.83+). See [CONTRIBUTING.md](CONTRIBUTING.md#feature-matrix) for the full feature matrix.

> All build profiles set `panic = "unwind"` — required by mlua. Already configured in `Cargo.toml`.

## Execution Modes

**Safe Mode (default):** filesystem allowlist, network rules, no subprocess spawning.

**Power Mode:** full system access, explicit opt-in with a Power Key (min 12 chars):
```bash
claw-kernel --power-mode --power-key <your-key>
```

Details: [Safe Mode](docs/guides/safe-mode.md) · [Power Mode](docs/guides/power-mode.md)

## Documentation

| | |
|--|--|
| [Architecture Overview](docs/architecture/overview.md) | 5-layer design, component relationships |
| [Crate Map](docs/architecture/crate-map.md) | Dependency graph |
| [Getting Started](docs/guides/getting-started.md) | Build your first agent |
| [Writing Tools](docs/guides/writing-tools.md) | Custom tools with Lua scripts |
| [ADRs](docs/adr/) | Architecture Decision Records (001–008) |
| [Changelog](CHANGELOG.md) | Version history |
| [Roadmap](ROADMAP.md) | Future milestones |
| [AGENTS.md](AGENTS.md) | Guide for AI coding agents |

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md). Priority areas: new LLM providers (Gemini, Mistral, GGUF) · Windows sandbox hardening · Deno/V8 bridge · integration test coverage.

- **Questions:** [GitHub Discussions](https://github.com/creeper-scr/claw-kernel/discussions)
- **Bugs:** [GitHub Issues](https://github.com/creeper-scr/claw-kernel/issues)
- **Security:** [SECURITY.md](SECURITY.md)

## License

[Apache-2.0](LICENSE-APACHE) OR [MIT](LICENSE-MIT) — your choice.
