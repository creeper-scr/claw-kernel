# claw-kernel Python SDK

> Zero-dependency Python client for the [claw-kernel](https://github.com/claw-project/claw-kernel) IPC daemon.

![Python 3.9+](https://img.shields.io/badge/python-3.9%2B-blue) ![Zero dependencies](https://img.shields.io/badge/dependencies-zero-brightgreen) ![Version 0.1.0](https://img.shields.io/badge/version-0.1.0-informational)

**Requirements**: Python 3.9+, no third-party packages. The SDK communicates with the claw-kernel daemon over a Unix socket using a length-prefixed JSON-RPC 2.0 protocol.

---

## Table of Contents

1. [Overview](#overview)
2. [Installation](#installation)
3. [Quick Start](#quick-start)
4. [Connection & Auth](#connection--auth)
5. [Session API](#session-api)
6. [Tool Callbacks](#tool-callbacks)
7. [Agent API](#agent-api)
8. [Channel API](#channel-api)
9. [Trigger API](#trigger-api)
10. [Provider API](#provider-api)
11. [Schedule API](#schedule-api)
12. [Skill API](#skill-api)
13. [Audit API](#audit-api)
14. [Async Client](#async-client)
15. [Data Models](#data-models)
16. [Error Handling](#error-handling)
17. [Configuration](#configuration)
18. [Backward Compatibility](#backward-compatibility)
19. [Running Tests](#running-tests)
20. [Examples](#examples)

---

## Overview

The claw-kernel Python SDK wraps the daemon's JSON-RPC 2.0 IPC protocol into a clean, namespace-organized API. Every feature the daemon exposes — sessions, agents, channels, triggers, providers, schedules, skills, and audit logs — is accessible through a single `KernelClient` (sync) or `AsyncKernelClient` (asyncio) instance.

The SDK has **zero runtime dependencies**. It ships as a pure-Python package and uses only the standard library.

---

## Installation

```bash
cd sdk/python
pip install -e .
```

Install with dev dependencies for testing:

```bash
pip install -e ".[dev]"
```

---

## Quick Start

### Synchronous

```python
from claw_kernel import KernelClient, SessionConfig

with KernelClient() as client:
    info = client.info()
    print(f"Connected to claw-kernel v{info.version}")

    session_id = client.session.create(
        SessionConfig(system_prompt="You are a helpful assistant.")
    )

    for token in client.session.send(session_id, "Hello!"):
        print(token, end="", flush=True)

    client.session.destroy(session_id)
```

### Asynchronous (asyncio)

```python
import asyncio
from claw_kernel import AsyncKernelClient, SessionConfig

async def main():
    async with AsyncKernelClient() as client:
        session_id = await client.session.create(
            SessionConfig(system_prompt="Be helpful.")
        )
        async for token in client.session.send(session_id, "Hello!"):
            print(token, end="", flush=True)
        await client.session.destroy(session_id)

asyncio.run(main())
```

---

## Connection & Auth

### Auto-discovery

`KernelClient()` with no arguments connects to the daemon automatically. The socket path is resolved in this order:

1. `CLAW_SOCKET_PATH` environment variable
2. Platform default (see table below)

| Platform | Default socket path |
|----------|---------------------|
| macOS | `~/Library/Application Support/claw-kernel/kernel.sock` |
| Linux | `$XDG_RUNTIME_DIR/claw/kernel.sock` or `~/.local/share/claw-kernel/kernel.sock` |
| Windows | `%LOCALAPPDATA%\claw-kernel\kernel.sock` |

The data directory can also be overridden with `CLAW_DATA_DIR`.

### Manual socket path

```python
client = KernelClient(socket_path="/tmp/my-kernel.sock")
```

### Auto-reconnect

By default the client reconnects transparently on broken-pipe errors. You can tune this:

```python
client = KernelClient(auto_reconnect=True, max_retries=5)
```

### Authentication

Authentication happens automatically during `__init__`. The SDK reads the token from `<data_dir>/kernel.token` and sends it in a `kernel.auth` handshake. If the file doesn't exist, an empty token is sent (anonymous access).

### Daemon auto-start

If the daemon isn't running, you can start it programmatically:

```python
from claw_kernel._auth import start_daemon

start_daemon()  # blocks until the socket appears (up to 10 seconds)

with KernelClient() as client:
    print(client.ping())
```

`start_daemon()` searches for `claw-kernel-server` in `$PATH`, `~/.cargo/bin`, and the package directory.

### Ping and info

```python
with KernelClient() as client:
    assert client.ping()  # True if daemon is alive

    info = client.info()
    print(info.version)           # e.g. "0.5.0"
    print(info.active_provider)   # e.g. "anthropic"
    print(info.active_model)      # e.g. "claude-3-5-sonnet-20241022"
    print(info.current_sessions)  # number of active sessions
    print(info.max_sessions)      # configured session limit
```

---

## Session API

`client.session` exposes all session lifecycle methods.

### Create a session

```python
from claw_kernel import KernelClient, SessionConfig

with KernelClient() as client:
    # Minimal — uses the daemon's default provider and model
    session_id = client.session.create()

    # With full config
    session_id = client.session.create(
        SessionConfig(
            system_prompt="You are a concise assistant.",
            max_turns=10,
            provider_override="anthropic",
            model_override="claude-3-5-haiku-20241022",
            persist_history=True,
        )
    )
```

`SessionConfig` fields:

| Field | Type | Description |
|-------|------|-------------|
| `system_prompt` | `str \| None` | System-level instruction |
| `max_turns` | `int \| None` | Max agent loop turns |
| `provider_override` | `str \| None` | Provider name (e.g. `"openai"`) |
| `model_override` | `str \| None` | Model name (e.g. `"gpt-4o"`) |
| `tools` | `list[ToolDef] \| None` | External tools the client handles |
| `persist_history` | `bool` | Persist conversation to SQLite |

### Stream a response

`session.send()` returns an iterator of string tokens. Tokens arrive as the model generates them.

```python
with KernelClient() as client:
    session_id = client.session.create()

    print("Assistant: ", end="", flush=True)
    for token in client.session.send(session_id, "Explain quantum entanglement briefly."):
        print(token, end="", flush=True)
    print()

    client.session.destroy(session_id)
```

### Collect the full response

When you don't need streaming, `send_collect()` waits for the full response and returns it as a single string:

```python
response = client.session.send_collect(session_id, "What is 2 + 2?")
print(response)  # "4"
```

### Destroy a session

Always destroy sessions when you're done to free server-side resources:

```python
client.session.destroy(session_id)
```

### Manual tool result

If you're driving the tool-call loop yourself (without the `tools=` shortcut), use `tool_result()` to send results back:

```python
client.session.tool_result(
    session_id=session_id,
    tool_call_id="call_abc123",
    result={"temperature": 22.5, "unit": "celsius"},
    success=True,
)
```

---

## Tool Callbacks

The SDK handles tool calls transparently when you pass a `tools` dict to `session.send()` or `session.send_collect()`.

### Define tools

```python
from claw_kernel import ToolDef, SessionConfig

weather_tool = ToolDef(
    name="get_weather",
    description="Get the current weather for a city",
    input_schema={
        "type": "object",
        "properties": {
            "city": {"type": "string", "description": "City name"},
            "units": {
                "type": "string",
                "enum": ["celsius", "fahrenheit"],
                "description": "Temperature units",
            },
        },
        "required": ["city"],
    },
)
```

### Pass tools to a session

Declare the tools in `SessionConfig` so the model knows they exist, then provide the Python handlers in `tools=`:

```python
def get_weather(city: str, units: str = "celsius") -> dict:
    # Your real implementation here
    return {"city": city, "temperature": 22, "units": units, "condition": "sunny"}

with KernelClient() as client:
    session_id = client.session.create(
        SessionConfig(
            system_prompt="You are a weather assistant.",
            tools=[weather_tool],
        )
    )

    response = client.session.send_collect(
        session_id,
        "What's the weather in Tokyo?",
        tools={"get_weather": get_weather},
    )
    print(response)

    client.session.destroy(session_id)
```

When the model calls `get_weather`, the SDK invokes your Python function, sends the result back, and continues streaming. You never see the tool-call interruption.

### Multiple tools

```python
import math
from datetime import datetime, timezone

def calculate(expression: str) -> str:
    try:
        result = eval(expression, {"__builtins__": {}}, {"math": math})
        return str(result)
    except Exception as exc:
        return f"Error: {exc}"

def get_current_time() -> str:
    return datetime.now(tz=timezone.utc).isoformat()

tools_config = [
    ToolDef(
        name="calculate",
        description="Evaluate a mathematical expression",
        input_schema={
            "type": "object",
            "properties": {
                "expression": {"type": "string", "description": "Math expression"}
            },
            "required": ["expression"],
        },
    ),
    ToolDef(
        name="get_current_time",
        description="Return the current UTC time in ISO 8601 format",
        input_schema={"type": "object", "properties": {}},
    ),
]

tool_handlers = {
    "calculate": calculate,
    "get_current_time": get_current_time,
}

with KernelClient() as client:
    session_id = client.session.create(
        SessionConfig(
            system_prompt="You have access to a calculator and a clock.",
            tools=tools_config,
        )
    )
    response = client.session.send_collect(
        session_id,
        "What is 123 * 456, and what time is it right now?",
        tools=tool_handlers,
    )
    print(response)
    client.session.destroy(session_id)
```

### Global tool registration

Register tools globally with the kernel so any session can use them:

```python
client.tool.register(weather_tool)

# List registered tools
for tool in client.tool.list():
    print(f"{tool.name}: {tool.description}")

# Remove a tool
client.tool.unregister("get_weather")
```

### Hot-reload

Watch a directory for tool script changes:

```python
client.tool.watch_dir("/path/to/tools")

# Manually trigger a reload
client.tool.reload("/path/to/tools/my_tool.py")
```

---

## Agent API

Agents are persistent, long-running entities backed by a session. Unlike one-shot sessions, agents can be steered repeatedly and announce capabilities for discovery.

### Spawn an agent

```python
from claw_kernel import AgentConfig, KernelClient

with KernelClient() as client:
    result = client.agent.spawn(
        AgentConfig(
            system_prompt="You are a data analysis agent.",
            provider="anthropic",
            model="claude-3-5-sonnet-20241022",
            max_turns=20,
        )
    )
    agent_id = result["agent_id"]
    session_id = result["session_id"]
    print(f"Agent: {agent_id}, Session: {session_id}")
```

`AgentConfig` fields:

| Field | Type | Description |
|-------|------|-------------|
| `system_prompt` | `str \| None` | Agent's system instruction |
| `provider` | `str \| None` | Provider name override |
| `model` | `str \| None` | Model name override |
| `max_turns` | `int \| None` | Max turns per steer call |
| `agent_id` | `str \| None` | Pre-assigned ID (UUID generated if omitted) |

### Steer an agent

Inject a message into the agent's conversation. The agent processes it asynchronously:

```python
client.agent.steer(agent_id, "Summarize: Q1 revenue $1.2M (+15% YoY), expenses $800K.")
```

### List agents

```python
agents = client.agent.list()
for agent in agents:
    print(f"{agent.agent_id} [{agent.status}] session={agent.session_id}")
```

### Announce capabilities

Agents can advertise what they can do, making them discoverable by other agents or orchestrators:

```python
client.agent.announce(agent_id, capabilities=["summarize", "analyze_data", "report"])
```

### Discover agents

```python
available = client.agent.discover()
for entry in available:
    print(f"{entry['agent_id']}: {entry.get('capabilities', [])}")
```

### Kill an agent

```python
client.agent.kill(agent_id)
```

### Full lifecycle example

```python
import time
from claw_kernel import AgentConfig, KernelClient

with KernelClient() as client:
    result = client.agent.spawn(
        AgentConfig(system_prompt="You are a monitoring agent.")
    )
    agent_id = result["agent_id"]

    client.agent.announce(agent_id, capabilities=["monitor", "alert"])

    # Steer it with a task
    client.agent.steer(agent_id, "Check system status and report any anomalies.")
    time.sleep(2)  # Give it time to process

    # Inspect
    for a in client.agent.list():
        if a.agent_id == agent_id:
            print(f"Status: {a.status}")

    client.agent.kill(agent_id)
```

---

## Channel API

Channels connect external message sources (webhooks, Discord, Slack, WebSocket clients) to agents. The routing layer decides which agent handles each inbound message.

### Register a channel

```python
from claw_kernel import ChannelConfig, KernelClient

with KernelClient() as client:
    result = client.channel.register(
        ChannelConfig(
            channel_type="webhook",
            channel_id="my-webhook",
            config={"description": "Main webhook endpoint"},
        )
    )
    print(result)  # {"channel_id": "my-webhook", ...}
```

### Create a managed channel

For channels the daemon manages directly (e.g. a WebSocket server):

```python
result = client.channel.create(
    session_id=session_id,
    channel_type="websocket",
    port=8765,
)
channel_id = result["channel_id"]
```

### Send and close

```python
client.channel.send(channel_id, "Hello from the server!")
client.channel.close(channel_id)
```

### Inbound messages

Route an inbound message through the channel pipeline to the matched agent:

```python
response = client.channel.inbound(
    channel_id="my-webhook",
    sender_id="user-001",
    content="Hello from webhook!",
    thread_id="thread-001",       # optional: for session continuity
    message_id="msg-xyz",         # optional: for deduplication
    metadata={"source": "slack"}, # optional: forwarded to the agent
)
```

### Broadcast (fan-out)

Send a message to all agents matched by routing rules:

```python
response = client.channel.broadcast(
    channel_id="announcements",
    sender_id="system",
    content="Deployment complete.",
)
```

### Routing rules

Route messages from a channel to a specific agent:

```python
# Route by channel
client.channel.route_add(
    agent_id=agent_id,
    rule_type="channel",
    channel_id="my-webhook",
)

# Route by sender
client.channel.route_add(
    agent_id=agent_id,
    rule_type="sender",
    sender_id="admin-user",
)

# Route by content pattern (regex)
client.channel.route_add(
    agent_id=agent_id,
    rule_type="pattern",
    pattern="^URGENT:.*",
)

# List all routing rules
rules = client.channel.route_list()
for rule in rules:
    print(rule)

# Remove all rules for an agent
client.channel.route_remove(agent_id)
```

### List and unregister

```python
channels = client.channel.list()
for ch in channels:
    print(ch)

client.channel.unregister("my-webhook")
```

---

## Trigger API

Triggers fire agents automatically based on time (cron), HTTP requests (webhook), or internal events.

### Cron trigger

```python
with KernelClient() as client:
    result = client.trigger.add_cron(
        trigger_id="hourly-report",
        target_agent=agent_id,
        cron_expr="0 * * * *",  # every hour
        message="Generate an hourly status report.",
    )
    print(result)
```

### Webhook trigger

The daemon exposes an HTTP endpoint. Requests to it fire the target agent:

```python
result = client.trigger.add_webhook(
    trigger_id="github-push",
    target_agent=agent_id,
    hmac_secret="my-secret-key",  # optional HMAC verification
)
print(result)  # includes the endpoint URL
```

### Event trigger

Fire an agent when an internal event matches a glob pattern:

```python
result = client.trigger.add_event(
    trigger_id="on-deploy",
    target_agent=agent_id,
    event_pattern="deploy.*",
    message="A deployment event occurred: {event.type}",
    condition={"env": "production"},  # optional filter
)
```

### List and remove

```python
triggers = client.trigger.list()
for t in triggers:
    print(t)

client.trigger.remove("hourly-report")
```

---

## Provider API

Register and manage LLM providers. The daemon supports multiple providers simultaneously.

### Register a provider

```python
with KernelClient() as client:
    client.provider.register(
        name="my-openai",
        provider_type="openai",
        api_key="sk-...",
        model="gpt-4o",
    )

    # Register an Anthropic provider
    client.provider.register(
        name="claude",
        provider_type="anthropic",
        api_key="sk-ant-...",
        model="claude-3-5-sonnet-20241022",
    )

    # Register a local provider with a custom base URL
    client.provider.register(
        name="local-llm",
        provider_type="openai",
        base_url="http://localhost:11434/v1",
        model="llama3.2",
    )
```

### List providers

```python
providers = client.provider.list()
for p in providers:
    print(f"{p.name} ({p.provider_type}): {p.model}")
```

### Use a specific provider in a session

```python
session_id = client.session.create(
    SessionConfig(provider_override="my-openai", model_override="gpt-4o-mini")
)
```

---

## Schedule API

Schedule recurring or one-shot prompts against a session.

### Create a scheduled task

```python
with KernelClient() as client:
    session_id = client.session.create(
        SessionConfig(system_prompt="You are a scheduled assistant.")
    )

    # Recurring: every minute
    task = client.schedule.create(
        session_id=session_id,
        cron="* * * * *",
        prompt="Provide a brief status update.",
        label="minutely-status",
    )
    print(f"Task {task.task_id} created (cron: {task.cron})")

    # One-shot
    oneshot = client.schedule.create(
        session_id=session_id,
        cron="once",
        prompt="Say hello once!",
        label="one-shot-hello",
    )
```

### List and cancel

```python
tasks = client.schedule.list(session_id)
for t in tasks:
    print(f"[{t.task_id}] {t.label or '(no label)'} | {t.cron} | {t.status}")

client.schedule.cancel(task.task_id)
```

`ScheduledTask` fields: `task_id`, `cron`, `label`, `status`.

---

## Skill API

Skills are reusable prompt templates or tool bundles loaded from the filesystem.

### Load skills from a directory

```python
with KernelClient() as client:
    client.skill.load_dir("/path/to/skills")
```

### List loaded skills

```python
skills = client.skill.list()
for skill in skills:
    print(f"{skill.name} v{skill.version}: {skill.description}")
```

### Get full skill content

```python
skill_data = client.skill.get_full("my-skill")
print(skill_data)  # dict with full content and metadata
```

---

## Audit API

Query the daemon's audit log for agent activity.

### List audit entries

```python
with KernelClient() as client:
    # Most recent 50 entries
    entries = client.audit.list(limit=50)

    # Filter by agent
    entries = client.audit.list(agent_id=agent_id, limit=100)

    # Filter by time (Unix ms)
    import time
    one_hour_ago = int((time.time() - 3600) * 1000)
    entries = client.audit.list(since_ms=one_hour_ago)

    for entry in entries:
        print(f"[{entry.timestamp_ms}] {entry.agent_id} {entry.event_type}", end="")
        if entry.tool_name:
            print(f" tool={entry.tool_name}", end="")
        if entry.error_code:
            print(f" error={entry.error_code}", end="")
        print()
```

`AuditEntry` fields: `timestamp_ms`, `agent_id`, `event_type`, `tool_name`, `args`, `error_code`.

---

## Async Client

`AsyncKernelClient` is a full asyncio counterpart to `KernelClient`. Every method is a coroutine, and `session.send()` is an async generator.

### Basic usage

```python
import asyncio
from claw_kernel import AsyncKernelClient, SessionConfig

async def main():
    async with AsyncKernelClient() as client:
        info = await client.info()
        print(f"Connected to claw-kernel v{info.version}")

        session_id = await client.session.create(
            SessionConfig(system_prompt="You are a creative storyteller.")
        )

        async for token in client.session.send(session_id, "Tell me a short story."):
            print(token, end="", flush=True)
        print()

        await client.session.destroy(session_id)

asyncio.run(main())
```

### Manual connection

If you can't use `async with`, connect and disconnect manually:

```python
client = AsyncKernelClient()
await client.connect()

# ... use client ...

await client.close()
```

### Async tool callbacks

The async client supports both sync and async tool handlers:

```python
import asyncio
import aiohttp
from claw_kernel import AsyncKernelClient, SessionConfig, ToolDef

async def fetch_url(url: str) -> str:
    async with aiohttp.ClientSession() as session:
        async with session.get(url) as resp:
            return await resp.text()

fetch_tool = ToolDef(
    name="fetch_url",
    description="Fetch the content of a URL",
    input_schema={
        "type": "object",
        "properties": {"url": {"type": "string"}},
        "required": ["url"],
    },
)

async def main():
    async with AsyncKernelClient() as client:
        session_id = await client.session.create(
            SessionConfig(tools=[fetch_tool])
        )
        response = await client.session.send_collect(
            session_id,
            "Fetch https://example.com and summarize it.",
            tools={"fetch_url": fetch_url},  # async function works directly
        )
        print(response)
        await client.session.destroy(session_id)

asyncio.run(main())
```

### Concurrent sessions

```python
async def chat(client, prompt: str) -> str:
    session_id = await client.session.create()
    result = await client.session.send_collect(session_id, prompt)
    await client.session.destroy(session_id)
    return result

async def main():
    async with AsyncKernelClient() as client:
        results = await asyncio.gather(
            chat(client, "What is Python?"),
            chat(client, "What is Rust?"),
            chat(client, "What is Go?"),
        )
        for r in results:
            print(r[:100])

asyncio.run(main())
```

### Async namespace methods

All namespaces have async equivalents. The signatures match the sync API exactly, with `await` added:

```python
async with AsyncKernelClient() as client:
    # Agent
    result = await client.agent.spawn(AgentConfig(...))
    await client.agent.steer(result["agent_id"], "Hello")
    await client.agent.kill(result["agent_id"])

    # Channel
    await client.channel.register(ChannelConfig(...))
    await client.channel.route_add(agent_id, rule_type="channel", channel_id="ch1")

    # Trigger
    await client.trigger.add_cron("t1", agent_id, "0 * * * *")
    await client.trigger.remove("t1")

    # Provider
    await client.provider.register("my-provider", "openai", api_key="sk-...")

    # Schedule
    task = await client.schedule.create(session_id, "* * * * *", "Status update")
    await client.schedule.cancel(task.task_id)

    # Skill
    await client.skill.load_dir("/path/to/skills")
    skills = await client.skill.list()

    # Audit
    entries = await client.audit.list(limit=20)
```

---

## Data Models

All models are plain `dataclasses` with no runtime dependencies. They map 1-to-1 to the daemon's JSON-RPC protocol types.

| Model | Key Fields | Used By |
|-------|-----------|---------|
| `SessionConfig` | `system_prompt`, `max_turns`, `provider_override`, `model_override`, `tools`, `persist_history` | `session.create()` |
| `AgentConfig` | `system_prompt`, `provider`, `model`, `max_turns`, `agent_id` | `agent.spawn()` |
| `ChannelConfig` | `channel_type`, `channel_id`, `config` | `channel.register()` |
| `TriggerConfig` | `trigger_id`, `trigger_type`, `target_agent`, `cron_expr`, `event_pattern`, `hmac_secret`, `message`, `condition` | Factory methods |
| `ToolDef` | `name`, `description`, `input_schema`, `permissions` | `session.create()`, `tool.register()` |
| `KernelInfo` | `version`, `protocol_version`, `providers`, `active_provider`, `active_model`, `features`, `max_sessions`, `current_sessions` | `client.info()` |
| `AgentInfo` | `agent_id`, `status`, `session_id` | `agent.list()` |
| `AuditEntry` | `timestamp_ms`, `agent_id`, `event_type`, `tool_name`, `args`, `error_code` | `audit.list()` |
| `ScheduledTask` | `task_id`, `cron`, `label`, `status` | `schedule.create()`, `schedule.list()` |
| `SkillInfo` | `name`, `description`, `version` | `skill.list()` |
| `ProviderInfo` | `name`, `provider_type`, `model` | `provider.list()` |

### TriggerConfig factory methods

`TriggerConfig` has three classmethods for type-safe construction:

```python
from claw_kernel import TriggerConfig

cron_cfg = TriggerConfig.cron(
    trigger_id="daily",
    target_agent=agent_id,
    cron_expr="0 9 * * *",
    message="Good morning! Here's your daily briefing.",
)

webhook_cfg = TriggerConfig.webhook(
    trigger_id="gh-push",
    target_agent=agent_id,
    hmac_secret="secret",
)

event_cfg = TriggerConfig.event(
    trigger_id="on-error",
    target_agent=agent_id,
    event_pattern="error.*",
    message="An error occurred: {event.type}",
)
```

---

## Error Handling

All SDK exceptions inherit from `ClawError`. The hierarchy:

```
ClawError
├── ConnectionError          # Socket disconnect or connection failure
├── AuthenticationError      # kernel.auth handshake rejected
├── FrameTooLargeError       # Frame exceeds 16 MiB limit
└── RpcError                 # JSON-RPC error response from the daemon
    ├── SessionNotFoundError     # code -32000: session doesn't exist
    ├── MaxSessionsReachedError  # code -32001: session limit hit
    ├── ProviderError            # code -32002: LLM provider error
    ├── AgentError               # code -32003: agent loop error
    └── ProviderNotFoundError    # code -32005: provider not registered
```

### Catching specific errors

```python
from claw_kernel import (
    KernelClient,
    SessionConfig,
    ClawError,
    ConnectionError,
    SessionNotFoundError,
    MaxSessionsReachedError,
    ProviderError,
    ProviderNotFoundError,
    RpcError,
)

with KernelClient() as client:
    try:
        session_id = client.session.create(
            SessionConfig(provider_override="nonexistent")
        )
    except ProviderNotFoundError:
        print("That provider isn't registered. Use client.provider.list() to see options.")
    except MaxSessionsReachedError:
        print("Session limit reached. Destroy an existing session first.")

    try:
        for token in client.session.send("bad-session-id", "Hello"):
            print(token, end="")
    except SessionNotFoundError:
        print("Session expired or was never created.")

    try:
        for token in client.session.send(session_id, "Write a novel"):
            print(token, end="")
    except ProviderError as exc:
        print(f"Provider error (code {exc.code}): {exc.message}")
```

### Connection errors

```python
from claw_kernel import KernelClient, ConnectionError as ClawConnectionError

try:
    client = KernelClient(socket_path="/nonexistent/path.sock")
except ClawConnectionError as exc:
    print(f"Could not connect: {exc}")
```

Note: `claw_kernel.ConnectionError` shadows the built-in `ConnectionError`. Import it explicitly if you need both.

### Catching all SDK errors

```python
from claw_kernel import ClawError

try:
    # any SDK call
    ...
except ClawError as exc:
    print(f"SDK error: {exc}")
```

### RpcError attributes

`RpcError` (and all its subclasses) expose:

| Attribute | Type | Description |
|-----------|------|-------------|
| `code` | `int` | JSON-RPC error code |
| `message` | `str` | Server-provided error message |
| `data` | `Any` | Optional additional error data |

---

## Configuration

| Environment Variable | Description | Default |
|----------------------|-------------|---------|
| `CLAW_SOCKET_PATH` | Override the Unix socket path | Platform default |
| `CLAW_DATA_DIR` | Override the data directory | Platform default |

Platform defaults for `CLAW_DATA_DIR`:

| Platform | Path |
|----------|------|
| macOS | `~/Library/Application Support/claw-kernel` |
| Linux | `$XDG_RUNTIME_DIR/claw` or `~/.local/share/claw-kernel` |
| Windows | `%LOCALAPPDATA%\claw-kernel` |

The socket path is always `<data_dir>/kernel.sock` unless overridden.

---

## Backward Compatibility

The original flat API is still available on `KernelClient` for migration purposes. These methods emit `DeprecationWarning`:

```python
# Old API (deprecated)
session_id = client.create_session(system_prompt="Be helpful.", model="gpt-4o")
for token in client.send_message(session_id, "Hello"):
    print(token, end="")
client.destroy_session(session_id)
```

### Migration guide

| Old method | New equivalent |
|------------|----------------|
| `client.create_session(system_prompt=..., model=..., max_turns=...)` | `client.session.create(SessionConfig(system_prompt=..., model_override=..., max_turns=...))` |
| `client.send_message(session_id, content, tools=...)` | `client.session.send(session_id, content, tools=...)` |
| `client.destroy_session(session_id)` | `client.session.destroy(session_id)` |

---

## Running Tests

```bash
cd sdk/python
pip install -e ".[dev]"
pytest
```

The test suite has 36 tests covering all namespaces, error handling, and transport behavior.

---

## Examples

All examples are in the `examples/` directory and can be run directly with `python <file>`.

| File | Description |
|------|-------------|
| `basic_chat.py` | Interactive sync chat loop with streaming output |
| `tool_use.py` | External tool callbacks (calculator + clock) |
| `async_streaming.py` | Async token streaming with `AsyncKernelClient` |
| `agent_spawn.py` | Persistent agent lifecycle: spawn, announce, steer, kill |
| `channel_webhook.py` | Channel registration, routing rules, and webhook trigger |
| `scheduler_demo.py` | Cron and one-shot scheduled tasks |
