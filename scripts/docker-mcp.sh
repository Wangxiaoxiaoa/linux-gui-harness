#!/usr/bin/env bash
# MCP stdio bridge to a running linux-gui-harness Docker container.
# Use this as the MCP server command in agent configs.
set -euo pipefail

CONTAINER="${LGH_CONTAINER:-linux-gui-harness}"
exec docker exec -i "$CONTAINER" linux-gui-harness "$@"
