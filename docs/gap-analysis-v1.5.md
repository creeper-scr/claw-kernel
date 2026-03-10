# claw-kernel v1.5 差距分析报告

> 生成日期：2026-03-10
> 基准规范：[docs/kernel-features.md](../kernel-features.md)
> 基准代码：v1.4.1（commit `2a57f37`）
> 分析范围：F1–F9 全部功能模块

---

## 目录

1. [总体评分](#总体评分)
2. [GAP-F6-01 · CronScheduler 缺失](#gap-f6-01--cronscheduler-缺失)
3. [GAP-F6-02 · TriggerStore 持久化缺失](#gap-f6-02--triggerstore-持久化缺失)
4. [GAP-F6-03 · WebhookServer 中央路由缺失](#gap-f6-03--webhookserver-中央路由缺失)
5. [GAP-F6-04 · Webhook 请求去重缺失](#gap-f6-04--webhook-请求去重缺失)
6. [GAP-F6-05 · Webhook 限流缺失](#gap-f6-05--webhook-限流缺失)
7. [GAP-F6-06 · EventTrigger 条件转发缺失](#gap-f6-06--eventtrigger-条件转发缺失)
8. [GAP-F8-01 · Linux seccomp-bpf 未实现](#gap-f8-01--linux-seccomp-bpf-未实现)
9. [GAP-F8-02 · Windows AppContainer 未实现](#gap-f8-02--windows-appcontainer-未实现)
10. [GAP-F8-03 · macOS 资源限制非强制](#gap-f8-03--macos-资源限制非强制)
11. [GAP-F4-01 · 权限字符串通配符缺失](#gap-f4-01--权限字符串通配符缺失)
12. [GAP-F4-02 · Safe/Power 全局模式与 ToolRegistry 未集成](#gap-f4-02--safepower-全局模式与-toolregistry-未集成)
13. [GAP-F4-03 · 双套审计日志系统未统一](#gap-f4-03--双套审计日志系统未统一)
14. [GAP-F1-01 · receive() 返回单条消息而非 Stream](#gap-f1-01--receive-返回单条消息而非-stream)
15. [修复优先级汇总](#修复优先级汇总)

---

## 总体评分

| 功能模块 | 完成度 | 状态 |
|---------|-------|------|
| F1 消息渠道抽象 | 95% | ✅ 接近完整，1 处设计偏差 |
| F2 对话上下文管理 | 100% | ✅ 完全实现 |
| F3 LLM 提供商抽象 | 100% | ✅ 完全实现 |
| F4 工具执行运行时 | 85% | ⚠️ 3 处缺口 |
| F5 技能按需加载 | 90% | ⚠️ 热加载/权限审计为应用层，可接受 |
| F6 事件触发系统 | 45% | ❌ 6 处缺口，最高优先级 |
| F7 多 Agent 编排 | 95% | ✅ 接近完整 |
| F8 安全与隔离 | 75% | ⚠️ 平台实现不完整 |
| F9 脚本扩展基础 | 98% | ✅ 接近完整 |

---

## GAP-F6-01 · CronScheduler 缺失

### 严重程度：🔴 高（阻塞 F6 核心能力）

### 规范要求

`kernel-features.md §F6` 规定：

```
CronScheduler（内核内置）
├── add(expr, trigger_id)   注册定时任务
├── remove(trigger_id)
└── list() -> Vec<CronJob>
```

- 支持 6 字段秒级 Cron 表达式（`s m h d M dow`）
- 内核保证精度：秒级

### 现状

`claw-runtime` 已定义 `TriggerEvent::Cron` 类型，`TriggerDispatcher` 也能正确处理它。但**没有任何代码会产生 Cron 类型的触发事件**：

- 无 `CronScheduler` struct 或 trait
- 无 Cron 表达式解析
- 无定时任务循环

值得注意的是，`claw-runtime/Cargo.toml` 中已存在：

```toml
cron = "0.12"
chrono = { version = "0.4", features = ["serde"] }
```

依赖已就绪，但未被使用。

### 不修复的后果

1. Agent 完全无法实现定时触发场景（定时报告、定时检查、定时清理等）
2. `TriggerEvent::Cron` 在代码中是死代码，无法被测试到
3. F6 整体设计的"主动行动"能力为零，Agent 只能被动等待输入
4. 文档与现实完全背离，影响用户信任

### 技术背景

`cron` crate（0.12）提供标准 6 字段解析：

```rust
use cron::Schedule;
use std::str::FromStr;

let schedule = Schedule::from_str("0 */5 * * * *")?; // 每5分钟
for dt in schedule.upcoming(Utc).take(3) {
    println!("{}", dt);
}
```

`chrono` 已在工作区依赖中（0.4），两者配合可实现完整调度。

### 修复方案

**方案 A（推荐）：在 `claw-runtime` 中实现 `CronScheduler`**

```rust
// crates/claw-runtime/src/cron_scheduler.rs

pub struct CronJob {
    pub id: String,
    pub expr: String,
    pub target_agent: Option<AgentId>,
    pub next_fire: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub enabled: bool,
}

pub struct CronScheduler {
    jobs: Arc<DashMap<String, CronJob>>,
    event_bus: EventBus,
}

impl CronScheduler {
    pub fn add(&self, expr: &str, trigger_id: &str, target: Option<AgentId>)
        -> Result<(), CronError>;
    pub fn remove(&self, trigger_id: &str) -> bool;
    pub fn list(&self) -> Vec<CronJob>;
    pub async fn run(self);  // tokio::spawn 驱动的调度循环
}
```

调度循环核心逻辑：
1. 每秒唤醒，扫描所有 job
2. 计算 `next_fire`，到期则 `event_bus.publish(Event::TriggerFired(TriggerEvent::cron(...)))`
3. `TriggerDispatcher` 已订阅 `Event::TriggerFired`，无需额外改动

**方案 B：使用 `tokio-cron-scheduler` 第三方 crate**

```toml
tokio-cron-scheduler = "0.10"
```

封装成内核 `CronScheduler` trait，内部委托给 `tokio-cron-scheduler`。优点是成熟稳定，缺点是增加依赖体积（约 200KB）。

**推荐：方案 A**，`cron` crate 已在依赖中，只需约 150 行实现，无新增依赖。

---

## GAP-F6-02 · TriggerStore 持久化缺失

### 严重程度：🔴 高（重启后触发器丢失）

### 规范要求

`kernel-features.md §F6` 规定：

> **触发器持久化：重启后自动恢复（存储在内核管理的 SQLite 中）**

### 现状

所有触发器（Cron job、Webhook 注册）仅存在于内存中。没有：

- `TriggerStore` trait 或实现
- SQLite 持久化逻辑
- 重启后恢复机制

`claw-runtime/Cargo.toml` 没有 `rusqlite` 依赖（`claw-memory` 有，但 `claw-runtime` 无）。

### 不修复的后果

1. 每次重启后，所有已注册的 Cron 任务和 Webhook 监听器全部消失
2. 生产环境需要应用层在每次启动时重新注册所有触发器，增加复杂度
3. 触发器的历史记录（触发次数、上次触发时间）无法持久化
4. 无法实现"错过的触发"（missed-fire）补偿逻辑
5. 与规范的"重启后自动恢复"承诺完全矛盾

### 技术背景

项目已使用 `rusqlite 0.32.0`（在 `claw-memory` crate 中），DDL 可复用。触发器持久化的数据量极小，单表足矣。

### 修复方案

**在 `claw-runtime` 中新增 `TriggerStore`：**

```rust
// crates/claw-runtime/src/trigger_store.rs

pub struct TriggerRecord {
    pub trigger_id: String,
    pub trigger_type: String,  // "cron" | "webhook" | "event"
    pub config: serde_json::Value,  // Cron: {expr}, Webhook: {path}
    pub target_agent: Option<String>,
    pub created_at: i64,
    pub enabled: bool,
    pub last_fired_at: Option<i64>,
    pub fire_count: i64,
}

pub struct TriggerStore {
    conn: Mutex<rusqlite::Connection>,
}

impl TriggerStore {
    pub fn open(path: &Path) -> Result<Self, TriggerStoreError>;
    pub fn save(&self, record: &TriggerRecord) -> Result<(), TriggerStoreError>;
    pub fn delete(&self, trigger_id: &str) -> Result<(), TriggerStoreError>;
    pub fn list_all(&self) -> Result<Vec<TriggerRecord>, TriggerStoreError>;
    pub fn update_last_fired(&self, trigger_id: &str, fired_at: i64) -> Result<(), TriggerStoreError>;
}
```

DDL（单文件 SQLite，存储路径 `~/.local/share/claw-kernel/triggers.db`）：

```sql
CREATE TABLE IF NOT EXISTS triggers (
    trigger_id TEXT PRIMARY KEY,
    trigger_type TEXT NOT NULL,
    config TEXT NOT NULL,
    target_agent TEXT,
    created_at INTEGER NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    last_fired_at INTEGER,
    fire_count INTEGER NOT NULL DEFAULT 0
);
```

需在 `claw-runtime/Cargo.toml` 中添加：

```toml
rusqlite = { version = "0.32.0", features = ["bundled"] }
```

`CronScheduler::run()` 启动时调用 `TriggerStore::list_all()` 恢复 job，实现重启恢复。

---

## GAP-F6-03 · WebhookServer 中央路由缺失

### 严重程度：🔴 高（Webhook 触发器架构不符合规范）

### 规范要求

```
WebhookServer（内核内置）
├── POST /hooks/{trigger_id}   接收外部回调
├── HMAC-SHA256 签名验证（可选，强烈建议配置）
└── 限流：默认 100 req/min/trigger
```

### 现状

现有的 `WebhookChannel`（`crates/claw-channel/src/webhook.rs`）是一个**通用 HTTP 双向渠道**，而非规范要求的"触发器专用入口"：

- 每个 `WebhookChannel` 实例独占一个端口（如 3000、3001、3002...）
- 没有统一的 `/hooks/{trigger_id}` 路径路由
- Webhook 触发事件进入的是 `ChannelMessage` 流，不直接进入 `TriggerEvent` 流
- 无法仅凭 URL 路径区分不同触发器

`claw-runtime/Cargo.toml` 已有 `axum 0.7.4`（通过 webhook feature），具备实现能力。

### 不修复的后果

1. 外部系统（GitHub Webhooks、Stripe、Slack 等）无法通过标准 `/hooks/{id}` 路径触发不同 Agent
2. 运维需要为每个触发器配置独立端口并开放防火墙规则
3. 无法在单一端口/域名上托管多个 Webhook 触发器
4. 无法与 `TriggerStore`（GAP-F6-02）配合实现完整的触发器生命周期管理

### 技术背景

规范的 WebhookServer 是**内核级 HTTP 服务**，与现有 `WebhookChannel`（应用层渠道）是两个不同层次的概念：

| | WebhookChannel | WebhookServer（规范） |
|---|---|---|
| 层次 | 应用层渠道（F1） | 内核触发基础设施（F6） |
| 用途 | 双向 HTTP 通信渠道 | 只负责接收触发信号 |
| 路由 | 单端口单实例 | `/hooks/{trigger_id}` 多路复用 |
| 输出 | `ChannelMessage` | `TriggerEvent::Webhook` |

### 修复方案

**在 `claw-runtime` 中新增 `WebhookTriggerServer`（不影响现有 `WebhookChannel`）：**

```rust
// crates/claw-runtime/src/webhook_server.rs

use axum::{Router, routing::post, extract::{Path, State}, body::Bytes};

pub struct WebhookTriggerServer {
    bind_addr: SocketAddr,
    secrets: Arc<DashMap<String, String>>,  // trigger_id -> hmac_secret
    event_bus: EventBus,
    trigger_store: Arc<TriggerStore>,
}

impl WebhookTriggerServer {
    pub async fn start(self) -> Result<SocketAddr, WebhookServerError> {
        let app = Router::new()
            .route("/hooks/:trigger_id", post(handle_webhook))
            .with_state(Arc::new(self));
        // tokio::spawn axum::serve(...)
    }

    pub fn register_trigger(&self, trigger_id: &str, secret: Option<String>);
    pub fn unregister_trigger(&self, trigger_id: &str);
}

async fn handle_webhook(
    Path(trigger_id): Path<String>,
    State(server): State<Arc<WebhookTriggerServer>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // 1. HMAC 验证（可选）
    // 2. 限流检查（后续 GAP-F6-05）
    // 3. 去重检查（后续 GAP-F6-04）
    // 4. 发布 TriggerEvent::webhook(trigger_id, payload, target_agent)
}
```

---

## GAP-F6-04 · Webhook 请求去重缺失

### 严重程度：🟡 中（幂等性保证缺失）

### 规范要求

> **Webhook 去重：相同 `X-Request-Id` 的请求在 60s 内只处理一次**

### 现状

现有 `WebhookChannel` 和规范中的 `WebhookServer` 均无 `X-Request-Id` 去重逻辑。重复请求会被多次处理。

对比：`ChannelRouter` 已有 `DeduplicatingRouter`（60s TTL，基于 `message_id`），证明项目内已有此模式。

### 不修复的后果

1. 外部系统重试（网络抖动、at-least-once 投递）会导致 Agent 收到重复触发
2. 同一 GitHub push 可能触发 CI 流程两次
3. 同一支付事件可能触发两次处理，造成业务错误
4. Webhook 提供商（GitHub/Stripe/Twilio）均使用 `X-Request-Id` 或 `X-Idempotency-Key` 机制，不支持去重则与生态不兼容

### 技术背景

`DeduplicatingRouter`（`claw-channel/src/router.rs` line 532-643）已实现相同语义的去重：

```rust
pub struct DeduplicatingRouter<R> {
    inner: R,
    seen: Arc<Mutex<HashMap<String, Instant>>>,
    ttl: Duration,
}
```

可直接参考此实现。不同点：Webhook 去重基于 HTTP Header 而非消息内部字段。

### 修复方案

**在 `WebhookTriggerServer` 中内嵌去重缓存（推荐内联，无需新 crate）：**

```rust
pub struct RequestDeduplicator {
    seen: Arc<Mutex<HashMap<String, Instant>>>,
    ttl: Duration,  // 60s
}

impl RequestDeduplicator {
    pub fn check_and_insert(&self, request_id: &str) -> bool {
        let mut map = self.seen.lock().unwrap();
        // 顺便清理过期条目（惰性 GC）
        let now = Instant::now();
        map.retain(|_, t| now.duration_since(*t) < self.ttl);
        if map.contains_key(request_id) {
            return false;  // 重复，拒绝
        }
        map.insert(request_id.to_string(), now);
        true  // 新请求，放行
    }
}
```

在 `handle_webhook` 中从 `X-Request-Id` 或 `X-Idempotency-Key` Header 提取 ID，无 ID 时跳过去重检查（兼容不支持幂等 ID 的调用方）。

---

## GAP-F6-05 · Webhook 限流缺失

### 严重程度：🟡 中（DoS 防护缺失）

### 规范要求

> **限流：默认 100 req/min/trigger**

### 现状

无任何速率限制。`WebhookChannel` 的 HTTP 服务直接接受所有请求。

### 不修复的后果

1. 恶意或异常外部系统可以每秒发送数千次 Webhook，导致 Agent 被淹没
2. 内核 EventBus 可能被瞬间打满（broadcast channel 容量 1024）
3. 无限制的触发事件会消耗 CPU 和内存，影响其他 Agent 的正常运行
4. 对公网暴露的 Webhook 端点而言，无限流是安全漏洞

### 技术背景

`claw-runtime/Cargo.toml` 已有 `tower 0.4.13`。`tower` 提供 `rate_limit` 中间件，但其粒度是全局而非 per-trigger。

Per-trigger 限流需要自定义令牌桶。

### 修复方案

**方案 A（推荐）：在 `WebhookTriggerServer` 中实现 per-trigger 令牌桶**

```rust
use std::time::{Duration, Instant};

pub struct TokenBucket {
    capacity: u32,          // 100
    tokens: f64,
    last_refill: Instant,
    refill_rate: f64,       // tokens/sec = 100.0/60.0
}

impl TokenBucket {
    pub fn try_consume(&mut self) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_rate)
            .min(self.capacity as f64);
        self.last_refill = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false  // 429 Too Many Requests
        }
    }
}

// 在 WebhookTriggerServer 中
rate_buckets: Arc<DashMap<String, Mutex<TokenBucket>>>,  // trigger_id -> bucket
```

**方案 B：使用 `tower::ServiceBuilder` + `RateLimit` 全局限流**

```rust
use tower::ServiceBuilder;
use tower::limit::RateLimit;

ServiceBuilder::new()
    .rate_limit(100, Duration::from_secs(60))
    .service(handle_webhook)
```

缺点：全局限流不区分 trigger_id，规范要求 per-trigger。

**推荐方案 A**，per-trigger 粒度符合规范，实现约 80 行。

---

## GAP-F6-06 · EventTrigger 条件转发缺失 ✅ 已修复

### 严重程度：🟢 低（可由应用层实现）

> **修复版本：v1.5.0**
> **实现文件：`crates/claw-runtime/src/event_trigger.rs`**
> **测试：15 个单元测试，全部通过**

### 规范要求

```
EventTrigger   订阅 EventBus 中的任意事件，条件满足时转发
```

### 现状

`TriggerType::Event` 枚举值存在，`TriggerEvent::event()` 工厂方法存在。但：

- 没有"条件规则"的定义结构
- 没有 EventBus 订阅 + 条件匹配的实现
- 应用需要自行订阅 EventBus 并手动发布 `TriggerFired` 事件

### 不修复的后果

1. 无法实现"当 Agent A 完成任务时自动触发 Agent B"的协作模式
2. 事件链（event chain）需要应用层手动编写粘合代码
3. `TriggerType::Event` 成为死代码，造成 API 误导

### 技术背景

此缺口属于"策略层"的边界地带：规范将 `EventTrigger` 列为内核机制，但"条件规则"本身是策略。可以通过提供简单的规则结构（事件类型匹配 + 可选 payload 过滤）来实现最小可用版本，复杂条件留给应用层。

### 修复方案

**最小可用实现（内核级）：**

```rust
pub struct EventTriggerRule {
    pub trigger_id: String,
    pub watch_event: String,   // 匹配 Event 的 type_tag，如 "agent.completed"
    pub payload_filter: Option<serde_json::Value>,  // JSONPath 风格简单过滤
    pub target_agent: Option<AgentId>,
}

pub struct EventTriggerRegistry {
    rules: Arc<DashMap<String, EventTriggerRule>>,
    event_bus: EventBus,
}

impl EventTriggerRegistry {
    pub fn register(&self, rule: EventTriggerRule);
    pub async fn run(self);  // 订阅 EventBus，匹配规则，发布 TriggerFired
}
```

复杂条件（正则、JMESPath 等）属于应用层，内核只提供 `watch_event` 字符串等值匹配即可满足大多数用例。

---

## GAP-F8-01 · Linux seccomp-bpf 未实现

### 严重程度：🟡 中（Linux 平台沙盒无实际约束）

### 规范要求

```
SandboxBackend Trait
├── Linux   seccomp-bpf + Namespaces
```

### 现状

`crates/claw-pal/src/traits/sandbox.rs` 定义了完整的 `SandboxBackend` trait。

macOS 实现（`crates/claw-pal/src/macos/sandbox.rs`）完整，基于 `sandbox_init()` FFI 和 SBPL 配置文件。

Linux 实现：
- `crates/claw-pal/src/linux/` 目录结构存在
- seccomp 相关类型定义存在
- **但 `apply()` 方法未实现真正的 syscall 过滤**
- `libseccomp 0.3.0` 在 `Cargo.toml` 中存在但功能未完全接入

在 Linux 上，Safe 模式的 `PermissionSet` 检查仅在**应用层（ToolRegistry）**进行，无内核级系统调用拦截。

### 不修复的后果

1. Linux 部署环境下，Safe 模式仅是"君子协定"，恶意或有 bug 的工具脚本可绕过 PermissionSet 直接访问文件系统
2. 规范承诺的"文件系统访问只能在 allowlist 路径内"在 Linux 上是谎言
3. 安全审计时无法通过 Linux 平台的沙盒检查
4. 与 macOS 行为不一致，跨平台表现不可预期

### 技术背景

`libseccomp`（Rust binding `libseccomp 0.3.0`）提供的 seccomp-bpf API：

```rust
use libseccomp::{ScmpFilterContext, ScmpAction, ScmpSyscall};

let mut filter = ScmpFilterContext::new_filter(ScmpAction::Allow)?;
// 拒绝 openat 到非白名单路径的调用（需配合 SCMP_CMP_EQ 过滤路径参数）
filter.add_rule(ScmpAction::Errno(libc::EPERM as u32), ScmpSyscall::from_name("openat")?)?;
filter.load()?;
```

注：seccomp 无法直接过滤路径字符串（BPF 无法访问内存），需配合 Linux Namespaces（`clone(CLONE_NEWNS)`）实现 bind mount 隔离，或使用 `landlock`（Linux 5.13+）。

### 修复方案

**方案 A（推荐）：使用 Linux Landlock（更现代，5.13+）**

```rust
// crates/claw-pal/src/linux/sandbox.rs
use landlock::{
    Access, AccessFs, Ruleset, RulesetAttr, RulesetCreatedAttr, ABI,
};

pub fn apply_safe_mode(allowlist: &[PathBuf]) -> Result<(), SandboxError> {
    let abi = ABI::V2;
    let access_all = AccessFs::from_all(abi);
    let ruleset = Ruleset::default()
        .handle_access(access_all)?
        .create()?;
    for path in allowlist {
        ruleset.add_rule(
            landlock::PathBeneath::new(
                landlock::PathFd::new(path)?,
                AccessFs::from_read(abi) | AccessFs::WriteFile,
            )
        )?;
    }
    ruleset.restrict_self()?;
    Ok(())
}
```

需添加依赖：`landlock = "0.4"`（约 30KB）

**方案 B：seccomp-bpf + SCMP_ACT_LOG 白名单模式**

仅过滤高危 syscall（`execve`, `ptrace`, `unshare` 等），不做路径级隔离。实现更简单，但保护不完整。

**方案 C：通知式 seccomp（SECCOMP_RET_USER_NOTIF）**

通过 seccomp-unotify 机制让父进程检查每次 syscall 的路径参数。保护最强，实现最复杂，有性能开销。

**建议：方案 A（landlock）适合 Linux 5.13+ 环境；方案 B 作为旧内核回退。**

---

## GAP-F8-02 · Windows AppContainer 未实现

### 严重程度：🟢 低（Windows 平台优先级低）

### 规范要求

```
SandboxBackend Trait
└── Windows  AppContainer
```

### 现状

Windows sandbox 相关类型定义（SID、Container 配置）存在，`apply()` 未实现。Windows 平台上 Safe 模式无沙盒约束。

### 不修复的后果

1. Windows 平台 Safe 模式等同于无保护
2. 对于将 claw-kernel 部署到 Windows 服务器的用户，安全承诺无法兑现
3. 在 Windows 上运行的工具可以自由访问文件系统

### 技术背景

Windows AppContainer 通过 `CreateAppContainerProfile` + `SECURITY_CAPABILITIES` 限制进程的资源访问。Rust 实现需通过 `windows-sys` crate 调用 Win32 API：

```rust
use windows_sys::Win32::Security::*;

// CreateAppContainerProfile 创建隔离容器
// InitializeProcThreadAttributeList + UpdateProcThreadAttribute 配置继承
// CreateProcessW 启动受限进程
```

### 修复方案

**短期：记录明确的 stub 警告**

```rust
#[cfg(windows)]
impl SandboxBackend for WindowsSandbox {
    fn apply(self) -> Result<SandboxHandle, SandboxError> {
        tracing::warn!(
            "Windows AppContainer sandbox not implemented. \
            Safe mode provides NO filesystem isolation on Windows."
        );
        Ok(SandboxHandle::noop())
    }
}
```

**中期：实现 AppContainer（依赖 `windows-sys 0.52`）**

当 Windows 平台优先级提升时实现完整方案。目前发出 warn 日志明确告知用户即可。

---

## GAP-F8-03 · macOS 资源限制非强制

### 严重程度：🟢 低（功能性影响小）

### 规范要求

`PermissionSet` 中包含 `ResourceLimits`（CPU / 内存上限）。

### 现状

`ResourceLimits` 结构（`max_memory_bytes`, `max_cpu_time_ms`）在代码中定义并被存储，但 macOS `sandbox_init()` API 不支持资源限制，限制值被**静默忽略**。

当前行为：
```rust
// macos/sandbox.rs
fn restrict_resources(&mut self, limits: ResourceLimits) -> &mut Self {
    self.resource_limits = Some(limits);  // 存储但不强制
    self
}
```

### 不修复的后果

1. 应用调用 `restrict_resources()` 会以为已生效，实际上无效
2. 工具可以无限制使用内存，导致内核 OOM
3. API 行为具有欺骗性（存储但不执行）

### 技术背景

macOS 上的进程资源限制需通过 `setrlimit(2)` 系统调用实现，与 `sandbox_init()` 是独立机制。

### 修复方案

**在 `apply()` 中补充 `setrlimit` 调用（不依赖 sandbox_init）：**

```rust
use nix::sys::resource::{setrlimit, Resource};

if let Some(limits) = self.resource_limits {
    if let Some(max_mem) = limits.max_memory_bytes {
        setrlimit(Resource::RLIMIT_AS, max_mem, max_mem)?;
    }
    if let Some(max_cpu) = limits.max_cpu_time_ms {
        let cpu_secs = (max_cpu / 1000 + 1) as u64;
        setrlimit(Resource::RLIMIT_CPU, cpu_secs, cpu_secs)?;
    }
}
```

`nix 0.27.1` 已在工作区依赖中，`nix::sys::resource` 提供 `setrlimit`，无新增依赖。

---

## GAP-F4-01 · 权限字符串通配符缺失

### 严重程度：🟡 中（配置灵活性受限）

### 规范要求

`PermissionSet.FsPermissions` 使用 glob 路径模式匹配（`kernel-features.md §F4`）：

```
FsPermissions    允许读/写的路径（glob）
```

### 现状

`crates/claw-tools/src/registry.rs` 中的权限检查使用**精确字符串包含匹配**：

```rust
// registry.rs 注释中已有 TODO:
// TODO: "tool.*" / "memory.*" 模式匹配未实现
fn check_fs_permission(allowed_paths: &HashSet<String>, request_path: &Path) -> bool {
    allowed_paths.iter().any(|p| request_path.starts_with(p))
    // 不支持 "/tmp/**" 或 "/home/user/docs/*.txt" 等 glob 模式
}
```

### 不修复的后果

1. 无法配置 `/tmp/**` 允许整个临时目录
2. 工具配置必须精确枚举每个允许路径，极难维护
3. 动态生成路径（如 `/tmp/claw-work-{uuid}/`）无法被 glob 覆盖，需要应用层预先注册

### 技术背景

`glob 0.3` 已在 `claw-tools/Cargo.toml` 中（HotLoader 使用）。

### 修复方案

**使用已有的 `glob` crate 替换精确匹配：**

```rust
use glob::Pattern;

fn fs_path_covered(allowed_patterns: &[String], request_path: &Path) -> bool {
    let path_str = request_path.to_string_lossy();
    allowed_patterns.iter().any(|pattern| {
        // 先尝试 glob 匹配
        if let Ok(pat) = Pattern::new(pattern) {
            return pat.matches(&path_str);
        }
        // 回退到前缀匹配（兼容非 glob 配置）
        path_str.starts_with(pattern.as_str())
    })
}
```

改动极小（约 15 行），无新增依赖。

---

## GAP-F4-02 · Safe/Power 全局模式与 ToolRegistry 未集成 ✅ 已修复

### 严重程度：🟡 中（安全层次不完整）

> **修复版本：v1.5.0**
> **实现文件：`crates/claw-tools/src/registry.rs`、`crates/claw-tools/src/types.rs`**
> **测试：4 个单元测试，全部通过**

### 规范要求

> Safe 模式下 PermissionSet 强制检查，违规立即拒绝

### 现状

`PermissionSet` 检查在 `ToolRegistry::execute()` 中通过 `ToolContext` 传入，但：

1. 无**全局** `Safe/Power` 模式状态——`ExecutionMode` 存在于 `claw-pal` 中，未注入到 `ToolRegistry`
2. `ToolRegistry` 的权限检查依赖调用方在 `ToolContext` 中正确设置 `execution_mode`
3. 无 `ModeTransitionGuard` 与 `ToolRegistry` 的集成——工具执行不自动记录模式切换审计

**实际风险：** 调用方可以简单地将 `ToolContext.execution_mode = ExecutionMode::Power` 绕过所有检查。

### 不修复的后果

1. 恶意或疏忽的应用层代码可以通过设置 `Power` 模式绕过工具权限检查
2. 安全约束成为"可选的"而非规范承诺的"不可绕过的硬约束"
3. 与 F8 沙盒层割裂，两层防御没有联动

### 修复方案

**在 `ToolRegistry` 中存储全局模式状态，外部无法覆盖：**

```rust
pub struct ToolRegistry {
    // 新增字段
    global_execution_mode: Arc<RwLock<ExecutionMode>>,
    power_mode_guard: Arc<Mutex<Option<PowerModeGuard>>>,
    // ...已有字段
}

impl ToolRegistry {
    /// 内核调用，需要有效的 power_key
    pub fn enter_power_mode(
        &self,
        power_key: &str,
        stored_hash: &PowerKeyHash,
        audit_sink: AuditSinkHandle,
        agent_id: &str,
    ) -> Result<PowerModeGuard, SecurityError>;

    /// execute() 内部合并全局模式：
    /// effective_mode = max(global_mode, ctx.execution_mode)
    /// 全局 Safe 不能被 ctx Power 覆盖
}
```

---

## GAP-F4-03 · 双套审计日志系统未统一 ✅ 已修复

### 严重程度：🟡 中（运维复杂度增加）

> **修复版本：v1.5.0**
> **实现文件：`crates/claw-tools/src/audit/mod.rs`**
> **测试：5 个单元测试，全部通过**

### 规范要求

> 每次工具执行 / 模式切换 / Agent 启动 均写入审计条目
> 格式：`{ timestamp, event_type, agent_id, details, mode }`

### 现状

项目中存在两套独立的审计系统：

**系统 1：`claw-tools/src/audit/`（ToolRegistry 审计）**
- `AuditEvent` enum：8 种事件类型，含 HMAC-SHA256 防篡改
- `AuditStore`：内存 VecDeque，支持持久化到文件
- `AuditLogWriter`：异步刷盘，10MB 轮转

**系统 2：`claw-pal/src/audit.rs`（安全事件审计）**
- `SecurityAuditEvent`：仅记录模式切换（from_mode/to_mode）
- `AuditSink` trait：fire-and-forget 接口
- `NoopAuditSink`：测试用

两系统相互独立，没有统一的审计流。`ToolRegistry` 的 `AuditEvent::ModeSwitch` 和 `claw-pal` 的 `SecurityAuditEvent` 记录同类事件但格式不同。

### 不修复的后果

1. 运维需要查阅两处日志才能重建完整的安全事件时间线
2. 审计日志格式不一致，无法用同一工具分析
3. `SecurityAuditEvent` 缺少关键字段（无 `tool_name`, `details`）
4. 持久化能力不对等：工具层有文件轮转，PAL 层只有内存

### 修复方案

**统一为单一 `AuditEvent` enum（以 `claw-tools/src/audit/` 为主体）：**

```rust
// claw-tools/src/audit/mod.rs 扩展
pub enum AuditEvent {
    // 已有...
    ToolCall { ... },
    ToolResult { ... },
    PermissionCheck { ... },
    ModeSwitch { ... },
    AgentSpawned { ... },
    // 新增（合并 PAL 层）
    SecurityModeEntered { timestamp_ms: u64, agent_id: String, power_key_hash: String },
    SecurityModeExited  { timestamp_ms: u64, agent_id: String, duration_ms: u64 },
}
```

`claw-pal/src/audit.rs` 的 `AuditSink` trait 保留，但实现改为委托给 `claw-tools` 的 `AuditStore`：

```rust
pub struct ToolsAuditSink(Arc<AuditStore>);
impl AuditSink for ToolsAuditSink {
    fn write_security_event(&self, event: SecurityAuditEvent) {
        self.0.push(AuditEvent::SecurityModeEntered { ... });
    }
}
```

---

## GAP-F1-01 · receive() 返回单条消息而非 Stream

### 严重程度：🟢 低（设计偏差，功能完整）

### 规范要求

```
Channel Trait
└── receive() -> Stream<InboundMessage>   异步消息流
```

### 现状

实际实现为：

```rust
pub trait Channel: Send + Sync {
    async fn recv(&self) -> Result<Option<ChannelMessage>, ChannelError>;
    // 而非: fn receive(&self) -> impl Stream<Item = ChannelMessage>;
}
```

调用方需自行轮询：
```rust
while let Some(msg) = channel.recv().await? {
    // 处理消息
}
```

`ChannelRouter` 内部封装了轮询循环，上层应用不直接调用 `recv()`。

### 不修复的后果

1. **API 语义偏差**：规范承诺 `Stream`，实际是循环 `recv()`
2. 无法直接使用 `futures::stream::select` 合并多个 Channel 流
3. 文档与现实不符，可能导致用户困惑

### 是否需要修复

**不急于修复**。`ChannelRouter` 已提供流式语义的等效封装，实际功能无损。若要对齐规范，可将 `recv()` 包装为 `Stream`：

```rust
fn receive(&self) -> impl Stream<Item = Result<ChannelMessage, ChannelError>> + '_ {
    futures::stream::unfold(self, |ch| async move {
        match ch.recv().await {
            Ok(Some(msg)) => Some((Ok(msg), ch)),
            Ok(None) => None,
            Err(e) => Some((Err(e), ch)),
        }
    })
}
```

无破坏性变更，可在下个次要版本中添加。

---

## 修复优先级汇总

| GAP ID | 模块 | 标题 | 优先级 | 预估工作量 | v1.5 目标 |
|--------|------|------|--------|-----------|-----------|
| F6-01 | F6 | CronScheduler 缺失 | 🔴 高 | ~200 行 | Sprint 1 |
| F6-02 | F6 | TriggerStore 持久化缺失 | 🔴 高 | ~150 行 + DDL | Sprint 1 |
| F6-03 | F6 | WebhookServer 中央路由缺失 | 🔴 高 | ~200 行 | Sprint 1 |
| F6-04 | F6 | Webhook 去重缺失 | 🟡 中 | ~80 行 | Sprint 1 |
| F6-05 | F6 | Webhook 限流缺失 | 🟡 中 | ~80 行 | Sprint 2 |
| F6-06 | F6 | EventTrigger 条件转发缺失 | 🟢 低 | ~120 行 | Sprint 2 |
| F8-01 | F8 | Linux seccomp-bpf 未实现 | 🟡 中 | ~300 行 | Sprint 2 |
| F8-02 | F8 | Windows AppContainer 未实现 | 🟢 低 | ~500 行 | v1.6 |
| F8-03 | F8 | macOS 资源限制非强制 | 🟢 低 | ~30 行 | Sprint 2 |
| F4-01 | F4 | 权限字符串通配符缺失 | 🟡 中 | ~15 行 | Sprint 1 |
| F4-02 | F4 | Safe/Power 与 ToolRegistry 未集成 | 🟡 中 | ~100 行 | Sprint 2 |
| F4-03 | F4 | 双套审计日志未统一 | 🟡 中 | ~100 行 | Sprint 2 |
| F1-01 | F1 | receive() 非 Stream | 🟢 低 | ~20 行 | v1.6 |

### Sprint 1（v1.5.0 必须）

优先修复阻塞 F6 核心能力的缺口。`cron`、`chrono`、`axum`、`rusqlite` 依赖均已在 Cargo.toml 中，实现风险低。

- [ ] GAP-F6-01：CronScheduler（`cron` crate 已就绪）
- [ ] GAP-F6-02：TriggerStore（添加 `rusqlite` 到 claw-runtime）
- [ ] GAP-F6-03：WebhookTriggerServer（`axum` feature 已就绪）
- [ ] GAP-F6-04：Webhook 去重（参考 DeduplicatingRouter 模式）
- [ ] GAP-F4-01：权限通配符（`glob` crate 已就绪，15 行改动）

### Sprint 2（v1.5.0 可选 / v1.5.x）

- [ ] GAP-F6-05：Webhook 限流
- [x] GAP-F6-06：EventTrigger（`EventTriggerRegistry` + `EventTriggerRule`，15 tests）
- [ ] GAP-F8-01：Linux Landlock
- [ ] GAP-F8-03：macOS setrlimit
- [x] GAP-F4-02：全局模式集成
- [x] GAP-F4-03：审计日志统一

### v1.6+

- [ ] GAP-F8-02：Windows AppContainer
- [ ] GAP-F1-01：receive() 改 Stream

---

*本文档由代码审查自动生成，最后更新：2026-03-10。*
*如发现缺口状态变化，请同步更新 `docs/KNOWN-ISSUES.md`。*
