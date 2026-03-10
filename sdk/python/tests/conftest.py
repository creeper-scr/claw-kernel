"""
Shared test fixtures for claw-kernel SDK tests.

Uses ``socket.socketpair()`` to create a connected socket pair so tests can
exercise the transport and RPC layers without a real daemon.
"""

from __future__ import annotations

import json
import socket
import struct
import threading
from typing import Any, Dict, Generator, Optional

import pytest


# ---------------------------------------------------------------------------
# Socket fixture helpers
# ---------------------------------------------------------------------------


def send_frame(sock: socket.socket, data: bytes) -> None:
    """Send a length-prefixed frame on *sock*."""
    header = struct.pack(">I", len(data))
    sock.sendall(header + data)


def recv_frame(sock: socket.socket) -> bytes:
    """Receive a length-prefixed frame from *sock*."""
    header = _recv_exact(sock, 4)
    length = struct.unpack(">I", header)[0]
    return _recv_exact(sock, length)


def _recv_exact(sock: socket.socket, n: int) -> bytes:
    buf = b""
    while len(buf) < n:
        chunk = sock.recv(n - len(buf))
        if not chunk:
            raise RuntimeError("Socket closed unexpectedly")
        buf += chunk
    return buf


def send_json(sock: socket.socket, obj: Any) -> None:
    """Serialise *obj* to JSON and send as a frame."""
    send_frame(sock, json.dumps(obj).encode())


def recv_json(sock: socket.socket) -> Any:
    """Receive a frame and parse as JSON."""
    return json.loads(recv_frame(sock))


# ---------------------------------------------------------------------------
# Minimal fake daemon fixture
# ---------------------------------------------------------------------------


class FakeDaemon:
    """A simple in-process fake daemon that handles one request at a time.

    Tests can customise ``responses`` to control what the fake daemon returns
    for each incoming request.
    """

    def __init__(self, server_sock: socket.socket) -> None:
        self._server = server_sock
        self.requests: list = []
        self.responses: list = []
        self._thread: Optional[threading.Thread] = None
        self._stopped = False

    def start(self) -> None:
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()

    def stop(self) -> None:
        self._stopped = True
        try:
            self._server.close()
        except OSError:
            pass

    def _run(self) -> None:
        while not self._stopped:
            try:
                raw = recv_frame(self._server)
            except (OSError, RuntimeError):
                break
            msg = json.loads(raw)
            self.requests.append(msg)
            if self.responses:
                reply = self.responses.pop(0)
            else:
                # Default: echo back {"result": {"ok": true}, "id": <req_id>}
                reply = {
                    "jsonrpc": "2.0",
                    "result": {"ok": True},
                    "id": msg.get("id"),
                }
            send_json(self._server, reply)


@pytest.fixture
def socket_pair() -> Generator[tuple, None, None]:
    """Yield ``(client_sock, server_sock)`` connected socket pair.

    Both sockets are closed after the test.
    """
    client_sock, server_sock = socket.socketpair(socket.AF_UNIX, socket.SOCK_STREAM)
    yield client_sock, server_sock
    client_sock.close()
    server_sock.close()
