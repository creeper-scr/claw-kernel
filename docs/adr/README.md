---
title: Architecture Decision Records
description: ADR index for claw-kernel
status: active
version: "0.1.0"
last_updated: "2026-03-01"
language: en
---

[中文版 →](README.zh.md)

# Architecture Decision Records (ADRs)

This directory contains Architecture Decision Records for claw-kernel.

## What is an ADR?

An Architecture Decision Record (ADR) captures an important architectural decision made along with its context and consequences. ADRs help new contributors understand why the codebase is structured the way it is.

## Format

Each ADR follows this structure:

```markdown
# ADR XXX: Title

**Status:** [Proposed | Accepted | Deprecated | Superseded by ADR-YYY]
**Date:** YYYY-MM-DD
**Deciders:** ...

## Context
What is the issue that we're seeing that is motivating this decision?

## Decision
What is the change that we're proposing or have agreed to implement?

## Consequences
What becomes easier or more difficult to do because of this change?
```

## Index

| ADR | Title | Status |
|-----|-------|--------|
| [001](001-architecture-layers.md) | Five-Layer Architecture with PAL | Accepted |
| [002](002-script-engine-selection.md) | Multi-Engine Script Support (Lua Default) | Accepted |
| [003](003-security-model.md) | Dual-Mode Security (Safe/Power) | Accepted |
| [004](004-hot-loading-mechanism.md) | Tool Hot-Loading as Extension Infrastructure | Accepted |
| [005](005-ipc-multi-agent.md) | IPC and Multi-Agent Coordination | Accepted |
| [006](006-message-format-abstraction.md) | Message Format Abstraction for LLM Providers | Accepted |
| [007](007-eventbus-implementation.md) | EventBus Implementation Strategy | Accepted |
| [008](008-hot-loading-file-watcher.md) | Hot-Loading File Watcher Strategy | Accepted |

## Contributing

To propose a new ADR:

1. Open a GitHub Discussion with the `adr` label
2. Reach consensus with maintainers
3. Create a PR adding the ADR file
4. Update this index

See [Contributing Guide](../../CONTRIBUTING.md) for more.
