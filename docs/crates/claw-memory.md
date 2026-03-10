---
title: claw-memory
description: "Memory layer: Ngram embedder, SQLite vector store, SecureMemoryStore with quota enforcement"
status: active
version: "0.1.0"
last_updated: "2026-03-08"
language: en
---


# claw-memory

Long-term memory layer for agent kernels — semantic search, persistent storage, and quota enforcement.

---

## Overview

`claw-memory` provides the Layer 2 memory subsystem for claw-kernel. It implements a lightweight semantic memory pipeline without requiring external embedding services.

## Components

- **NgramEmbedder**: 64-dimensional bigram + trigram character-level embedder (defined within `claw-memory`; requires no external embedding service)
- **SqliteMemoryStore**: Cosine similarity search performed in-memory over SQLite-backed records
- **SecureMemoryStore**: Wraps `SqliteMemoryStore` with a 50 MB per-agent quota

## Architecture

```
Agent
  └── SecureMemoryStore (50 MB quota)
        └── SqliteMemoryStore (cosine sim, in-memory index)
              └── NgramEmbedder (64-dim bigram+trigram, defined in claw-memory)
                    └── SQLite (rusqlite + sqlite-vec)
```

## Usage

See [Writing Tools](../guides/writing-tools.md) for integration examples.

## Core Components

### NgramEmbedder

64-dimensional character-level embedder using bigram + trigram n-grams. Produces deterministic embeddings with no external API dependency.

> **Note:** `NgramEmbedder` is defined within `claw-memory` and requires no external embedding service.

```rust
use claw_memory::NgramEmbedder;

let embedder = NgramEmbedder::new();
let embedding = embedder.embed("search query text");
// Returns a 64-dimensional f32 vector
```

### SqliteMemoryStore

SQLite-backed persistent store with in-memory cosine similarity search.

```rust
use claw_memory::SqliteMemoryStore;

let store = SqliteMemoryStore::open("./memory.db").await?;
```

### SecureMemoryStore

Wraps `SqliteMemoryStore` with per-agent namespace isolation and a 50 MB quota (Safe Mode).

```rust
use claw_memory::SecureMemoryStore;

let store = SecureMemoryStore::new(inner_store, "agent-1", 50 * 1024 * 1024);
```

## API Reference

### MemoryItem

```rust
use claw_memory::MemoryItem;

// Create a new memory item
let item = MemoryItem::new("agent-1", "The user prefers concise answers.")
    .with_tags(vec!["preference".to_string()])
    .with_importance(0.8)
    .with_embedding(embedding);

let id = store.store(item).await?;
```

### MemoryStore Trait

```rust
use claw_memory::{MemoryStore, MemoryItem, MemoryId};

// Store a single item
let id = store.store(item).await?;

// Store multiple items in batch
let ids = store.store_batch(vec![item1, item2, item3]).await?;

// Store with quota check (atomic)
let id = store.store_with_quota_check(item, estimated_size_bytes, quota_bytes).await?;

// Retrieve by ID
let item = store.retrieve(&id).await?;

// Semantic search
let results = store.semantic_search(&query_embedding, top_k).await?;

// Search episodic memory
let entries = store.search_episodic(&filter).await?;

// Delete
store.delete(&id).await?;

// Clear all in namespace
let count = store.clear_namespace("agent-1").await?;

// Get namespace usage in bytes
let bytes = store.namespace_usage("agent-1").await?;
```

### EpisodicFilter

```rust
use claw_memory::EpisodicFilter;

let filter = EpisodicFilter::new()
    .for_namespace("agent-1")
    .limit(100);

// Or construct directly with fields
let filter = EpisodicFilter {
    namespace: Some("agent-1".to_string()),
    after_ms: Some(1700000000000),   // Unix ms
    before_ms: Some(1700100000000),
    limit: Some(100),
    ..Default::default()
};
```

### Memory Types

```rust
/// Unique identifier for a memory item
pub struct MemoryId(pub String);

/// Unique identifier for an episode
pub struct EpisodeId(pub String);

/// A single memory item
pub struct MemoryItem {
    pub id: MemoryId,
    pub namespace: String,
    pub content: String,
    pub embedding: Option<Vec<f32>>,
    pub tags: Vec<String>,
    pub created_at_ms: u64,
    pub accessed_at_ms: u64,
    pub importance: f32,  // 0.0–1.0
}

/// An episodic memory entry
pub struct EpisodicEntry {
    pub id: MemoryId,
    pub episode_id: EpisodeId,
    pub namespace: String,
    pub role: String,  // "user" or "assistant"
    pub content: String,
    pub timestamp_ms: u64,
    pub turn_index: u32,
}

/// Filter for querying episodic memory
pub struct EpisodicFilter {
    pub episode_id: Option<EpisodeId>,
    pub namespace: Option<String>,
    pub after_ms: Option<u64>,
    pub before_ms: Option<u64>,
    pub limit: Option<usize>,
}
```

## Memory Worker

For background memory operations, use the `MemoryWorker`:

```rust
use claw_memory::{MemoryWorker, ArchiveRequest};

// Create worker
let (worker, handle) = MemoryWorker::new(store);

// Queue an archive request
handle.request(ArchiveRequest {
    namespace: "agent-1".to_string(),
    messages: vec![...],
}).await?;

// Shutdown worker
handle.shutdown().await?;
```

## Security Notes

- **Safe Mode**: Each agent operates in an isolated namespace; total storage capped at 50 MB per agent.
- **Power Mode**: Full access, no quota enforcement.
- Exceeding the quota returns `MemoryError::QuotaExceeded`.

## Error Handling

```rust
use claw_memory::MemoryError;

match result {
    Err(MemoryError::QuotaExceeded { namespace, used, limit }) => {
        eprintln!("Quota exceeded for {}: {}/{} bytes", namespace, used, limit);
    }
    Err(MemoryError::NotFound(id)) => {
        eprintln!("Memory item not found: {}", id);
    }
    Err(e) => eprintln!("Memory error: {}", e),
    Ok(id) => println!("Stored with ID: {}", id),
}
```
