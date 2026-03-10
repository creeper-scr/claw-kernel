"""
claw-kernel SDK — Asynchronous client with namespace API (asyncio).

Provides :class:`AsyncKernelClient`, a full asyncio counterpart to
:class:`~claw_kernel.client.KernelClient`.

Usage example::

    import asyncio
    from claw_kernel import AsyncKernelClient, SessionConfig

    async def main():
        async with AsyncKernelClient() as client:
            info = await client.info()
            print(info)

            session_id = await client.session.create(
                SessionConfig(system_prompt="You are helpful.")
            )
            async for token in client.session.send(session_id, "Hello!"):
                print(token, end="", flush=True)
            await client.session.destroy(session_id)

    asyncio.run(main())
"""

from __future__ import annotations

import asyncio
import json
from typing import Any, AsyncGenerator, AsyncIterator, Callable, Dict, List, Optional

from ._auth import ClawPaths, read_token
from .async_transport import AsyncTransport
from .errors import (
    AuthenticationError,
    ConnectionError as ClawConnectionError,
    _rpc_error_from_code,
)
from .models import (
    AgentConfig,
    AgentInfo,
    AuditEntry,
    ChannelConfig,
    KernelInfo,
    ProviderInfo,
    ScheduledTask,
    SessionConfig,
    SkillInfo,
    ToolDef,
)


# ---------------------------------------------------------------------------
# Async RPC layer (internal)
# ---------------------------------------------------------------------------


class _AsyncRpc:
    """Internal asyncio JSON-RPC 2.0 call layer.

    Uses per-session :class:`asyncio.Queue` objects for notifications.
    All operations are coroutine-safe within a single event loop.

    Args:
        transport: Connected :class:`AsyncTransport`.
    """

    def __init__(self, transport: AsyncTransport) -> None:
        self._transport = transport
        self._req_id: int = 1
        self._id_lock = asyncio.Lock()
        self._notif_queues: Dict[str, asyncio.Queue] = {}  # type: ignore[type-arg]
        # Pending responses keyed by request id.
        self._pending: Dict[int, asyncio.Future] = {}  # type: ignore[type-arg]
        self._reader_task: Optional[asyncio.Task] = None  # type: ignore[type-arg]

    async def start(self) -> None:
        """Start the background frame reader task."""
        self._reader_task = asyncio.ensure_future(self._reader_loop())

    async def stop(self) -> None:
        """Stop the background frame reader task."""
        if self._reader_task is not None and not self._reader_task.done():
            self._reader_task.cancel()
            try:
                await self._reader_task
            except (asyncio.CancelledError, Exception):
                pass
            self._reader_task = None

    async def _reader_loop(self) -> None:
        """Continuously read frames and route them to futures / queues."""
        try:
            while True:
                raw = await self._transport.recv_frame()
                msg = json.loads(raw)
                self._dispatch(msg)
        except (ClawConnectionError, asyncio.CancelledError):
            # Resolve all pending futures with an exception.
            exc = ClawConnectionError("Connection closed")
            for fut in list(self._pending.values()):
                if not fut.done():
                    fut.set_exception(exc)
            raise

    def _dispatch(self, msg: dict) -> None:
        """Route a received message to the correct future or queue."""
        msg_id = msg.get("id")
        if msg_id is not None:
            # It's a response.
            fut = self._pending.pop(msg_id, None)
            if fut is not None and not fut.done():
                fut.set_result(msg)
        elif "method" in msg:
            # It's a notification.
            params = msg.get("params") or {}
            session_id = params.get("session_id", "")
            if session_id and session_id in self._notif_queues:
                self._notif_queues[session_id].put_nowait(msg)

    async def _next_id(self) -> int:
        async with self._id_lock:
            req_id = self._req_id
            self._req_id += 1
        return req_id

    async def authenticate(self) -> None:
        """Perform ``kernel.auth`` handshake."""
        token = read_token()
        result = await self.call("kernel.auth", {"token": token})
        if not result.get("ok"):
            raise AuthenticationError("kernel.auth failed: server rejected the token")

    async def call(
        self,
        method: str,
        params: Optional[Dict[str, Any]] = None,
    ) -> Any:
        """Send a JSON-RPC request and await the response.

        Args:
            method: Method name.
            params: Optional parameters.

        Returns:
            The ``result`` value from the server.

        Raises:
            RpcError: On server error response.
            ConnectionError: On transport error.
        """
        req_id = await self._next_id()
        request: Dict[str, Any] = {
            "jsonrpc": "2.0",
            "method": method,
            "id": req_id,
        }
        if params is not None:
            request["params"] = params

        loop = asyncio.get_event_loop()
        fut: asyncio.Future = loop.create_future()  # type: ignore[type-arg]
        self._pending[req_id] = fut

        await self._transport.send_frame(json.dumps(request).encode("utf-8"))

        try:
            msg = await fut
        except ClawConnectionError:
            raise

        if "error" in msg:
            err = msg["error"]
            raise _rpc_error_from_code(
                err["code"],
                err.get("message", ""),
                err.get("data"),
            )
        return msg.get("result", {})

    def _get_or_create_queue(self, session_id: str) -> "asyncio.Queue[dict]":
        if session_id not in self._notif_queues:
            self._notif_queues[session_id] = asyncio.Queue()
        return self._notif_queues[session_id]

    def drop_queue(self, session_id: str) -> None:
        """Remove the notification queue for a session."""
        self._notif_queues.pop(session_id, None)

    async def subscribe(self, session_id: str) -> AsyncGenerator[dict, None]:
        """Async generator yielding notifications for *session_id*.

        Args:
            session_id: Session to subscribe to.

        Yields:
            Notification dicts from the server.
        """
        q = self._get_or_create_queue(session_id)
        while True:
            notification = await q.get()
            yield notification


# ---------------------------------------------------------------------------
# Async namespace base
# ---------------------------------------------------------------------------


class _AsyncNamespace:
    def __init__(self, rpc: _AsyncRpc) -> None:
        self._rpc = rpc


# ---------------------------------------------------------------------------
# AsyncSessionNamespace
# ---------------------------------------------------------------------------


class AsyncSessionNamespace(_AsyncNamespace):
    """Async methods under ``client.session.*``."""

    async def create(self, config: Optional[SessionConfig] = None) -> str:
        """Create a new session. Returns the ``session_id``."""
        params: Dict[str, Any] = {}
        if config is not None:
            params["config"] = config.to_dict()
        result = await self._rpc.call("createSession", params or None)
        return result["session_id"]

    async def send(
        self,
        session_id: str,
        content: str,
        tools: Optional[Dict[str, Callable[..., Any]]] = None,
    ) -> AsyncGenerator[str, None]:
        """Stream response tokens for *content* in *session_id*.

        Yields:
            Text delta strings.
        """
        await self._rpc.call(
            "sendMessage",
            {"session_id": session_id, "content": content},
        )

        async for notification in self._rpc.subscribe(session_id):
            method = notification.get("method", "")
            params = notification.get("params", {}) or {}

            if method == "agent/streamChunk":
                delta = params.get("delta", "")
                if delta:
                    yield delta
                if params.get("done"):
                    break

            elif method == "agent/toolCall":
                if tools:
                    tool_name = params.get("tool_name", "")
                    tool_call_id = params.get("tool_call_id", "")
                    arguments = params.get("arguments", {})
                    tool_fn = tools.get(tool_name)
                    if tool_fn is not None:
                        try:
                            if asyncio.iscoroutinefunction(tool_fn):
                                result = await tool_fn(**arguments)
                            else:
                                result = tool_fn(**arguments)
                            success = True
                        except Exception as exc:  # noqa: BLE001
                            result = str(exc)
                            success = False
                    else:
                        result = f"Unknown tool: {tool_name}"
                        success = False
                    await self._rpc.call(
                        "toolResult",
                        {
                            "session_id": session_id,
                            "tool_call_id": tool_call_id,
                            "result": result,
                            "success": success,
                        },
                    )

            elif method == "agent/finish":
                break

        self._rpc.drop_queue(session_id)

    async def send_collect(
        self,
        session_id: str,
        content: str,
        tools: Optional[Dict[str, Callable[..., Any]]] = None,
    ) -> str:
        """Return the complete response as a single string."""
        parts: List[str] = []
        async for token in self.send(session_id, content, tools=tools):
            parts.append(token)
        return "".join(parts)

    async def destroy(self, session_id: str) -> None:
        """Destroy a session."""
        await self._rpc.call("destroySession", {"session_id": session_id})
        self._rpc.drop_queue(session_id)

    async def tool_result(
        self,
        session_id: str,
        tool_call_id: str,
        result: Any,
        success: bool = True,
    ) -> None:
        """Send a tool result back to the agent."""
        await self._rpc.call(
            "toolResult",
            {
                "session_id": session_id,
                "tool_call_id": tool_call_id,
                "result": result,
                "success": success,
            },
        )


# ---------------------------------------------------------------------------
# AsyncAgentNamespace
# ---------------------------------------------------------------------------


class AsyncAgentNamespace(_AsyncNamespace):
    """Async methods under ``client.agent.*``."""

    async def spawn(self, config: AgentConfig) -> Dict[str, str]:
        result = await self._rpc.call("agent.spawn", config.to_dict())
        return {
            "agent_id": result.get("agent_id", ""),
            "session_id": result.get("session_id", ""),
        }

    async def kill(self, agent_id: str) -> None:
        await self._rpc.call("agent.kill", {"agent_id": agent_id})

    async def steer(self, agent_id: str, message: str) -> None:
        await self._rpc.call("agent.steer", {"agent_id": agent_id, "message": message})

    async def list(self) -> List[AgentInfo]:
        result = await self._rpc.call("agent.list")
        return [AgentInfo.from_dict(a) for a in result.get("agents", [])]

    async def announce(self, agent_id: str, capabilities: List[str]) -> None:
        await self._rpc.call(
            "agent.announce",
            {"agent_id": agent_id, "capabilities": capabilities},
        )

    async def discover(self) -> List[Dict[str, Any]]:
        result = await self._rpc.call("agent.discover")
        return result.get("agents", [])


# ---------------------------------------------------------------------------
# AsyncToolNamespace
# ---------------------------------------------------------------------------


class AsyncToolNamespace(_AsyncNamespace):
    """Async methods under ``client.tool.*``."""

    async def register(self, tool: ToolDef) -> None:
        await self._rpc.call(
            "tool.register",
            {
                "name": tool.name,
                "description": tool.description,
                "schema": tool.input_schema,
                **({"permissions": tool.permissions} if tool.permissions else {}),
            },
        )

    async def unregister(self, name: str) -> None:
        await self._rpc.call("tool.unregister", {"name": name})

    async def list(self) -> List[ToolDef]:
        result = await self._rpc.call("tool.list")
        return [
            ToolDef(
                name=t.get("name", ""),
                description=t.get("description", ""),
                input_schema=t.get("schema", t.get("input_schema", {})),
                permissions=t.get("permissions"),
            )
            for t in result.get("tools", [])
        ]

    async def watch_dir(self, path: str) -> None:
        await self._rpc.call("tool.watch_dir", {"path": path})

    async def reload(self, path: str) -> None:
        await self._rpc.call("tool.reload", {"path": path})


# ---------------------------------------------------------------------------
# AsyncChannelNamespace
# ---------------------------------------------------------------------------


class AsyncChannelNamespace(_AsyncNamespace):
    """Async methods under ``client.channel.*``."""

    async def register(self, config: ChannelConfig) -> Dict[str, Any]:
        return await self._rpc.call("channel.register", config.to_dict())

    async def unregister(self, channel_id: str) -> None:
        await self._rpc.call("channel.unregister", {"channel_id": channel_id})

    async def list(self) -> List[Dict[str, Any]]:
        result = await self._rpc.call("channel.list")
        return result.get("channels", [])

    async def create(
        self,
        session_id: str,
        channel_type: str,
        port: Optional[int] = None,
    ) -> Dict[str, Any]:
        params: Dict[str, Any] = {
            "session_id": session_id,
            "channel_type": channel_type,
        }
        if port is not None:
            params["port"] = port
        return await self._rpc.call("channel.create", params)

    async def send(self, channel_id: str, message: str) -> None:
        await self._rpc.call(
            "channel.send", {"channel_id": channel_id, "message": message}
        )

    async def close(self, channel_id: str) -> None:
        await self._rpc.call("channel.close", {"channel_id": channel_id})

    async def inbound(
        self,
        channel_id: str,
        sender_id: str,
        content: str,
        thread_id: Optional[str] = None,
        message_id: Optional[str] = None,
        metadata: Optional[Dict[str, Any]] = None,
    ) -> Dict[str, Any]:
        params: Dict[str, Any] = {
            "channel_id": channel_id,
            "sender_id": sender_id,
            "content": content,
        }
        if thread_id is not None:
            params["thread_id"] = thread_id
        if message_id is not None:
            params["message_id"] = message_id
        if metadata is not None:
            params["metadata"] = metadata
        return await self._rpc.call("channel.inbound", params)

    async def broadcast(
        self,
        channel_id: str,
        sender_id: str,
        content: str,
        thread_id: Optional[str] = None,
        message_id: Optional[str] = None,
        metadata: Optional[Dict[str, Any]] = None,
    ) -> Dict[str, Any]:
        params: Dict[str, Any] = {
            "channel_id": channel_id,
            "sender_id": sender_id,
            "content": content,
        }
        if thread_id is not None:
            params["thread_id"] = thread_id
        if message_id is not None:
            params["message_id"] = message_id
        if metadata is not None:
            params["metadata"] = metadata
        return await self._rpc.call("channel.broadcast", params)

    async def route_add(
        self,
        agent_id: str,
        rule_type: str = "channel",
        channel_id: Optional[str] = None,
        sender_id: Optional[str] = None,
        pattern: Optional[str] = None,
    ) -> Dict[str, Any]:
        params: Dict[str, Any] = {"rule_type": rule_type, "agent_id": agent_id}
        if channel_id is not None:
            params["channel_id"] = channel_id
        if sender_id is not None:
            params["sender_id"] = sender_id
        if pattern is not None:
            params["pattern"] = pattern
        return await self._rpc.call("channel.route_add", params)

    async def route_remove(self, agent_id: str) -> Dict[str, Any]:
        return await self._rpc.call("channel.route_remove", {"agent_id": agent_id})

    async def route_list(self) -> List[Dict[str, Any]]:
        result = await self._rpc.call("channel.route_list")
        return result.get("routes", [])


# ---------------------------------------------------------------------------
# AsyncTriggerNamespace
# ---------------------------------------------------------------------------


class AsyncTriggerNamespace(_AsyncNamespace):
    """Async methods under ``client.trigger.*``."""

    async def add_cron(
        self,
        trigger_id: str,
        target_agent: str,
        cron_expr: str,
        message: Optional[str] = None,
    ) -> Dict[str, Any]:
        params: Dict[str, Any] = {
            "trigger_id": trigger_id,
            "target_agent": target_agent,
            "cron_expr": cron_expr,
        }
        if message is not None:
            params["message"] = message
        return await self._rpc.call("trigger.add_cron", params)

    async def add_webhook(
        self,
        trigger_id: str,
        target_agent: str,
        hmac_secret: Optional[str] = None,
    ) -> Dict[str, Any]:
        params: Dict[str, Any] = {
            "trigger_id": trigger_id,
            "target_agent": target_agent,
        }
        if hmac_secret is not None:
            params["hmac_secret"] = hmac_secret
        return await self._rpc.call("trigger.add_webhook", params)

    async def add_event(
        self,
        trigger_id: str,
        target_agent: str,
        event_pattern: str,
        message: Optional[str] = None,
        condition: Optional[Dict[str, Any]] = None,
    ) -> Dict[str, Any]:
        params: Dict[str, Any] = {
            "trigger_id": trigger_id,
            "target_agent": target_agent,
            "event_pattern": event_pattern,
        }
        if message is not None:
            params["message"] = message
        if condition is not None:
            params["condition"] = condition
        return await self._rpc.call("trigger.add_event", params)

    async def remove(self, trigger_id: str) -> None:
        await self._rpc.call("trigger.remove", {"trigger_id": trigger_id})

    async def list(self) -> List[Dict[str, Any]]:
        result = await self._rpc.call("trigger.list")
        return result.get("triggers", [])


# ---------------------------------------------------------------------------
# AsyncProviderNamespace
# ---------------------------------------------------------------------------


class AsyncProviderNamespace(_AsyncNamespace):
    """Async methods under ``client.provider.*``."""

    async def register(
        self,
        name: str,
        provider_type: str,
        api_key: Optional[str] = None,
        base_url: Optional[str] = None,
        model: Optional[str] = None,
    ) -> None:
        params: Dict[str, Any] = {
            "name": name,
            "provider_type": provider_type,
        }
        if api_key is not None:
            params["api_key"] = api_key
        if base_url is not None:
            params["base_url"] = base_url
        if model is not None:
            params["model"] = model
        await self._rpc.call("provider.register", params)

    async def list(self) -> List[ProviderInfo]:
        result = await self._rpc.call("provider.list")
        return [ProviderInfo.from_dict(p) for p in result.get("providers", [])]


# ---------------------------------------------------------------------------
# AsyncScheduleNamespace
# ---------------------------------------------------------------------------


class AsyncScheduleNamespace(_AsyncNamespace):
    """Async methods under ``client.schedule.*``."""

    async def create(
        self,
        session_id: str,
        cron: str,
        prompt: str,
        label: Optional[str] = None,
    ) -> ScheduledTask:
        params: Dict[str, Any] = {
            "session_id": session_id,
            "cron": cron,
            "prompt": prompt,
        }
        if label is not None:
            params["label"] = label
        result = await self._rpc.call("schedule.create", params)
        return ScheduledTask.from_dict(result)

    async def cancel(self, task_id: str) -> None:
        await self._rpc.call("schedule.cancel", {"task_id": task_id})

    async def list(self, session_id: str) -> List[ScheduledTask]:
        result = await self._rpc.call("schedule.list", {"session_id": session_id})
        return [ScheduledTask.from_dict(t) for t in result.get("tasks", [])]


# ---------------------------------------------------------------------------
# AsyncSkillNamespace
# ---------------------------------------------------------------------------


class AsyncSkillNamespace(_AsyncNamespace):
    """Async methods under ``client.skill.*``."""

    async def load_dir(self, path: str) -> None:
        await self._rpc.call("skill.load_dir", {"path": path})

    async def list(self) -> List[SkillInfo]:
        result = await self._rpc.call("skill.list")
        return [SkillInfo.from_dict(s) for s in result.get("skills", [])]

    async def get_full(self, name: str) -> Dict[str, Any]:
        return await self._rpc.call("skill.get_full", {"name": name})


# ---------------------------------------------------------------------------
# AsyncAuditNamespace
# ---------------------------------------------------------------------------


class AsyncAuditNamespace(_AsyncNamespace):
    """Async methods under ``client.audit.*``."""

    async def list(
        self,
        limit: Optional[int] = None,
        agent_id: Optional[str] = None,
        since_ms: Optional[int] = None,
    ) -> List[AuditEntry]:
        params: Dict[str, Any] = {}
        if limit is not None:
            params["limit"] = limit
        if agent_id is not None:
            params["agent_id"] = agent_id
        if since_ms is not None:
            params["since_ms"] = since_ms
        result = await self._rpc.call("audit.list", params or None)
        return [AuditEntry.from_dict(e) for e in result.get("entries", [])]


# ---------------------------------------------------------------------------
# AsyncKernelClient
# ---------------------------------------------------------------------------


class AsyncKernelClient:
    """Asynchronous claw-kernel IPC client.

    Must be instantiated and then awaited via :meth:`connect` or used as an
    async context manager::

        async with AsyncKernelClient() as client:
            info = await client.info()

    Args:
        socket_path: Override the default Unix socket path.
    """

    def __init__(self, socket_path: Optional[str] = None) -> None:
        self._socket_path = socket_path or str(ClawPaths.socket_path())
        self._transport: Optional[AsyncTransport] = None
        self._rpc: Optional[_AsyncRpc] = None

        # Namespace singletons (created after connect).
        self._session_ns: Optional[AsyncSessionNamespace] = None
        self._agent_ns: Optional[AsyncAgentNamespace] = None
        self._tool_ns: Optional[AsyncToolNamespace] = None
        self._channel_ns: Optional[AsyncChannelNamespace] = None
        self._trigger_ns: Optional[AsyncTriggerNamespace] = None
        self._provider_ns: Optional[AsyncProviderNamespace] = None
        self._schedule_ns: Optional[AsyncScheduleNamespace] = None
        self._skill_ns: Optional[AsyncSkillNamespace] = None
        self._audit_ns: Optional[AsyncAuditNamespace] = None

    async def connect(self) -> None:
        """Open the connection and authenticate.

        Called automatically by ``async with AsyncKernelClient():``.
        """
        transport = AsyncTransport(self._socket_path)
        await transport.connect()
        self._transport = transport
        rpc = _AsyncRpc(transport)
        await rpc.start()
        await rpc.authenticate()
        self._rpc = rpc

    async def close(self) -> None:
        """Close the connection."""
        if self._rpc is not None:
            await self._rpc.stop()
            self._rpc = None
        if self._transport is not None:
            await self._transport.close()
            self._transport = None

    # ------------------------------------------------------------------
    # Namespace properties
    # ------------------------------------------------------------------

    def _require_rpc(self) -> _AsyncRpc:
        if self._rpc is None:
            raise ClawConnectionError(
                "AsyncKernelClient is not connected. "
                "Call await client.connect() or use async with."
            )
        return self._rpc

    @property
    def session(self) -> AsyncSessionNamespace:
        if self._session_ns is None:
            self._session_ns = AsyncSessionNamespace(self._require_rpc())
        return self._session_ns

    @property
    def agent(self) -> AsyncAgentNamespace:
        if self._agent_ns is None:
            self._agent_ns = AsyncAgentNamespace(self._require_rpc())
        return self._agent_ns

    @property
    def tool(self) -> AsyncToolNamespace:
        if self._tool_ns is None:
            self._tool_ns = AsyncToolNamespace(self._require_rpc())
        return self._tool_ns

    @property
    def channel(self) -> AsyncChannelNamespace:
        if self._channel_ns is None:
            self._channel_ns = AsyncChannelNamespace(self._require_rpc())
        return self._channel_ns

    @property
    def trigger(self) -> AsyncTriggerNamespace:
        if self._trigger_ns is None:
            self._trigger_ns = AsyncTriggerNamespace(self._require_rpc())
        return self._trigger_ns

    @property
    def provider(self) -> AsyncProviderNamespace:
        if self._provider_ns is None:
            self._provider_ns = AsyncProviderNamespace(self._require_rpc())
        return self._provider_ns

    @property
    def schedule(self) -> AsyncScheduleNamespace:
        if self._schedule_ns is None:
            self._schedule_ns = AsyncScheduleNamespace(self._require_rpc())
        return self._schedule_ns

    @property
    def skill(self) -> AsyncSkillNamespace:
        if self._skill_ns is None:
            self._skill_ns = AsyncSkillNamespace(self._require_rpc())
        return self._skill_ns

    @property
    def audit(self) -> AsyncAuditNamespace:
        if self._audit_ns is None:
            self._audit_ns = AsyncAuditNamespace(self._require_rpc())
        return self._audit_ns

    # ------------------------------------------------------------------
    # Top-level convenience methods
    # ------------------------------------------------------------------

    async def info(self) -> KernelInfo:
        """Return kernel server information."""
        result = await self._require_rpc().call("kernel.info")
        return KernelInfo.from_dict(result)

    async def ping(self) -> bool:
        """Ping the daemon. Returns ``True`` on success."""
        result = await self._require_rpc().call("kernel.ping")
        return bool(result.get("pong", False))

    # ------------------------------------------------------------------
    # Async context manager
    # ------------------------------------------------------------------

    async def __aenter__(self) -> "AsyncKernelClient":
        await self.connect()
        return self

    async def __aexit__(self, *_: object) -> None:
        await self.close()
