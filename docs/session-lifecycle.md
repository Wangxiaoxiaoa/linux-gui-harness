# Session lifecycle hooks

Harness displays are scoped to the agent session that created them: when
the session ends, every harness display it created is destroyed
(automatically, when the MCP client connection closes). Attached external
displays are never destroyed automatically.

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
server = "lxh"
tool = "lxh_session_end"
```

With this in place, ending a Codex session destroys its displays the
moment the hook fires, rather than when the host reaps the MCP server
process.

Session identity needs no configuration: Codex attaches
`params._meta.threadId` to every tool call, and the harness tracks
display ownership by it.

## pi

No lifecycle wiring needed. `linux-x11-harness setup` writes the
`mcpServers` entry into `~/.pi/agent/mcp.json`; pi sessions are
identified by the proxy PID (each session spawns its own MCP client),
so the harness attributes displays to the right session and destroys
them when the session's connection closes — without extra wiring.
(A code extension was considered and dropped: pi's event handlers do
not expose tool execution, and the connection-close cleanup already
covers it.)

## Other agents

Any agent without lifecycle hooks still works: displays are destroyed
when the agent closes the MCP connection. If the agent kills the whole
process tree (as OpenCode does), the child displays are reaped with it;
stale X11 sockets from a SIGKILLed daemon are swept at the next daemon
startup.
