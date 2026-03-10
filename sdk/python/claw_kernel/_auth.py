"""
claw-kernel SDK — Platform paths, token reading, and daemon auto-start.

Mirrors the path resolution logic in the Rust ``claw-pal`` crate so that
the Python SDK discovers the same socket and token files as the daemon.
"""

from __future__ import annotations

import os
import platform
import shutil
import subprocess
import time
from pathlib import Path
from typing import Optional

from .errors import ConnectionError as ClawConnectionError

# Maximum time (seconds) to wait for the daemon socket to appear.
_DEFAULT_DAEMON_TIMEOUT: float = 10.0
# Poll interval (seconds) while waiting for the socket.
_SOCKET_POLL_INTERVAL: float = 0.1


class ClawPaths:
    """Platform-aware path resolver for claw-kernel data files.

    All paths are consistent with the Rust ``claw_pal::dirs::KernelDirs``
    implementation so that the Python SDK and the daemon agree on locations.
    """

    @staticmethod
    def data_dir() -> Path:
        """Return the platform-standard claw-kernel data directory.

        | Platform | Path |
        |----------|------|
        | macOS    | ``~/Library/Application Support/claw-kernel`` |
        | Windows  | ``%LOCALAPPDATA%\\claw-kernel`` |
        | Linux    | ``$XDG_RUNTIME_DIR/claw`` or ``~/.local/share/claw-kernel`` |
        """
        system = platform.system()
        env_override = os.environ.get("CLAW_DATA_DIR")
        if env_override:
            return Path(env_override)

        if system == "Darwin":
            return Path.home() / "Library" / "Application Support" / "claw-kernel"
        elif system == "Windows":
            local_app_data = os.environ.get(
                "LOCALAPPDATA",
                str(Path.home() / "AppData" / "Local"),
            )
            return Path(local_app_data) / "claw-kernel"
        else:  # Linux and other POSIX
            xdg_runtime = os.environ.get("XDG_RUNTIME_DIR")
            if xdg_runtime:
                return Path(xdg_runtime) / "claw"
            return Path.home() / ".local" / "share" / "claw-kernel"

    @classmethod
    def socket_path(cls) -> Path:
        """Return the IPC Unix socket path.

        Can be overridden by the ``CLAW_SOCKET_PATH`` environment variable.
        """
        env_override = os.environ.get("CLAW_SOCKET_PATH")
        if env_override:
            return Path(env_override)
        return cls.data_dir() / "kernel.sock"

    @classmethod
    def token_path(cls) -> Path:
        """Return the path to the authentication token file."""
        return cls.data_dir() / "kernel.token"

    @classmethod
    def pid_path(cls) -> Path:
        """Return the path to the daemon PID file."""
        return cls.data_dir() / "kernel.pid"


def read_token() -> str:
    """Read the authentication token from the token file.

    Returns an empty string if the file does not exist (anonymous access).
    """
    token_path = ClawPaths.token_path()
    try:
        return token_path.read_text(encoding="utf-8").strip()
    except FileNotFoundError:
        return ""
    except OSError:
        return ""


def _find_daemon_binary() -> Optional[str]:
    """Locate the ``claw-kernel-server`` binary.

    Search order:
    1. ``$PATH`` (via :func:`shutil.which`)
    2. ``~/.cargo/bin/claw-kernel-server``
    3. Same directory as this module file
    """
    binary_name = "claw-kernel-server"

    # 1. System PATH
    found = shutil.which(binary_name)
    if found:
        return found

    # 2. Cargo bin
    cargo_bin = Path.home() / ".cargo" / "bin" / binary_name
    if cargo_bin.exists():
        return str(cargo_bin)

    # 3. Package directory
    pkg_dir = Path(__file__).parent.parent
    local_bin = pkg_dir / binary_name
    if local_bin.exists():
        return str(local_bin)

    return None


def start_daemon(
    socket_path: Optional[str] = None,
    timeout: float = _DEFAULT_DAEMON_TIMEOUT,
) -> None:
    """Start the claw-kernel-server daemon in the background.

    The function blocks until the socket file appears (up to *timeout* seconds)
    or raises :class:`~claw_kernel.errors.ConnectionError` on failure.

    Args:
        socket_path: Override the default socket path.
        timeout: How long (in seconds) to wait for the daemon to become ready.

    Raises:
        ConnectionError: If the binary is not found or the daemon fails to start.
    """
    path = socket_path or str(ClawPaths.socket_path())

    binary = _find_daemon_binary()
    if binary is None:
        raise ClawConnectionError(
            "claw-kernel-server not found in PATH, ~/.cargo/bin, or the package directory.\n"
            "Install it with:  cargo install claw-kernel"
        )

    try:
        subprocess.Popen(
            [binary, "--socket-path", path],
            env=os.environ.copy(),
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
    except OSError as exc:
        raise ClawConnectionError(f"Failed to start claw-kernel-server: {exc}") from exc

    # Poll until the socket appears.
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if Path(path).exists():
            return
        time.sleep(_SOCKET_POLL_INTERVAL)

    raise ClawConnectionError(
        f"claw-kernel-server did not become ready within {timeout:.1f}s "
        f"(socket path: {path!r})"
    )
