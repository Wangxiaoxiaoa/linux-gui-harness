#!/usr/bin/env python3
"""Persistent sandbox keeper: holds the MCP connection open so the
displays created through it stay alive (displays are destroyed when
their creating connection disconnects).

Usage: sandbox-keeper.py [num_sandboxes]
Creates N Wayland-headless sandboxes, each with an xterm, a preview
panel and a VNC viewer, then sleeps forever (SIGTERM/SIGINT cleans up
gracefully through the daemon's session-end path).
"""
import json
import os
import select
import signal
import subprocess
import sys
import time

WSP = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "target", "debug", "linux-gui-harness")
WSP = os.path.normpath(WSP)
RUNTIME = os.path.expanduser("~/wsp-runtime")

N = int(sys.argv[1]) if len(sys.argv) > 1 else 1

env = dict(os.environ)
env["LD_LIBRARY_PATH"] = RUNTIME + "/usr/lib/x86_64-linux-gnu"
env["PATH"] = RUNTIME + "/usr/bin:" + env.get("PATH", "")

proc = subprocess.Popen(
    [WSP, "mcp"],
    stdin=subprocess.PIPE, stdout=subprocess.PIPE,
    stderr=subprocess.DEVNULL, text=True, bufsize=1, env=env,
)
_next_id = [1]


def rpc(method, params=None, timeout=30):
    mid = _next_id[0]
    _next_id[0] += 1
    msg = {"jsonrpc": "2.0", "method": method, "id": mid}
    if params is not None:
        msg["params"] = params
    proc.stdin.write(json.dumps(msg) + "\n")
    proc.stdin.flush()
    end = time.time() + timeout
    while time.time() < end:
        if select.select([proc.stdout], [], [], 0.5)[0]:
            line = proc.stdout.readline()
            if not line:
                raise RuntimeError("daemon connection closed")
            r = json.loads(line)
            if r.get("id") == mid:
                return r
    raise RuntimeError(f"timeout waiting for {method}")


rpc("initialize", {"capabilities": {}})
rpc("notifications/initialized")

worlds = []
for i in range(N):
    r = rpc("tools/call", {"name": "lgh_display_create", "arguments": {
        "width": 1280, "height": 720,
        "backend": "wayland-headless", "name": f"sandbox-{i+1}",
    }})
    if "result" not in r:
        print(f"✗ sandbox {i+1}: {r.get('error', {}).get('message', r)}", flush=True)
        continue
    d = json.loads(r["result"]["content"][0]["text"])
    did = d["display_id"]
    worlds.append(did)
    print(f"✓ sandbox {i+1}: {did} display={d['display']}", flush=True)

    rpc("tools/call", {"name": "lgh_app_launch", "arguments": {
        "display_id": did, "command": "xterm", "args": []}})
    rpc("tools/call", {"name": "lgh_preview_open", "arguments": {"display_id": did}})
    rpc("tools/call", {"name": "lgh_display_enter", "arguments": {"display_id": did}})
    print(f"  xterm + preview + VNC opened", flush=True)

print(f"keeper holding {len(worlds)} sandbox(es) alive — Ctrl-C to stop", flush=True)
try:
    signal.pause()
except KeyboardInterrupt:
    pass
finally:
    for did in worlds:
        try:
            rpc("tools/call", {"name": "lgh_display_destroy", "arguments": {"display_id": did}}, timeout=10)
        except Exception:
            pass
    proc.terminate()
    print("keeper exited, sandboxes cleaned up", flush=True)
