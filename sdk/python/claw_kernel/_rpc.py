"""
claw-kernel SDK — Synchronous JSON-RPC 2.0 call layer.

Sits on top of :class:`~claw_kernel._transport.SyncTransport` and provides:

* :meth:`SyncRpc.call` — request / response round-trip
* :meth:`SyncRpc.subscribe` — blocking iterator over streaming notifications
* Thread-safe multiplexing via :class:`threading.Lock`
* Per-session notification queues so multi-session concurrency works correctly
* Transparent re-authentication after reconnect

Protocol details:
    Framing: 4-byte big-endian length prefix + UTF-8 JSON payload
    Notifications have no ``id`` field (or ``id`` is ``null``).
"""

from __future__ import annotations

import json
import queue
import threading
from typing import Any, Dict, Iterator, Optional

from ._auth import read_token
from ._transport import SyncTransport
from .errors import (
    AuthenticationError,
    ConnectionError as ClawConnectionError,
    RpcError,
    _rpc_error_from_code,
)


class SyncRpc:
    """Thread-safe JSON-RPC 2.0 call layer (synchronous).

    Args:
        transport: An already-connected :class:`SyncTransport` instance.
        auto_reconnect: If ``True``, transparently reconnect + re-auth on
            broken-pipe errors.
        max_retries: Maximum number of reconnect attempts.
    """

    def __init__(
        self,
        transport: SyncTransport,
        *,
        auto_reconnect: bool = True,
        max_retries: int = 3,
    ) -> None:
        self._transport = transport
        self._auto_reconnect = auto_reconnect
        self._max_retries = max_retries

        # Monotonically increasing request ID.
        self._req_id: int = 1
        self._id_lock = threading.Lock()

        # Serialise socket writes; individual reads happen in the same thread
        # per call so no separate read lock is needed for single-threaded use.
        self._write_lock = threading.Lock()

        # Per-session notification queues.
        # Maps session_id -> SimpleQueue of notification dicts.
        self._notif_queues: Dict[str, queue.SimpleQueue] = {}  # type: ignore[type-arg]
        self._notif_lock = threading.Lock()

        # Responses that arrived out-of-order (keyed by request id).
        self._pending_responses: Dict[int, dict] = {}
        self._response_lock = threading.Lock()

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def authenticate(self) -> None:
        """Perform ``kernel.auth`` handshake.

        Called automatically by :class:`~claw_kernel.client.KernelClient`
        on initial connect and after each reconnect.

        Raises:
            AuthenticationError: If the server rejects the token.
        """
        token = read_token()
        result = self.call("kernel.auth", {"token": token})
        if not result.get("ok"):
            raise AuthenticationError("kernel.auth failed: server rejected the token")

    def call(
        self,
        method: str,
        params: Optional[Dict[str, Any]] = None,
    ) -> Any:
        """Send a JSON-RPC request and return the ``result`` field.

        Notifications received while waiting for the response are buffered
        in the appropriate per-session queue so they are not lost.

        Args:
            method: JSON-RPC method name (e.g. ``"createSession"``).
            params: Optional parameters dictionary.

        Returns:
            The ``result`` value from the server response (may be any JSON
            type).

        Raises:
            RpcError: On a JSON-RPC error response.
            ConnectionError: On a transport error.
        """
        req_id = self._next_id()
        request: Dict[str, Any] = {
            "jsonrpc": "2.0",
            "method": method,
            "id": req_id,
        }
        if params is not None:
            request["params"] = params

        payload = json.dumps(request).encode("utf-8")

        retries = 0
        while True:
            try:
                with self._write_lock:
                    self._transport.send_frame(payload)

                # Drain until we get our response.
                while True:
                    raw = self._transport.recv_frame()
                    msg = json.loads(raw)

                    # Check if this is our response.
                    if self._is_response_for(msg, req_id):
                        if "error" in msg:
                            err = msg["error"]
                            raise _rpc_error_from_code(
                                err["code"],
                                err.get("message", ""),
                                err.get("data"),
                            )
                        return msg.get("result", {})

                    # Otherwise it's a notification or a response for another
                    # request — dispatch accordingly.
                    self._dispatch(msg)

            except ClawConnectionError:
                if not self._auto_reconnect or retries >= self._max_retries:
                    raise
                retries += 1
                self._transport.reconnect()
                self.authenticate()

    def subscribe(self, session_id: str) -> Iterator[dict]:
        """Yield notifications for *session_id* until the stream ends.

        The caller drives the iteration; this method blocks on each
        :meth:`recv_frame` call until the server sends a notification.

        Convention: iteration ends when the caller stops iterating or when
        the generator is closed by the client.  The caller is responsible
        for detecting finish/done conditions in the notification payload.

        Args:
            session_id: The session whose notifications to yield.

        Yields:
            Raw notification dicts (including ``method`` and ``params``).
        """
        q = self._get_or_create_queue(session_id)

        while True:
            # First drain any already-queued notifications.
            try:
                yield q.get_nowait()
                continue
            except queue.Empty:
                pass

            # Block on the transport for new frames.
            raw = self._transport.recv_frame()
            msg = json.loads(raw)

            if self._is_notification(msg):
                params = msg.get("params") or {}
                msg_session = params.get("session_id", "")
                if msg_session == session_id:
                    yield msg
                else:
                    # Route to another session's queue.
                    self._dispatch_notification(msg)
            # Ignore unexpected response frames here.

    # ------------------------------------------------------------------
    # Notification queue management
    # ------------------------------------------------------------------

    def _get_or_create_queue(self, session_id: str) -> "queue.SimpleQueue[dict]":
        with self._notif_lock:
            if session_id not in self._notif_queues:
                self._notif_queues[session_id] = queue.SimpleQueue()
            return self._notif_queues[session_id]

    def drop_queue(self, session_id: str) -> None:
        """Remove the notification queue for a destroyed session."""
        with self._notif_lock:
            self._notif_queues.pop(session_id, None)

    # ------------------------------------------------------------------
    # Internal helpers
    # ------------------------------------------------------------------

    def _next_id(self) -> int:
        with self._id_lock:
            req_id = self._req_id
            self._req_id += 1
        return req_id

    @staticmethod
    def _is_response_for(msg: dict, req_id: int) -> bool:
        """Return True if *msg* is the response for *req_id*."""
        msg_id = msg.get("id")
        return msg_id is not None and msg_id == req_id

    @staticmethod
    def _is_notification(msg: dict) -> bool:
        """Return True if *msg* is a push notification (no ``id``)."""
        return msg.get("id") is None and "method" in msg

    def _dispatch(self, msg: dict) -> None:
        """Route an unexpected frame to the correct queue or discard."""
        if self._is_notification(msg):
            self._dispatch_notification(msg)
        else:
            # Store out-of-order response for future retrieval (rare).
            msg_id = msg.get("id")
            if isinstance(msg_id, int):
                with self._response_lock:
                    self._pending_responses[msg_id] = msg

    def _dispatch_notification(self, msg: dict) -> None:
        """Put *msg* into the queue of the matching session."""
        params = msg.get("params") or {}
        session_id = params.get("session_id", "")
        if session_id:
            q = self._get_or_create_queue(session_id)
            q.put(msg)
        # Notifications without a session_id are silently dropped.
