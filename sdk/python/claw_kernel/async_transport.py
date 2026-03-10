"""
claw-kernel SDK — Asynchronous framing layer (asyncio).

Provides :class:`AsyncTransport`, which wraps ``asyncio.open_unix_connection``
with the same 4-byte big-endian length-prefix protocol used by the sync layer.
"""

from __future__ import annotations

import asyncio
import struct
from pathlib import Path
from typing import Optional, Tuple

from ._auth import ClawPaths, start_daemon
from .errors import (
    ConnectionError as ClawConnectionError,
    FrameTooLargeError,
)

# Maximum allowed frame size: 16 MiB.
_MAX_FRAME_SIZE: int = 16 * 1024 * 1024


class AsyncTransport:
    """Asynchronous framing transport over a Unix domain socket.

    Uses :func:`asyncio.open_unix_connection` under the hood.

    Usage::

        transport = AsyncTransport()
        await transport.connect()
        await transport.send_frame(b"...")
        data = await transport.recv_frame()
        await transport.close()

    Or as an async context manager::

        async with AsyncTransport() as t:
            await t.send_frame(b"...")
            data = await t.recv_frame()

    Args:
        socket_path: Override the default Unix socket path.
    """

    def __init__(self, socket_path: Optional[str] = None) -> None:
        self._socket_path = socket_path or str(ClawPaths.socket_path())
        self._reader: Optional[asyncio.StreamReader] = None
        self._writer: Optional[asyncio.StreamWriter] = None

    # ------------------------------------------------------------------
    # Connection management
    # ------------------------------------------------------------------

    async def connect(self) -> None:
        """Open the connection.  Starts the daemon if the socket is absent.

        Raises:
            ConnectionError: If the socket cannot be reached.
        """
        path = self._socket_path
        if not Path(path).exists():
            # start_daemon is a blocking call; run in a thread executor to
            # avoid blocking the event loop.
            loop = asyncio.get_event_loop()
            await loop.run_in_executor(None, start_daemon, path)

        try:
            self._reader, self._writer = await asyncio.open_unix_connection(path)
        except OSError as exc:
            raise ClawConnectionError(
                f"Cannot connect to claw-kernel socket at {path!r}: {exc}"
            ) from exc

    async def close(self) -> None:
        """Close the underlying transport."""
        if self._writer is not None:
            try:
                self._writer.close()
                await self._writer.wait_closed()
            except OSError:
                pass
            finally:
                self._writer = None
                self._reader = None

    # ------------------------------------------------------------------
    # Frame I/O
    # ------------------------------------------------------------------

    async def send_frame(self, data: bytes) -> None:
        """Send *data* as a length-prefixed frame.

        Args:
            data: Raw bytes payload.

        Raises:
            ConnectionError: If the writer is not available.
        """
        if self._writer is None:
            raise ClawConnectionError("AsyncTransport is not connected")
        header = struct.pack(">I", len(data))
        self._writer.write(header + data)
        try:
            await self._writer.drain()
        except (OSError, ConnectionResetError) as exc:
            raise ClawConnectionError(f"Send failed: {exc}") from exc

    async def recv_frame(self) -> bytes:
        """Receive the next length-prefixed frame.

        Returns:
            The raw frame payload bytes.

        Raises:
            FrameTooLargeError: If the declared size exceeds 16 MiB.
            ConnectionError: If the connection is closed.
        """
        if self._reader is None:
            raise ClawConnectionError("AsyncTransport is not connected")

        header = await self._read_exact(4)
        length = struct.unpack(">I", header)[0]
        if length > _MAX_FRAME_SIZE:
            raise FrameTooLargeError(length)
        return await self._read_exact(length)

    # ------------------------------------------------------------------
    # Internal helpers
    # ------------------------------------------------------------------

    async def _read_exact(self, n: int) -> bytes:
        """Read exactly *n* bytes from the stream."""
        assert self._reader is not None
        try:
            data = await self._reader.readexactly(n)
        except asyncio.IncompleteReadError as exc:
            raise ClawConnectionError(
                "Connection closed by remote while reading frame"
            ) from exc
        except OSError as exc:
            raise ClawConnectionError(f"Recv failed: {exc}") from exc
        return data

    # ------------------------------------------------------------------
    # Async context manager support
    # ------------------------------------------------------------------

    async def __aenter__(self) -> "AsyncTransport":
        await self.connect()
        return self

    async def __aexit__(self, *_: object) -> None:
        await self.close()
