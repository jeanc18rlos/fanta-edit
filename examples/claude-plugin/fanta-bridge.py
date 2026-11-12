#!/usr/bin/env python3
"""Attach-or-launch bridge from MCP stdio to the Fanta live MCP socket.

The running app advertises its live MCP server (a Unix socket speaking
newline-delimited JSON-RPC, the same framing as MCP stdio) in
`<data-dir>/fanta_live_mcp.json`. This bridge attaches to it and pumps
bytes both ways; when no live server is up it serves the project headlessly
with `fanta-mcp` (the same 14 tools, no app), and only launches the app when
no headless server is available either. Attaching needs no Fanta binary at
all — only the fallbacks do.

Configuration, all optional:
    FANTA_BIN            the fanta binary for launching (default: `fanta` on PATH)
    FANTA_MCP_BIN        the headless server (default: `fanta-mcp` on PATH)
    FANTA_PROJECT        the project directory (default: the working directory,
                         when it holds a fanta.json)
    FANTA_USER_DATA_DIR  isolated profile, used to find the discovery file
    FANTA_LAUNCH_TIMEOUT seconds to wait for the live server (default 60)

The bridge never prints to stdout: stdout belongs to the MCP protocol.
"""

import json
import os
import shutil
import socket
import subprocess
import sys
import threading
import time
from pathlib import Path


def data_dir() -> Path:
    custom = os.environ.get("FANTA_USER_DATA_DIR", "").strip()
    if custom:
        return Path(custom)
    if sys.platform == "darwin":
        return Path.home() / "Library" / "Application Support" / "Fanta"
    if sys.platform.startswith("linux"):
        base = os.environ.get("XDG_DATA_HOME", "").strip()
        root = Path(base) if base else Path.home() / ".local" / "share"
        return root / "fanta"
    raise RuntimeError(f"unsupported platform: {sys.platform}")


def live_socket_path(discovery: Path):
    try:
        record = json.loads(discovery.read_text())
        pid = int(record["pid"])
        path = record["socket"]
    except (OSError, ValueError, KeyError):
        return None
    try:
        os.kill(pid, 0)
    except OSError:
        return None
    return path


def headless_server():
    binary = os.environ.get("FANTA_MCP_BIN", "").strip() or shutil.which("fanta-mcp")
    if not binary or not Path(binary).exists():
        return None
    project = os.environ.get("FANTA_PROJECT", "").strip() or os.getcwd()
    if not (Path(project) / "fanta.json").exists():
        return None
    return [binary, project]


def launch_app(discovery: Path):
    fanta = os.environ.get("FANTA_BIN", "").strip() or shutil.which("fanta")
    if not fanta or not Path(fanta).exists():
        print(
            "fanta-bridge: no running Fanta app, no `fanta-mcp` for headless "
            "serving, and no binary to launch the app; start the app, or set "
            "FANTA_MCP_BIN or FANTA_BIN (or put one on PATH)",
            file=sys.stderr,
        )
        return None
    launch = [fanta]
    project = os.environ.get("FANTA_PROJECT", "").strip()
    if project:
        launch.append(project)
    # The app must outlive this bridge process: detach it fully.
    subprocess.Popen(
        launch,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
    )
    deadline = time.monotonic() + float(os.environ.get("FANTA_LAUNCH_TIMEOUT", "60"))
    while True:
        path = live_socket_path(discovery)
        if path is not None:
            return path
        if time.monotonic() > deadline:
            print(
                "fanta-bridge: launched the app but no live MCP server "
                f"appeared at {discovery} (settings: fanta_live_mcp.enabled)",
                file=sys.stderr,
            )
            return None
        time.sleep(0.25)


def write_all(fd: int, chunk: bytes):
    view = memoryview(chunk)
    while view:
        view = view[os.write(fd, view) :]


def main() -> int:
    # shortcut: Unix sockets only (Linux and macOS); upgrade when the app
    # gains a Windows live-MCP transport.
    if os.name == "nt":
        print("fanta-bridge: Windows is not supported yet", file=sys.stderr)
        return 2

    discovery = data_dir() / "fanta_live_mcp.json"
    path = live_socket_path(discovery)
    if path is None:
        headless = headless_server()
        if headless is not None:
            # Replaces the bridge: MCP stdio flows straight to fanta-mcp.
            os.execv(headless[0], headless)
        path = launch_app(discovery)
    if path is None:
        return 2

    stream = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    stream.connect(path)

    def stdin_to_socket():
        try:
            while True:
                chunk = os.read(0, 65536)
                if not chunk:
                    break
                stream.sendall(chunk)
        except OSError:
            pass
        finally:
            try:
                stream.shutdown(socket.SHUT_WR)
            except OSError:
                pass

    threading.Thread(target=stdin_to_socket, daemon=True).start()
    try:
        while True:
            chunk = stream.recv(65536)
            if not chunk:
                break
            write_all(1, chunk)
    except OSError:
        pass
    return 0


if __name__ == "__main__":
    sys.exit(main())
