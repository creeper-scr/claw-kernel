---
title: Changelog
description: Version history for claw-kernel
status: v0.4.0
version: "0.4.0"
last_updated: "2026-03-10"
language: english
---

<!--
本文件记录 claw-kernel 的所有显著变更。
格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.0.0/)，
版本号遵循 [Semantic Versioning](https://semver.org/lang/zh-CN/)。

This file records all notable changes to claw-kernel.
Format based on Keep a Changelog, versioning follows Semantic Versioning.
-->

# Changelog

All notable changes to claw-kernel will be documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/)
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [0.4.0] - 2026-03-10

### Added

- **GAP-01: `claw.llm` LLM Bridge** (`claw-script`) — `LlmBridge` with `complete()` and `stream()` methods; registered in both Lua engine (`crates/claw-script/src/bridge/llm.rs`) and V8 bridge, enabling scripts to call LLM providers directly
- **GAP-02: `ChannelRouter::broadcast_route()`** (`claw-channel`) — Fan-out to all matching agents with automatic deduplication (`router.rs`)
- **GAP-03: `RetryableChannel`** (`claw-channel`) — Exponential backoff retry wrapper for channel `send()`; 14 new integration tests (`retry.rs`)
- **GAP-06: `ResourceSnapshot` + `resource_monitor_task`** (`claw-runtime`) — Per-`AgentState` resource snapshot; `start_resource_monitor_task()` samples via `sysinfo` every 5s

### Fixed

- **GAP-04: `ChannelMessage` top-level fields** (`claw-channel`) — `sender_id: Option<String>` and `thread_id: Option<String>` promoted from nested to top-level fields in `types.rs` (per ADR-014)
- **GAP-05: Inbound → EventBus pipeline** (`claw-server`) — `handler.rs:2417` now calls `event_bus.publish()` to close the inbound channel message → EventBus pipeline
- **GAP-07: `tokio::spawn` panic isolation** (`claw-runtime`) — Nested `tokio::spawn` in `spawn_ipc_message_loop()` catches and isolates panics, preventing task agent crashes from propagating to the orchestrator

### Documentation

- **ROADMAP.md**: Corrected GAP-01 status from `❌ Not started` to `✅ Fixed` — `claw.llm` Lua + V8 bridge is fully implemented in `crates/claw-script/src/bridge/llm.rs` (both `complete()` and `stream()` methods, registered in Lua engine and V8 bridge)
- **ROADMAP.md**: Corrected streaming support status for Gemini/Mistral/Azure OpenAI providers — all three inherit `complete_stream()` via `OpenAIProvider` (they are OpenAI-compatible aliases); marked as `✅` in v1.6.0 section
- **docs/KNOWN-ISSUES.md**: Updated KI-001 Windows sandbox description from "stub implementation" to accurate "Job Objects partial implementation" — Windows sandbox enforces memory limits and blocks child processes via Job Objects, but filesystem and network isolation are NOT enforced (AppContainer planned for v1.7.0); added explicit security warning

---

## [1.4.1] - 2026-03-10

### Fixed

**G-10：Agent health_check + RestartPolicy 修复**

- `agent_handle.rs`：`IpcAgentHandle.shared_tx` 类型改为 `SharedSender`（`Arc<Mutex<Option<Sender>>>`），支持重启时热替换发送端，无需替换整个 Handle 实例
- `orchestrator.rs`：`AgentState.ipc_tx` 类型改为 `Option<SharedSender>`；`spawn_ipc_message_loop()` 提取为独立函数，task 退出时自动将 Agent 状态设置为 Error 并触发重启流程
- `orchestrator.rs`：`trigger_restart()` 提取为独立函数 — 读取 RestartState 策略、等待指数退避时间、热替换 SharedSender、重新派生 IPC 消息循环
- `orchestrator.rs`：`start_health_check_task()` — 移除错误的心跳自动刷新逻辑；超时检测现在仅对持有 `process_handle` 的进程型 Agent 生效
- `orchestrator.rs`：`start_auto_restart_task()` — 仅作用于无独立 `RestartState` 的 Agent；在重新派生前设置 `Starting` 状态作为双重重启防护

### Tests

新增 4 个集成测试（`claw-runtime`）：

- `test_spawn_agent_ipc_tx_stored`：验证 spawn 后 `AgentState.ipc_tx` 被正确存储
- `test_health_check_heartbeat_timeout_marks_error`：验证心跳超时后 Agent 状态置为 Error
- `test_trigger_restart_never_policy_publishes_agent_failed`：验证 `RestartPolicy::Never` 时发布 `AgentFailed` 事件而非重启
- `test_trigger_restart_hot_swaps_sender`：验证重启后 SharedSender 被热替换，旧 Handle 持有的 Arc 自动获得新 Sender

---

## [1.4.0] - 2026-03-10

### Added

- **GlobalToolRegistry**：跨 Agent 全局工具注册表，支持运行时动态注册与查询，所有 Agent 共享同一工具命名空间
- **GlobalSkillRegistry**：全局技能注册表，支持 `SkillManifest` 扫描与索引，技能按命名空间隔离
- **TriggerStore SQLite 持久化**：触发器定义写入 SQLite，内核重启后自动从数据库恢复全部触发器，无需重新注册
- **AxumWebhookServer 集成**：`POST /hooks/{trigger_id}` 标准化路由；请求体自动转发至对应 TriggerStore 条目；HMAC 签名验证复用 `WebhookChannel` 逻辑
- **ChannelRouter IPC 动态路由**：通过 IPC 消息动态注册/取消注册路由规则；`channel.route.add` / `channel.route.remove` 端点
- **TypeScript SDK 参考实现**（`examples/sdk/typescript/`）：Node.js 原生实现，与 Python SDK 功能对等

### Fixed

- `trigger.add_cron` 回调现在真实调用 `orchestrator.steer()`；此前为未实现的 stub，cron 触发不会产生任何效果

---

## [1.3.0] - 2026-03-10

### Breaking Changes

- **F2 架构边界重划（D1）**：`claw-memory`（`MemoryStore` / `hybrid_search` / `NgramEmbedder`）降级为可选应用层依赖，从内核核心依赖中移除。IPC 端点 `memory.search` / `memory.store` 直接删除，无降级 stub。内核 F2 职责收窄为 `HistoryManager`（短期上下文窗口管理），`claw-memory` crate 本身保留，作为独立可选组件供应用层使用。
- **`Runtime::new()` 改为 async**：现在签名为 `async fn new(endpoint) -> Result<Self, RuntimeError>`，后台任务（IPC router、event bus）在构造时自动启动。调用方须改为 `Runtime::new(endpoint).await?`。旧 `start()` 方法标记为 `#[deprecated(since = "1.1.0")]`，调用时打印警告并作为空操作执行。

### Added

- **IPC token 认证（D2）**：连接级 token 认证机制。daemon 启动时使用 `DefaultHasher + SystemTime + PID` 生成 token，以 `0o600` 权限写入 `~/.local/share/claw-kernel/kernel.token`。每条新连接的第一帧必须为 `kernel.auth` 握手帧；认证失败立即断开连接。`authenticated` 布尔字段在 `handle_connection()` 中按连接维护。
- **ChannelRegistry（D3）**：新增 `crates/claw-server/src/channel_registry.rs`。`DashMap` 后端存储，带 60s TTL 去重缓存（防止同一 channel 重复注册事件）。`channel.register` / `channel.unregister` / `channel.list` 三个 IPC 端点完整实现。
- **AgentOrchestrator 真实对接（D4）**：`agent.spawn` / `agent.kill` / `agent.steer` / `agent.list` 四个 IPC 端点现在真实调用 orchestrator API（`AgentId::new`、`AgentConfig::with_meta`、`SteerCommand::Custom`、`orchestrator.agent_ids()`），不再是 stub 响应。
- **V8/TypeScript 引擎**（`engine-v8` feature）：基于 `deno_core`，per-execution isolate 强隔离。`V8Engine` + `V8EngineOptions`（超时、堆限制、TypeScript 支持配置）。`Script::javascript()` 和 `Script::typescript()` 构造器。全部 7 个 bridge 对 JS/TS 暴露：`claw.fs`、`claw.net`、`claw.tools`、`claw.memory`、`claw.events`、`claw.agent`、`claw.dirs`、`claw.json`。
- **Python SDK 参考实现**（`examples/sdk/python/`）：仅依赖 stdlib，实现 4 字节 BE 帧协议。包含 `kernel_client.py`（客户端封装）、`example_chat.py`、`example_tools.py`、`README.md`。
- **协议版本更新**：`handle_kernel_info()` 返回 `protocol_version: 2`。

---

## [0.2.0] - 2026-03-08

### Added
- **Script Bridges**: `DirsBridge`, `MemoryBridge`, `EventsBridge`, `AgentBridge` — expose host
  capabilities (filesystem paths, memory store, event bus, agent lifecycle) to Lua scripts
- **SQLite history**: `SqliteHistoryStore` (claw-memory) + `SqliteHistory` (claw-loop) for
  persistent conversation history backed by SQLite; zero direct rusqlite dependency in claw-loop
- **`StreamChunk` type** (claw-loop): groundwork type for future streaming API; holds partial
  token text, tool call deltas, and finish reason
- **`ExtensionEvent` type** (claw-runtime): hot-loading and script reload events published on the
  `EventBus`; includes `LoadError` and `ReloadError` thiserror enums
- **`AgentOrchestrator::spawn()`** (claw-runtime): out-of-process agent management via PAL
  `TokioProcessManager`; adds `terminate()` and `kill()` lifecycle methods
- **`AgentStatus` field** on `AgentInfo` (claw-runtime): tracks `Running` / `Stopped` / `Error`
  states; set automatically by `register`, `spawn`, `terminate`, and `kill`
- **`AgentResult` convenience fields** (claw-loop): `content`, `tool_calls`,
  `execution_time_ms` — extracted from `last_message` for ergonomic access without pattern
  matching
- **ADR-009** (`docs/adr/009-bridge-roadmap.md`): Lua bridge capability roadmap
- **ADR-010** (`docs/adr/010-memory-system-boundary.md`): memory system boundary decisions
- **ADR-011** (`docs/adr/011-multi-language-ipc-daemon.md`): KernelServer / multi-language IPC
  daemon design

### Changed (Breaking)
- **`NgramEmbedder` + `Embedder` trait moved**: relocated from `claw_memory` to
  `claw_provider::embedding`.  The `claw_memory::embedding` module and
  `claw_memory::traits::Embedder` trait have been removed.
  - **Migration**: replace
    ```rust
    use claw_memory::{embedding::NgramEmbedder, traits::Embedder};
    ```
    with
    ```rust
    use claw_provider::embedding::{Embedder, NgramEmbedder};
    ```
- **`AgentOrchestrator::new()` constructor changed** (claw-runtime): now creates an internal
  `TokioProcessManager`; a new `with_process_manager()` constructor allows injection.  Callers
  that previously shared a process manager via `Runtime` must use `with_process_manager`.
- **`AgentInfo` struct extended** (claw-runtime): added `process_handle` and `status` fields;
  any manual construction of `AgentInfo` must supply these.

### Removed
- `claw_memory::embedding` module (moved to `claw_provider::embedding`)
- `claw_memory::traits::Embedder` trait (moved to `claw_provider::embedding::Embedder`)
- All Chinese `.zh.md` documentation duplicates (English-only documentation going forward)
- Stale planning artifacts: `BUILD_PLAN.md`, `DOCUMENTATION_AUDIT_REPORT.md`,
  `TECHNICAL_SPECIFICATION.md`, `.sisyphus/` directory

### Fixed
- Upgraded `rusqlite` from 0.30.0 to 0.32.1 (bundled SQLite, improved API)
- `claw-provider/src/embedding.rs`: module is now self-contained; stale module-level doc comment
  updated to reflect that `NgramEmbedder` is no longer imported from `claw_memory`
- `claw-pal`: Windows IPC stub now returns `IpcError::ConnectionRefused` instead of panicking
- Applied `cargo fmt` across all crates (formatting only, no logic changes)
- All crate documentation updated to match the v0.2.0 API surface

---

## [0.1.0] - 2026-03-01

Initial release. **9 crates, 670+ tests passing, zero clippy errors.**

### claw-pal — Platform Abstraction Layer

- `IpcTransport`: Unix Domain Socket with 4-byte little-endian length-prefix framing
- `ProcessManager`: spawn, kill, list processes via `DashMap`; graceful `SIGTERM → SIGKILL` escalation with configurable timeout
- Platform config directory resolution via `dirs`
- Security key validation via `argon2` (Power Mode key strength enforcement)
- Linux sandbox stub (`seccomp-bpf` + namespace groundwork)
- macOS sandbox stub (Seatbelt / `sandbox(7)` groundwork)
- Integration tests for IPC round-trip and process lifecycle

### claw-runtime — System Runtime

- `EventBus`: Tokio broadcast channel with capacity 1024; typed event dispatch
- `AgentOrchestrator`: concurrent agent registration and lifecycle management via `DashMap`; `AgentId` generated by `AtomicU64` counter (collision-safe under parallel tests)
- `IpcRouter`: routes `IpcMessage` between agents over the PAL IPC transport
- `Runtime`: unified async entry point wrapping EventBus, Orchestrator, and IpcRouter
- Agent event types: `AgentStarted`, `AgentStopped`, `MessageReceived`, `ToolCalled`, `ToolResult`

### claw-provider — LLM Providers

- `LLMProvider` trait: three-layer design (`MessageFormat` → `HttpTransport` → `LLMProvider`)
- `DefaultHttpTransport`: `reqwest 0.11` with `rustls` (no native TLS dependency)
- Provider implementations:
  - **Anthropic** (Claude): Messages API, streaming, tool use
  - **OpenAI-compatible**: chat completions, function calling, streaming
  - **Ollama**: local model HTTP API
  - **DeepSeek**: OpenAI-compatible endpoint
  - **Moonshot**: OpenAI-compatible endpoint
- `ProviderRegistry`: runtime provider selection by name
- Comprehensive format conversion tests (40 tests)

### claw-tools — Tool Registry

- `Tool` trait with permission declarations and JSON Schema via `schemars`
- `ToolRegistry`: `DashMap`-backed; per-call configurable timeout; structured audit log
- `HotLoader`: `notify 6.1.1` file watcher with 50 ms debounce for automatic tool reload
- Script-based tool loading infrastructure (Lua bridge via claw-script)

### claw-loop — Agent Loop

- `AgentLoop` struct and builder pattern (`AgentLoopBuilder`)
- `InMemoryHistory`: ring-buffer with configurable max entries; overflow callback closure
- Stop conditions trait + built-in implementations:
  - `MaxTurns`: halt after N turns
  - `TokenBudget`: halt when estimated token usage exceeds budget
  - `NoToolCall`: halt when a turn produces no tool calls
- `HistoryManager` trait for pluggable history backends

### claw-memory — Memory System

- `NgramEmbedder`: 64-dimensional bigram + trigram text embeddings (no external model dependency)
- `SqliteMemoryStore`: `rusqlite` (bundled SQLite) backend; cosine similarity search computed in-process
- `SecureMemoryStore`: 50 MB quota enforcement; argon2-protected at-rest entries
- Memory worker: async `mpsc` channel (capacity 256) for non-blocking batch archiving
- `MemoryEntry`, `MemoryQuery`, `MemoryResult` types

### claw-channel — Channel Integrations

- `Channel` trait + `ChannelMessage` protocol type
- Platform adapters:
  - **Discord**: Twilight gateway integration
  - **HTTP Webhook**: Axum receiver endpoint
  - **Stdin**: synchronous adapter for testing and CLI use
- `Platform` enum: `Discord`, `Webhook`, `Stdin`

### claw-script — Script Engine

- `ScriptEngine` trait + `EngineType` enum
- `LuaEngine`: `mlua 0.9.4` (Lua 5.4, `send` feature); runs inside `tokio::task::spawn_blocking` (required for mlua sync API on async runtimes)
- `ToolsBridge`: exposes the tool registry to Lua scripts; bidirectional type conversion via `mlua` serde

### claw-kernel — Meta-Crate

- Re-exports all sub-crates under a unified namespace
- `prelude` module: one-line import for the most commonly used types and traits
- Feature flags: `engine-lua` (default), `engine-v8` (optional)

### Infrastructure

- Workspace `Cargo.toml` with pinned dependency versions (verified compatibility)
- All profiles (`dev`, `release`, `test`) set `panic = "unwind"` (mlua hard requirement)
- MSRV: Rust 1.83+ (mlua 0.9.4 / deno_core requirement)
- Complete bilingual documentation (English + Chinese): README, CHANGELOG, ROADMAP, CONTRIBUTING, SECURITY

---

### 中文版本说明 / Chinese Release Notes

**v0.1.0 — 2026-03-01 — 首次发布**

**9 个 crate，670+ 个测试全部通过，零 clippy 错误。**

#### 核心亮点

- **claw-pal**：Unix Domain Socket IPC（4 字节小端帧协议）；SIGTERM → SIGKILL 优雅进程管理；argon2 安全密钥验证
- **claw-runtime**：广播容量 1024 的 EventBus；AtomicU64 防重复 AgentId；IpcRouter 多 Agent 消息路由
- **claw-provider**：5 个 LLM Provider（Anthropic、OpenAI 兼容、Ollama、DeepSeek、Moonshot）；rustls（无 native TLS 依赖）
- **claw-tools**：DashMap 工具注册表（超时+审计日志）；notify 6.1.1 文件热加载（50ms 防抖）
- **claw-loop**：环形缓冲历史（溢出回调）；MaxTurns/TokenBudget/NoToolCall 三种停止条件
- **claw-memory**：64 维 bigram+trigram 嵌入；SQLite 余弦相似度检索；50 MB 安全配额；异步批量归档
- **claw-channel**：Discord（Twilight）、HTTP Webhook（Axum）、Stdin 三种渠道适配器
- **claw-script**：mlua 0.9.4 Lua 5.4 引擎（spawn_blocking 安全运行）；ToolsBridge 双向类型转换
- **claw-kernel**：统一元 crate，prelude 一行导入所有常用类型

---

[Unreleased]: https://github.com/claw-project/claw-kernel/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/claw-project/claw-kernel/compare/v1.4.1...v0.4.0
[1.4.1]: https://github.com/claw-project/claw-kernel/compare/v1.4.0...v1.4.1
[1.4.0]: https://github.com/claw-project/claw-kernel/compare/v1.3.0...v1.4.0
[1.3.0]: https://github.com/claw-project/claw-kernel/compare/v0.2.0...v1.3.0
[0.2.0]: https://github.com/claw-project/claw-kernel/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/claw-project/claw-kernel/releases/tag/v0.1.0
