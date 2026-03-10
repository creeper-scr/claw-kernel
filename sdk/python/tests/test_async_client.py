"""Tests for the async client using asyncio socketpair."""

from __future__ import annotations

import asyncio
import json
import socket
import struct

import pytest

from claw_kernel.async_client import AsyncKernelClient, _AsyncRpc
from claw_kernel.async_transport import AsyncTransport
from claw_kernel.errors import RpcError, SessionNotFoundError


# ---------------------------------------------------------------------------
# Helper: minimal fake async daemon
# ---------------------------------------------------------------------------


async def _fake_server(writer: asyncio.StreamWriter, responses: list) -> None:
    """Send each response in *responses* as a length-prefixed frame."""
    for resp in responses:
        data = json.dumps(resp).encode()
        header = struct.pack(">I", len(data))
        writer.write(header + data)
        await writer.drain()
    writer.close()


def _frame(obj) -> bytes:
    data = json.dumps(obj).encode()
    return struct.pack(">I", len(data)) + data


class TestAsyncTransport:
    @pytest.mark.asyncio
    async def test_send_recv_frame(self):
        client_sock, server_sock = socket.socketpair(socket.AF_UNIX, socket.SOCK_STREAM)

        # Wrap server side in asyncio streams.
        loop = asyncio.get_event_loop()
        server_reader, server_writer = await asyncio.open_unix_connection(
            sock=server_sock
        )

        # Wrap client side.
        transport = AsyncTransport.__new__(AsyncTransport)
        transport._socket_path = ""
        client_reader, client_writer = await asyncio.open_unix_connection(
            sock=client_sock
        )
        transport._reader = client_reader
        transport._writer = client_writer

        payload = b'{"jsonrpc":"2.0","result":{}}'

        # Server sends frame.
        server_writer.write(struct.pack(">I", len(payload)) + payload)
        await server_writer.drain()

        received = await transport.recv_frame()
        assert received == payload

        client_writer.close()
        server_writer.close()

    @pytest.mark.asyncio
    async def test_frame_too_large_raises(self):
        from claw_kernel.errors import FrameTooLargeError

        client_sock, server_sock = socket.socketpair(socket.AF_UNIX, socket.SOCK_STREAM)

        transport = AsyncTransport.__new__(AsyncTransport)
        transport._socket_path = ""
        client_reader, client_writer = await asyncio.open_unix_connection(
            sock=client_sock
        )
        transport._reader = client_reader
        transport._writer = client_writer

        # Server sends header claiming 32 MiB.
        server_reader, server_writer = await asyncio.open_unix_connection(
            sock=server_sock
        )
        server_writer.write(struct.pack(">I", 32 * 1024 * 1024))
        await server_writer.drain()

        with pytest.raises(FrameTooLargeError):
            await transport.recv_frame()

        client_writer.close()
        server_writer.close()


class TestAsyncRpc:
    @pytest.mark.asyncio
    async def test_call_returns_result(self):
        client_sock, server_sock = socket.socketpair(socket.AF_UNIX, socket.SOCK_STREAM)

        # Client side transport.
        transport = AsyncTransport.__new__(AsyncTransport)
        transport._socket_path = ""
        cr, cw = await asyncio.open_unix_connection(sock=client_sock)
        transport._reader = cr
        transport._writer = cw

        rpc = _AsyncRpc(transport)
        await rpc.start()

        # Server side: read request, send response.
        sr, sw = await asyncio.open_unix_connection(sock=server_sock)

        async def _server():
            header = await sr.readexactly(4)
            length = struct.unpack(">I", header)[0]
            body = await sr.readexactly(length)
            msg = json.loads(body)
            resp = {"jsonrpc": "2.0", "result": {"pong": True}, "id": msg["id"]}
            data = json.dumps(resp).encode()
            sw.write(struct.pack(">I", len(data)) + data)
            await sw.drain()

        server_task = asyncio.ensure_future(_server())
        result = await rpc.call("kernel.ping")
        await server_task

        assert result == {"pong": True}

        await rpc.stop()
        cw.close()
        sw.close()

    @pytest.mark.asyncio
    async def test_error_raises_rpc_error(self):
        client_sock, server_sock = socket.socketpair(socket.AF_UNIX, socket.SOCK_STREAM)

        transport = AsyncTransport.__new__(AsyncTransport)
        transport._socket_path = ""
        cr, cw = await asyncio.open_unix_connection(sock=client_sock)
        transport._reader = cr
        transport._writer = cw

        rpc = _AsyncRpc(transport)
        await rpc.start()

        sr, sw = await asyncio.open_unix_connection(sock=server_sock)

        async def _server():
            header = await sr.readexactly(4)
            length = struct.unpack(">I", header)[0]
            body = await sr.readexactly(length)
            msg = json.loads(body)
            resp = {
                "jsonrpc": "2.0",
                "error": {"code": -32000, "message": "Session not found"},
                "id": msg["id"],
            }
            data = json.dumps(resp).encode()
            sw.write(struct.pack(">I", len(data)) + data)
            await sw.drain()

        server_task = asyncio.ensure_future(_server())
        with pytest.raises(SessionNotFoundError):
            await rpc.call("destroySession", {"session_id": "missing"})
        await server_task

        await rpc.stop()
        cw.close()
        sw.close()

    @pytest.mark.asyncio
    async def test_subscribe_yields_notifications(self):
        client_sock, server_sock = socket.socketpair(socket.AF_UNIX, socket.SOCK_STREAM)

        transport = AsyncTransport.__new__(AsyncTransport)
        transport._socket_path = ""
        cr, cw = await asyncio.open_unix_connection(sock=client_sock)
        transport._reader = cr
        transport._writer = cw

        rpc = _AsyncRpc(transport)
        await rpc.start()

        # Pre-create the queue and put notifications in it.
        q = rpc._get_or_create_queue("s-test")
        q.put_nowait(
            {
                "method": "agent/streamChunk",
                "params": {"session_id": "s-test", "delta": "hello", "done": False},
            }
        )
        q.put_nowait(
            {
                "method": "agent/finish",
                "params": {"session_id": "s-test", "reason": "stop"},
            }
        )

        received = []
        async for notif in rpc.subscribe("s-test"):
            received.append(notif)
            if notif.get("method") == "agent/finish":
                break

        assert len(received) == 2
        assert received[0]["params"]["delta"] == "hello"
        assert received[1]["method"] == "agent/finish"

        await rpc.stop()
        cw.close()
