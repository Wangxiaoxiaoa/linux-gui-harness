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

Register the hook in `hooks.toml` (or ship it inside a plugin). The
`mcp_tool` handler calls the lxh MCP server directly — no script needed.

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

A minimal extension forwards pi's session events to the harness:

```typescript
// ~/.pi/agent/extensions/lxh-lifecycle.ts
export default function (pi) {
  const end = () =>
    pi.executeTool("lxh_session_end", {}).catch(() => {});
  pi.on("session_shutdown", end);
}
```

pi sessions are identified by the proxy PID (each session spawns its own
MCP client), so the harness attributes displays to the right session
without extra wiring.

## Other agents

Any agent without lifecycle hooks still works: displays are destroyed
when the agent closes the MCP connection. If the agent kills the whole
process tree (as OpenCode does), the child displays are reaped with it;
stale X11 sockets from a SIGKILLed daemon are swept at the next daemon
startup.
