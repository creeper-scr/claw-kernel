"""
claw-kernel SDK — Exception hierarchy.

All public exceptions raised by the SDK are subclasses of :class:`ClawError`.
"""

from __future__ import annotations


class ClawError(Exception):
    """Base class for all claw-kernel SDK errors."""


class ConnectionError(ClawError):  # noqa: A001 (shadow builtins intentionally)
    """Raised when the IPC socket connection is lost or cannot be established."""


class AuthenticationError(ClawError):
    """Raised when ``kernel.auth`` returns a failure response."""


class FrameTooLargeError(ClawError):
    """Raised when a frame exceeds the 16 MiB size limit."""

    def __init__(self, length: int) -> None:
        super().__init__(f"Frame too large: {length} bytes (max 16 MiB)")
        self.length = length


class RpcError(ClawError):
    """Raised when the server returns a JSON-RPC error response.

    Attributes:
        code: The JSON-RPC error code (e.g. -32000).
        message: The server-provided error message.
        data: Optional additional error data from the server.
    """

    def __init__(self, code: int, message: str, data: object = None) -> None:
        super().__init__(f"[{code}] {message}")
        self.code = code
        self.message = message
        self.data = data

    def __repr__(self) -> str:
        return f"{type(self).__name__}(code={self.code!r}, message={self.message!r})"


class SessionNotFoundError(RpcError):
    """Raised when the target session does not exist (code -32000)."""

    def __init__(self, message: str = "Session not found", data: object = None) -> None:
        super().__init__(-32000, message, data)


class MaxSessionsReachedError(RpcError):
    """Raised when the daemon's session limit is reached (code -32001)."""

    def __init__(
        self, message: str = "Max sessions reached", data: object = None
    ) -> None:
        super().__init__(-32001, message, data)


class ProviderError(RpcError):
    """Raised when the LLM provider returns an error (code -32002)."""

    def __init__(self, message: str, data: object = None) -> None:
        super().__init__(-32002, message, data)


class AgentError(RpcError):
    """Raised when the agent loop encounters an error (code -32003)."""

    def __init__(self, message: str, data: object = None) -> None:
        super().__init__(-32003, message, data)


class ProviderNotFoundError(RpcError):
    """Raised when a requested provider is not registered (code -32005)."""

    def __init__(
        self, message: str = "Provider not found", data: object = None
    ) -> None:
        super().__init__(-32005, message, data)


# Error code → exception class mapping used by the RPC layer.
_ERROR_CODE_MAP: dict[int, type[RpcError]] = {
    -32000: SessionNotFoundError,
    -32001: MaxSessionsReachedError,
    -32002: ProviderError,
    -32003: AgentError,
    -32005: ProviderNotFoundError,
}


def _rpc_error_from_code(code: int, message: str, data: object = None) -> RpcError:
    """Construct the most specific :class:`RpcError` subclass for *code*."""
    cls = _ERROR_CODE_MAP.get(code, RpcError)
    if cls is RpcError:
        return RpcError(code, message, data)
    # Subclasses accept (message, data) — code is baked in.
    return cls(message, data)  # type: ignore[call-arg]
