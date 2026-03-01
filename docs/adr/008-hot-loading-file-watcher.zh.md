---
title: "ADR 008: 热加载文件监听器实现"
description: "使用 notify crate 实现热加载文件监听器"
status: accepted
date: 2026-02-28
type: adr
last_updated: "2026-03-01"
language: zh
---

[English →](008-hot-loading-file-watcher.md)

# ADR 008: 热加载文件监听器实现

**状态：** 已接受  
**日期：** 2026-02-28  
**决策者：** claw-kernel 核心团队

---

## 背景

ADR-004 确立了工具热加载的高层契约：脚本存放在被监听的目录中，内核检测到变更后无需重启即可重新加载工具，Rust 核心代码永远不会被热更新。该 ADR 有意将实现细节留待后续决定。

本 ADR 为第 5 阶段（`claw-script`）填补这些细节，具体回答以下问题：

- 使用哪个 `notify` 监听器后端，以及原因
- 如何对快速保存事件进行防抖处理
- 如何原子性地将运行中的工具替换为新版本
- 同一脚本同时收到两个变更事件时如何处理
- 错误替换后如何回滚

---

## 决策

### 1. 监听器后端：`notify::RecommendedWatcher`

使用 `notify` crate（v6.x）的 `notify::RecommendedWatcher`，它会自动选择操作系统原生后端：

| 平台 | 后端 | 说明 |
|------|------|------|
| Linux | inotify | 内核级别，零轮询 |
| macOS | FSEvents | Apple 原生，省电 |
| Windows | ReadDirectoryChangesW | Win32 API，无轮询 |

**为什么不用轮询监听器？** 轮询会增加延迟并浪费 CPU。操作系统原生后端在写入系统调用完成后数毫秒内即可传递事件。跨平台行为对我们的使用场景足够一致。

启用 `notify` 的 `tokio` 特性，使事件通过 Tokio 通道传递，而非阻塞线程：

```toml
[dependencies]
notify = { version = "6", features = ["tokio"] }
notify-debouncer-mini = "0.4"
```

### 2. 防抖策略：通过 `notify-debouncer-mini` 实现 50ms 窗口

编辑器通常通过多次系统调用写入文件（截断、写入、刷新、关闭）。不进行防抖处理时，一次保存会触发 2-5 个事件。我们使用 `notify-debouncer-mini`，设置 **50ms 防抖窗口**。

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

50ms 窗口可通过 `HotLoadingConfig::debounce_ms` 配置。低于 20ms 的值在慢速文件系统上可能产生重复事件；高于 200ms 的值会让开发时的热重载感觉迟钝。

### 3. 配置：`HotLoadingConfig`

```rust
pub struct HotLoadingConfig {
    /// 监听脚本变更的目录列表。
    pub watch_dirs: Vec<PathBuf>,
    /// 防抖窗口（毫秒）。默认：50。
    pub debounce_ms: u64,
    /// 脚本编译允许的最长时间。默认：5s。
    pub compile_timeout: Duration,
    /// 成功替换后保留旧版本的时间。默认：60s。
    pub keep_previous_secs: u64,
    /// 是否递归监听子目录。默认：true。
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

### 4. 原子替换：`Arc<RwLock<HashMap>>` 与排空策略

`ToolRegistry` 将运行中的工具存储在：

```rust
tools: Arc<RwLock<HashMap<String, Arc<dyn Tool>>>>,
```

替换流程如下：

1. 检测到文件变更（防抖后）
2. 在**后台 Tokio 任务**中编译新版本（不阻塞注册表）
3. 编译成功
4. 获取 `tools` 的**写锁**
5. 将旧的 `Arc<dyn Tool>` 替换为新版本
6. 释放写锁

进行中的调用在写锁获取前已持有旧 `Arc<dyn Tool>` 的克隆，这些调用会正常完成。旧 `Arc` 在最后一个进行中的调用完成时被释放，而非在替换发生时。这就是"排空策略"：无需显式等待，依靠引用计数自动管理。

```rust
pub async fn hot_swap(&self, name: &str, new_tool: Arc<dyn Tool>) -> Result<(), SwapError> {
    let mut tools = self.tools.write().await;
    let old = tools.insert(name.to_string(), Arc::clone(&new_tool));
    drop(tools); // 立即释放锁

    // 保留旧版本用于回滚
    if let Some(prev) = old {
        self.store_previous(name, prev, self.config.keep_previous_secs).await;
    }
    Ok(())
}
```

### 5. 版本追踪：单调递增的 `VersionId`

每个加载的脚本获得一个单调递增的 `VersionId: u64`，计数器以 `AtomicU64` 形式存储在 `ToolRegistry` 中。

```rust
pub struct LoadedTool {
    pub tool: Arc<dyn Tool>,
    pub version_id: u64,
    pub loaded_at: Instant,
    pub source_path: PathBuf,
}
```

当同一脚本的两个文件变更事件在防抖窗口内到达时，`notify-debouncer-mini` 会将它们合并为一个事件。如果两个事件恰好落在窗口边界外（例如两次快速保存），第二个编译任务会检查是否已有更新的 `VersionId` 被安装。如果是，则静默丢弃结果。

```rust
// 替换前检查是否仍是最新版本
let current_version = self.version_of(name).await;
if current_version >= candidate_version_id {
    // 更新的版本已经胜出，丢弃此结果
    return Ok(());
}
```

### 6. 编译失败：保留旧版本，发送错误事件

如果编译失败（语法错误、权限违规、Schema 不匹配、超时），注册表保持当前运行版本不变，不进行替换。

在内核事件总线上发送错误事件：

```rust
pub struct HotLoadError {
    pub tool_name: String,
    pub source_path: PathBuf,
    pub error: CompileError,
    pub version_id: u64,
}

// 以如下形式发送：
Event::Extension(ExtensionEvent::HotLoadError(HotLoadError { ... }))
```

应用可以订阅此事件，将错误呈现给用户（例如打印到终端、发送到日志接收器）。

### 7. 回滚：保留旧版本 60 秒

成功替换后，注册表将旧的 `Arc<dyn Tool>` 保存在 `previous_versions` 映射中，保留时间为 `keep_previous_secs`（默认 60s）。提供显式的回滚 API：

```rust
impl ToolRegistry {
    /// 将工具回滚到上一个版本。
    /// 如果不存在上一个版本或保留窗口已过期，返回 Err。
    pub async fn rollback(&self, name: &str) -> Result<(), RollbackError>;
}
```

保留窗口到期后，旧版本被释放。每个工具只保留一个旧版本，不支持多级历史。如需更深的历史记录，请使用 ADR-004 中描述的文件系统版本化布局（`v1/`、`v2/`、`current -> v2/`）。

### 8. 完整热重载生命周期

```
文件保存到 watch_dir
        │
        ▼
notify::RecommendedWatcher 检测到变更
        │
        ▼ (50ms 防抖窗口)
notify-debouncer-mini 合并事件
        │
        ▼
HotLoadWorker 接收防抖后的事件
        │
        ├─── 文件扩展名是否受支持？(.lua / .js / .py)
        │    否  ──► 忽略
        │    是  ──► 继续
        │
        ▼
分配下一个 VersionId（AtomicU64 fetch_add）
        │
        ▼
启动后台编译任务（tokio::spawn）
        │
        ├─── 超时：compile_timeout（默认 5s）
        │    超出  ──► 发送 HotLoadError，保留旧版本
        │
        ▼
验证管道（来自 ADR-004）：
  1. 语法检查
  2. 权限审计（安全模式）
  3. Schema 验证
  4. 沙箱编译
        │
        ├─── 任意步骤失败 ──► 发送 HotLoadError，保留旧版本
        │
        ▼
检查 VersionId：是否仍是最新版本？
        │
        ├─── 否（更新版本已替换）──► 静默丢弃
        │
        ▼
获取 ToolRegistry.tools 的写锁
        │
        ▼
替换 Arc<dyn Tool>（旧 → 新）
        │
        ▼
释放写锁
        │
        ▼
将旧版本存入 previous_versions（60s TTL）
        │
        ▼
发送 Event::Extension(ToolReloaded { name, version_id })
```

ASCII 时序图：

```
文件系统    防抖器    HotLoadWorker    ScriptEngine    ToolRegistry    EventBus
    │          │            │               │               │             │
    │──变更──► │            │               │               │             │
    │          │ (50ms)     │               │               │             │
    │          │──事件──►   │               │               │             │
    │          │            │──分配 vid──►  │               │             │
    │          │            │──编译─────────►               │             │
    │          │            │◄──编译完成────│               │             │
    │          │            │──写锁─────────────────────────►             │
    │          │            │──替换旧/新────────────────────►             │
    │          │            │──释放锁───────────────────────►             │
    │          │            │──存储旧版本───────────────────►             │
    │          │            │──发送 ToolReloaded──────────────────────────►│
    │          │            │               │               │             │
```

---

## 后果

### 积极方面

- **零停机替换：** 进行中的调用在旧版本上完成，不会丢失任何请求
- **操作系统原生效率：** 无轮询；inotify/FSEvents/ReadDirectoryChangesW 使用内核通知
- **防抖防止抖动：** 快速保存（例如编辑器自动保存）只产生一次重载，而非五次
- **失败是安全的：** 错误脚本永远不会替换正常运行的版本
- **回滚速度快：** 旧版本在内存中，无需读取磁盘

### 消极方面

- **只保留一个旧版本：** 深度回滚需要 ADR-004 中的文件系统版本化布局
- **60s 内存开销：** 旧版本在保留窗口内占用内存
- **防抖增加延迟：** 重载开始前有 50ms 延迟（对开发工作流可接受）
- **写锁竞争：** 高频工具调用可能与替换写锁短暂竞争（通常低于 1ms）

### 缓解措施

- 写锁仅在 `HashMap::insert` 调用期间持有，不在编译期间持有
- `keep_previous_secs` 可配置；设为 0 可禁用保留窗口
- `debounce_ms` 可配置；对延迟敏感的开发环境可降低此值

---

## 考虑的替代方案

### 替代方案 1：使用 `tokio::time::sleep` 手动防抖

手动实现防抖：收到第一个事件时，启动一个睡眠 50ms 的任务，然后处理。窗口内的后续事件重置计时器。

**已拒绝：** `notify-debouncer-mini` 已正确实现了这一逻辑，并处理了边缘情况（快速重命名、编辑器交换文件模式）。重新实现只会增加维护负担，没有任何收益。

### 替代方案 2：`RwLock<Arc<HashMap>>`（替换整个映射）

不对每次替换加锁，而是将整个映射放在 `Arc` 后面，原子性地替换 `Arc`。

**已拒绝：** 替换整个映射意味着每次重载都要复制所有工具条目。加载了大量工具时，这很浪费。按条目加锁更细粒度，开销更小。

### 替代方案 3：无锁 `DashMap`

使用 `dashmap::DashMap` 实现无锁并发访问。

**已推迟：** DashMap 增加了依赖，其分片行为在替换时可能导致微妙的顺序问题。`RwLock` 方案更易于推理。如果性能分析显示锁竞争是真正的瓶颈，可以重新考虑。

### 替代方案 4：200ms 防抖窗口

更长的窗口可以进一步减少重复事件的可能性。

**已拒绝：** 200ms 会让交互式开发中的热重载感觉明显迟缓。50ms 对人类来说几乎无感，足以合并编辑器保存序列。

---

## 与 ADR-004 的关系

ADR-004 决定了：
- 工具可以无需重启地热加载
- 脚本存放在被监听的目录中
- Rust 核心代码永远不会被热更新
- 注册前运行验证管道

本 ADR 规定了：
- 使用哪个监听器后端（`RecommendedWatcher`）及原因
- 50ms 防抖窗口和 `notify-debouncer-mini` crate
- `Arc<RwLock<HashMap>>` 原子替换与排空策略
- 用于冲突解决的单调 `VersionId`
- 60 秒回滚保留窗口
- 完整的事件驱动生命周期

ADR-004 和 ADR-008 互为补充。ADR-004 是契约；ADR-008 是实现。

---

## 参考

- [ADR-004: 工具热加载](004-hot-loading-mechanism.md)
- [notify crate 文档](https://docs.rs/notify)
- [notify-debouncer-mini crate 文档](https://docs.rs/notify-debouncer-mini)
- [claw-script crate 文档](../crates/claw-script.md)
- [编写工具指南](../guides/writing-tools.md)
- [ROADMAP.md](../../ROADMAP.md)
