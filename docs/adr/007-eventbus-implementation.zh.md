---
title: ADR 007: EventBus 实现策略
status: accepted
date: 2026-02-28
type: adr
last_updated: "2026-03-01"
language: zh
---

[English →](007-eventbus-implementation.md)

# ADR 007: EventBus 实现策略

**状态：** 已接受  
**日期：** 2026-02-28  
**决策者：** claw-kernel 维护者

---

## 背景

构建计划第 2 阶段引入了 `claw-runtime`（第 1 层），其核心组件是 `EventBus`。BUILD_PLAN.md 规范有意将内部实现留空：

```rust
pub struct EventBus {
    // 内部实现  ← 待填充
}

impl EventBus {
    pub fn emit(&self, event: Event);
    pub fn subscribe(&self, filter: EventFilter) -> Receiver<Event>;
}
```

`EventBus` 必须支持：

- **扇出投递** — 多个独立订阅者各自接收每一个事件
- **过滤订阅** — 调用方传入 `EventFilter`，只接收相关事件变体
- **非阻塞发送** — 即使订阅者处理缓慢，`emit` 也绝不阻塞调用方
- **延迟检测** — 慢速订阅者应收到警告并被丢弃，而不是静默阻塞总线
- **IpcRouter 集成** — 通过 PAL IPC 传输到达的跨进程事件必须流入同一总线

评估了三种 Tokio 原语：

| 原语 | 扇出 | 背压 | 延迟检测 | 备注 |
|------|:----:|:----:|:--------:|------|
| `tokio::sync::broadcast` | 原生支持 | 丢弃最旧消息 | 内置 `RecvError::Lagged` | 要求 `Event: Clone` |
| `tokio::sync::mpsc` | 不支持（单消费者） | 有界队列 | 手动实现 | 每个订阅者需要独立的分发循环 |
| `tokio::sync::watch` | 原生支持 | 仅保留最新值 | 无 | 不适合事件流 |

还考虑了第四个选项 `crossbeam-channel`，但被拒绝：它是同步 API，在异步优先的代码库中需要 `spawn_blocking` 包装，增加了不必要的复杂性。

---

## 决策

**使用 `tokio::sync::broadcast` 作为 EventBus 的核心机制，容量设为 1024。**

`Event` 枚举在设计上是 `Clone` 的（所有变体携带拥有所有权的数据）。`broadcast` 无需手动分发循环即可原生地将每条消息投递给每个活跃接收者。延迟检测通过 `RecvError::Lagged(n)` 内置实现，能精确告知慢速订阅者丢失了多少条消息。

### 内部结构体布局

```rust
use tokio::sync::broadcast;

/// 进程内事件总线。每个事件都会被克隆并投递给所有订阅者。
/// 容量 1024 意味着最多可以排队 1024 条未读事件，
/// 超出后最旧的消息会被丢弃，延迟的订阅者会收到通知。
pub struct EventBus {
    sender: broadcast::Sender<Event>,
}

impl EventBus {
    /// 创建新的总线。broadcast 通道在此创建；
    /// 初始 Receiver 立即被丢弃，因为订阅者通过 subscribe() 按需创建。
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(1024);
        Self { sender }
    }

    /// 向所有活跃订阅者发送事件。
    /// 返回接收到事件的接收者数量。
    /// 永不阻塞；如果通道已满，最旧的消息会被丢弃，
    /// 延迟的接收者会被标记。
    pub fn emit(&self, event: Event) -> usize {
        // 只有在没有接收者时 send() 才返回 Err，这是正常情况（尚无订阅者）。
        self.sender.send(event).unwrap_or(0)
    }

    /// 订阅匹配给定过滤器的事件。
    /// 返回一个包装了 broadcast::Receiver 的过滤接收者。
    pub fn subscribe(&self, filter: EventFilter) -> FilteredReceiver {
        FilteredReceiver {
            inner: self.sender.subscribe(),
            filter,
        }
    }
}

/// 跳过不匹配过滤器事件的 broadcast 接收者。
pub struct FilteredReceiver {
    inner: broadcast::Receiver<Event>,
    filter: EventFilter,
}

impl FilteredReceiver {
    /// 接收下一个匹配的事件。
    /// 透明地跳过不匹配的事件。
    /// 如果该订阅者落后了 n 条消息，返回 `Err(RecvError::Lagged(n))`；
    /// 调用方应记录警告并决定是继续还是取消订阅。
    pub async fn recv(&mut self) -> Result<Event, broadcast::error::RecvError> {
        loop {
            let event = self.inner.recv().await?;
            if self.filter.matches(&event) {
                return Ok(event);
            }
        }
    }
}
```

### 容量选择：1024

1024 个槽位足以吸收短暂的突发流量（工具调用风暴、快速的智能体生命周期转换），同时不会造成无限制的内存增长。按每个 `Event` 变体约 200 字节（保守估计），满缓冲区每个总线实例消耗约 200 KB，对守护进程来说完全可以接受。

如果订阅者落后超过 1024 个事件，`broadcast` 会丢弃最旧的消息，并在下次 `recv()` 时设置 `Lagged` 错误。订阅者必须显式处理这种情况：

```rust
match rx.recv().await {
    Ok(event) => handle(event),
    Err(broadcast::error::RecvError::Lagged(n)) => {
        tracing::warn!("EventBus 订阅者延迟了 {} 个事件，部分事件已丢失", n);
        // 从当前位置继续接收。
    }
    Err(broadcast::error::RecvError::Closed) => break,
}
```

持续延迟的订阅者（例如慢速日志接收器）应移至专用的后台任务，使用独立的有界 `mpsc` 队列，由一个永不阻塞的 broadcast 订阅者来填充。

### IpcRouter 集成

`EventBus` 是纯进程内组件。跨进程事件通过 PAL IPC 传输（Linux/macOS 上的 Unix 域套接字，Windows 上的命名管道）传输，由 `IpcRouter` 桥接到总线：

```
远程智能体                      本地进程
    │                               │
    │  序列化的 Event（bincode）      │
    ├──────────────────────────────►│
    │                               │  IpcRouter::on_incoming()
    │                               │      │
    │                               │      ▼
    │                               │  event_bus.emit(event)
    │                               │      │
    │                               │      ▼
    │                               │  所有本地订阅者
```

`IpcRouter` 持有 `Arc<EventBus>` 并对每个反序列化的传入事件调用 `emit()`。本地事件由其生产者（智能体循环、工具执行器等）直接发送，完全不经过 IPC。

出站路由对称工作：`IpcRouter` 订阅 `Event::A2A(_)` 变体，并通过 IPC 将其转发给相应的远程智能体。

```rust
pub struct IpcRouter {
    event_bus: Arc<EventBus>,
    transport: Arc<dyn IpcTransport>,  // 来自 claw-pal
}

impl IpcRouter {
    /// 当来自远程智能体的帧到达时，由 PAL IPC 层调用。
    pub fn on_incoming(&self, raw: &[u8]) {
        if let Ok(event) = bincode::deserialize::<Event>(raw) {
            self.event_bus.emit(event);
        }
    }

    /// 后台任务：将 A2A 事件转发给远程智能体。
    pub async fn run_outbound(&self) {
        let mut rx = self.event_bus.subscribe(EventFilter::A2A);
        loop {
            match rx.recv().await {
                Ok(Event::A2A(msg)) => {
                    let _ = self.transport.send(msg.to, &bincode::serialize(&msg).unwrap()).await;
                }
                Ok(_) => unreachable!(),
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("IpcRouter 出站延迟了 {} 个 A2A 事件", n);
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    }
}
```

这使 `EventBus` 完全不包含任何 IPC 知识。总线是纯进程内的扇出原语；`IpcRouter` 是跨进程桥接器。

---

## 后果

### 积极方面

- **无需分发循环** — `broadcast` 原生处理扇出，无需额外任务
- **延迟可观测** — `RecvError::Lagged(n)` 提供精确诊断；慢速订阅者无法静默地破坏总线
- **API 简洁** — `emit` 和 `subscribe` 是唯一的公共方法；`FilteredReceiver` 隐藏了循环跳过逻辑
- **IpcRouter 解耦** — EventBus 没有 IPC 依赖；可以在没有任何 PAL 代码的情况下进行单元测试
- **背压显式** — 容量 1024 是一个有文档记录的、可调整的常量，而不是无界队列

### 消极方面

- **Event 必须是 Clone** — 所有 `Event` 变体携带拥有所有权的数据；每次发送时克隆有成本。对于高频事件（例如流式 token 输出），调用方应批量处理或使用总线外的专用通道。
- **容量在构建时固定** — 更改容量需要重启总线。对守护进程来说可以接受，但值得注意。
- **延迟的订阅者会丢失消息** — 没有重放机制。需要保证投递的订阅者（例如审计日志记录器）必须通过从持久存储中重新读取来处理 `Lagged`，而不是依赖总线。

---

## 待解决问题（已解决）

| 问题 | 解决方案 |
|------|----------|
| 1. 扇出用 `broadcast` 还是 `mpsc`？ | **`broadcast`** — 原生扇出，无需分发循环，内置延迟检测。`mpsc` 需要每个订阅者一个通道加上手动分发任务。 |
| 2. broadcast 通道的容量是多少？ | **1024** — 吸收短暂突发（最大约 200 KB），对典型智能体工作负载足够大，同时限制内存。未来可通过 `EventBusConfig` 调整。 |
| 3. `IpcRouter` 如何集成而不将 EventBus 耦合到 IPC？ | **`IpcRouter` 持有 `Arc<EventBus>`** 并对传入帧调用 `emit()`。EventBus 没有 IPC 知识。桥接在类型层面是单向的。 |
| 4. 慢速订阅者会发生什么？ | **下次 `recv()` 时返回 `RecvError::Lagged(n)`**。订阅者记录警告并从当前位置继续。持续缓慢的订阅者应使用专用的缓冲任务。 |
| 5. `crossbeam-channel` 作为替代方案？ | **已拒绝** — 同步 API 在异步优先的代码库中需要 `spawn_blocking` 包装。对此用例没有优于 `broadcast` 的优势。 |

---

## 参考

- [claw-runtime crate 文档](../crates/claw-runtime.md)
- [ADR-005: IPC 和多智能体协调](005-ipc-multi-agent.md)
- [平台抽象层](../architecture/pal.md)（IPC 部分）
- [Tokio broadcast 文档](https://docs.rs/tokio/latest/tokio/sync/broadcast/index.html)
