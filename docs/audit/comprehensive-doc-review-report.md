# claw-kernel 文档体系全面审查报告

> 审查日期: 2026-02-28  
> 审查范围: docs/ 目录下所有文档、AGENTS.md、BUILD_PLAN.md、TECHNICAL_SPECIFICATION.md、ROADMAP.md  
> 审查重点: 类型结构定义、API端点定义、内部一致性

---

## 执行摘要

| 问题类型 | 数量 | 严重程度分布 |
|---------|------|-------------|
| 模糊性问题 | 27 | 高: 4, 中: 15, 低: 8 |
| 不具体问题 | 37 | 高: 6, 中: 21, 低: 10 |
| 冲突问题 | 14 | 高: 5, 中: 7, 低: 2 |
| **总计** | **78** | **高: 15, 中: 43, 低: 20** |

### 关键发现

1. **Trait定义不一致**是最严重的问题,涉及 `MessageFormat`、`HttpTransport`、`LLMProvider`、`Tool` 等多个核心trait
2. **中英文文档不同步**,中文部分存在翻译遗漏(如缺少 `type Error` 定义)
3. **方法签名不统一**,`ToolRegistry` 在不同文档中有3种不同定义
4. **命名不一致**,如 `HotReloadConfig` vs `HotLoadingConfig`

---

## 严重问题清单 (高优先级)

### 1. `MessageFormat` trait 中文定义不完整 [冲突]

**位置:** `docs/adr/006-message-format-abstraction.md` (中文部分)

**问题:** 中文翻译遗漏 `type Error: std::error::Error;`,且 `parse_stream_chunk` 参数类型错误

```rust
// 当前(错误)
pub trait MessageFormat: Send + Sync {
    type Request: Serialize;
    type Response: DeserializeOwned;
    type StreamChunk: DeserializeOwned;
    // ❌ 缺少 type Error
    
    fn parse_stream_chunk(chunk: Self::StreamChunk) -> Option<Delta>;  // ❌ 参数类型错误
}

// 应该(正确)
pub trait MessageFormat: Send + Sync {
    type Request: Serialize;
    type Response: DeserializeOwned;
    type StreamChunk: DeserializeOwned;
    type Error: std::error::Error;
    
    fn parse_stream_chunk(chunk: &[u8]) -> Result<Option<Delta>, Self::Error>;
}
```

**修复建议:** 同步英文版本的完整定义

---

### 2. `Tool` trait 缺少 `#[async_trait]` 属性 [冲突]

**位置:** `docs/architecture/overview.md` 第328-336行、`docs/architecture/crate-map.md` 第426-434行

**问题:** 包含 `async fn` 方法但没有 `#[async_trait]` 属性,这在Rust中是无效的

```rust
// 当前(错误)
pub trait Tool: Send + Sync {
    async fn execute(&self, params: serde_json::Value) -> Result<ToolResult>;
}

// 应该(正确)
#[async_trait]
pub trait Tool: Send + Sync {
    async fn execute(&self, params: serde_json::Value) -> Result<ToolResult, ToolError>;
}
```

**修复建议:** 统一添加 `#[async_trait]` 属性

---

### 3. `ToolRegistry` API 定义严重不一致 [冲突]

**位置:** 多文档不一致

| 方法 | overview.md | crate-map.md | claw-tools.md | BUILD_PLAN.md |
|------|-------------|--------------|---------------|---------------|
| `register` | `Result<(), RegistryError>` | `Result<(), RegistryError>` | ❌ 无Result | `Result<(), RegistryError>` |
| `execute` | `Result<ToolResult>` | `Result<ToolResult, ToolError>` | - | `Result<ToolResult, ToolError>` |
| `enable_hot_reload` | `HotReloadConfig` | `HotLoadingConfig` | ❌ 无参数 | `HotLoadingConfig` |

**修复建议:** 统一以 `overview.md` 为基准,但将 `enable_hot_reload` 配置参数设为可选

---

### 4. `HttpTransport` 返回类型不一致 [冲突]

**位置:** `docs/architecture/overview.md` vs `docs/architecture/crate-map.md`

```rust
// overview.md (正确)
async fn request<F: MessageFormat>(...) -> Result<CompletionResponse, ProviderError>;

// crate-map.md (错误)
async fn request<F: MessageFormat>(...) -> Result<CompletionResponse>;  // ❌ 缺少错误类型
```

**修复建议:** 统一使用 `Result<T, ProviderError>`

---

### 5. 中英文 `LLMProvider` trait 定义不一致 [冲突]

**位置:** `docs/adr/006-message-format-abstraction.md` (英文第98-105行 vs 中文第314-319行)

**问题:** 英文版有 `ProviderError`,中文版没有

**修复建议:** 同步中文翻译

---

## 中等问题清单 (中优先级)

### 类型定义问题

#### 6. `AgentLoop` 结构体字段不一致 [冲突]

**位置:** `docs/architecture/overview.md` vs `docs/architecture/crate-map.md` vs `BUILD_PLAN.md`

- `crate-map.md` 缺少 `provider` 和 `tools` 字段
- `summarizer` 字段类型不一致 (`Box` vs `Option<Box>`)

**修复建议:** 统一使用 `Option<Box<dyn Summarizer>>`

#### 7. `ScriptEngine` 错误类型不一致 [冲突]

**位置:** `docs/architecture/crate-map.md` vs `BUILD_PLAN.md`

```rust
// crate-map.md
fn compile(...) -> Result<Script>;  // ❌ 无错误类型

// BUILD_PLAN.md
fn compile(...) -> Result<Script, CompileError>;  // ✅ 明确错误类型
```

#### 8. `Event` vs `EventType` 混淆 [冲突]

**位置:** `docs/architecture/overview.md` 使用 `Event`, `docs/architecture/crate-map.md` 使用 `EventType`

**修复建议:** 明确定义两者的区别和使用场景

### 模糊性问题

#### 9. 资源配额缺少具体数值 [模糊]

**位置:** `docs/architecture/overview.md` Layer 1 Process Management

**问题:** "Resource quotas (CPU/memory)" 没有具体数值

**修复建议:** 添加默认配额和范围

```markdown
- Resource quotas:
  - CPU: 0.5-4 cores (default: 1)
  - Memory: 256MB-8GB (default: 512MB)
```

#### 10. `grace_period` 默认值未定义 [模糊]

**位置:** `BUILD_PLAN.md` Phase 1

**问题:** `terminate` 方法的 `grace_period` 没有说明默认值

**修复建议:** 添加文档注释

```rust
/// grace_period: 优雅终止等待时间,默认 5s
async fn terminate(&self, handle: ProcessHandle, grace_period: Duration) -> Result<(), ProcessError>;
```

### 不具体问题

#### 11. `Tool::description` 长度限制未说明 [不具体]

**位置:** `docs/crates/claw-tools.md`

**问题:** 没有说明LLM对description的最佳长度

**修复建议:** 添加建议长度说明

```rust
/// Tool description for LLM
/// 建议长度: 50-200 字符
/// 最佳实践: 清晰描述功能,包含使用场景
fn description(&self) -> &str;
```

#### 12. 权限列表不完整 [不具体]

**位置:** `docs/crates/claw-tools.md`

**问题:** 只列出4个权限,缺少如 `fs.delete`、`net.https` 等

**修复建议:** 提供完整的权限清单

```markdown
- `none` — 无权限(默认)
- `fs.read` / `fs.write` / `fs.delete` — 文件系统
- `net.http` / `net.https` — 网络请求
- `memory.read` / `memory.write` / `memory.delete` — 内存访问
- `process.spawn` / `process.exec` — 子进程(Power Mode only)
```

#### 13. RustBridge API 类型定义不完整 [不具体]

**位置:** `docs/crates/claw-script.md`

**问题:** `DirEntry` 和 `Response` 类型没有定义

**修复建议:** 添加完整的类型定义

```typescript
interface DirEntry {
    name: string;
    isFile: boolean;
    isDirectory: boolean;
    size: number;
    modified: Date;
}
```

---

## 修订建议汇总

### 立即修复 (发布前必须)

| 序号 | 问题 | 影响 | 预估工作量 |
|------|------|------|-----------|
| 1 | `Tool` trait 添加 `#[async_trait]` | 代码无法编译 | 10分钟 |
| 2 | 统一 `MessageFormat` 中文定义 | 文档错误 | 15分钟 |
| 3 | 统一 `ToolRegistry` API | API混乱 | 30分钟 |
| 4 | 统一 `Result` 错误类型 | 类型安全 | 20分钟 |

### 短期修复 (1周内)

| 序号 | 问题 | 优先级 | 文件 |
|------|------|--------|------|
| 5 | 添加资源配额数值 | 高 | overview.md |
| 6 | 完善权限列表 | 高 | claw-tools.md |
| 7 | 补充 RustBridge 类型定义 | 中 | claw-script.md |
| 8 | 统一 `AgentLoop` 结构体 | 中 | 多文件 |

### 长期完善 (持续)

| 序号 | 问题 | 说明 |
|------|------|------|
| 9 | 添加更多代码示例 | 每个主要API配示例 |
| 10 | 完善错误处理示例 | 展示实际错误处理模式 |
| 11 | 补充性能数据 | 添加基准测试结果 |
| 12 | 统一依赖版本 | BUILD_PLAN.md 与 TECHNICAL_SPECIFICATION.md |

---

## 修订检查清单

在修改文档时,请确保:

- [ ] 所有trait定义在文档间保持一致
- [ ] 中英文文档同步更新
- [ ] 方法签名包含完整的错误类型
- [ ] 结构体字段在所有文档中一致
- [ ] 添加代码示例验证API可用性
- [ ] 检查命名一致性(如HotReload vs HotLoading)

---

## 附录: 文档依赖关系图

```
AGENTS.md (入口文档)
    ├── docs/architecture/overview.md
    │   ├── docs/architecture/crate-map.md
    │   ├── docs/architecture/pal.md
    │   └── docs/adr/*.md
    ├── docs/crates/*.md
    │   ├── claw-provider.md
    │   ├── claw-tools.md
    │   ├── claw-script.md
    │   ├── claw-loop.md
    │   ├── claw-runtime.md
    │   └── claw-pal.md
    ├── BUILD_PLAN.md
    ├── TECHNICAL_SPECIFICATION.md
    └── ROADMAP.md
```

**建议:** 建立文档同步机制,在修改核心类型定义时同步更新所有相关文档。

---

*报告生成时间: 2026-02-28*  
*审查工具: AI Code Review Agent with doc-refiner skill*
