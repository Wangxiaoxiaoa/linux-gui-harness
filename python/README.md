# linux-gui-harness Python package

Python wrapper and distribution package for
[linux-gui-harness](https://github.com/Wangxiaoxiaoa/linux-gui-harness).

It bundles the Rust MCP server binary and provides a one-click setup command to
register the skill and MCP server with common agents.

## Install

Requires Python 3.10+, Rust toolchain, and Linux.

```bash
pip install linux-gui-harness
```

During installation, pip compiles the Rust binary from source and bundles it
into the wheel.

## Use the MCP server

```bash
linux-gui-harness
```

The server speaks MCP over stdin/stdout.

## Register with your agent

```bash
linux-gui-harness setup
```

This detects installed agents and registers:

- Skill for **pi**, **Codex**, and OpenCode.
- MCP server for **pi**, **Codex**, and **OpenCode**.

Restart your agent after setup.

## Python API

```python
from linux_gui_harness import get_binary_path

print(get_binary_path())
```

For the full Rust SDK, see the project repository.
