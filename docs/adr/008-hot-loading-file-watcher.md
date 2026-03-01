---
title: ADR-008: Hot-Loading File Watcher Implementation
status: accepted
date: 2026-02-28
type: adr
last_updated: "2026-03-01"
language: en
---

[中文版 →](008-hot-loading-file-watcher.zh.md)

# ADR 008: Hot-Loading File Watcher Implementation

**Status:** Accepted  
**Date:** 2026-02-28  
**Deciders:** claw-kernel core team

---

## Context

ADR-004 established the high-level contract for tool hot-loading: scripts live in a watched directory, the kernel detects changes and reloads tools without restart, and the Rust core is never hot-patched. That ADR intentionally left the implementation details open.

This ADR fills in those details for Phase 5 (`claw-script`). Specifically, it answers:

- Which `notify` watcher backend to use and why
- How to debounce rapid file-save events
- How to atomically swap a running tool for a new version
- What happens when two changes arrive for the same script simultaneously
- How rollback works after a bad swap

---

## Decision

### 1. Watcher Backend: `notify::RecommendedWatcher`

Use `notify::RecommendedWatcher` from the `notify` crate (v6.x). This selects the OS-native backend automatically:

| Platform | Backend | Notes |
|----------|---------|-------|
| Linux | inotify | Kernel-level, zero polling |
| macOS | FSEvents | Apple-native, battery-friendly |
| Windows | ReadDirectoryChangesW | Win32 API, no polling |

**Why not a polling watcher?** Polling adds latency and wastes CPU. The OS-native backends deliver events within milliseconds of a write syscall completing. Cross-platform behavior is consistent enough for our use case.

Enable the `tokio` feature on `notify` so events arrive on a Tokio channel rather than a blocking thread:

```toml
[dependencies]
notify = { version = "6", features = ["tokio"] }
notify-debouncer-mini = "0.4"
```

### 2. Debounce Strategy: 50ms Window via `notify-debouncer-mini`

Editors typically write a file in multiple syscalls (truncate, write, flush, close). Without debouncing, a single save triggers 2-5 events. We use `notify-debouncer-mini` with a **50ms debounce window**.

```rust
use notify_debouncer_mini::{new_debouncer, DebounceEventResult};
use std::time::Duration;

let (tx, rx) = tokio::sync::mpsc::channel(64);
let mut debouncer = new_debouncer(Duration::from_millis(50), move |res: DebounceEventResult| {
    if let Ok(events) = res {
        let _ = tx.blocking_send(events);
    }
})?;
debouncer.watcher().watch(&watch_dir, RecursiveMode::Recursive)?;
```

The 50ms window is configurable via `HotLoadingConfig::debounce_ms`. Values below 20ms risk duplicate events on slow filesystems; values above 200ms make hot-reload feel sluggish during development.

### 3. Configuration: `HotLoadingConfig`

```rust
pub struct HotLoadingConfig {
    /// Directories to watch for script changes.
    pub watch_dirs: Vec<PathBuf>,
    /// Debounce window in milliseconds. Default: 50.
    pub debounce_ms: u64,
    /// Maximum time allowed for script compilation. Default: 5s.
    pub compile_timeout: Duration,
    /// How long to keep the previous version after a successful swap. Default: 60s.
    pub keep_previous_secs: u64,
    /// Watch subdirectories recursively. Default: true.
    pub recursive: bool,
}

impl Default for HotLoadingConfig {
    fn default() -> Self {
        Self {
            watch_dirs: vec![default_tools_dir()],
            debounce_ms: 50,
            compile_timeout: Duration::from_secs(5),
            keep_previous_secs: 60,
            recursive: true,
        }
    }
}
```

### 4. Atomic Swap: `Arc<RwLock<HashMap>>` with Drain Strategy

`ToolRegistry` stores live tools in:

```rust
tools: Arc<RwLock<HashMap<String, Arc<dyn Tool>>>>,
```

The swap sequence is:

1. File change detected (after debounce)
2. New version compiled in a **background Tokio task** (never blocks the registry)
3. Compilation succeeds
4. Acquire **write lock** on `tools`
5. Replace old `Arc<dyn Tool>` with new one
6. Release write lock

In-flight calls hold a clone of the old `Arc<dyn Tool>` before the write lock is acquired. Those calls complete normally against the old version. The old `Arc` is dropped when the last in-flight call finishes, not when the swap happens. This is the "drain strategy": no explicit waiting, just reference counting.

```rust
pub async fn hot_swap(&self, name: &str, new_tool: Arc<dyn Tool>) -> Result<(), SwapError> {
    let mut tools = self.tools.write().await;
    let old = tools.insert(name.to_string(), Arc::clone(&new_tool));
    drop(tools); // release lock immediately

    // Keep old version for rollback window
    if let Some(prev) = old {
        self.store_previous(name, prev, self.config.keep_previous_secs).await;
    }
    Ok(())
}
```

### 5. Version Tracking: Monotonic `VersionId`

Each loaded script gets a monotonically increasing `VersionId: u64`. The counter lives in `ToolRegistry` as an `AtomicU64`.

```rust
pub struct LoadedTool {
    pub tool: Arc<dyn Tool>,
    pub version_id: u64,
    pub loaded_at: Instant,
    pub source_path: PathBuf,
}
```

When two file-change events arrive for the same script within the debounce window, `notify-debouncer-mini` coalesces them into one event. If two events somehow slip through (e.g., two rapid saves just outside the window), the second compilation task checks whether a newer `VersionId` has already been installed. If so, it discards its result silently.

```rust
// Before swapping, check we're still the latest
let current_version = self.version_of(name).await;
if current_version >= candidate_version_id {
    // A newer version already won; discard this result
    return Ok(());
}
```

### 6. Compilation Failure: Keep Old Version, Emit Error Event

If compilation fails (syntax error, permission violation, schema mismatch, timeout), the registry keeps the currently running version untouched. No swap occurs.

An error event is emitted on the kernel event bus:

```rust
pub struct HotLoadError {
    pub tool_name: String,
    pub source_path: PathBuf,
    pub error: CompileError,
    pub version_id: u64,
}

// Emitted as:
Event::Extension(ExtensionEvent::HotLoadError(HotLoadError { ... }))
```

Applications can subscribe to this event to surface errors to the user (e.g., print to terminal, send to a log sink).

### 7. Rollback: Keep Previous Version for 60 Seconds

After a successful swap, the registry keeps the previous `Arc<dyn Tool>` in a `previous_versions` map for `keep_previous_secs` (default 60s). An explicit API allows rollback:

```rust
impl ToolRegistry {
    /// Roll back a tool to its previous version.
    /// Returns Err if no previous version exists or the window has expired.
    pub async fn rollback(&self, name: &str) -> Result<(), RollbackError>;
}
```

After the retention window expires, the previous version is dropped. There is no multi-level history; only one previous version is kept per tool. If you need deeper history, use the filesystem versioning layout described in ADR-004 (`v1/`, `v2/`, `current -> v2/`).

### 8. Complete Hot-Reload Lifecycle

```
File saved to watch_dir
        │
        ▼
notify::RecommendedWatcher detects change
        │
        ▼ (50ms debounce window)
notify-debouncer-mini coalesces events
        │
        ▼
HotLoadWorker receives debounced event
        │
        ├─── Is file extension supported? (.lua / .js / .py)
        │    No  ──► Ignore
        │    Yes ──► Continue
        │
        ▼
Assign next VersionId (AtomicU64 fetch_add)
        │
        ▼
Spawn background compile task (tokio::spawn)
        │
        ├─── Timeout: compile_timeout (default 5s)
        │    Exceeded ──► Emit HotLoadError, keep old version
        │
        ▼
Validation pipeline (from ADR-004):
  1. Syntax check
  2. Permission audit (Safe Mode)
  3. Schema validation
  4. Sandbox compilation
        │
        ├─── Any step fails ──► Emit HotLoadError, keep old version
        │
        ▼
Check VersionId: is this still the latest?
        │
        ├─── No (newer version already swapped) ──► Discard silently
        │
        ▼
Acquire write lock on ToolRegistry.tools
        │
        ▼
Replace Arc<dyn Tool> (old → new)
        │
        ▼
Release write lock
        │
        ▼
Store old version in previous_versions (60s TTL)
        │
        ▼
Emit Event::Extension(ToolReloaded { name, version_id })
```

ASCII sequence diagram:

```
FileSystem    Debouncer    HotLoadWorker    ScriptEngine    ToolRegistry    EventBus
    │              │              │               │               │             │
    │──change──►   │              │               │               │             │
    │              │ (50ms)       │               │               │             │
    │              │──event──►    │               │               │             │
    │              │              │──assign vid──►│               │             │
    │              │              │──compile──────►               │             │
    │              │              │◄──compiled────│               │             │
    │              │              │──write lock───────────────────►             │
    │              │              │──swap old/new─────────────────►             │
    │              │              │──release lock─────────────────►             │
    │              │              │──store prev───────────────────►             │
    │              │              │──emit ToolReloaded─────────────────────────►│
    │              │              │               │               │             │
```

---

## Consequences

### Positive

- **Zero-downtime swaps:** In-flight calls complete against the old version; no request is dropped
- **OS-native efficiency:** No polling; inotify/FSEvents/ReadDirectoryChangesW use kernel notifications
- **Debounce prevents thrash:** Rapid saves (e.g., editor auto-save) produce one reload, not five
- **Failure is safe:** A bad script never replaces a working one
- **Rollback is fast:** Previous version is in memory, no disk read needed

### Negative

- **One previous version only:** Deep rollback requires the filesystem versioning layout from ADR-004
- **60s memory overhead:** Previous versions stay in memory for the retention window
- **Debounce adds latency:** 50ms delay before reload starts (acceptable for development workflow)
- **Write lock contention:** High-frequency tool calls may briefly contend with the swap write lock (typically sub-millisecond)

### Mitigations

- The write lock is held only for the `HashMap::insert` call, not during compilation
- `keep_previous_secs` is configurable; set to 0 to disable the retention window
- `debounce_ms` is configurable; lower it for latency-sensitive development setups

---

## Alternatives Considered

### Alternative 1: Manual Debounce with `tokio::time::sleep`

Implement debounce manually: on first event, spawn a task that sleeps 50ms then processes. Subsequent events within the window reset the timer.

**Rejected:** `notify-debouncer-mini` already implements this correctly and handles edge cases (rapid renames, editor swap-file patterns). Reimplementing it adds maintenance burden with no benefit.

### Alternative 2: `RwLock<Arc<HashMap>>` (Swap the Whole Map)

Instead of locking the map for each swap, keep the entire map behind an `Arc` and swap the `Arc` atomically.

**Rejected:** Swapping the whole map means copying all tool entries on every reload. With many tools loaded, this is wasteful. Per-entry locking is more granular and cheaper.

### Alternative 3: Lock-Free `DashMap`

Use `dashmap::DashMap` for concurrent access without a `RwLock`.

**Deferred:** DashMap adds a dependency and its sharding behavior can cause subtle ordering issues during swap. The `RwLock` approach is simpler to reason about. This can be revisited if profiling shows lock contention is a real bottleneck.

### Alternative 4: Debounce Window of 200ms

A longer window reduces the chance of duplicate events further.

**Rejected:** 200ms makes hot-reload feel noticeably slow during interactive development. 50ms is imperceptible to humans and sufficient to coalesce editor save sequences.

---

## Relationship to ADR-004

ADR-004 decided:
- Tools are hot-loadable without restart
- Scripts live in a watched directory
- The Rust core is never hot-patched
- A validation pipeline runs before registration

This ADR specifies:
- Which watcher backend (`RecommendedWatcher`) and why
- The 50ms debounce window and the `notify-debouncer-mini` crate
- The `Arc<RwLock<HashMap>>` atomic swap with drain strategy
- Monotonic `VersionId` for conflict resolution
- The 60-second rollback retention window
- The complete event-driven lifecycle

ADR-004 and ADR-008 are complementary. ADR-004 is the contract; ADR-008 is the implementation.

---

## References

- [ADR-004: Tool Hot-Loading](004-hot-loading-mechanism.md)
- [notify crate docs](https://docs.rs/notify)
- [notify-debouncer-mini crate docs](https://docs.rs/notify-debouncer-mini)
- [claw-script crate docs](../crates/claw-script.md)
- [Writing Tools Guide](../guides/writing-tools.md)
- [BUILD_PLAN.md Phase 5](../../BUILD_PLAN.md)

---
