"""Tests for data models (dataclasses)."""

from __future__ import annotations

from claw_kernel.models import (
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


class TestToolDef:
    def test_basic_serialisation(self):
        t = ToolDef(
            name="get_weather",
            description="Get weather",
            input_schema={"type": "object", "properties": {"city": {"type": "string"}}},
        )
        d = t.to_dict()
        assert d["name"] == "get_weather"
        assert d["description"] == "Get weather"
        assert "permissions" not in d

    def test_with_permissions(self):
        t = ToolDef(
            name="read_file",
            description="Read a file",
            input_schema={},
            permissions={"read": ["fs"]},
        )
        d = t.to_dict()
        assert d["permissions"] == {"read": ["fs"]}


class TestSessionConfig:
    def test_empty_config(self):
        cfg = SessionConfig()
        d = cfg.to_dict()
        assert d == {}

    def test_full_config(self):
        cfg = SessionConfig(
            system_prompt="You are helpful",
            max_turns=10,
            provider_override="openai",
            model_override="gpt-4o",
            persist_history=True,
        )
        d = cfg.to_dict()
        assert d["system_prompt"] == "You are helpful"
        assert d["max_turns"] == 10
        assert d["provider_override"] == "openai"
        assert d["model_override"] == "gpt-4o"
        assert d["persist_history"] is True

    def test_tools_serialisation(self):
        tool = ToolDef("calc", "Calculator", {"type": "object"})
        cfg = SessionConfig(tools=[tool])
        d = cfg.to_dict()
        assert len(d["tools"]) == 1
        assert d["tools"][0]["name"] == "calc"


class TestAgentConfig:
    def test_minimal(self):
        cfg = AgentConfig()
        d = cfg.to_dict()
        assert "config" in d
        assert d["config"] == {}

    def test_full(self):
        cfg = AgentConfig(
            system_prompt="You are an agent",
            provider="anthropic",
            model="claude-sonnet-4-6",
            max_turns=5,
            agent_id="my-agent",
        )
        d = cfg.to_dict()
        assert d["agent_id"] == "my-agent"
        assert d["config"]["system_prompt"] == "You are an agent"
        assert d["config"]["provider"] == "anthropic"
        assert d["config"]["model"] == "claude-sonnet-4-6"
        assert d["config"]["max_turns"] == 5


class TestChannelConfig:
    def test_to_dict(self):
        cfg = ChannelConfig(
            channel_type="webhook",
            channel_id="ch-001",
            config={"port": 8080},
        )
        d = cfg.to_dict()
        assert d["type"] == "webhook"
        assert d["channel_id"] == "ch-001"
        assert d["config"]["port"] == 8080


class TestTriggerConfig:
    def test_cron_factory(self):
        tc = TriggerConfig.cron("t1", "agent-1", "0 * * * *", message="tick")
        assert tc.trigger_type == "cron"
        assert tc.cron_expr == "0 * * * *"
        assert tc.message == "tick"

    def test_webhook_factory(self):
        tc = TriggerConfig.webhook("t2", "agent-2", hmac_secret="secret")
        assert tc.trigger_type == "webhook"
        assert tc.hmac_secret == "secret"

    def test_event_factory(self):
        tc = TriggerConfig.event("t3", "agent-3", "agent.*", message="{event.type}")
        assert tc.trigger_type == "event"
        assert tc.event_pattern == "agent.*"


class TestKernelInfo:
    def test_from_dict(self):
        d = {
            "version": "1.0.0",
            "protocol_version": 2,
            "providers": ["anthropic", "openai"],
            "active_provider": "anthropic",
            "active_model": "claude-sonnet-4-6",
            "features": ["streaming"],
            "max_sessions": 16,
            "current_sessions": 2,
        }
        info = KernelInfo.from_dict(d)
        assert info.version == "1.0.0"
        assert info.protocol_version == 2
        assert "anthropic" in info.providers
        assert info.max_sessions == 16

    def test_from_empty_dict(self):
        info = KernelInfo.from_dict({})
        assert info.version == ""
        assert info.protocol_version == 0


class TestAgentInfo:
    def test_from_dict_with_session(self):
        d = {"agent_id": "a1", "status": "Running", "session_id": "s1"}
        info = AgentInfo.from_dict(d)
        assert info.agent_id == "a1"
        assert info.session_id == "s1"

    def test_from_dict_without_session(self):
        d = {"agent_id": "a2", "status": "Idle"}
        info = AgentInfo.from_dict(d)
        assert info.session_id is None


class TestAuditEntry:
    def test_from_dict(self):
        d = {
            "timestamp_ms": 1700000000000,
            "agent_id": "a1",
            "event_type": "tool_called",
            "tool_name": "read_file",
            "args": {"path": "/tmp/x"},
        }
        entry = AuditEntry.from_dict(d)
        assert entry.timestamp_ms == 1700000000000
        assert entry.tool_name == "read_file"
        assert entry.error_code is None


class TestScheduledTask:
    def test_from_dict(self):
        d = {
            "task_id": "t1",
            "cron": "0 * * * *",
            "label": "hourly",
            "status": "active",
        }
        task = ScheduledTask.from_dict(d)
        assert task.task_id == "t1"
        assert task.label == "hourly"


class TestSkillInfo:
    def test_from_dict(self):
        d = {"name": "my-skill", "description": "does stuff", "version": "1.0"}
        skill = SkillInfo.from_dict(d)
        assert skill.name == "my-skill"
        assert skill.version == "1.0"


class TestProviderInfo:
    def test_from_dict(self):
        d = {"name": "openai", "provider_type": "openai", "model": "gpt-4o"}
        p = ProviderInfo.from_dict(d)
        assert p.name == "openai"
        assert p.model == "gpt-4o"
