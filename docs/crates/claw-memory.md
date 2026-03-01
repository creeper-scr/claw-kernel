---
title: claw-memory
description: "Memory layer: Ngram embedder, SQLite vector store, SecureMemoryStore with quota enforcement"
status: active
version: "0.1.0"
last_updated: "2026-03-01"
language: en
---

[中文版 →](claw-memory.zh.md)

# claw-memory

Long-term memory layer for agent kernels — semantic search, persistent storage, and quota enforcement.

---

## Overview

`claw-memory` provides the Layer 2 memory subsystem for claw-kernel. It implements a lightweight semantic memory pipeline without requiring external embedding services.

## Components

- **NgramEmbedder**: 64-dimensional bigram + trigram character-level embedder
- **SqliteMemoryStore**: Cosine similarity search performed in-memory over SQLite-backed records
- **SecureMemoryStore**: Wraps `SqliteMemoryStore` with a 50 MB per-agent quota

## Architecture

```
Agent
  └── SecureMemoryStore (50 MB quota)
        └── SqliteMemoryStore (cosine sim, in-memory index)
              └── NgramEmbedder (64-dim bigram+trigram)
                    └── SQLite (rusqlite + sqlite-vec)
```

## Usage

See [Writing Tools](../guides/writing-tools.md) for integration examples.
