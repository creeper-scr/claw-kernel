"""
claw-kernel Python SDK 参考实现
依赖：Python 3.9+，标准库（无第三方依赖）
"""
import json
import os
import platform
import socket
import struct
import subprocess
import threading
from pathlib import Path
from typing import Iterator, Callable, Optional, Dict, Any


class KernelClient:
    """
    claw-kernel IPC 客户端。

    使用方法：
        client = KernelClient()           # 自动发现并连接
        session = client.create_session("You are helpful.")
        for token in client.send_message(session, "Hello!"):
            print(token, end="", flush=True)
        client.destroy_session(session)
    """

    def __init__(self, socket_path: Optional[str] = None):
        """初始化客户端，自动发现 socket 并建立连接。"""
        self._sock = None
        self._req_id = 1
        self._lock = threading.Lock()
        self._pending: Dict[int, Any] = {}  # id -> response
        self._notifications: list = []
        self._connect(socket_path)

    # --- 自动发现 ---

    @staticmethod
    def _data_dir() -> Path:
        """返回与 claw-pal dirs 模块一致的平台数据目录。"""
        system = platform.system()
        if system == "Darwin":
            return Path.home() / "Library" / "Application Support" / "claw-kernel"
        elif system == "Windows":
            local_app_data = os.environ.get(
                "LOCALAPPDATA", str(Path.home() / "AppData" / "Local")
            )
            return Path(local_app_data) / "claw-kernel"
        else:  # Linux / 其他 Unix
            xdg_runtime = os.environ.get("XDG_RUNTIME_DIR")
            if xdg_runtime:
                return Path(xdg_runtime) / "claw"
            return Path.home() / ".local" / "share" / "claw-kernel"

    @classmethod
    def _default_socket_path(cls) -> str:
        """返回默认 socket 路径（跨平台，与 Rust claw-pal 保持一致）。"""
        return str(cls._data_dir() / "kernel.sock")

    @classmethod
    def _token_path(cls) -> Path:
        return cls._data_dir() / "kernel.token"

    def _read_token(self) -> str:
        tp = self._token_path()
        if tp.exists():
            return tp.read_text().strip()
        return ""

    def _start_daemon(self, socket_path: str) -> None:
        """尝试在后台启动 claw-kernel-server daemon。"""
        try:
            subprocess.Popen(
                ["claw-kernel-server", "--socket-path", socket_path],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            # 等待 daemon 启动（最多 3 秒）
            import time
            for _ in range(30):
                time.sleep(0.1)
                if Path(socket_path).exists():
                    break
        except FileNotFoundError:
            raise RuntimeError(
                "claw-kernel-server not found in PATH. "
                "Please install claw-kernel-server first."
            )

    def _connect(self, socket_path: Optional[str]) -> None:
        """连接到 Unix socket，如果不存在则尝试启动 daemon。"""
        path = socket_path or self._default_socket_path()

        if not Path(path).exists():
            self._start_daemon(path)

        sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        sock.connect(path)
        self._sock = sock

        # 握手认证
        token = self._read_token()
        result = self._call("kernel.auth", {"token": token})
        if not result.get("ok"):
            raise RuntimeError("kernel.auth failed: invalid token")

    # --- 帧层 ---

    def _send_frame(self, data: bytes) -> None:
        header = struct.pack(">I", len(data))
        self._sock.sendall(header + data)

    def _recv_frame(self) -> bytes:
        header = self._recv_exact(4)
        length = struct.unpack(">I", header)[0]
        if length > 16 * 1024 * 1024:
            raise RuntimeError(f"Frame too large: {length}")
        return self._recv_exact(length)

    def _recv_exact(self, n: int) -> bytes:
        buf = b""
        while len(buf) < n:
            chunk = self._sock.recv(n - len(buf))
            if not chunk:
                raise ConnectionError("Connection closed")
            buf += chunk
        return buf

    # --- RPC 层 ---

    def _next_id(self) -> int:
        with self._lock:
            req_id = self._req_id
            self._req_id += 1
            return req_id

    def _call(self, method: str, params: Optional[dict] = None) -> dict:
        """发送 RPC 请求并等待响应（同步）。"""
        req_id = self._next_id()
        request = {"jsonrpc": "2.0", "method": method, "id": req_id}
        if params is not None:
            request["params"] = params
        self._send_frame(json.dumps(request).encode())

        # 读取响应（跳过通知帧）
        while True:
            raw = self._recv_frame()
            msg = json.loads(raw)
            if "id" in msg and msg["id"] == req_id:
                if "error" in msg:
                    raise RuntimeError(
                        f"RPC error [{msg['error']['code']}]: {msg['error']['message']}"
                    )
                return msg.get("result", {})
            # 通知帧先缓存
            if msg.get("id") is None:
                self._notifications.append(msg)

    # --- 公开 API ---

    def create_session(
        self,
        system_prompt: str = "",
        tools: Optional[list] = None,
        model: Optional[str] = None,
        max_turns: Optional[int] = None,
    ) -> str:
        """创建会话，返回 session_id。"""
        config: dict = {}
        if system_prompt:
            config["system_prompt"] = system_prompt
        if model:
            config["model_override"] = model
        if max_turns is not None:
            config["max_turns"] = max_turns
        if tools:
            config["tools"] = tools
        result = self._call("createSession", {"config": config})
        return result["session_id"]

    def send_message(
        self,
        session_id: str,
        content: str,
        tools: Optional[Dict[str, Callable]] = None,
    ) -> Iterator[str]:
        """
        发送消息，以生成器形式逐个 yield token（流式）。
        若 tools 不为 None，自动处理工具回调循环。
        """
        req_id = self._next_id()
        request = {
            "jsonrpc": "2.0",
            "method": "sendMessage",
            "id": req_id,
            "params": {"session_id": session_id, "content": content},
        }
        self._send_frame(json.dumps(request).encode())

        # 先等 sendMessage 响应
        while True:
            raw = self._recv_frame()
            msg = json.loads(raw)
            if "id" in msg and msg["id"] == req_id:
                break
            if msg.get("id") is None:
                self._notifications.append(msg)

        # 处理缓存的通知 + 继续收 stream
        while True:
            if self._notifications:
                notification = self._notifications.pop(0)
            else:
                raw = self._recv_frame()
                notification = json.loads(raw)

            method = notification.get("method", "")
            params = notification.get("params", {})

            if params.get("session_id") != session_id:
                continue

            if method == "agent/streamChunk":
                delta = params.get("delta", "")
                if delta:
                    yield delta
                if params.get("done"):
                    break

            elif method == "agent/toolCall" and tools:
                tool_name = params.get("tool_name", "")
                tool_call_id = params.get("tool_call_id", "")
                arguments = params.get("arguments", {})

                tool_fn = tools.get(tool_name)
                if tool_fn:
                    try:
                        result = tool_fn(**arguments)
                        success = True
                    except Exception as e:
                        result = str(e)
                        success = False
                else:
                    result = f"Unknown tool: {tool_name}"
                    success = False

                self._call("toolResult", {
                    "session_id": session_id,
                    "tool_call_id": tool_call_id,
                    "result": result,
                    "success": success,
                })

            elif method == "agent/finish":
                break

    def destroy_session(self, session_id: str) -> None:
        """销毁会话，释放服务器端资源。"""
        self._call("destroySession", {"session_id": session_id})

    def info(self) -> dict:
        """获取内核信息（版本、provider 等）。"""
        return self._call("kernel.info")

    def close(self) -> None:
        """关闭连接。"""
        if self._sock:
            self._sock.close()
            self._sock = None

    def __enter__(self):
        return self

    def __exit__(self, *args):
        self.close()
