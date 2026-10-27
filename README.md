<div align="center">

# linux-gui-harness

**Run Linux GUI apps in isolated X11 displays for AI agents.**

[![CI](https://github.com/Wangxiaoxiaoa/linux-gui-harness/actions/workflows/ci.yml/badge.svg)](https://github.com/Wangxiaoxiaoa/linux-gui-harness/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

[English](README.md) · [中文](README.zh-CN.md)

</div>

---

## What is this?

`linux-gui-harness` creates isolated X11 displays and lets AI agents control GUI applications. Each display gets its own X server, window manager, and application process group, so agents can click, type, capture screenshots, and read accessibility trees without touching the host desktop.

A live preview panel opens on your desktop while the agent works, so you can watch every click and screenshot in real time. Previews are read-only by default; double-click one to open an expanded interactive view where your mouse and keyboard directly control the sandboxed app.

Typical uses:

- Let an agent operate a GUI app without taking over your real desktop.
- Watch what the agent is doing live in a small preview window.
- Run end-to-end tests that need real input, focus, and screenshots.
- Capture accessibility trees or click UI elements programmatically.
- Discover installed/running apps and launch them by name.
- Invoke application menus by path and assert UI state after actions.
- Test GUI installers, system settings, and multi-app workflows.
- Verify accessibility compliance by auditing AT-SPI element metadata.
- Read screen text with local OCR — works even without a vision model.

See the [agent skill](skills/linux-gui-harness/SKILL.md) for concrete
workflows and tool usage patterns.

## Quick start

Install and register with your agents in one step:

```bash
pip install linux-gui-harness
linux-gui-harness setup
```

After setup, open your agent and start asking it to use GUI apps. The agent will launch the harness automatically when needed.

Requirements: Python 3.10+, Rust toolchain, Linux with `xvfb` and `openbox`.

## Supported agents

`linux-gui-harness setup` auto-configures the MCP server and the agent skill where the agent supports them. Displays are always destroyed when the owning session ends; Codex can end them earlier via its `SessionEnd` hook.

| Agent | MCP server | Agent skill | Session-end hook |
|---|---|---|---|
| [Codex](https://github.com/openai/codex) | ✅ | ✅ | ✅ (`hooks.json`) |
| [OpenCode](https://opencode.ai/) | ✅ | — | — (disconnect cleanup) |
| [Pi](https://pi.ai/) | ✅ | ✅ | — (disconnect cleanup) |

Other MCP-compatible agents can connect manually with `linux-gui-harness mcp`. See [Session lifecycle](docs/session-lifecycle.md).

## Documentation

- [Usage guide](docs/usage.md) — display previews, sockets, and multi-agent isolation.
- [Agent skill](skills/linux-gui-harness/SKILL.md) — quick reference for agents.
- [Session lifecycle](docs/session-lifecycle.md) — how displays are tied to agent sessions, and per-host hooks.
- [Architecture](docs/ARCHITECTURE.md) — crate layout and design notes.

## License

MIT
