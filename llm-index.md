---
title: claw-kernel LLM Index
description: Machine-readable documentation index for AI systems
version: "0.1.0"
last_updated: "2026-02-28"
language: en
---

# claw-kernel LLM Index

> Machine-readable documentation index for AI systems

## Document Taxonomy

### Core Project Documents

| Document | Type | Language | Purpose | Lines |
|----------|------|----------|---------|-------|
| [README.md](README.md) | Overview | Bilingual (EN/ZH) | Project introduction, quick start | ~400 |
| [AGENTS.md](AGENTS.md) | Developer Guide | English (primary) | AI agent development, security model, build commands | ~540 |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Contribution | Bilingual | How to contribute, PR checklist, development setup | ~470 |
| [BUILD_PLAN.md](BUILD_PLAN.md) | Roadmap | Chinese | Phase-by-phase implementation plan, trait definitions | ~540 |
| [ROADMAP.md](ROADMAP.md) | Roadmap | English | High-level milestones, design decisions | ~100 |
| [TECHNICAL_SPECIFICATION.md](TECHNICAL_SPECIFICATION.md) | Specification | Bilingual | Dependency versions, feature matrix, compatibility | ~400 |
| [SECURITY.md](SECURITY.md) | Security | Bilingual | Vulnerability reporting, security model, disclosure policy | ~180 |
| [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) | Governance | Bilingual | Contributor Covenant, enforcement guidelines | ~280 |
| [CHANGELOG.md](CHANGELOG.md) | Changelog | English | Version history | ~25 |

### Architecture Documentation

| Document | Type | Content | Lines |
|----------|------|---------|-------|
| [docs/architecture/overview.md](docs/architecture/overview.md) | Deep Dive | 5-layer architecture, design philosophy, RustBridge API | ~1000 |
| [docs/architecture/crate-map.md](docs/architecture/crate-map.md) | Reference | Crate dependency graph, versioning strategy | ~1000 |
| [docs/architecture/pal.md](docs/architecture/pal.md) | Deep Dive | Platform Abstraction Layer implementation details | ~800 |

### Architecture Decision Records (ADRs)

| ADR | Title | Status | Key Decision |
|-----|-------|--------|--------------|
| [ADR-001](docs/adr/001-architecture-layers.md) | Five-Layer Architecture with PAL | Accepted | Layer 0-3 + 0.5 PAL separation |
| [ADR-002](docs/adr/002-script-engine-selection.md) | Multi-Engine Script Support | Accepted | Lua default, Deno/V8 optional |
| [ADR-003](docs/adr/003-security-model.md) | Dual-Mode Security | Accepted | Safe Mode default, Power Mode opt-in |
| [ADR-004](docs/adr/004-hot-loading-mechanism.md) | Tool Hot-Loading | Accepted | Kernel provides foundation, apps implement logic |
| [ADR-005](docs/adr/005-ipc-multi-agent.md) | IPC and Multi-Agent | Accepted | A2A protocol, AgentOrchestrator |
| [ADR-006](docs/adr/006-message-format-abstraction.md) | Message Format Abstraction | Accepted | 3-layer provider architecture |
| [ADR-007](docs/adr/007-eventbus-implementation.md) | EventBus Implementation Strategy | Accepted | tokio::sync::broadcast, capacity 1024 |
| [ADR-008](docs/adr/008-hot-loading-file-watcher.md) | Hot-Loading File Watcher Strategy | Accepted | notify crate, debounce 50ms, atomic swap |

### Design Documents

| Document | Purpose | Content | Lines |
|----------|---------|---------|-------|
| [docs/design/agent-loop-state-machine.md](docs/design/agent-loop-state-machine.md) | Design | Agent loop state machine, turn lifecycle, stop conditions, history truncation | ~1166 |
| [docs/design/channel-message-protocol.md](docs/design/channel-message-protocol.md) | Design | ChannelMessage protocol, Telegram/Discord/Webhook mapping, rate limiting, retry | ~1359 |
### User Guides

| Document | Purpose | Language |
|----------|---------|----------|
| [docs/guides/getting-started.md](docs/guides/getting-started.md) | Build your first agent, API examples | Bilingual |
| [docs/guides/writing-tools.md](docs/guides/writing-tools.md) | Create custom tools with scripts | Bilingual |
| [docs/guides/safe-mode.md](docs/guides/safe-mode.md) | Safe mode configuration, allowlists | Bilingual |
| [docs/guides/power-mode.md](docs/guides/power-mode.md) | Power mode activation, Power Key setup | Bilingual |
| [docs/guides/extension-capabilities.md](docs/guides/extension-capabilities.md) | Runtime extensibility, hot-loading | Bilingual |

### Per-Crate Documentation

| Document | Crate | Layer | Content |
|----------|-------|-------|---------|
| [docs/crates/claw-pal.md](docs/crates/claw-pal.md) | claw-pal | 0.5 | SandboxBackend, IpcTransport, ProcessManager traits |
| [docs/crates/claw-runtime.md](docs/crates/claw-runtime.md) | claw-runtime | 1 | EventBus, AgentOrchestrator, Runtime |
| [docs/crates/claw-provider.md](docs/crates/claw-provider.md) | claw-provider | 2 | LLMProvider, HttpTransport, MessageFormat traits |
| [docs/crates/claw-tools.md](docs/crates/claw-tools.md) | claw-tools | 2 | Tool trait, ToolRegistry, PermissionSet |
| [docs/crates/claw-loop.md](docs/crates/claw-loop.md) | claw-loop | 2 | AgentLoop, StopCondition, HistoryManager |
| [docs/crates/claw-script.md](docs/crates/claw-script.md) | claw-script | 3 | ScriptEngine, RustBridge, hot-loading |

### Platform-Specific Guides

| Document | Platform | Sandbox Technology |
|----------|----------|-------------------|
| [docs/platform/linux.md](docs/platform/linux.md) | Linux | seccomp-bpf + Namespaces |
| [docs/platform/macos.md](docs/platform/macos.md) | macOS | sandbox(7) profile (Seatbelt) |
| [docs/platform/windows.md](docs/platform/windows.md) | Windows | AppContainer + Job Objects |

## Cross-Reference Map

### Starting Points by Use Case

| Use Case | Starting Path |
|----------|---------------|
| **New User** | README.md → docs/guides/getting-started.md |
| **New Contributor** | CONTRIBUTING.md → AGENTS.md → docs/architecture/overview.md |
| **Security Review** | SECURITY.md → AGENTS.md (Security Model section) → ADR-003 |
| **Implementation** | BUILD_PLAN.md → docs/architecture/crate-map.md → relevant crate docs |
| **Adding Provider** | docs/architecture/overview.md (Provider section) → docs/crates/claw-provider.md |
| **Platform Porting** | docs/architecture/pal.md → docs/platform/{linux,macos,windows}.md |

### Related Document Clusters

1. **Security Cluster**
   - SECURITY.md
   - AGENTS.md (Security Model section)
   - ADR-003
   - docs/guides/safe-mode.md
   - docs/guides/power-mode.md

2. **Architecture Cluster**
   - docs/architecture/overview.md
   - docs/architecture/crate-map.md
   - docs/architecture/pal.md
   - ADR-001

3. **Extension Cluster**
   - docs/guides/extension-capabilities.md
   - ADR-004
   - docs/crates/claw-tools.md
   - docs/crates/claw-script.md

4. **Provider Cluster**
   - docs/crates/claw-provider.md
   - ADR-006
   - docs/architecture/overview.md (Provider section)

5. **Build/Development Cluster**
   - BUILD_PLAN.md
   - CONTRIBUTING.md
   - AGENTS.md (Build Commands section)

## Key Entities Reference

| Entity | Type | Defined In | Description |
|--------|------|------------|-------------|
| **Layer 0** | Architecture | AGENTS.md, overview.md | Rust Hard Core — immutable trust root |
| **Layer 0.5 (PAL)** | Architecture | AGENTS.md, pal.md | Platform Abstraction Layer |
| **Layer 1** | Architecture | AGENTS.md, overview.md | System Runtime — Event Bus, IPC, Tokio |
| **Layer 2** | Architecture | AGENTS.md, overview.md | Agent Kernel Protocol — Provider, Tools, Loop |
| **Layer 3** | Architecture | AGENTS.md, overview.md | Extension Foundation — Scripts, Hot-loading |
| **Safe Mode** | Security Model | SECURITY.md, ADR-003 | Default sandboxed execution |
| **Power Mode** | Security Model | SECURITY.md, ADR-003 | Full system access, opt-in |
| **Power Key** | Security Mechanism | AGENTS.md | User credential for mode switch (min 12 chars) |
| **SandboxBackend** | Trait | BUILD_PLAN.md, claw-pal.md | Platform sandbox interface |
| **LLMProvider** | Trait | BUILD_PLAN.md, claw-provider.md | Unified LLM API interface |
| **ToolRegistry** | Component | BUILD_PLAN.md, claw-tools.md | Tool registration and hot-loading |
| **AgentLoop** | Component | BUILD_PLAN.md, claw-loop.md | Multi-turn conversation management |
| **ScriptEngine** | Trait | BUILD_PLAN.md, claw-script.md | Multi-engine script execution |

## Version History

| Date | Change |
|------|--------|
| 2026-02-28 | Initial index creation |

---
*This index is maintained for AI systems and LLM-based tools. For human-readable documentation, see [README.md](README.md).*
