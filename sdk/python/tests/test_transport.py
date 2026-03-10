"""Tests for the synchronous framing transport layer."""

from __future__ import annotations

import struct
import threading

import pytest

from claw_kernel._transport import SyncTransport, _MAX_FRAME_SIZE
from claw_kernel.errors import (
    ConnectionError as ClawConnectionError,
    FrameTooLargeError,
)

from conftest import FakeDaemon, recv_json, send_json


class TestSyncTransportFraming:
    """Test _transport.SyncTransport frame encode/decode via socketpair."""

    def test_send_and_recv_small_frame(self, socket_pair):
        client_sock, server_sock = socket_pair

        # Build a transport around the pre-connected client socket.
        transport = SyncTransport.__new__(SyncTransport)
        transport._sock = client_sock
        transport._auto_reconnect = False
        transport._max_retries = 0
        transport._retry_delay = 0.0
        transport._socket_path = ""
        transport._buf = bytearray()

        payload = b'{"jsonrpc":"2.0","result":{},"id":1}'

        # Server sends frame.
        def _server():
            header = struct.pack(">I", len(payload))
            server_sock.sendall(header + payload)

        t = threading.Thread(target=_server)
        t.start()
        received = transport.recv_frame()
        t.join()

        assert received == payload

    def test_send_frame_writes_correct_header(self, socket_pair):
        client_sock, server_sock = socket_pair

        transport = SyncTransport.__new__(SyncTransport)
        transport._sock = client_sock
        transport._auto_reconnect = False
        transport._max_retries = 0
        transport._retry_delay = 0.0
        transport._socket_path = ""
        transport._buf = bytearray()

        data = b"hello world"

        def _server():
            header = server_sock.recv(4)
            length = struct.unpack(">I", header)[0]
            body = server_sock.recv(length)
            server_sock.sendall(header + body)  # echo back

        t = threading.Thread(target=_server)
        t.start()
        transport.send_frame(data)
        received = transport.recv_frame()
        t.join()

        assert received == data

    def test_frame_too_large_raises(self, socket_pair):
        client_sock, server_sock = socket_pair

        transport = SyncTransport.__new__(SyncTransport)
        transport._sock = client_sock
        transport._auto_reconnect = False
        transport._max_retries = 0
        transport._retry_delay = 0.0
        transport._socket_path = ""
        transport._buf = bytearray()

        def _server():
            # Send a frame header claiming 32 MiB.
            too_big = 32 * 1024 * 1024
            header = struct.pack(">I", too_big)
            server_sock.sendall(header)

        t = threading.Thread(target=_server)
        t.start()

        with pytest.raises(FrameTooLargeError):
            transport.recv_frame()

        t.join()

    def test_connection_closed_raises(self, socket_pair):
        client_sock, server_sock = socket_pair

        transport = SyncTransport.__new__(SyncTransport)
        transport._sock = client_sock
        transport._auto_reconnect = False
        transport._max_retries = 0
        transport._retry_delay = 0.0
        transport._socket_path = ""
        transport._buf = bytearray()

        server_sock.close()  # abruptly close the other end

        with pytest.raises(ClawConnectionError):
            transport.recv_frame()

    def test_recv_exact_accumulates_correctly(self, socket_pair):
        """Verify bytearray accumulation works for multi-chunk reads."""
        client_sock, server_sock = socket_pair

        transport = SyncTransport.__new__(SyncTransport)
        transport._sock = client_sock
        transport._auto_reconnect = False
        transport._max_retries = 0
        transport._retry_delay = 0.0
        transport._socket_path = ""
        transport._buf = bytearray()

        payload = b"A" * 1024

        def _server():
            header = struct.pack(">I", len(payload))
            # Send in small chunks to exercise accumulation.
            full = header + payload
            for i in range(0, len(full), 64):
                server_sock.sendall(full[i : i + 64])

        t = threading.Thread(target=_server)
        t.start()
        received = transport.recv_frame()
        t.join()

        assert received == payload
