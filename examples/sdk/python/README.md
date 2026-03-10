# claw-kernel Python SDK

claw-kernel 的 Python 参考客户端，基于标准库实现，无第三方依赖。

## 安装要求

- Python 3.9+
- 无第三方依赖（仅使用标准库）
- 运行中的 `claw-kernel-server`（客户端会自动尝试启动）

## 快速开始

```python
from kernel_client import KernelClient

with KernelClient() as kernel:
    session = kernel.create_session("You are a helpful assistant.")
    for token in kernel.send_message(session, "Hello!"):
        print(token, end="", flush=True)
    kernel.destroy_session(session)
```

运行示例：

```bash
# 进入示例目录
cd examples/sdk/python

# 最简对话示例
python example_chat.py

# 工具回调示例
python example_tools.py
```

## API 参考

### `KernelClient(socket_path=None)`

创建客户端并自动连接。`socket_path` 默认为 `~/.local/share/claw-kernel/kernel.sock`。

---

### `create_session(system_prompt="", tools=None, model=None, max_turns=None) -> str`

创建新会话，返回 `session_id`。

| 参数 | 类型 | 说明 |
|------|------|------|
| `system_prompt` | `str` | 系统提示词 |
| `tools` | `list` | 工具 Schema 列表（Anthropic 格式） |
| `model` | `str` | 覆盖默认模型，如 `"claude-3-5-sonnet-20241022"` |
| `max_turns` | `int` | 最大对话轮数 |

---

### `send_message(session_id, content, tools=None) -> Iterator[str]`

发送消息，以生成器形式逐 token 流式返回响应。

| 参数 | 类型 | 说明 |
|------|------|------|
| `session_id` | `str` | 会话 ID |
| `content` | `str` | 用户消息内容 |
| `tools` | `dict[str, Callable]` | 工具名 -> Python 函数的映射，传入后自动处理工具回调 |

---

### `destroy_session(session_id) -> None`

销毁会话，释放服务器端资源。每次会话结束后应调用此方法。

---

### `info() -> dict`

获取内核信息（版本号、已加载的 provider 等）。

---

### `close() -> None`

关闭 socket 连接。使用 `with` 语句时自动调用。

## 工具回调

`send_message` 的 `tools` 参数接受一个 `{工具名: Python函数}` 字典。当 LLM 发起工具调用时，客户端自动执行对应函数并将结果返回给服务器，整个过程对调用方透明。

```python
def get_weather(city: str) -> str:
    return f"{city}: 晴天 22°C"

TOOL_HANDLERS = {"get_weather": get_weather}

for token in kernel.send_message(session, "北京天气？", tools=TOOL_HANDLERS):
    print(token, end="", flush=True)
```

工具 Schema（用于 `create_session` 的 `tools` 参数）需符合 Anthropic `input_schema` 格式：

```python
TOOLS = [{
    "name": "get_weather",
    "description": "获取城市天气",
    "input_schema": {
        "type": "object",
        "properties": {"city": {"type": "string"}},
        "required": ["city"],
    },
}]
```

## 连接机制

1. **Socket 发现**：默认路径 `~/.local/share/claw-kernel/kernel.sock`（macOS / Linux 相同）。可通过 `KernelClient(socket_path="/custom/path.sock")` 覆盖。
2. **自动启动 Daemon**：若 socket 文件不存在，客户端自动执行 `claw-kernel-server --socket-path <path>`，并等待最多 3 秒直到 socket 就绪。
3. **Token 认证**：连接建立后立即发送 `kernel.auth` 握手帧。Token 从 `~/.local/share/claw-kernel/kernel.token`（权限 600）读取；文件不存在时发送空 token。
4. **帧格式**：4 字节 Big Endian 长度前缀 + JSON payload，最大帧 16 MiB。

## 文件说明

| 文件 | 说明 |
|------|------|
| `kernel_client.py` | 核心客户端实现（约 200 行，无第三方依赖） |
| `example_chat.py` | 最简对话示例 |
| `example_tools.py` | 工具回调完整示例（天气查询 + 计算器） |
