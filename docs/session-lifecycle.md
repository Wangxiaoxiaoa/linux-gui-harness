# Session lifecycle hooks

Harness displays are scoped to the agent session that created them:
display ownership is keyed by the host's official session id (codex's
`sessionId`, pi's `getSessionId()`, opencode's `sessionID`), carried on
every tool call. When the session ends, every display it created is
destroyed — automatically when the MCP client connection closes, or
explicitly via `lgh_session_end`. Attached external displays are never
destroyed automatically.

Hosts with lifecycle hooks can end a session explicitly instead of
waiting for the disconnect. `lgh_session_end` destroys every harness
display owned by the calling session. It is safe to call at any point in
a session; calling it twice is a no-op the second time.

## Fork inheritance and reown

`/fork` and `/clone` create a new session id, which would leave the
display bound to the old one. `lgh_display_reown` transfers a display
to another session id; only the display's current owner may call it
(the caller's session identity must match), so the cross-session guard
stays intact.

The pi extension uses this automatically: a `session_shutdown` with
reason `fork` stashes the session's display ids (keyed by the target
session file pi reports), and the forked session's `session_start`
re-owns them — the GUI keeps running across the fork. A `reload`
shutdown keeps displays too, since the same session continues. All
other reasons (quit, new, resume) destroy them.

For codex, whose hook set has no tool-level or fork events, a forked
thread cannot reclaim the old thread's displays automatically. Resume
the original thread (its `sessionId` still matches the owner) and call
`lgh_display_reown` from there, or destroy the display and start fresh.

## Identity details

The session key is sticky per connection: an un-attributed request
never strips it, only an explicit new identity may replace it. Displays
created without any session envelope (older proxies, direct socket
clients) have no owner and are reclaimed only by the connection that
created them — never by another connection disconnecting.

## Codex

`linux-gui-harness setup` registers the hook automatically: it merges a
`session_end` entry into `~/.codex/hooks.json` whose `mcp_tool` handler
calls the lgh MCP server directly — no script needed. Manual setup, if
you prefer, is the same shape in `hooks.toml` (or shipped inside a
plugin):

```toml
[hooks.session_end]
[[hooks.session_end.handlers]]
type = "mcp_tool"
server = "linux-gui-harness"
tool = "lgh_session_end"
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

`linux-gui-harness setup` writes the `mcpServers` entry into
`~/.pi/agent/mcp.json` and installs a small extension
(`~/.pi/agent/extensions/lgh-lifecycle.ts`) whose `tool_call` hook
injects the session id into every `lgh_*` call, so displays bind to pi's
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
