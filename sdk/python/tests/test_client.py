"""Tests for the SyncRpc layer using a fake socket-based daemon."""

from __future__ import annotations

import json
import struct
import threading

import pytest

from claw_kernel._rpc import SyncRpc
from claw_kernel._transport import SyncTransport
from claw_kernel.errors import RpcError, SessionNotFoundError

from conftest import FakeDaemon, recv_json, send_json


def _make_rpc(client_sock, server_sock):
    """Helper: build an SyncRpc backed by a pre-connected socket pair."""
    transport = SyncTransport.__new__(SyncTransport)
    transport._sock = client_sock
    transport._auto_reconnect = False
    transport._max_retries = 0
    transport._retry_delay = 0.0
    transport._socket_path = ""
    transport._buf = bytearray()

    rpc = SyncRpc(transport, auto_reconnect=False)
    return rpc


class TestSyncRpcCall:
    def test_simple_call_returns_result(self, socket_pair):
        client_sock, server_sock = socket_pair
        rpc = _make_rpc(client_sock, server_sock)

        def _server():
            msg = recv_json(server_sock)
            reply = {"jsonrpc": "2.0", "result": {"pong": True}, "id": msg["id"]}
            send_json(server_sock, reply)

        t = threading.Thread(target=_server)
        t.start()
        result = rpc.call("kernel.ping")
        t.join()

        assert result == {"pong": True}

    def test_error_response_raises_rpc_error(self, socket_pair):
        client_sock, server_sock = socket_pair
        rpc = _make_rpc(client_sock, server_sock)

        def _server():
            msg = recv_json(server_sock)
            reply = {
                "jsonrpc": "2.0",
                "error": {"code": -32000, "message": "Session not found"},
                "id": msg["id"],
            }
            send_json(server_sock, reply)

        t = threading.Thread(target=_server)
        t.start()
        with pytest.raises(SessionNotFoundError):
            rpc.call("destroySession", {"session_id": "missing"})
        t.join()

    def test_generic_rpc_error(self, socket_pair):
        client_sock, server_sock = socket_pair
        rpc = _make_rpc(client_sock, server_sock)

        def _server():
            msg = recv_json(server_sock)
            reply = {
                "jsonrpc": "2.0",
                "error": {"code": -32601, "message": "Method not found"},
                "id": msg["id"],
            }
            send_json(server_sock, reply)

        t = threading.Thread(target=_server)
        t.start()
        with pytest.raises(RpcError) as exc_info:
            rpc.call("nonexistent.method")
        t.join()

        assert exc_info.value.code == -32601

    def test_incrementing_request_ids(self, socket_pair):
        client_sock, server_sock = socket_pair
        rpc = _make_rpc(client_sock, server_sock)
        ids = []

        def _server():
            for _ in range(3):
                msg = recv_json(server_sock)
                ids.append(msg["id"])
                send_json(
                    server_sock, {"jsonrpc": "2.0", "result": {}, "id": msg["id"]}
                )

        t = threading.Thread(target=_server)
        t.start()
        rpc.call("m1")
        rpc.call("m2")
        rpc.call("m3")
        t.join()

        assert ids == list(range(ids[0], ids[0] + 3))

    def test_notification_buffered_correctly(self, socket_pair):
        """A notification arriving before the response should be buffered."""
        client_sock, server_sock = socket_pair
        rpc = _make_rpc(client_sock, server_sock)

        def _server():
            msg = recv_json(server_sock)
            # First send a notification, then the actual response.
            notif = {
                "jsonrpc": "2.0",
                "method": "agent/streamChunk",
                "params": {"session_id": "s1", "delta": "hello", "done": False},
            }
            send_json(server_sock, notif)
            send_json(
                server_sock, {"jsonrpc": "2.0", "result": {"ok": True}, "id": msg["id"]}
            )

        t = threading.Thread(target=_server)
        t.start()
        result = rpc.call("sendMessage", {"session_id": "s1", "content": "hi"})
        t.join()

        assert result == {"ok": True}
        # Notification should now be in the queue for session s1.
        q = rpc._get_or_create_queue("s1")
        assert not q.empty()
        notif = q.get_nowait()
        assert notif["method"] == "agent/streamChunk"


class TestSyncRpcSubscribe:
    def test_subscribe_yields_notifications(self, socket_pair):
        client_sock, server_sock = socket_pair
        rpc = _make_rpc(client_sock, server_sock)
        received = []

        def _server():
            for i in range(2):
                send_json(
                    server_sock,
                    {
                        "jsonrpc": "2.0",
                        "method": "agent/streamChunk",
                        "params": {
                            "session_id": "s-abc",
                            "delta": f"token{i}",
                            "done": i == 1,
                        },
                    },
                )

        t = threading.Thread(target=_server)
        t.start()

        gen = rpc.subscribe("s-abc")
        n1 = next(gen)
        n2 = next(gen)
        t.join()

        assert n1["params"]["delta"] == "token0"
        assert n2["params"]["done"] is True

    def test_drop_queue_cleans_up(self, socket_pair):
        client_sock, server_sock = socket_pair
        rpc = _make_rpc(client_sock, server_sock)
        rpc._get_or_create_queue("s-cleanup")
        assert "s-cleanup" in rpc._notif_queues
        rpc.drop_queue("s-cleanup")
        assert "s-cleanup" not in rpc._notif_queues
