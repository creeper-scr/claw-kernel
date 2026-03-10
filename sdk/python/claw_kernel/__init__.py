"""
claw-kernel Python SDK
======================

A zero-dependency Python client for the claw-kernel IPC daemon.

Quick start (synchronous)::

    from claw_kernel import KernelClient, SessionConfig

    with KernelClient() as client:
        print(client.info())
        session_id = client.session.create(SessionConfig(system_prompt="Be helpful."))
        for token in client.session.send(session_id, "Hello!"):
            print(token, end="", flush=True)
        client.session.destroy(session_id)

Quick start (async)::

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
"""

from .async_client import AsyncKernelClient
from .client import KernelClient
from .errors import (
    AgentError,
    AuthenticationError,
    ClawError,
    ConnectionError,
    FrameTooLargeError,
    MaxSessionsReachedError,
    ProviderError,
    ProviderNotFoundError,
    RpcError,
    SessionNotFoundError,
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
    TriggerConfig,
)

__version__ = "0.1.0"
__all__ = [
    # Clients
    "KernelClient",
    "AsyncKernelClient",
    # Errors
    "ClawError",
    "ConnectionError",
    "AuthenticationError",
    "RpcError",
    "SessionNotFoundError",
    "MaxSessionsReachedError",
    "ProviderError",
    "AgentError",
    "ProviderNotFoundError",
    "FrameTooLargeError",
    # Models
    "SessionConfig",
    "AgentConfig",
    "ChannelConfig",
    "TriggerConfig",
    "ToolDef",
    "KernelInfo",
    "AgentInfo",
    "AuditEntry",
    "ScheduledTask",
    "SkillInfo",
    "ProviderInfo",
    "__version__",
]
