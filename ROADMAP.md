---
title: claw-kernel Roadmap
description: Milestone-based roadmap for claw-kernel
status: v0.4.0-released
version: "0.4.0"
last_updated: "2026-03-10"
language: bilingual
---

[English](#english) | [中文](#chinese)

<a name="english"></a>

# claw-kernel Roadmap

> ⚠️ **Pre-release notice:** v0.4.0 is a beta and may be unstable. APIs are subject to change without notice.
> **Strategy: Fast to v1.0.0 — Capture market share first, add features later.**

---

## Current Status

| Item | Status |
|------|--------|
| Architecture design | ✅ Complete |
| ADRs (001-014) | ✅ 001-013 accepted, 014 proposed |
| Core implementation (13 crates) | ✅ Complete |
| 129+ unit + integration tests (claw-runtime) | ✅ All passing |
| Clippy / fmt / doc checks | ✅ Clean (zero warnings) |
|| Current release | ✅ v0.4.0 |
| Kernel Features (F1-F9) | 📚 See [kernel-features.md](docs/kernel-features.md) |

See [CHANGELOG.md](CHANGELOG.md) for full version history.

---

## Kernel Features Coverage (v0.4.0)

**Reference:** [docs/kernel-features.md](docs/kernel-features.md) — comprehensive specification of internal vs. application-layer boundaries.

| Feature | Description | Status | Version |
|---------|-------------|--------|---------|
| **F1** | Message Channel Abstraction | ✅ Complete | v1.0.0+ |
| **F2** | Conversation Context Management | ✅ Complete | v1.0.0+ |
| **F3** | LLM Provider Abstraction | ✅ Complete | v1.2.0+ |
| **F4** | Tool Execution Runtime | ✅ Complete (G-1 audit fix) | v1.5.0-dev |
| **F5** | Skill On-Demand Loading Engine | ✅ Complete | v1.4.0+ |
| **F6** | Event Trigger System (Cron + Webhook) | ✅ Complete | v1.4.0+ |
| **F7** | Multi-Agent Orchestration | ✅ Complete (G-10 restart fix) | v1.4.1+ |
| **F8** | Security & Isolation Model | ✅ Complete | v1.0.0+ |
| **F9** | Script Extension Foundation | ✅ Complete (Lua + V8) | v1.3.0+ |

**Legend:**
- ✅ = Fully implemented and tested
- 🔧 = In-progress (see v1.5.0 sprint plan below)
- ⬜ = Deferred to future release

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
- [x] Windows Named Pipe IPC (full implementation in claw-pal/src/ipc/)

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
- [x] All 7 Lua bridges: `tools`, `fs`, `net`, `llm`, `agent`, `events`, `dirs`
- [x] `V8Engine` (deno_core, ES2022+, TypeScript) — `engine-v8` feature
- [x] All V8 bridges: `tools`, `fs`, `net`, `llm`, `agent`, `events`, `dirs`
- [x] Hot-reload support for scripts

### claw-server (KernelServer / IPC Daemon)
- [x] JSON-RPC 2.0 server over Unix Domain Socket / Named Pipe
- [x] Session lifecycle: `create_session`, `send_message`, `tool_result`, `destroy_session`
- [x] Server-push streaming: `chunk`, `tool_call`, `finish` events
- [x] Tool bridge: exposes host ToolRegistry to IPC sessions

### claw-kernel (Meta-Crate)
- [x] Re-exports all sub-crates
- [x] `prelude` module

---

## Milestones

### v0.2.0 → v0.5.0 — Stabilization Sprint

**Target:** 2026 Q2 (8–10 weeks)

**Goal:** Prepare for v1.0.0 — stabilize API, fill documentation gaps, ensure cross-platform reliability.

- [x] **Documentation**
  - [x] Full rustdoc API documentation with doctests
  - [ ] Architecture guide for contributors
  - [ ] Quick-start guide for end users
  - [ ] Migration guide template (for future breaking changes)
  
- [ ] **Examples** (runnable, tested in CI)
  - [x] `examples/simple-agent` — basic agent with tools
  - [x] `examples/custom-tool` — writing Lua tools
  - [x] `examples/memory-agent` — using SqliteMemoryStore with overflow callback
  - ~~`examples/self-evolving-agent`~~ — **intentionally not implemented here**; self-evolution is the showcase of the evoclaw application, not a kernel concern. The kernel provides the infrastructure (AgentBridge, HotLoader, LuaEngine); evoclaw owns the demo.

- [x] **Script Bridges** — all 4 shipped ahead of schedule in v0.1.0 (see [ADR-009](docs/adr/009-bridge-roadmap.md))
  - [x] `DirsBridge` — platform config/data/cache/tools paths
  - [x] `LlmBridge` — LLM completions and streaming for scripts
  - [x] `EventsBridge` — emit / subscribe to EventBus from Lua
  - [x] `AgentBridge` — spawn and manage sub-agents from Lua (was P3/v0.3.0)
  
- [x] **API Hardening**
  - [x] Audit all public APIs for semver compliance
  - [ ] Hide internal implementation details
  - [ ] Finalize error type design
  
- [x] **Testing**
  - [x] Cross-platform CI (Linux + macOS + Windows)
  - [ ] Integration test coverage for all providers
  - [ ] Sandbox integration tests (Linux)

---

### v1.0.0 — Stable Release ✅

**Released**: 2026-03-08

**Target:** 2026 Q2 (immediately after stabilization)

**Goal:** Establish stable API baseline and ecosystem presence.

- [ ] **crates.io Publication**
  - [ ] All 9 crates published with stable version
  - [ ] README, LICENSE, metadata complete
  - [ ] `claw-kernel` meta-crate ready for `cargo install`
  
- [x] **API Stability Guarantee**
  - [x] Semver policy documented
  - [ ] Public API surface locked
  - [ ] Deprecation policy established
  
- [x] **Production Readiness**
  - [x] Security audit passed
  - [ ] Performance benchmarks published
  - [x] Known issues documented

**What v1.0.0 DOES include:**
- 5 LLM providers (Anthropic, OpenAI, Ollama, DeepSeek, Moonshot)
- Lua scripting engine with all 7 bridges: `fs`, `net`, `tools`, `dirs`, `llm`, `events`, `agent`
- In-memory history (SQLite history deferred to v1.1)
- Basic sandbox (Linux seccomp stub, macOS Seatbelt stub, Windows skeleton)
- Hot-loading tools
- `claw-memory` as an optional reference implementation for persistent storage

**What v1.0.0 DOES NOT include (by design, per [ADR-010](docs/adr/010-memory-system-boundary.md)):**
- Mid/long-term memory built into `AgentLoop` — the kernel manages only the context window (`HistoryManager`). Applications wire mid/long-term storage via `overflow_callback`.

---

### v1.0.0 — Multi-Language Foundation Layer (KernelServer) ✅

**Released**: 2026-03-08

**Rationale:** Per [ADR-011](docs/adr/011-multi-language-ipc-daemon.md), the kernel cannot exist as an isolated Rust library — it requires multi-language support to become a shared ecosystem asset. Non-Rust applications must be able to leverage the kernel's unified AI infrastructure without reimplementation.

**KernelServer (claw-server crate):**
- [x] Unix Domain Socket / Named Pipe transport with claw-pal framing
- [x] JSON-RPC 2.0 protocol: `create_session`, `send_message`, `tool_result`, `destroy_session`
- [x] Server → client streaming: `chunk`, `tool_call`, `finish` events
- [x] Session lifecycle management (parallel sessions, isolation)
- [x] Integration with existing AgentLoop, Provider, ToolRegistry (no core changes)

**claw-kernel-server binary:**
- [x] CLI: `--socket-path`, `--provider`, `--model`, `--power-key`, `--max-sessions`
- [x] Graceful shutdown on SIGTERM
- [x] Process lifecycle management (systemd / launchd compatible)

**Design:** The kernel daemon is infrastructure, not a feature. It enables the entire ecosystem (OpenClaw, ZeroClaw, PicoClaw) to unify on one Rust core regardless of implementation language. IPC overhead (~0.001% of LLM latency) is negligible.

---

## Post-v1.0 Roadmap — Completed

### v1.1.0 — Bug Fixes & Security Hardening ✅

**Released**: 2026-03-10

- [x] WebhookChannel logs error when no HMAC secret configured (Fix-A)
- [x] SecureMemoryStore quota check atomized — mutex held across `namespace_usage()` + `store()` (Fix-B)
- [x] WindowsSandbox Safe mode logs security warning stub (Fix-C)
- [x] `Runtime::new()` auto-starts orchestrator + IPC; `start()` deprecated (Fix-D)
- [x] `FsBridge::glob_files()` + Lua `glob` method (Fix-E)
- [x] NetBridge response body cap 4 MiB; `block_on` pattern (Fix-F)
- [x] `bridge/conversion.rs` extracted from tools.rs (Fix-G)

### v1.2.0 — Provider Expansion ✅

**Released**: 2026-03-10

- [x] Additional LLM providers groundwork merged into existing provider infrastructure

### v1.3.0 — IPC Auth, ChannelRegistry & AgentOrchestrator Wiring ✅

**Released**: 2026-03-10

- [x] IPC token auth — daemon generates token via `DefaultHasher+SystemTime+PID`; writes to `~/.local/share/claw-kernel/kernel.token`; first frame must be `kernel.auth`
- [x] ChannelRegistry — DashMap-backed, 60s TTL dedup cache; register/unregister/list wired to handler
- [x] AgentOrchestrator injection — `KernelServer` holds `Arc<AgentOrchestrator>`; agent.spawn/kill/steer/list use real orchestrator API
- [x] Python SDK — `examples/sdk/python/` (stdlib only, 4-byte BE framing)
- [x] `handle_kernel_info()` returns `protocol_version: 2`
- [x] V8 engine (deno_core, ES2022+, TypeScript) — `engine-v8` feature fully wired

### v1.4.0 — GlobalToolRegistry, TriggerStore, AxumWebhookServer, TypeScript SDK ✅

**Released**: 2026-03-10

- [x] `GlobalToolRegistry` + `GlobalSkillRegistry` — cross-session tool/skill sharing
- [x] Schedule callback fix — corrected timer-based trigger dispatch
- [x] TypeScript SDK — `examples/sdk/typescript/`
- [x] `TriggerStore` — SQLite persistence for scheduled and webhook triggers
- [x] `AxumWebhookServer` integration — routes wired into KernelServer
- [x] `ChannelRouter` IPC dynamic routing — inbound channel messages dispatched over IPC

### v1.4.1 — Agent Health Check & RestartPolicy (G-10 Fix) ✅

**Released**: 2026-03-10

- [x] `IpcAgentHandle.shared_tx: SharedSender` (`Arc<Mutex<Option<Sender>>>`) for hot-swap on restart
- [x] `AgentState.ipc_tx: Option<SharedSender>`; `spawn_ipc_message_loop()` sets Error + triggers restart on task exit
- [x] `trigger_restart()` — checks RestartState, sleeps backoff, hot-swaps SharedSender, re-spawns loop
- [x] `start_health_check_task()` — timeout detection gated on `process_handle.is_some()`; bogus heartbeat auto-refresh removed
- [x] `start_auto_restart_task()` scoped to agents WITHOUT per-agent RestartState; Starting status as double-restart guard
- [x] 4 new tests: `test_spawn_agent_ipc_tx_stored`, `test_health_check_heartbeat_timeout_marks_error`, `test_trigger_restart_never_policy_publishes_agent_failed`, `test_trigger_restart_hot_swaps_sender`

---

## Completed — v1.5.0 ✅

**Target:** 2026 Q2 (2026-03-24 estimated, 5-week sprint)

**Goal:** Close the most critical gaps identified in `docs/v1.5-gap-report.md`.

**Kernel Features Enhanced:**
- **F4** (Tool Execution Runtime) — Audit logging with HMAC-signed events (G-1)
- **F1** (Message Channels) — Retry logic and deduplication refinements
- **F7** (Multi-Agent Orchestration) — Process isolation and restart policies (G-10)

### Gap Summary

| Gap | Priority | Description | Status |
|-----|----------|-------------|--------|
| GAP-01 | P0 | `claw.llm` bridge missing — scripts cannot call LLM | ✅ Fixed (`LlmBridge` in `claw-script`; Lua + V8 bridges fully implemented) |
| GAP-02 | P1 | `ChannelRouter.broadcast()` not implemented | ✅ Fixed (`broadcast_route()` in `router.rs`) |
| GAP-03 | P1 | Channel `send()` has no exponential backoff retry | ✅ Fixed (`RetryableChannel` in `retry.rs`) |
| GAP-04 | P2 | `UnifiedMessage` / `ChannelMessage` missing top-level `sender_id` / `thread_id` | ✅ Fixed (top-level fields in `types.rs`) |
| GAP-05 | P2 | Inbound → EventBus pipeline not closed | ✅ Fixed (`handler.rs:2417` — `event_bus.publish()`) |
| GAP-06 | P2 | `AgentHandle` has no `resource_usage` field | ✅ Fixed (G-6: `ResourceSnapshot` + `resource_monitor_task`) |
| GAP-07 | P2 | Task agent panics may propagate (needs `catch_unwind`) | ✅ Fixed (GAP-07: nested `tokio::spawn` panic isolation in `orchestrator.rs`) |
| GAP-08 | P3 | Webhook URL format non-standard | ⬜ Deferred to v1.5.1 |

### Sprint Plan

**Sprint 1 (2 weeks, 2026-03-10 → 2026-03-24):** Stability & Script LLM Access

- [x] **GAP-07** — ✅ Panic isolation via nested `tokio::spawn` in `spawn_ipc_message_loop()`
- [ ] **GAP-08** — Normalize webhook URL format; add validation and structured path helper
- [x] **GAP-01** — ✅ Implemented: `claw.llm` Lua bridge + V8 bridge fully implemented in `crates/claw-script/src/bridge/llm.rs`; `complete()` and `stream()` methods available in both Lua and V8 engines
- [x] **GAP-05** — ✅ Fixed: `handler.rs:2417` — `event_bus.publish()` closes inbound → EventBus pipeline

**Sprint 2 (3 weeks, 2026-03-24 → 2026-04-14):** Channel Layer Hardening

- [x] **GAP-03** — ✅ Fixed: `RetryableChannel` wrapper with exponential backoff (`retry.rs`); supports 14 tests
- [x] **GAP-04** — ✅ Fixed: `sender_id: Option<String>` and `thread_id: Option<String>` promoted to top-level `ChannelMessage` fields
- [x] **GAP-02** — ✅ Fixed: `ChannelRouter::broadcast_route(msg)` — fan-out to all matching agents (deduplicated)
- [x] **GAP-06** — ✅ Fixed (G-6): `resource_snapshot` per `AgentState`; `start_resource_monitor_task()` samples via `sysinfo` every 5s

**Remaining for Sprint 1:**
- [x] **GAP-01** — ✅ `claw.llm` Lua + V8 bridge (implemented — `LlmBridge` with `complete()` + `stream()`)
- [ ] **GAP-08** — Webhook URL normalization (P3 — deferred)

---

## Future Roadmap

**Strategy:** Rapid minor releases adding providers and features. Semver-controlled breaking changes.

### v1.6.0 — Channel Layer Enhancement & More LLM Providers

**Target:** 2026 Q3

- [ ] Telegram channel integration
- [ ] Slack channel integration (including Thread support)
- [x] ~~WebSocket bidirectional channel~~ — ✅ Implemented in `claw-channels` (`WebSocketChannel`, multi-client fan-out)
- [x] ~~Gemini (Google) provider~~ — ✅ Implemented in `claw-provider` (`gemini` feature)
- [x] ~~Mistral provider~~ — ✅ Implemented in `claw-provider` (`mistral` feature)
- [x] ~~Azure OpenAI provider~~ — ✅ Implemented in `claw-provider` (`azure-openai` feature)
- [x] ~~Streaming response support across all providers~~ — ✅ Gemini/Mistral/Azure OpenAI inherit `complete_stream()` via `OpenAIProvider` (all three are OpenAI-compatible aliases)

### v1.7.0 — Sandbox Hardening

**Target:** 2026 Q4

- [ ] Linux: full seccomp-bpf syscall allowlist
- [ ] macOS: complete Seatbelt profile
- [ ] Windows: AppContainer + Job Objects

### v1.8.0 — Local Models & Advanced Memory

**Target:** 2027 Q1

- [ ] Local GGUF model support via `llama-cpp-rs` (optional feature)
- [x] ~~SQLite history backend for `claw-loop` (`sqlite-history` feature)~~ — ✅ Implemented (`SqliteHistory` + `SqliteHistoryStore`)
- [ ] Performance benchmarks (provider latency, tool throughput)

1. **Documentation** — examples, guides, rustdoc
2. **Cross-platform CI** — Windows testing, macOS CI
3. **API audit** — ensuring semver compliance
4. **Provider testing** — integration tests for all 5 providers

**Deferred (post-v1.0):**
- New providers (Gemini, Mistral, Azure)
- GGUF local models
- Advanced sandbox features

---

## Design Decisions

Key architectural choices are recorded as ADRs in [docs/adr/](docs/adr/):

| ADR | Decision |
|-----|----------|
| 001 | 5-layer architecture |
| 002 | Script engine selection (Lua default✅, V8✅) |
| 003 | Security model (Safe/Power dual-mode) |
| 004 | Hot-loading mechanism |
| 005 | IPC and multi-agent protocol |
| 006–008 | Message format, EventBus, file watcher |
| 009 | claw-script bridge roadmap (dirs → memory → events → agent) |
| 010 | Memory boundary: kernel = HistoryManager only; claw-memory = optional |
| 011 | Multi-language support via KernelServer IPC daemon |
| 012 | V8 engine implementation (deno_core) |
| 013 | Runtime v1.0 evolution from ADR-005 IPC design |
| 014 | Channel Message Protocol v2 — promote sender_id and thread_id to top-level fields (Proposed) |

---

## Contributing

Want to help? Check [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

**Current priority areas:**
- New LLM providers: Gemini, Mistral, local GGUF
- ~~**Windows Named Pipe IPC support**~~ — ✅ Implemented (claw-pal/src/ipc/)
- Windows sandbox implementation
- ~~Deno/V8 engine~~ — ✅ Implemented (see ADR-012)
- ~~`claw-server` KernelServer implementation~~ — ✅ Implemented (see ADR-011)
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
| ADR（001-014） | ✅ 001-013 已接受，014 Proposed |
| 核心实现（13 个 crate） | ✅ 已完成 |
| 129+ 个单元+集成测试（claw-runtime） | ✅ 全部通过 |
| Clippy / fmt / 文档检查 | ✅ 干净 |
|| 当前版本 | ✅ v0.4.0 |
| 内核功能（F1-F9） | 📚 见 [kernel-features.md](docs/kernel-features.md) |

完整版本历史见 [CHANGELOG.md](CHANGELOG.md)。

---

## 内核功能覆盖度（v0.4.0）

**参考文档：** [docs/kernel-features.md](docs/kernel-features.md) — 完整定义内核职责与应用层边界。

| 功能 | 描述 | 状态 | 版本 |
|------|------|------|------|
| **F1** | 消息渠道抽象 | ✅ 完成 | v1.0.0+ |
| **F2** | 对话上下文管理 | ✅ 完成 | v1.0.0+ |
| **F3** | LLM 提供商抽象 | ✅ 完成 | v1.2.0+ |
| **F4** | 工具执行运行时 | ✅ 完成（G-1 审计修复） | v1.5.0-dev |
| **F5** | 技能按需加载引擎 | ✅ 完成 | v1.4.0+ |
| **F6** | 事件触发系统（Cron + Webhook） | ✅ 完成 | v1.4.0+ |
| **F7** | 多 Agent 编排 | ✅ 完成（G-10 重启修复） | v1.4.1+ |
| **F8** | 安全与隔离模型 | ✅ 完成 | v1.0.0+ |
| **F9** | 脚本扩展基础 | ✅ 完成（Lua + V8） | v1.3.0+ |

**图例：**
- ✅ = 已完整实现和测试
- 🔧 = 进行中（见下方 v1.5.0 冲刺计划）
- ⬜ = 推迟到后续版本

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

- [x] **claw-pal**：Unix Domain Socket IPC + Windows Named Pipe 完整实现 + 进程管理 + 安全密钥验证
- [x] **claw-runtime**：EventBus + AgentOrchestrator + IpcRouter + Runtime
- [x] **claw-provider**：5 个 LLM Provider（Anthropic/OpenAI/Ollama/DeepSeek/Moonshot）
- [x] **claw-tools**：工具注册表 + 热加载 + JSON Schema 生成
- [x] **claw-loop**：环形历史 + 三种停止条件 + Builder
- [x] **claw-memory**：Ngram 嵌入 + SQLite 检索 + 安全配额存储
- [x] **claw-channel**：Discord / Webhook / Stdin 三种适配器
- [x] **claw-script**：Lua 引擎 + 全部 7 个 Lua bridge + V8/TypeScript 引擎 + 全部 V8 bridge
- [x] **claw-server**：JSON-RPC 2.0 KernelServer IPC 守护进程
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
  - ~~`examples/self-evolving-agent`~~ —— **有意不在此实现**；自进化是 evoclaw 应用层的核心卖点，不是内核职责。内核提供基础设施（AgentBridge、HotLoader、LuaEngine），evoclaw 负责展示。

- [x] **脚本 Bridge** —— 全部 4 个在 v0.1.0 提前完成（见 [ADR-009](docs/adr/009-bridge-roadmap.md)）
  - [x] `DirsBridge` —— 平台配置/数据/缓存/工具目录路径
  - [x] `LlmBridge` —— 脚本内 LLM 补全与流式调用
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

### v1.0.0 — 稳定版发布 ✅

**发布时间：** 2026-03-08

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
- Lua 脚本引擎，含全部 7 个 Bridge：`fs`、`net`、`tools`、`dirs`、`llm`、`events`、`agent`
- 内存历史（SQLite 历史推迟到 v1.1）
- 基础沙箱（Linux seccomp stub、macOS Seatbelt stub、Windows skeleton）
- 热加载工具
- `claw-memory` 作为持久化存储的可选参考实现

**v1.0.0 设计上不包含（见 [ADR-010](docs/adr/010-memory-system-boundary.md)）：**
- 内置在 `AgentLoop` 中的中/长期记忆 —— 内核只管理上下文窗口（`HistoryManager`），应用通过 `overflow_callback` 接管中/长期存储。

---

### v1.0.0 — 多语言基础层（KernelServer） ✅

**发布时间：** 2026-03-08

**目标时间：** 2026 Q2（与 v1.0.0 发布同时进行）

**设计理由：** 根据 [ADR-011](docs/adr/011-multi-language-ipc-daemon.md)，内核不能作为隔离的 Rust 库而存在 —— 它需要多语言支持才能成为真正的共享生态资产。非 Rust 应用必须能够无需重新实现就利用内核的统一 AI 基础设施。

**KernelServer（claw-server crate）：**
- [x] Unix Domain Socket / Named Pipe 传输，使用 claw-pal 帧协议
- [x] JSON-RPC 2.0 协议：`create_session`、`send_message`、`tool_result`、`destroy_session`
- [x] 服务器推流：`chunk`、`tool_call`、`finish` 事件
- [x] 会话生命周期管理（并行会话、隔离）
- [x] 与现有 AgentLoop、Provider、ToolRegistry 集成（无核心变更）

**claw-kernel-server 二进制：**
- [x] CLI：`--socket-path`、`--provider`、`--model`、`--power-key`、`--max-sessions`
- [x] SIGTERM 优雅关闭
- [x] 进程生命周期管理（systemd / launchd 兼容）

**设计原则：** 内核守护进程是基础设施，不是功能。它使得整个生态（OpenClaw、ZeroClaw、PicoClaw）无论使用何种实现语言都能在统一的 Rust 核心上运行。IPC 开销（≈ LLM 延迟的 0.001%）可忽略不计。

---

## v1.0 之后路线图 — 已完成

### v1.1.0 — Bug 修复与安全加固 ✅

**发布时间：** 2026-03-10

- [x] WebhookChannel 无 HMAC secret 时记录错误日志（Fix-A）
- [x] SecureMemoryStore 配额检查原子化 — 互斥锁跨越 `namespace_usage()` + `store()`（Fix-B）
- [x] WindowsSandbox Safe 模式记录安全警告 stub（Fix-C）
- [x] `Runtime::new()` 自动启动 orchestrator + IPC；`start()` 标记为 deprecated（Fix-D）
- [x] `FsBridge::glob_files()` + Lua `glob` 方法（Fix-E）
- [x] NetBridge 响应体上限 4 MiB；`block_on` 模式（Fix-F）
- [x] `bridge/conversion.rs` 从 tools.rs 中提取（Fix-G）

### v1.2.0 — Provider 扩展 ✅

**发布时间：** 2026-03-10

- [x] 额外 LLM Provider 基础工作合并入现有 provider 基础设施

### v1.3.0 — IPC Auth、ChannelRegistry 和 AgentOrchestrator 接入 ✅

**发布时间：** 2026-03-10

- [x] IPC token 认证 — daemon 生成 token（`DefaultHasher+SystemTime+PID`），写入 `~/.local/share/claw-kernel/kernel.token`；首帧必须为 `kernel.auth`
- [x] ChannelRegistry — DashMap 支持，60s TTL 去重缓存；register/unregister/list 接入 handler
- [x] AgentOrchestrator 注入 — `KernelServer` 持有 `Arc<AgentOrchestrator>`；agent.spawn/kill/steer/list 使用真实 orchestrator API
- [x] Python SDK — `examples/sdk/python/`（stdlib，4 字节 BE 帧）
- [x] `handle_kernel_info()` 返回 `protocol_version: 2`
- [x] V8 引擎（deno_core，ES2022+，TypeScript）全面接入

### v1.4.0 — GlobalToolRegistry、TriggerStore、AxumWebhookServer、TypeScript SDK ✅

**发布时间：** 2026-03-10

- [x] `GlobalToolRegistry` + `GlobalSkillRegistry` — 跨会话工具/技能共享
- [x] Schedule 回调修复 — 纠正基于定时器的触发分发
- [x] TypeScript SDK — `examples/sdk/typescript/`
- [x] `TriggerStore` — 计划和 webhook 触发器的 SQLite 持久化
- [x] `AxumWebhookServer` 集成 — 路由接入 KernelServer
- [x] `ChannelRouter` IPC 动态路由 — 入站渠道消息通过 IPC 分发

### v1.4.1 — Agent 健康检查与 RestartPolicy（G-10 修复） ✅

**发布时间：** 2026-03-10

- [x] `IpcAgentHandle.shared_tx: SharedSender`（`Arc<Mutex<Option<Sender>>>`），支持重启时热替换
- [x] `AgentState.ipc_tx: Option<SharedSender>`；`spawn_ipc_message_loop()` 在任务退出时设置 Error + 触发重启
- [x] `trigger_restart()` — 检查 RestartState，休眠 backoff，热替换 SharedSender，重启 loop
- [x] `start_health_check_task()` — 超时检测限于 `process_handle.is_some()`；移除错误的心跳自动刷新
- [x] `start_auto_restart_task()` 仅限于无独立 RestartState 的 agent；Starting 状态作为双重重启防护
- [x] 4 个新测试：`test_spawn_agent_ipc_tx_stored`、`test_health_check_heartbeat_timeout_marks_error`、`test_trigger_restart_never_policy_publishes_agent_failed`、`test_trigger_restart_hot_swaps_sender`

---

## 已完成 — v1.5.0 ✅

**目标时间：** 2026 Q2（预计 2026-03-24，5 周冲刺）

**目标：** 修复 `docs/v1.5-gap-report.md` 中识别的最关键缺口。

**内核功能增强：**
- **F4**（工具执行运行时）— 含 HMAC 签名事件的审计日志（G-1）
- **F1**（消息渠道）— 重试逻辑和去重机制完善
- **F7**（多 Agent 编排）— 进程隔离和重启策略（G-10）

### Gap 汇总

| Gap | 优先级 | 描述 | 状态 |
|-----|--------|------|------|
| GAP-01 | P0 | `claw.llm` bridge 缺失 — 脚本层无法调用 LLM | ✅ 已修复（`LlmBridge` in `claw-script`；Lua + V8 bridge 均已完整实现） |
| GAP-02 | P1 | `ChannelRouter.broadcast()` 未实现 | ✅ 已修复（`broadcast_route()` in `router.rs`） |
| GAP-03 | P1 | 渠道 `send()` 无指数退避重试 | ✅ 已修复（`RetryableChannel` in `retry.rs`） |
| GAP-04 | P2 | `UnifiedMessage` / `ChannelMessage` 缺少顶层 `sender_id` / `thread_id` | ✅ 已修复（`types.rs` 顶层字段） |
| GAP-05 | P2 | 入站 → EventBus 链路未闭合 | ✅ 已修复（`handler.rs:2417` — `event_bus.publish()`） |
| GAP-06 | P2 | `AgentHandle` 无 `resource_usage` 字段 | ✅ 已修复（G-6: `ResourceSnapshot` + `resource_monitor_task`） |
| GAP-07 | P2 | Task Agent 崩溃可能传播（需要 `catch_unwind`） | ✅ 已修复（GAP-07: `orchestrator.rs` 嵌套 `tokio::spawn` panic 隔离） |
| GAP-08 | P3 | Webhook URL 格式不规范 | ⬜ 延至 v1.5.1 |

### Sprint 计划

**Sprint 1（2 周，2026-03-10 → 2026-03-24）：** 稳定性与脚本 LLM 访问

- [x] **GAP-07** — ✅ 通过 `spawn_ipc_message_loop()` 中嵌套 `tokio::spawn` 实现 panic 隔离
- [ ] **GAP-08** — 规范化 webhook URL 格式；添加验证和结构化路径 helper
- [x] **GAP-01** — ✅ 已实现：`claw.llm` Lua bridge + V8 bridge 已在 `crates/claw-script/src/bridge/llm.rs` 完整实现；Lua 和 V8 引擎均支持 `complete()` 和 `stream()` 方法
- [x] **GAP-05** — ✅ 已修复：`handler.rs:2417` — `event_bus.publish()` 闭合入站 → EventBus 链路

**Sprint 2（3 周，2026-03-24 → 2026-04-14）：** 渠道层加固

- [x] **GAP-03** — ✅ 已修复：`RetryableChannel` 包装器实现指数退避（`retry.rs`），含 14 个测试
- [x] **GAP-04** — ✅ 已修复：`sender_id`/`thread_id` 已提升为 `ChannelMessage` 顶层字段
- [x] **GAP-02** — ✅ 已修复：`ChannelRouter::broadcast_route(msg)` — 向所有匹配 agent 广播（自动去重）
- [x] **GAP-06** — ✅ 已修复（G-6）：每 `AgentState` 的 `resource_snapshot`；`sysinfo` 每 5s 采样

**Sprint 1 剩余工作：**
- [x] **GAP-01** — ✅ `claw.llm` Lua + V8 bridge（已实现 — `LlmBridge` 含 `complete()` + `stream()`）
- [ ] **GAP-08** — Webhook URL 规范化（P3 — 已延期）

---

## 远期路线图

**策略：** 快速次要版本发布，添加 Provider 和功能。Semver 管理破坏性变更。

### v1.6.0 — 渠道层增强与更多 LLM Provider

**目标时间：** 2026 Q3

- [ ] Telegram 渠道集成
- [ ] Slack 渠道集成（含 Thread 支持）
- [x] ~~WebSocket 双向渠道~~ — ✅ 已在 `claw-channels` 中实现（`WebSocketChannel`，多客户端广播）
- [x] ~~Gemini（Google）Provider~~ — ✅ 已在 `claw-provider` 中实现（`gemini` feature）
- [x] ~~Mistral Provider~~ — ✅ 已在 `claw-provider` 中实现（`mistral` feature）
- [x] ~~Azure OpenAI Provider~~ — ✅ 已在 `claw-provider` 中实现（`azure-openai` feature）
- [x] ~~所有 Provider 的流式响应支持~~ — ✅ Gemini/Mistral/Azure OpenAI 通过 `OpenAIProvider` 继承 `complete_stream()`（三者均为 OpenAI 兼容别名）

### v1.7.0 — 沙箱加固

**目标时间：** 2026 Q4

- [ ] Linux：完整 seccomp-bpf 系统调用白名单
- [ ] macOS：完整 Seatbelt profile
- [ ] Windows：AppContainer + Job Objects

### v1.8.0 — 本地模型与高级记忆

**目标时间：** 2027 Q1

- [ ] 通过 `llama-cpp-rs` 支持本地 GGUF 模型（可选 feature）
- [x] ~~`claw-loop` SQLite 历史后端（`sqlite-history` feature）~~ — ✅ 已实现（`SqliteHistory` + `SqliteHistoryStore`）
- [ ] 性能基准测试（Provider 延迟、工具吞吐量）

---

## 贡献优先领域

达到 v1.5.0 的当前优先领域：

1. **GAP-07** — `catch_unwind` 保护 Task Agent 崩溃
2. **GAP-01** — 实现 `claw.llm` bridge（脚本层 LLM 访问）
3. **GAP-05** — 闭合入站 → EventBus 链路
4. **GAP-03** — 渠道重试机制（指数退避）
5. **GAP-04** — `ChannelMessage` 顶层 `sender_id`/`thread_id`（见 ADR-014）

**推迟（v1.6+）：**
- 新 Provider（Gemini、Mistral、Azure）
- ~~KernelServer 多语言 IPC 守护进程~~ —— ✅ 已在 v1.0.0 实现（见 ADR-011）
- GGUF 本地模型
- 高级沙箱功能
