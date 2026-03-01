---
title: claw-kernel Roadmap
description: Milestone-based roadmap for claw-kernel
status: v0.1.0-released
version: "0.1.0"
last_updated: "2026-03-01"
language: bilingual
---

[English](#english) | [中文](#chinese)

<a name="english"></a>

# claw-kernel Roadmap

> **Current Status: v0.1.0 Released — 9 crates, 389 tests passing**

---

## Current Status

| Item | Status |
|------|--------|
| Architecture design | ✅ Complete |
| ADRs (001-008) | ✅ All accepted |
| Core implementation (9 crates) | ✅ Complete |
| 389 unit + integration tests | ✅ All passing |
| Clippy / fmt / doc checks | ✅ Clean |
| Published on crates.io | ⬜ Planned (v0.2.0) |

See [CHANGELOG.md](CHANGELOG.md) for what shipped in v0.1.0.

---

## Completed — v0.1.0 (2026-03-01)

### claw-pal (Platform Abstraction Layer)
- [x] Unix Domain Socket IPC with 4-byte little-endian frame protocol
- [x] `ProcessManager` (DashMap + SIGTERM → SIGKILL escalation)
- [x] Platform config directories (`dirs`)
- [x] Security key validation (`argon2`)
- [x] Linux and macOS sandbox groundwork

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

### v0.2.0 — Provider Expansion & SQLite History

**Target:** 2026 Q2

- [ ] Gemini (Google) provider implementation
- [ ] Mistral provider implementation
- [ ] Azure OpenAI provider implementation
- [ ] Local GGUF model support via `llama-cpp-rs` or similar
- [ ] SQLite history backend for `claw-loop` (optional feature `sqlite`)
- [ ] Streaming response support across all providers
- [ ] Publish individual crates to crates.io

---

### v0.3.0 — Script Engine Expansion

**Target:** 2026 Q3

- [ ] Deno/V8 engine (`engine-v8` feature, via `deno_core`)
- [ ] Python engine (`engine-py` feature, via `pyo3`)
- [ ] Full `RustBridge` API: `llm`, `tools`, `memory`, `events`, `fs`, `net`
- [ ] Hot-reload: file change → script re-eval without process restart

---

### v0.4.0 — Sandbox Hardening

**Target:** 2026 Q3

- [ ] Linux: full seccomp-bpf syscall allowlist
- [ ] Linux: mount + user namespace isolation
- [ ] macOS: complete Seatbelt profile (network + file rules)
- [ ] Windows: AppContainer + Job Objects implementation
- [ ] Platform integration test suite (`--features sandbox-tests`)

---

### v0.5.0 — Channel Expansion

**Target:** 2026 Q4

- [ ] Telegram integration
- [ ] Slack integration
- [ ] WebSocket bidirectional channel
- [ ] Channel multiplexer (fan-out to multiple platforms)

---

### v0.9.0-beta — Examples, Benchmarks & Docs

**Target:** 2027 Q1

- [ ] Runnable `simple-agent` example
- [ ] Runnable `custom-tool` example
- [ ] Runnable `self-evolving-agent` example
- [ ] Performance benchmarks (provider latency, tool throughput, memory recall)
- [ ] Full rustdoc API documentation with doctests
- [ ] Migration guide for any breaking changes

---

### v1.0.0 — Stable Release

**Target:** 2027 Q1

- [ ] Stable API guarantee (semver)
- [ ] Cross-platform integration test suite (Linux + macOS + Windows CI)
- [ ] All crates published on crates.io with stable versions
- [ ] Security audit

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
| 006-008 | Additional accepted decisions |

---

## Contributing

Want to help? Check [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

**Current priority areas:**
- New LLM providers: Gemini, Mistral, local GGUF
- Windows sandbox implementation
- Deno/V8 engine bridge
- Expanded integration test coverage

---

<a name="chinese"></a>

# claw-kernel 路线图

> **当前状态：v0.1.0 已发布 —— 9 个 crate，389 个测试全部通过**

---

## 当前状态

| 项目 | 状态 |
|------|------|
| 架构设计 | ✅ 已完成 |
| ADR（001-008） | ✅ 全部已接受 |
| 核心实现（9 个 crate） | ✅ 已完成 |
| 389 个单元+集成测试 | ✅ 全部通过 |
| Clippy / fmt / 文档检查 | ✅ 干净 |
| 发布到 crates.io | ⬜ 计划中（v0.2.0） |

v0.1.0 详细发布内容见 [CHANGELOG.md](CHANGELOG.md)。

---

## 已完成 — v0.1.0（2026-03-01）

### 核心亮点（完整列表见上方英文部分）

- [x] **claw-pal**：Unix Domain Socket IPC + 进程管理 + 安全密钥验证
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

### v0.2.0 — Provider 扩展 & SQLite 历史

**目标时间：** 2026 年 Q2

- [ ] Gemini（Google）Provider 实现
- [ ] Mistral Provider 实现
- [ ] Azure OpenAI Provider 实现
- [ ] 本地 GGUF 模型支持（`llama-cpp-rs` 等）
- [ ] `claw-loop` SQLite 历史后端（可选 feature `sqlite`）
- [ ] 流式响应全面支持
- [ ] 各 crate 发布到 crates.io

---

### v0.3.0 — 脚本引擎扩展

**目标时间：** 2026 年 Q3

- [ ] Deno/V8 引擎（`engine-v8` feature，`deno_core`）
- [ ] Python 引擎（`engine-py` feature，`pyo3`）
- [ ] 完整 `RustBridge` API（llm、tools、memory、events、fs、net）
- [ ] 热加载：文件变更 → 脚本重新求值（无需重启进程）

---

### v0.4.0 — 沙箱加固

**目标时间：** 2026 年 Q3

- [ ] Linux：完整 seccomp-bpf 系统调用白名单 + 命名空间隔离
- [ ] macOS：完整 Seatbelt profile（网络+文件规则）
- [ ] Windows：AppContainer + Job Objects 完整实现
- [ ] 平台沙箱集成测试套件

---

### v0.5.0 — 渠道扩展

**目标时间：** 2026 年 Q4

- [ ] Telegram 集成
- [ ] Slack 集成
- [ ] WebSocket 双向渠道
- [ ] 渠道多路复用器（扇出到多平台）

---

### v0.9.0-beta — 示例、基准与文档

**目标时间：** 2027 年 Q1

- [ ] 可运行示例（simple-agent、custom-tool、self-evolving-agent）
- [ ] 性能基准测试（Provider 延迟、工具吞吐量、记忆召回率）
- [ ] 完整 rustdoc API 文档（含 doctests）

---

### v1.0.0 — 稳定版发布

**目标时间：** 2027 年 Q1

- [ ] 稳定 API 保证（语义化版本）
- [ ] Linux + macOS + Windows 跨平台 CI 集成测试
- [ ] 所有 crate 发布到 crates.io 稳定版
- [ ] 安全审计

---

## 贡献

想参与构建？请查看 [CONTRIBUTING.md](CONTRIBUTING.md) 了解指南。

**当前优先领域：**
- 新 LLM Provider：Gemini、Mistral、本地 GGUF
- Windows 沙箱完整实现
- Deno/V8 引擎桥接
- 扩展集成测试覆盖
