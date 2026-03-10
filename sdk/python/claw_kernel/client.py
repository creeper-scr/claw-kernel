"""
claw-kernel SDK — Synchronous client with namespace API.

The primary public class is :class:`KernelClient`.  It provides a clean,
namespace-organized API for every IPC method exposed by the kernel daemon.

Usage example::

    from claw_kernel import KernelClient, SessionConfig

    with KernelClient() as client:
        print(client.info())

        session_id = client.session.create(SessionConfig(system_prompt="You are helpful."))
        for token in client.session.send(session_id, "Hello!"):
            print(token, end="", flush=True)
        client.session.destroy(session_id)

Backward-compatible methods (``create_session``, ``send_message``, etc.) are
also available directly on :class:`KernelClient` to ease migration from the
previous reference implementation.
"""

from __future__ import annotations

import warnings
from typing import Any, Callable, Dict, Iterator, List, Optional

from ._auth import ClawPaths
from ._rpc import SyncRpc
from ._transport import SyncTransport
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
    TriggerConfig,
)


# ---------------------------------------------------------------------------
# Namespace helpers
# ---------------------------------------------------------------------------


class _Namespace:
    """Base class for all namespace objects."""

    def __init__(self, rpc: SyncRpc) -> None:
        self._rpc = rpc


# ---------------------------------------------------------------------------
# SessionNamespace
# ---------------------------------------------------------------------------


class SessionNamespace(_Namespace):
    """Methods under ``client.session.*``."""

    def create(self, config: Optional[SessionConfig] = None) -> str:
        """Create a new conversation session.

        Args:
            config: Optional :class:`~claw_kernel.models.SessionConfig`.

        Returns:
            The new ``session_id`` string.
        """
        params: Dict[str, Any] = {}
        if config is not None:
            params["config"] = config.to_dict()
        result = self._rpc.call("createSession", params or None)
        return result["session_id"]

    def send(
        self,
        session_id: str,
        content: str,
        tools: Optional[Dict[str, Callable[..., Any]]] = None,
    ) -> Iterator[str]:
        """Send a message and stream the response token-by-token.

        If *tools* is provided, tool-call notifications are handled
        transparently — ``toolResult`` is sent and streaming continues.

        Args:
            session_id: Target session ID.
            content: User message content.
            tools: Optional dict mapping tool names to Python callables.

        Yields:
            Text delta strings as they arrive from the server.
        """
        self._rpc.call(
            "sendMessage",
            {"session_id": session_id, "content": content},
        )

        for notification in self._rpc.subscribe(session_id):
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
                            result = tool_fn(**arguments)
                            success = True
                        except Exception as exc:  # noqa: BLE001
                            result = str(exc)
                            success = False
                    else:
                        result = f"Unknown tool: {tool_name}"
                        success = False
                    self._rpc.call(
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

    def send_collect(
        self,
        session_id: str,
        content: str,
        tools: Optional[Dict[str, Callable[..., Any]]] = None,
    ) -> str:
        """Send a message and return the complete response as a single string.

        Convenience wrapper around :meth:`send`.

        Args:
            session_id: Target session ID.
            content: User message content.
            tools: Optional tool dispatch dict.

        Returns:
            The full model response text.
        """
        return "".join(self.send(session_id, content, tools=tools))

    def destroy(self, session_id: str) -> None:
        """Destroy a session and release its server-side resources.

        Args:
            session_id: Session to destroy.
        """
        self._rpc.call("destroySession", {"session_id": session_id})
        self._rpc.drop_queue(session_id)

    def tool_result(
        self,
        session_id: str,
        tool_call_id: str,
        result: Any,
        success: bool = True,
    ) -> None:
        """Send a tool result back to the agent.

        Use this when you are driving the tool-call loop manually (i.e. not
        using the ``tools=`` parameter of :meth:`send`).

        Args:
            session_id: Session that issued the tool call.
            tool_call_id: ID of the tool call to respond to.
            result: The tool output (JSON-serialisable).
            success: Whether the tool executed successfully.
        """
        self._rpc.call(
            "toolResult",
            {
                "session_id": session_id,
                "tool_call_id": tool_call_id,
                "result": result,
                "success": success,
            },
        )


# ---------------------------------------------------------------------------
# AgentNamespace
# ---------------------------------------------------------------------------


class AgentNamespace(_Namespace):
    """Methods under ``client.agent.*``."""

    def spawn(self, config: AgentConfig) -> Dict[str, str]:
        """Spawn a persistent agent.

        Args:
            config: :class:`~claw_kernel.models.AgentConfig` parameters.

        Returns:
            Dict with ``agent_id`` and ``session_id`` keys.
        """
        result = self._rpc.call("agent.spawn", config.to_dict())
        return {
            "agent_id": result.get("agent_id", ""),
            "session_id": result.get("session_id", ""),
        }

    def kill(self, agent_id: str) -> None:
        """Terminate a running agent.

        Args:
            agent_id: Agent to kill.
        """
        self._rpc.call("agent.kill", {"agent_id": agent_id})

    def steer(self, agent_id: str, message: str) -> None:
        """Inject a message into a running agent's conversation.

        The agent will process the message asynchronously; stream output
        arrives as notifications on the agent's backing session.

        Args:
            agent_id: Target agent ID.
            message: Message to inject.
        """
        self._rpc.call("agent.steer", {"agent_id": agent_id, "message": message})

    def list(self) -> List[AgentInfo]:
        """Return a list of all registered agents.

        Returns:
            List of :class:`~claw_kernel.models.AgentInfo` objects.
        """
        result = self._rpc.call("agent.list")
        return [AgentInfo.from_dict(a) for a in result.get("agents", [])]

    def announce(self, agent_id: str, capabilities: List[str]) -> None:
        """Announce capability labels for an agent (for discovery).

        Args:
            agent_id: The announcing agent's ID.
            capabilities: List of capability strings.
        """
        self._rpc.call(
            "agent.announce",
            {"agent_id": agent_id, "capabilities": capabilities},
        )

    def discover(self) -> List[Dict[str, Any]]:
        """Discover agents and their announced capabilities.

        Returns:
            List of agent capability dicts with ``agent_id`` and
            ``capabilities`` keys.
        """
        result = self._rpc.call("agent.discover")
        return result.get("agents", [])


# ---------------------------------------------------------------------------
# ToolNamespace
# ---------------------------------------------------------------------------


class ToolNamespace(_Namespace):
    """Methods under ``client.tool.*``."""

    def register(self, tool: ToolDef) -> None:
        """Register a global tool with the kernel.

        Args:
            tool: :class:`~claw_kernel.models.ToolDef` to register.
        """
        self._rpc.call(
            "tool.register",
            {
                "name": tool.name,
                "description": tool.description,
                "schema": tool.input_schema,
                **({"permissions": tool.permissions} if tool.permissions else {}),
            },
        )

    def unregister(self, name: str) -> None:
        """Remove a globally registered tool.

        Args:
            name: Tool name to unregister.
        """
        self._rpc.call("tool.unregister", {"name": name})

    def list(self) -> List[ToolDef]:
        """List all globally registered tools.

        Returns:
            List of :class:`~claw_kernel.models.ToolDef` objects.
        """
        result = self._rpc.call("tool.list")
        tools = []
        for t in result.get("tools", []):
            tools.append(
                ToolDef(
                    name=t.get("name", ""),
                    description=t.get("description", ""),
                    input_schema=t.get("schema", t.get("input_schema", {})),
                    permissions=t.get("permissions"),
                )
            )
        return tools

    def watch_dir(self, path: str) -> None:
        """Watch a directory for tool hot-reload events.

        Args:
            path: Absolute path to monitor.
        """
        self._rpc.call("tool.watch_dir", {"path": path})

    def reload(self, path: str) -> None:
        """Manually trigger a hot-reload for a script file.

        Args:
            path: Absolute path of the script to reload.
        """
        self._rpc.call("tool.reload", {"path": path})


# ---------------------------------------------------------------------------
# ChannelNamespace
# ---------------------------------------------------------------------------


class ChannelNamespace(_Namespace):
    """Methods under ``client.channel.*``."""

    def register(self, config: ChannelConfig) -> Dict[str, Any]:
        """Register an external channel adapter.

        Args:
            config: :class:`~claw_kernel.models.ChannelConfig`.

        Returns:
            Server response dict (includes ``channel_id``).
        """
        return self._rpc.call("channel.register", config.to_dict())

    def unregister(self, channel_id: str) -> None:
        """Unregister a channel adapter.

        Args:
            channel_id: Channel to remove.
        """
        self._rpc.call("channel.unregister", {"channel_id": channel_id})

    def list(self) -> List[Dict[str, Any]]:
        """List all registered channels.

        Returns:
            List of channel info dicts.
        """
        result = self._rpc.call("channel.list")
        return result.get("channels", [])

    def create(
        self, session_id: str, channel_type: str, port: Optional[int] = None
    ) -> Dict[str, Any]:
        """Create a managed channel (e.g. WebSocket server).

        Args:
            session_id: Owning session ID.
            channel_type: Channel type string (e.g. ``"websocket"``).
            port: Optional port number.

        Returns:
            Server response dict (includes ``channel_id``).
        """
        params: Dict[str, Any] = {
            "session_id": session_id,
            "channel_type": channel_type,
        }
        if port is not None:
            params["port"] = port
        return self._rpc.call("channel.create", params)

    def send(self, channel_id: str, message: str) -> None:
        """Send a message to a channel.

        Args:
            channel_id: Target channel.
            message: Message content.
        """
        self._rpc.call("channel.send", {"channel_id": channel_id, "message": message})

    def close(self, channel_id: str) -> None:
        """Close a managed channel.

        Args:
            channel_id: Channel to close.
        """
        self._rpc.call("channel.close", {"channel_id": channel_id})

    def inbound(
        self,
        channel_id: str,
        sender_id: str,
        content: str,
        thread_id: Optional[str] = None,
        message_id: Optional[str] = None,
        metadata: Optional[Dict[str, Any]] = None,
    ) -> Dict[str, Any]:
        """Deliver an inbound message through the channel pipeline.

        Args:
            channel_id: Registered channel identifier.
            sender_id: Sender identifier (user ID, IP, etc.).
            content: Message content.
            thread_id: Optional thread ID for session continuity.
            message_id: Optional deduplication ID.
            metadata: Optional extra metadata forwarded to the agent.

        Returns:
            Server response dict.
        """
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
        return self._rpc.call("channel.inbound", params)

    def broadcast(
        self,
        channel_id: str,
        sender_id: str,
        content: str,
        thread_id: Optional[str] = None,
        message_id: Optional[str] = None,
        metadata: Optional[Dict[str, Any]] = None,
    ) -> Dict[str, Any]:
        """Broadcast a message to all routing-matched agents (fan-out).

        Args:
            channel_id: Channel used for routing rule evaluation.
            sender_id: Sender identifier.
            content: Message content.
            thread_id: Optional thread ID.
            message_id: Optional deduplication ID.
            metadata: Optional metadata.

        Returns:
            Server response dict.
        """
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
        return self._rpc.call("channel.broadcast", params)

    def route_add(
        self,
        agent_id: str,
        rule_type: str = "channel",
        channel_id: Optional[str] = None,
        sender_id: Optional[str] = None,
        pattern: Optional[str] = None,
    ) -> Dict[str, Any]:
        """Add a channel routing rule.

        Args:
            agent_id: Target agent for matched messages.
            rule_type: ``"channel"``, ``"sender"``, ``"pattern"``, or
                ``"default"``.
            channel_id: Channel to match (for ``"channel"`` rules).
            sender_id: Sender to match (for ``"sender"`` rules).
            pattern: Regex pattern (for ``"pattern"`` rules).

        Returns:
            Server response dict.
        """
        params: Dict[str, Any] = {
            "rule_type": rule_type,
            "agent_id": agent_id,
        }
        if channel_id is not None:
            params["channel_id"] = channel_id
        if sender_id is not None:
            params["sender_id"] = sender_id
        if pattern is not None:
            params["pattern"] = pattern
        return self._rpc.call("channel.route_add", params)

    def route_remove(self, agent_id: str) -> Dict[str, Any]:
        """Remove all routing rules targeting an agent.

        Args:
            agent_id: Agent whose rules should be removed.

        Returns:
            Server response dict.
        """
        return self._rpc.call("channel.route_remove", {"agent_id": agent_id})

    def route_list(self) -> List[Dict[str, Any]]:
        """List all channel routing rules.

        Returns:
            List of routing rule dicts.
        """
        result = self._rpc.call("channel.route_list")
        return result.get("routes", [])


# ---------------------------------------------------------------------------
# TriggerNamespace
# ---------------------------------------------------------------------------


class TriggerNamespace(_Namespace):
    """Methods under ``client.trigger.*``."""

    def add_cron(
        self,
        trigger_id: str,
        target_agent: str,
        cron_expr: str,
        message: Optional[str] = None,
    ) -> Dict[str, Any]:
        """Add a cron-based trigger.

        Args:
            trigger_id: Unique identifier for the trigger.
            target_agent: Agent to fire against.
            cron_expr: Cron expression (e.g. ``"0 * * * *"``).
            message: Optional message injected when the trigger fires.

        Returns:
            Server response dict.
        """
        params: Dict[str, Any] = {
            "trigger_id": trigger_id,
            "target_agent": target_agent,
            "cron_expr": cron_expr,
        }
        if message is not None:
            params["message"] = message
        return self._rpc.call("trigger.add_cron", params)

    def add_webhook(
        self,
        trigger_id: str,
        target_agent: str,
        hmac_secret: Optional[str] = None,
    ) -> Dict[str, Any]:
        """Add a webhook-based trigger.

        Args:
            trigger_id: Unique identifier.
            target_agent: Agent to fire against.
            hmac_secret: Optional HMAC secret for request verification.

        Returns:
            Server response dict (includes ``endpoint`` URL).
        """
        params: Dict[str, Any] = {
            "trigger_id": trigger_id,
            "target_agent": target_agent,
        }
        if hmac_secret is not None:
            params["hmac_secret"] = hmac_secret
        return self._rpc.call("trigger.add_webhook", params)

    def add_event(
        self,
        trigger_id: str,
        target_agent: str,
        event_pattern: str,
        message: Optional[str] = None,
        condition: Optional[Dict[str, Any]] = None,
    ) -> Dict[str, Any]:
        """Add an event-bus trigger.

        Args:
            trigger_id: Unique identifier.
            target_agent: Agent to fire against.
            event_pattern: Glob pattern matched against event type names.
            message: Optional message template (supports ``{event.type}``).
            condition: Optional condition filter dict.

        Returns:
            Server response dict.
        """
        params: Dict[str, Any] = {
            "trigger_id": trigger_id,
            "target_agent": target_agent,
            "event_pattern": event_pattern,
        }
        if message is not None:
            params["message"] = message
        if condition is not None:
            params["condition"] = condition
        return self._rpc.call("trigger.add_event", params)

    def remove(self, trigger_id: str) -> None:
        """Remove a trigger.

        Args:
            trigger_id: Trigger to remove.
        """
        self._rpc.call("trigger.remove", {"trigger_id": trigger_id})

    def list(self) -> List[Dict[str, Any]]:
        """List all active triggers.

        Returns:
            List of trigger info dicts.
        """
        result = self._rpc.call("trigger.list")
        return result.get("triggers", [])


# ---------------------------------------------------------------------------
# ProviderNamespace
# ---------------------------------------------------------------------------


class ProviderNamespace(_Namespace):
    """Methods under ``client.provider.*``."""

    def register(
        self,
        name: str,
        provider_type: str,
        api_key: Optional[str] = None,
        base_url: Optional[str] = None,
        model: Optional[str] = None,
    ) -> None:
        """Register an LLM provider with the kernel.

        Args:
            name: Name to register the provider under.
            provider_type: Type string (e.g. ``"anthropic"``, ``"openai"``).
            api_key: API key (if required by the provider).
            base_url: Base URL override.
            model: Default model name.
        """
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
        self._rpc.call("provider.register", params)

    def list(self) -> List[ProviderInfo]:
        """List all registered providers.

        Returns:
            List of :class:`~claw_kernel.models.ProviderInfo` objects.
        """
        result = self._rpc.call("provider.list")
        return [ProviderInfo.from_dict(p) for p in result.get("providers", [])]


# ---------------------------------------------------------------------------
# ScheduleNamespace
# ---------------------------------------------------------------------------


class ScheduleNamespace(_Namespace):
    """Methods under ``client.schedule.*``."""

    def create(
        self,
        session_id: str,
        cron: str,
        prompt: str,
        label: Optional[str] = None,
    ) -> ScheduledTask:
        """Create a scheduled task.

        Args:
            session_id: Session that will receive the scheduled prompt.
            cron: Cron expression or ``"once"`` for a one-shot task.
            prompt: Message/prompt to send on schedule.
            label: Optional human-readable label.

        Returns:
            :class:`~claw_kernel.models.ScheduledTask` info.
        """
        params: Dict[str, Any] = {
            "session_id": session_id,
            "cron": cron,
            "prompt": prompt,
        }
        if label is not None:
            params["label"] = label
        result = self._rpc.call("schedule.create", params)
        return ScheduledTask.from_dict(result)

    def cancel(self, task_id: str) -> None:
        """Cancel a scheduled task.

        Args:
            task_id: Task to cancel.
        """
        self._rpc.call("schedule.cancel", {"task_id": task_id})

    def list(self, session_id: str) -> List[ScheduledTask]:
        """List scheduled tasks for a session.

        Args:
            session_id: Session to query.

        Returns:
            List of :class:`~claw_kernel.models.ScheduledTask` objects.
        """
        result = self._rpc.call("schedule.list", {"session_id": session_id})
        return [ScheduledTask.from_dict(t) for t in result.get("tasks", [])]


# ---------------------------------------------------------------------------
# SkillNamespace
# ---------------------------------------------------------------------------


class SkillNamespace(_Namespace):
    """Methods under ``client.skill.*``."""

    def load_dir(self, path: str) -> None:
        """Load all skills from a directory.

        Args:
            path: Absolute path to the skills directory.
        """
        self._rpc.call("skill.load_dir", {"path": path})

    def list(self) -> List[SkillInfo]:
        """List all loaded skills.

        Returns:
            List of :class:`~claw_kernel.models.SkillInfo` objects.
        """
        result = self._rpc.call("skill.list")
        return [SkillInfo.from_dict(s) for s in result.get("skills", [])]

    def get_full(self, name: str) -> Dict[str, Any]:
        """Get the full content of a skill.

        Args:
            name: Skill name.

        Returns:
            Dict containing skill details and full content.
        """
        return self._rpc.call("skill.get_full", {"name": name})


# ---------------------------------------------------------------------------
# AuditNamespace
# ---------------------------------------------------------------------------


class AuditNamespace(_Namespace):
    """Methods under ``client.audit.*``."""

    def list(
        self,
        limit: Optional[int] = None,
        agent_id: Optional[str] = None,
        since_ms: Optional[int] = None,
    ) -> List[AuditEntry]:
        """Retrieve audit log entries.

        Args:
            limit: Maximum number of entries (most-recent-first).
            agent_id: Filter to a specific agent.
            since_ms: Only return entries at or after this Unix timestamp (ms).

        Returns:
            List of :class:`~claw_kernel.models.AuditEntry` objects.
        """
        params: Dict[str, Any] = {}
        if limit is not None:
            params["limit"] = limit
        if agent_id is not None:
            params["agent_id"] = agent_id
        if since_ms is not None:
            params["since_ms"] = since_ms
        result = self._rpc.call("audit.list", params or None)
        return [AuditEntry.from_dict(e) for e in result.get("entries", [])]


# ---------------------------------------------------------------------------
# KernelClient
# ---------------------------------------------------------------------------


class KernelClient:
    """Synchronous claw-kernel IPC client.

    Connects to the running daemon (or starts one if absent), authenticates,
    and exposes the full protocol surface through namespace properties.

    Usage::

        client = KernelClient()
        print(client.info())
        client.close()

        # Or use as a context manager:
        with KernelClient() as client:
            ...

    Args:
        socket_path: Override the default Unix socket path.
        auto_reconnect: Transparently reconnect on broken-pipe errors.
        max_retries: Maximum reconnect attempts.
    """

    def __init__(
        self,
        socket_path: Optional[str] = None,
        *,
        auto_reconnect: bool = True,
        max_retries: int = 3,
    ) -> None:
        self._transport = SyncTransport(
            socket_path,
            auto_reconnect=auto_reconnect,
            max_retries=max_retries,
        )
        self._rpc = SyncRpc(
            self._transport,
            auto_reconnect=auto_reconnect,
            max_retries=max_retries,
        )
        self._rpc.authenticate()

        # Lazily-created namespace singletons.
        self._session_ns: Optional[SessionNamespace] = None
        self._agent_ns: Optional[AgentNamespace] = None
        self._tool_ns: Optional[ToolNamespace] = None
        self._channel_ns: Optional[ChannelNamespace] = None
        self._trigger_ns: Optional[TriggerNamespace] = None
        self._provider_ns: Optional[ProviderNamespace] = None
        self._schedule_ns: Optional[ScheduleNamespace] = None
        self._skill_ns: Optional[SkillNamespace] = None
        self._audit_ns: Optional[AuditNamespace] = None

    # ------------------------------------------------------------------
    # Namespace properties
    # ------------------------------------------------------------------

    @property
    def session(self) -> SessionNamespace:
        """Session management namespace."""
        if self._session_ns is None:
            self._session_ns = SessionNamespace(self._rpc)
        return self._session_ns

    @property
    def agent(self) -> AgentNamespace:
        """Agent lifecycle namespace."""
        if self._agent_ns is None:
            self._agent_ns = AgentNamespace(self._rpc)
        return self._agent_ns

    @property
    def tool(self) -> ToolNamespace:
        """Tool registry namespace."""
        if self._tool_ns is None:
            self._tool_ns = ToolNamespace(self._rpc)
        return self._tool_ns

    @property
    def channel(self) -> ChannelNamespace:
        """Channel management namespace."""
        if self._channel_ns is None:
            self._channel_ns = ChannelNamespace(self._rpc)
        return self._channel_ns

    @property
    def trigger(self) -> TriggerNamespace:
        """Trigger management namespace."""
        if self._trigger_ns is None:
            self._trigger_ns = TriggerNamespace(self._rpc)
        return self._trigger_ns

    @property
    def provider(self) -> ProviderNamespace:
        """Provider registry namespace."""
        if self._provider_ns is None:
            self._provider_ns = ProviderNamespace(self._rpc)
        return self._provider_ns

    @property
    def schedule(self) -> ScheduleNamespace:
        """Scheduler namespace."""
        if self._schedule_ns is None:
            self._schedule_ns = ScheduleNamespace(self._rpc)
        return self._schedule_ns

    @property
    def skill(self) -> SkillNamespace:
        """Skill management namespace."""
        if self._skill_ns is None:
            self._skill_ns = SkillNamespace(self._rpc)
        return self._skill_ns

    @property
    def audit(self) -> AuditNamespace:
        """Audit log namespace."""
        if self._audit_ns is None:
            self._audit_ns = AuditNamespace(self._rpc)
        return self._audit_ns

    # ------------------------------------------------------------------
    # Top-level convenience methods
    # ------------------------------------------------------------------

    def info(self) -> KernelInfo:
        """Return kernel server information.

        Returns:
            :class:`~claw_kernel.models.KernelInfo` populated from
            ``kernel.info``.
        """
        result = self._rpc.call("kernel.info")
        return KernelInfo.from_dict(result)

    def ping(self) -> bool:
        """Ping the daemon.

        Returns:
            ``True`` if the daemon responded with ``{"pong": true}``.
        """
        result = self._rpc.call("kernel.ping")
        return bool(result.get("pong", False))

    def close(self) -> None:
        """Close the IPC connection."""
        self._transport.close()

    # ------------------------------------------------------------------
    # Context manager support
    # ------------------------------------------------------------------

    def __enter__(self) -> "KernelClient":
        return self

    def __exit__(self, *_: object) -> None:
        self.close()

    # ------------------------------------------------------------------
    # Backward-compatible methods (deprecated, for migration from v0)
    # ------------------------------------------------------------------

    def create_session(
        self,
        system_prompt: str = "",
        tools: Optional[list] = None,
        model: Optional[str] = None,
        max_turns: Optional[int] = None,
    ) -> str:
        """Create a session.

        .. deprecated::
            Use ``client.session.create(SessionConfig(...))`` instead.
        """
        warnings.warn(
            "KernelClient.create_session() is deprecated; "
            "use client.session.create(SessionConfig(...)) instead.",
            DeprecationWarning,
            stacklevel=2,
        )
        config = SessionConfig(
            system_prompt=system_prompt or None,
            model_override=model,
            max_turns=max_turns,
        )
        return self.session.create(config)

    def send_message(
        self,
        session_id: str,
        content: str,
        tools: Optional[Dict[str, Callable[..., Any]]] = None,
    ) -> Iterator[str]:
        """Send a message and stream the response.

        .. deprecated::
            Use ``client.session.send(session_id, content)`` instead.
        """
        warnings.warn(
            "KernelClient.send_message() is deprecated; "
            "use client.session.send(session_id, content) instead.",
            DeprecationWarning,
            stacklevel=2,
        )
        return self.session.send(session_id, content, tools=tools)

    def destroy_session(self, session_id: str) -> None:
        """Destroy a session.

        .. deprecated::
            Use ``client.session.destroy(session_id)`` instead.
        """
        warnings.warn(
            "KernelClient.destroy_session() is deprecated; "
            "use client.session.destroy(session_id) instead.",
            DeprecationWarning,
            stacklevel=2,
        )
        self.session.destroy(session_id)
