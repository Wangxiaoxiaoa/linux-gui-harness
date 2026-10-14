# Session lifecycle hooks

Harness displays are scoped to the agent session that created them:
display ownership is keyed by the host's official session id (codex's
`sessionId`, pi's `getSessionId()`, opencode's `sessionID`), carried on
every tool call. When the session ends, every display it created is
destroyed — automatically when the MCP client connection closes, or
explicitly via `lxh_session_end`. Attached external displays are never
destroyed automatically.

Hosts with lifecycle hooks can end a session explicitly instead of
waiting for the disconnect. `lxh_session_end` destroys every harness
display owned by the calling session. It is safe to call at any point in
a session; calling it twice is a no-op the second time.

## Codex

`linux-x11-harness setup` registers the hook automatically: it merges a
`session_end` entry into `~/.codex/hooks.json` whose `mcp_tool` handler
calls the lxh MCP server directly — no script needed. Manual setup, if
you prefer, is the same shape in `hooks.toml` (or shipped inside a
plugin):

```toml
[hooks.session_end]
[[hooks.session_end.handlers]]
type = "mcp_tool"
server = "linux-x11-harness"
tool = "lxh_session_end"
```

With this in place, ending a Codex session destroys its displays the
moment the hook fires, rather than when the host reaps the MCP server
process.

Session identity needs no configuration for the ownership key: Codex
attaches `params._meta.sessionId` to every tool call. Subagent threads
resolve to the same session id, so their displays belong to the session;
foreign session ids are rejected by the cross-session guard. Thread ids
are ignored — the harness binds displays to session ids only.

## pi

`linux-x11-harness setup` writes the `mcpServers` entry into
`~/.pi/agent/mcp.json` and installs a small extension
(`~/.pi/agent/extensions/lxh-lifecycle.ts`) whose `tool_call` hook
injects the session id into every `lxh_*` call, so displays bind to pi's
official session id. Its `session_shutdown` hook — pi fires it on quit,
reload, and in-process session switches (`/new`, `/fork`, `/clone`,
`/resume`) — ends the session explicitly over the daemon socket.
pi also closes the MCP client on `session_shutdown`, so the daemon's
connection-close cleanup backs the explicit call up; both paths are
idempotent.

## Other agents

Any agent without lifecycle hooks still works: displays are destroyed
when the agent closes the MCP connection. If the agent kills the whole
process tree (as OpenCode does), the child displays are reaped with it;
stale X11 sockets from a SIGKILLed daemon are swept at the next daemon
startup.
