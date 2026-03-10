"""
claw-kernel SDK — Data models (dataclasses, zero runtime dependencies).

All public types exposed by the SDK live here.  They map 1-to-1 to the
JSON-RPC protocol types defined in the Rust ``protocol.rs`` module.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, List, Optional


# ---------------------------------------------------------------------------
# Tool definition
# ---------------------------------------------------------------------------


@dataclass
class ToolDef:
    """Client-side external tool definition.

    Passed to :class:`~claw_kernel.models.SessionConfig` or registered
    globally via ``client.tool.register()``.

    Attributes:
        name: Tool name in ``snake_case``.
        description: Human-readable description shown to the LLM.
        input_schema: JSON Schema object describing the tool's parameters.
        permissions: Optional declared permission set (informational only;
            the kernel cannot enforce permissions in an external process).
    """

    name: str
    description: str
    input_schema: dict[str, Any]
    permissions: Optional[dict[str, Any]] = None

    def to_dict(self) -> dict[str, Any]:
        d: dict[str, Any] = {
            "name": self.name,
            "description": self.description,
            "input_schema": self.input_schema,
        }
        if self.permissions is not None:
            d["permissions"] = self.permissions
        return d


# ---------------------------------------------------------------------------
# Session configuration
# ---------------------------------------------------------------------------


@dataclass
class SessionConfig:
    """Configuration for a new conversation session.

    Attributes:
        system_prompt: System-level instruction prepended to the conversation.
        max_turns: Maximum number of agent loop turns (default: server decides).
        provider_override: Name of the provider to use (e.g. ``"openai"``).
        model_override: Model name override (e.g. ``"gpt-4o"``).
        tools: External tools the client will handle callbacks for.
        persist_history: If ``True``, the server persists history to SQLite.
    """

    system_prompt: Optional[str] = None
    max_turns: Optional[int] = None
    provider_override: Optional[str] = None
    model_override: Optional[str] = None
    tools: Optional[List[ToolDef]] = None
    persist_history: bool = False

    def to_dict(self) -> dict[str, Any]:
        d: dict[str, Any] = {}
        if self.system_prompt is not None:
            d["system_prompt"] = self.system_prompt
        if self.max_turns is not None:
            d["max_turns"] = self.max_turns
        if self.provider_override is not None:
            d["provider_override"] = self.provider_override
        if self.model_override is not None:
            d["model_override"] = self.model_override
        if self.tools:
            d["tools"] = [t.to_dict() for t in self.tools]
        if self.persist_history:
            d["persist_history"] = True
        return d


# ---------------------------------------------------------------------------
# Agent configuration
# ---------------------------------------------------------------------------


@dataclass
class AgentConfig:
    """Configuration for spawning a persistent agent.

    Attributes:
        system_prompt: System-level instruction for the agent.
        provider: Provider name override.
        model: Model name override.
        max_turns: Maximum turns per steer invocation.
        agent_id: Optional pre-assigned agent ID (UUID generated if omitted).
    """

    system_prompt: Optional[str] = None
    provider: Optional[str] = None
    model: Optional[str] = None
    max_turns: Optional[int] = None
    agent_id: Optional[str] = None

    def to_dict(self) -> dict[str, Any]:
        params: dict[str, Any] = {}
        config: dict[str, Any] = {}
        if self.agent_id is not None:
            params["agent_id"] = self.agent_id
        if self.system_prompt is not None:
            config["system_prompt"] = self.system_prompt
        if self.provider is not None:
            config["provider"] = self.provider
        if self.model is not None:
            config["model"] = self.model
        if self.max_turns is not None:
            config["max_turns"] = self.max_turns
        params["config"] = config
        return params


# ---------------------------------------------------------------------------
# Channel configuration
# ---------------------------------------------------------------------------


@dataclass
class ChannelConfig:
    """Configuration for registering an external channel adapter.

    Attributes:
        channel_type: Channel type string (``"webhook"``, ``"discord"``, etc.).
        channel_id: Unique identifier for this channel.
        config: Type-specific configuration dictionary.
    """

    channel_type: str
    channel_id: str
    config: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return {
            "type": self.channel_type,
            "channel_id": self.channel_id,
            "config": self.config,
        }


# ---------------------------------------------------------------------------
# Trigger configuration
# ---------------------------------------------------------------------------


@dataclass
class TriggerConfig:
    """Configuration for adding a trigger.

    Use the factory classmethods for type-safe construction.

    Attributes:
        trigger_id: Unique trigger identifier.
        trigger_type: ``"cron"``, ``"webhook"``, or ``"event"``.
        target_agent: Agent ID to fire against.
        cron_expr: Cron expression (for cron triggers).
        event_pattern: Glob pattern (for event triggers).
        hmac_secret: HMAC secret (for webhook triggers).
        message: Optional message injected when the trigger fires.
        condition: Optional event condition filter (for event triggers).
    """

    trigger_id: str
    trigger_type: str
    target_agent: str
    cron_expr: Optional[str] = None
    event_pattern: Optional[str] = None
    hmac_secret: Optional[str] = None
    message: Optional[str] = None
    condition: Optional[dict[str, Any]] = None

    @classmethod
    def cron(
        cls,
        trigger_id: str,
        target_agent: str,
        cron_expr: str,
        message: Optional[str] = None,
    ) -> "TriggerConfig":
        """Create a cron-based trigger."""
        return cls(
            trigger_id=trigger_id,
            trigger_type="cron",
            target_agent=target_agent,
            cron_expr=cron_expr,
            message=message,
        )

    @classmethod
    def webhook(
        cls,
        trigger_id: str,
        target_agent: str,
        hmac_secret: Optional[str] = None,
    ) -> "TriggerConfig":
        """Create a webhook-based trigger."""
        return cls(
            trigger_id=trigger_id,
            trigger_type="webhook",
            target_agent=target_agent,
            hmac_secret=hmac_secret,
        )

    @classmethod
    def event(
        cls,
        trigger_id: str,
        target_agent: str,
        event_pattern: str,
        message: Optional[str] = None,
        condition: Optional[dict[str, Any]] = None,
    ) -> "TriggerConfig":
        """Create an event-bus trigger."""
        return cls(
            trigger_id=trigger_id,
            trigger_type="event",
            target_agent=target_agent,
            event_pattern=event_pattern,
            message=message,
            condition=condition,
        )


# ---------------------------------------------------------------------------
# Result / info types
# ---------------------------------------------------------------------------


@dataclass
class KernelInfo:
    """Information returned by ``kernel.info``.

    Attributes:
        version: Kernel server version string.
        protocol_version: Protocol version number.
        providers: Available provider names.
        active_provider: Currently active default provider.
        active_model: Currently active default model.
        features: List of enabled features.
        max_sessions: Maximum number of concurrent sessions.
        current_sessions: Current number of active sessions.
    """

    version: str
    protocol_version: int
    providers: List[str]
    active_provider: str
    active_model: str
    features: List[str]
    max_sessions: int
    current_sessions: int

    @classmethod
    def from_dict(cls, d: dict[str, Any]) -> "KernelInfo":
        return cls(
            version=d.get("version", ""),
            protocol_version=d.get("protocol_version", 0),
            providers=d.get("providers", []),
            active_provider=d.get("active_provider", ""),
            active_model=d.get("active_model", ""),
            features=d.get("features", []),
            max_sessions=d.get("max_sessions", 0),
            current_sessions=d.get("current_sessions", 0),
        )


@dataclass
class AgentInfo:
    """Information about a running agent.

    Attributes:
        agent_id: Unique agent identifier.
        status: Current status string (e.g. ``"Running"``).
        session_id: Backing session ID (if available).
    """

    agent_id: str
    status: str
    session_id: Optional[str] = None

    @classmethod
    def from_dict(cls, d: dict[str, Any]) -> "AgentInfo":
        return cls(
            agent_id=d.get("agent_id", ""),
            status=d.get("status", ""),
            session_id=d.get("session_id"),
        )


@dataclass
class AuditEntry:
    """A single audit log entry.

    Attributes:
        timestamp_ms: Unix timestamp in milliseconds.
        agent_id: Agent that generated this event.
        event_type: Type of event (e.g. ``"tool_called"``).
        tool_name: Tool name (if applicable).
        args: Tool arguments (if applicable).
        error_code: Error code (if the event was an error).
    """

    timestamp_ms: int
    agent_id: str
    event_type: str
    tool_name: Optional[str] = None
    args: Optional[dict[str, Any]] = None
    error_code: Optional[int] = None

    @classmethod
    def from_dict(cls, d: dict[str, Any]) -> "AuditEntry":
        return cls(
            timestamp_ms=d.get("timestamp_ms", 0),
            agent_id=d.get("agent_id", ""),
            event_type=d.get("event_type", ""),
            tool_name=d.get("tool_name"),
            args=d.get("args"),
            error_code=d.get("error_code"),
        )


@dataclass
class ScheduledTask:
    """Information about a scheduled task.

    Attributes:
        task_id: Unique task identifier.
        cron: Cron expression or ``"once"``.
        label: Optional human-readable label.
        status: Task status string.
    """

    task_id: str
    cron: str
    label: Optional[str] = None
    status: str = "active"

    @classmethod
    def from_dict(cls, d: dict[str, Any]) -> "ScheduledTask":
        return cls(
            task_id=d.get("task_id", ""),
            cron=d.get("cron", ""),
            label=d.get("label"),
            status=d.get("status", "active"),
        )


@dataclass
class SkillInfo:
    """Metadata for a loaded skill.

    Attributes:
        name: Skill name.
        description: Short description.
        version: Version string.
    """

    name: str
    description: str = ""
    version: str = ""

    @classmethod
    def from_dict(cls, d: dict[str, Any]) -> "SkillInfo":
        return cls(
            name=d.get("name", ""),
            description=d.get("description", ""),
            version=d.get("version", ""),
        )


@dataclass
class ProviderInfo:
    """Information about a registered provider.

    Attributes:
        name: Provider name.
        provider_type: Provider type string.
        model: Default model name.
    """

    name: str
    provider_type: str
    model: str = ""

    @classmethod
    def from_dict(cls, d: dict[str, Any]) -> "ProviderInfo":
        return cls(
            name=d.get("name", ""),
            provider_type=d.get("provider_type", d.get("type", "")),
            model=d.get("model", ""),
        )
