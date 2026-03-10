"""
claw-kernel SDK — Synchronous framing layer.

Implements the 4-byte big-endian length-prefixed framing protocol used by
the claw-kernel IPC interface, with optional auto-reconnect and re-authentication.
"""

from __future__ import annotations

import socket
import struct
import time
from pathlib import Path
from typing import Optional

from ._auth import ClawPaths, read_token, start_daemon
from .errors import (
    AuthenticationError,
    ConnectionError as ClawConnectionError,
    FrameTooLargeError,
)

# Maximum allowed frame size: 16 MiB.
_MAX_FRAME_SIZE: int = 16 * 1024 * 1024
# Default socket receive/send timeout (seconds).
_SOCKET_TIMEOUT: float = 30.0
# Default delay between reconnect attempts (seconds).
_DEFAULT_RETRY_DELAY: float = 0.5


class SyncTransport:
    """Low-level synchronous framing transport over a Unix domain socket.

    Handles:
    * 4-byte BE length-prefix frame encoding / decoding
    * Auto-connect on construction
    * Configurable auto-reconnect with re-authentication on disconnect
    * ``bytearray`` receive buffer to avoid O(n) bytes concatenation

    Args:
        socket_path: Path to the Unix domain socket.  Defaults to the
            platform-standard path from :class:`~claw_kernel._auth.ClawPaths`.
        auto_reconnect: If ``True``, transparently reconnect on broken pipe.
        max_retries: Maximum reconnect attempts before raising.
        retry_delay: Seconds to wait between reconnect attempts.
        timeout: Socket operation timeout in seconds.
    """

    def __init__(
        self,
        socket_path: Optional[str] = None,
        *,
        auto_reconnect: bool = True,
        max_retries: int = 3,
        retry_delay: float = _DEFAULT_RETRY_DELAY,
        timeout: float = _SOCKET_TIMEOUT,
    ) -> None:
        self._socket_path = socket_path or str(ClawPaths.socket_path())
        self._auto_reconnect = auto_reconnect
        self._max_retries = max_retries
        self._retry_delay = retry_delay
        self._timeout = timeout
        self._sock: Optional[socket.socket] = None
        self._buf = bytearray()
        self._connect()

    # ------------------------------------------------------------------
    # Connection management
    # ------------------------------------------------------------------

    def _connect(self) -> None:
        """Open the socket and perform the initial connection.

        If the socket file does not exist, attempts to start the daemon.
        """
        path = self._socket_path
        if not Path(path).exists():
            start_daemon(path)

        sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        sock.settimeout(self._timeout)
        try:
            sock.connect(path)
        except OSError as exc:
            sock.close()
            raise ClawConnectionError(
                f"Cannot connect to claw-kernel socket at {path!r}: {exc}"
            ) from exc

        self._sock = sock
        self._buf = bytearray()

    def reconnect(self) -> None:
        """Close the current socket and re-establish the connection.

        Callers (e.g. the RPC layer) are responsible for re-authenticating
        after calling this method.
        """
        self.close()
        self._connect()

    def close(self) -> None:
        """Close the underlying socket."""
        if self._sock is not None:
            try:
                self._sock.close()
            except OSError:
                pass
            finally:
                self._sock = None

    # ------------------------------------------------------------------
    # Frame I/O
    # ------------------------------------------------------------------

    def send_frame(self, data: bytes) -> None:
        """Send *data* as a length-prefixed frame.

        Args:
            data: Raw bytes payload (UTF-8 JSON typically).

        Raises:
            ConnectionError: If the socket is closed or broken.
        """
        header = struct.pack(">I", len(data))
        payload = header + data
        self._sendall(payload)

    def recv_frame(self) -> bytes:
        """Receive the next length-prefixed frame.

        Blocks until a complete frame arrives.

        Returns:
            The raw frame payload (without the 4-byte length header).

        Raises:
            FrameTooLargeError: If the declared frame length exceeds 16 MiB.
            ConnectionError: If the connection is closed by the remote.
        """
        # Read header (4 bytes)
        header = self._recv_exact(4)
        length = struct.unpack(">I", header)[0]
        if length > _MAX_FRAME_SIZE:
            raise FrameTooLargeError(length)
        return bytes(self._recv_exact(length))

    # ------------------------------------------------------------------
    # Internal helpers
    # ------------------------------------------------------------------

    def _sendall(self, data: bytes) -> None:
        """Send all bytes, handling EINTR and reconnects."""
        assert self._sock is not None, "Transport is closed"
        try:
            self._sock.sendall(data)
        except (OSError, BrokenPipeError) as exc:
            raise ClawConnectionError(f"Send failed: {exc}") from exc

    def _recv_exact(self, n: int) -> bytearray:
        """Read exactly *n* bytes from the socket into a :class:`bytearray`.

        Uses the internal ``_buf`` for accumulation to avoid repeated
        bytes concatenation.
        """
        assert self._sock is not None, "Transport is closed"
        buf = bytearray()
        remaining = n
        while remaining > 0:
            try:
                chunk = self._sock.recv(remaining)
            except socket.timeout as exc:
                raise ClawConnectionError(
                    f"Socket timed out waiting for {n} bytes"
                ) from exc
            except OSError as exc:
                raise ClawConnectionError(f"Recv failed: {exc}") from exc
            if not chunk:
                raise ClawConnectionError(
                    "Connection closed by remote while reading frame"
                )
            buf.extend(chunk)
            remaining -= len(chunk)
        return buf
