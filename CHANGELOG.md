---
title: Changelog
description: Version history for claw-kernel
status: design-phase
version: "0.1.0"
last_updated: "2026-02-28"
language: bilingual
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

## [Unreleased]

### Design and Documentation

- Complete 5-layer architecture documentation (`docs/architecture/`)
- 8 Architecture Decision Records (ADRs 001-008) covering layers, script engines, security model, hot-loading, IPC, and more
- Platform-specific guides for Linux, macOS, and Windows (`docs/platform/`)
- Design documents: Agent Loop state machine, Channel message protocol
- EventBus implementation strategy
- Hot-loading file watcher strategy
- Technical feasibility analysis for cross-platform sandbox backends
- Per-crate documentation stubs (`docs/crates/`)
- Bilingual README, CONTRIBUTING, SECURITY, and CHANGELOG

---

## [0.1.0] - TBD

*Planning phase. See [BUILD_PLAN.md](BUILD_PLAN.md) for the implementation roadmap.*

### Planned

- `claw-pal`: Platform Abstraction Layer (sandbox, IPC, process management)
- `claw-provider`: LLM provider trait with Anthropic, OpenAI, and Ollama implementations
- `claw-tools`: Tool registry, schema generation, and hot-loading
- `claw-loop`: Agent loop engine with history management and stop conditions
- `claw-runtime`: Event bus and multi-agent orchestration
- `claw-script`: Embedded Lua engine (default), optional Deno/V8 and PyO3
- `claw-kernel`: Meta-crate re-exporting all of the above

---

[Unreleased]: https://github.com/claw-project/claw-kernel/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/claw-project/claw-kernel/releases/tag/v0.1.0
