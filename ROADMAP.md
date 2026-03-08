---
title: claw-kernel Roadmap
description: Milestone-based roadmap for claw-kernel
status: v0.1.0-released
version: "0.1.0"
last_updated: "2026-03-03"
language: bilingual
---

[English](#english) | [中文](#chinese)

<a name="english"></a>

# claw-kernel Roadmap

> **Strategy: Fast to v1.0.0 — Capture market share first, add features later.**

---

## Current Status

| Item | Status |
|------|--------|
| Architecture design | ✅ Complete |
| ADRs (001-011) | ✅ 001-010 accepted, 011 proposed |
| Core implementation (9 crates) | ✅ Complete |
| 670+ unit + integration tests | ✅ All passing |
| Clippy / fmt / doc checks | ✅ Clean (zero warnings) |
| Published on crates.io | ⬜ v1.0.0 target |

See [CHANGELOG.md](CHANGELOG.md) for what shipped in v0.1.0.

---

## Release Strategy

**Core Principle:** Ship v1.0.0 as fast as possible to establish ecosystem presence. Additional providers and features will be delivered through post-v1.0 minor releases.

**v1.0.0 = Minimal Stable Core:**
- Existing 5 providers are sufficient for launch
- Focus on documentation, examples, and API stability
- Sandbox improvements can be incremental

---

## Completed — v0.1.0 (2026-03-01)

### claw-pal (Platform Abstraction Layer)
- [x] Unix Domain Socket IPC with 4-byte Big Endian frame protocol
- [x] `ProcessManager` (DashMap + SIGTERM → SIGKILL escalation)
- [x] Platform config directories (`dirs`)
- [x] Security key validation (`argon2`)
- [x] Linux and macOS sandbox groundwork
- [x] Windows IPC skeleton (Named Pipe foundation)

### claw-runtime (System Runtime)
- [x] `EventBus` (Tokio broadcast, capacity 1024)
- [x] `AgentOrchestrator` (DashMap + AtomicU64 AgentId)
- [x] `IpcRouter` for inter-agent message routing
- [x] `Runtime` unified async entry point

### claw-provider (LLM Providers)
- [x] Three-layer `LLMProvider` trait
- [x] `DefaultHttpTransport` (reqwest + rustls)
- [x] Anthropic (Claude) implementation
- [x] OpenAI-compatible implementation
- [x] Ollama (local) implementation
- [x] DeepSeek implementation
- [x] Moonshot implementation

### claw-tools (Tool Registry)
- [x] `ToolRegistry` (DashMap, timeout, audit log)
- [x] `HotLoader` (notify 6.1.1, 50 ms debounce)
- [x] JSON Schema generation via `schemars`

### claw-loop (Agent Loop)
- [x] `AgentLoop` + `AgentLoopBuilder`
- [x] `InMemoryHistory` (ring-buffer + overflow callback)
- [x] Stop conditions: `MaxTurns`, `TokenBudget`, `NoToolCall`
- [x] `HistoryManager` trait

### claw-memory (Memory System)
- [x] `NgramEmbedder` (64-dim bigram + trigram)
- [x] `SqliteMemoryStore` (cosine similarity, in-process)
- [x] `SecureMemoryStore` (50 MB quota)
- [x] Async memory worker (mpsc, capacity 256)

### claw-channel (Channel Integrations)
- [x] `Channel` trait + `ChannelMessage` protocol
- [x] Discord adapter (Twilight)
- [x] HTTP Webhook adapter (Axum)
- [x] Stdin adapter (testing / CLI)

### claw-script (Script Engine)
- [x] `ScriptEngine` trait + `EngineType` enum
- [x] `LuaEngine` (mlua 0.9.4, Lua 5.4, spawn_blocking)
- [x] `ToolsBridge` (tool registry → Lua)

### claw-kernel (Meta-Crate)
- [x] Re-exports all sub-crates
- [x] `prelude` module

---

## Milestones

### v0.2.0 → v0.5.0 — Stabilization Sprint

**Target:** 2026 Q2 (8–10 weeks)

**Goal:** Prepare for v1.0.0 — stabilize API, fill documentation gaps, ensure cross-platform reliability.

- [ ] **Documentation**
  - [ ] Full rustdoc API documentation with doctests
  - [ ] Architecture guide for contributors
  - [ ] Quick-start guide for end users
  - [ ] Migration guide template (for future breaking changes)
  
- [ ] **Examples** (runnable, tested in CI)
  - [x] `examples/simple-agent` — basic agent with tools
  - [x] `examples/custom-tool` — writing Lua tools
  - [x] `examples/memory-agent` — using SqliteMemoryStore with overflow callback
  - `examples/self-evolving-agent` — **intentionally not implemented here**; self-evolution is the showcase of the evoclaw application, not a kernel concern. The kernel provides the infrastructure (AgentBridge, HotLoader, LuaEngine); evoclaw owns the demo.

- [x] **Script Bridges** — all 4 shipped ahead of schedule in v0.1.0 (see [ADR-009](docs/adr/009-bridge-roadmap.md))
  - [x] `DirsBridge` — platform config/data/cache/tools paths
  - [x] `MemoryBridge` — key-value + semantic search for scripts
  - [x] `EventsBridge` — emit / subscribe to EventBus from Lua
  - [x] `AgentBridge` — spawn and manage sub-agents from Lua (was P3/v0.3.0)
  
- [ ] **API Hardening**
  - [ ] Audit all public APIs for semver compliance
  - [ ] Hide internal implementation details
  - [ ] Finalize error type design
  
- [ ] **Testing**
  - [ ] Cross-platform CI (Linux + macOS + Windows)
  - [ ] Integration test coverage for all providers
  - [ ] Sandbox integration tests (Linux)

---

### v1.0.0 — Stable Release

**Target:** 2026 Q2 (immediately after stabilization)

**Goal:** Establish stable API baseline and ecosystem presence.

- [ ] **crates.io Publication**
  - [ ] All 9 crates published with stable version
  - [ ] README, LICENSE, metadata complete
  - [ ] `claw-kernel` meta-crate ready for `cargo install`
  
- [ ] **API Stability Guarantee**
  - [ ] Semver policy documented
  - [ ] Public API surface locked
  - [ ] Deprecation policy established
  
- [ ] **Production Readiness**
  - [ ] Security audit passed
  - [ ] Performance benchmarks published
  - [ ] Known issues documented

**What v1.0.0 DOES include:**
- 5 LLM providers (Anthropic, OpenAI, Ollama, DeepSeek, Moonshot)
- Lua scripting engine with all 7 bridges: `fs`, `net`, `tools`, `dirs`, `memory`, `events`, `agent`
- In-memory history (SQLite history deferred to v1.1)
- Basic sandbox (Linux seccomp stub, macOS Seatbelt stub, Windows skeleton)
- Hot-loading tools
- `claw-memory` as an optional reference implementation for persistent storage

**What v1.0.0 DOES NOT include (by design, per [ADR-010](docs/adr/010-memory-system-boundary.md)):**
- Mid/long-term memory built into `AgentLoop` — the kernel manages only the context window (`HistoryManager`). Applications wire mid/long-term storage via `overflow_callback`.
- Multi-language SDK / KernelServer — deferred to v1.4.0.

---

## Post-v1.0 Roadmap

**Strategy:** Rapid minor releases adding providers and features. No breaking changes.

### v1.1.0 — SQLite History & Streaming

**Target:** 2026 Q3

- [ ] SQLite history backend for `claw-loop` (`sqlite-history` feature)
- [ ] Streaming response support across all providers
- [ ] Performance benchmarks (provider latency, tool throughput)

### v1.2.0 — Additional Providers

**Target:** 2026 Q3

- [ ] Gemini (Google) provider
- [ ] Mistral provider
- [ ] Azure OpenAI provider

### v1.3.0 — Enhanced Scripting

**Target:** 2026 Q4

- [ ] Deno/V8 engine (`engine-v8` feature)
- [ ] Python engine (`engine-py` feature)
- [x] `AgentBridge` — shipped ahead of schedule in v0.1.0 (see ADR-009)
- [x] Full `RustBridge` API: `llm`, `tools`, `memory`, `events`, `fs`, `net`, `agent`, `dirs` — all bridges shipped in v0.1.0

### v1.4.0 — KernelServer: Multi-Language IPC Daemon

**Target:** 2027 Q1

> See [ADR-011](docs/adr/011-multi-language-ipc-daemon.md) for full protocol spec and rationale.

- [ ] New crate `claw-server`: `KernelServer` over Unix Domain Socket / Named Pipe
- [ ] JSON-RPC 2.0 protocol: `create_session`, `send_message`, `tool_result`, `destroy_session`
- [ ] Server → client events: `chunk`, `tool_call`, `finish`
- [ ] `claw-kernel-server` binary (CLI: `--socket-path`, `--provider`, `--power-key`)
- [ ] Python SDK (`claw-sdk-python`, ~100 lines)
- [ ] TypeScript/Node SDK (`claw-sdk-ts`, ~100 lines)
- [ ] Go SDK (`claw-sdk-go`, ~100 lines)

**Design principle:** IPC overhead (≈ 0.001% of total response time) is negligible vs LLM latency. The Rust performance advantage is preserved — all computation stays in the kernel process. Client language is irrelevant to throughput.

### v1.5.0 — Sandbox Hardening

**Target:** 2027 Q1

- [ ] Linux: full seccomp-bpf syscall allowlist
- [ ] macOS: complete Seatbelt profile
- [ ] Windows: AppContainer + Job Objects

### v1.6.0 — Local Models

**Target:** 2027 Q2

- [ ] Local GGUF model support via `llama-cpp-rs` (optional feature)

### v1.7.0+ — Channel Expansion

**Target:** 2027 Q2+

- [ ] Telegram integration
- [ ] Slack integration
- [ ] WebSocket bidirectional channel

---

## Contributing Priorities

Current priority areas to reach v1.0.0:

1. **Documentation** — examples, guides, rustdoc
2. **Cross-platform CI** — Windows testing, macOS CI
3. **API audit** — ensuring semver compliance
4. **Provider testing** — integration tests for all 5 providers

**Deferred (post-v1.0):**
- New providers (Gemini, Mistral, Azure)
- Additional script engines (Deno, Python)
- GGUF local models
- Advanced sandbox features

---

## Design Decisions

Key architectural choices are recorded as ADRs in [docs/adr/](docs/adr/):

| ADR | Decision |
|-----|----------|
| 001 | 5-layer architecture |
| 002 | Script engine selection (Lua default, V8/Python optional) |
| 003 | Security model (Safe/Power dual-mode) |
| 004 | Hot-loading mechanism |
| 005 | IPC and multi-agent protocol |
| 006–008 | Message format, EventBus, file watcher |
| 009 | claw-script bridge roadmap (dirs → memory → events → agent) |
| 010 | Memory boundary: kernel = HistoryManager only; claw-memory = optional |
| 011 | Multi-language support via KernelServer IPC daemon |

---

## Contributing

Want to help? Check [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

**Current priority areas:**
- New LLM providers: Gemini, Mistral, local GGUF
- **Windows Named Pipe IPC support (High Priority)**
- Windows sandbox implementation
- Deno/V8 engine bridge
- `claw-server` KernelServer implementation (see ADR-011)
- Expanded integration test coverage

---

<a name="chinese"></a>

# claw-kernel 路线图

> **策略：快速推进至 v1.0.0 —— 先抢占生态位，后完善功能。**

---

## 当前状态

| 项目 | 状态 |
|------|------|
| 架构设计 | ✅ 已完成 |
| ADR（001-011） | ✅ 001-010 已接受，011 待议 |
| 核心实现（9 个 crate） | ✅ 已完成 |
| 670+ 个单元+集成测试 | ✅ 全部通过 |
| Clippy / fmt / 文档检查 | ✅ 干净 |
| 发布到 crates.io | ⬜ v1.0.0 目标 |

v0.1.0 详细发布内容见 [CHANGELOG.md](CHANGELOG.md)。

---

## 发布策略

**核心原则：** 以最快速度发布 v1.0.0，建立生态系统影响力。额外的 Provider 和功能将通过 v1.0 之后的次要版本迭代交付。

**v1.0.0 = 最小稳定核心：**
- 现有的 5 个 Provider 足以启动
- 重点是文档、示例和 API 稳定性
- 沙箱改进可以渐进式进行

---

## 已完成 — v0.1.0（2026-03-01）

### 核心亮点

- [x] **claw-pal**：Unix Domain Socket IPC + Windows Named Pipe 骨架 + 进程管理 + 安全密钥验证
- [x] **claw-runtime**：EventBus + AgentOrchestrator + IpcRouter + Runtime
- [x] **claw-provider**：5 个 LLM Provider（Anthropic/OpenAI/Ollama/DeepSeek/Moonshot）
- [x] **claw-tools**：工具注册表 + 热加载 + JSON Schema 生成
- [x] **claw-loop**：环形历史 + 三种停止条件 + Builder
- [x] **claw-memory**：Ngram 嵌入 + SQLite 检索 + 安全配额存储
- [x] **claw-channel**：Discord / Webhook / Stdin 三种适配器
- [x] **claw-script**：Lua 引擎 + ToolsBridge
- [x] **claw-kernel**：元 crate + prelude

---

## 里程碑

### v0.2.0 → v0.5.0 — 稳定化冲刺

**目标时间：** 2026 Q2（8–10 周）

**目标：** 为 v1.0.0 做准备 —— 稳定 API、完善文档、确保跨平台可靠性。

- [ ] **文档**
  - [ ] 完整 rustdoc API 文档（含 doctests）
  - [ ] 贡献者架构指南
  - [ ] 终端用户快速入门指南
  - [ ] 破坏性变更迁移指南模板
  
- [ ] **示例**（可运行，CI 测试）
  - [x] `examples/simple-agent` —— 带工具的基础 Agent
  - [x] `examples/custom-tool` —— 编写 Lua 工具
  - [x] `examples/memory-agent` —— 使用 SqliteMemoryStore + overflow_callback
  - `examples/self-evolving-agent` —— **有意不在此实现**；自进化是 evoclaw 应用层的核心卖点，不是内核职责。内核提供基础设施（AgentBridge、HotLoader、LuaEngine），evoclaw 负责展示。

- [x] **脚本 Bridge** —— 全部 4 个在 v0.1.0 提前完成（见 [ADR-009](docs/adr/009-bridge-roadmap.md)）
  - [x] `DirsBridge` —— 平台配置/数据/缓存/工具目录路径
  - [x] `MemoryBridge` —— 脚本内键值存储 + 语义搜索
  - [x] `EventsBridge` —— 从 Lua 发送/订阅 EventBus 事件
  - [x] `AgentBridge` —— 从 Lua 创建和管理子 Agent（原 P3/v0.3.0，提前完成）
  
- [ ] **API 加固**
  - [ ] 审计所有公共 API 的 semver 合规性
  - [ ] 隐藏内部实现细节
  - [ ] 错误类型设计定稿
  
- [ ] **测试**
  - [ ] 跨平台 CI（Linux + macOS + Windows）
  - [ ] 所有 Provider 的集成测试覆盖
  - [ ] 沙箱集成测试（Linux）

---

### v1.0.0 — 稳定版发布

**目标时间：** 2026 Q2（稳定化之后立即发布）

**目标：** 建立稳定 API 基线和生态系统影响力。

- [ ] **crates.io 发布**
  - [ ] 全部 9 个 crate 以稳定版本发布
  - [ ] README、LICENSE、元数据完整
  - [ ] `claw-kernel` 元 crate 支持 `cargo install`
  
- [ ] **API 稳定性保证**
  - [ ] Semver 策略文档化
  - [ ] 公共 API 表面锁定
  - [ ] 弃用策略确立
  
- [ ] **生产就绪**
  - [ ] 通过安全审计
  - [ ] 发布性能基准测试
  - [ ] 已知问题文档化

**v1.0.0 包含内容：**
- 5 个 LLM Provider（Anthropic、OpenAI、Ollama、DeepSeek、Moonshot）
- Lua 脚本引擎，含全部 7 个 Bridge：`fs`、`net`、`tools`、`dirs`、`memory`、`events`、`agent`
- 内存历史（SQLite 历史推迟到 v1.1）
- 基础沙箱（Linux seccomp stub、macOS Seatbelt stub、Windows skeleton）
- 热加载工具
- `claw-memory` 作为持久化存储的可选参考实现

**v1.0.0 设计上不包含（见 [ADR-010](docs/adr/010-memory-system-boundary.md)）：**
- 内置在 `AgentLoop` 中的中/长期记忆 —— 内核只管理上下文窗口（`HistoryManager`），应用通过 `overflow_callback` 接管中/长期存储。
- 多语言 SDK / KernelServer —— 推迟到 v1.4.0。

---

## v1.0 之后路线图

**策略：** 快速次要版本发布，添加 Provider 和功能。无破坏性变更。

### v1.1.0 — SQLite 历史 & 流式响应

**目标时间：** 2026 Q3

- [ ] `claw-loop` SQLite 历史后端（`sqlite-history` feature）
- [ ] 所有 Provider 的流式响应支持
- [ ] 性能基准测试（Provider 延迟、工具吞吐量）

### v1.2.0 — 额外 Provider

**目标时间：** 2026 Q3

- [ ] Gemini（Google）Provider
- [ ] Mistral Provider
- [ ] Azure OpenAI Provider

### v1.3.0 — 增强脚本

**目标时间：** 2026 Q4

- [ ] Deno/V8 引擎（`engine-v8` feature）
- [ ] Python 引擎（`engine-py` feature）
- [x] `AgentBridge` —— v0.1.0 已提前完成（见 ADR-009）
- [x] 完整 `RustBridge` API：全部 7 个 Bridge 已在 v0.1.0 完成

### v1.4.0 — KernelServer：多语言 IPC 守护进程

**目标时间：** 2027 Q1

> 完整协议规范与设计理由见 [ADR-011](docs/adr/011-multi-language-ipc-daemon.md)。

- [ ] 新 crate `claw-server`：`KernelServer`，通过 Unix Domain Socket / Named Pipe 暴露完整 AgentLoop
- [ ] JSON-RPC 2.0 协议：`create_session`、`send_message`、`tool_result`、`destroy_session`
- [ ] 服务器推送事件：`chunk`、`tool_call`、`finish`
- [ ] `claw-kernel-server` 二进制（CLI：`--socket-path`、`--provider`、`--power-key`）
- [ ] Python SDK（`claw-sdk-python`，约 100 行）
- [ ] TypeScript/Node SDK（`claw-sdk-ts`，约 100 行）
- [ ] Go SDK（`claw-sdk-go`，约 100 行）

**设计原则：** IPC 开销（≈ 总响应时间的 0.001%）相对 LLM 延迟可忽略不计。Rust 性能优势完整保留 —— 所有计算仍在内核进程中完成，客户端语言与吞吐量无关。

### v1.5.0 — 沙箱加固

**目标时间：** 2027 Q1

- [ ] Linux：完整 seccomp-bpf 系统调用白名单
- [ ] macOS：完整 Seatbelt profile
- [ ] Windows：AppContainer + Job Objects

### v1.6.0 — 本地模型

**目标时间：** 2027 Q2

- [ ] 通过 `llama-cpp-rs` 支持本地 GGUF 模型（可选 feature）

### v1.7.0+ — 渠道扩展

**目标时间：** 2027 Q2+

- [ ] Telegram 集成
- [ ] Slack 集成
- [ ] WebSocket 双向渠道

---

## 贡献优先领域

达到 v1.0.0 的当前优先领域：

1. **文档** —— 示例、指南、rustdoc
2. **脚本 Bridge** —— DirsBridge（P0）、MemoryBridge（P1）、EventsBridge（P2）
3. **跨平台 CI** —— Windows 测试、macOS CI
4. **API 审计** —— 确保 semver 合规性
5. **Provider 测试** —— 全部 5 个 Provider 的集成测试

**推迟（v1.0 之后）：**
- 新 Provider（Gemini、Mistral、Azure）
- 额外脚本引擎（Deno、Python）+ AgentBridge（v1.3.0）
- KernelServer 多语言 IPC 守护进程（v1.4.0，见 ADR-011）
- GGUF 本地模型
- 高级沙箱功能
