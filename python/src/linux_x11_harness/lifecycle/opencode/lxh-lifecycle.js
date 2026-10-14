/**
 * opencode plugin — lxh display lifecycle, literal contract:
 *
 * 1. create-hook: `tool.execute.before` grabs the session id (and injects
 *    it so the daemon attributes the create); `tool.execute.after` catches
 *    the lxh_display_create result and binds session id → display id in
 *    the local hash table below.
 * 2. end-hook: when the active root session changes (opencode's /new
 *    creates the new session on its first prompt → session.created) or a
 *    session is deleted, look up the table for that session's display ids
 *    and close each one directly over the daemon socket.
 */
import net from "node:net";

const socketPath = () =>
  process.env.LXH_SOCKET_PATH ??
  `${process.env.XDG_RUNTIME_DIR ?? "/tmp"}/linux-x11-harness.sock`;

// The hash table: session id → Set of display ids it created.
const displayTable = new Map();
// Last session that touched an lxh tool — the switch-detection anchor.
let activeSession = null;

// opencode/pi MCP tool names may be bare ("lxh_display_create") or
// server-prefixed ("mcp__linux-x11-harness__lxh_display_create") — accept
// any convention whose tail is an lxh tool name.
const isLxhTool = (name) => /(^|[^a-z0-9])lxh_[a-z0-9_]+$/.test(name ?? "");
const isCreateCall = (name) => /lxh_display_create$/.test(name ?? "");

// display_id survives content-array wrapping and JSON escaping alike.
const extractDisplayId = (text) =>
  String(text ?? "").match(/display_id\\?":\\?"([^"\\]+)/)?.[1] ?? null;

function bindDisplay(sessionId, displayId) {
  let set = displayTable.get(sessionId);
  if (!set) displayTable.set(sessionId, (set = new Set()));
  set.add(displayId);
}

/** Direct daemon-socket call (dsk): close one display. */
function destroyDisplay(sessionId, displayId) {
  return new Promise((resolve) => {
    const sock = net.connect(socketPath());
    let done = false;
    const finish = () => {
      if (done) return;
      done = true;
      sock.destroy();
      resolve();
    };
    sock.setTimeout(15000, finish);
    sock.on("error", finish);
    sock.on("connect", () => {
      sock.write(
        JSON.stringify({
          jsonrpc: "2.0",
          id: 1,
          method: "tools/call",
          params: {
            name: "lxh_display_destroy",
            arguments: { display_id: displayId, _meta: { session_id: sessionId } },
          },
        }) + "\n",
      );
    });
    sock.on("data", finish); // any response ends it; errors are fine here
  });
}

/** End-hook: look up the table, close every display bound to the session. */
async function endSessionDisplays(sessionId) {
  const ids = displayTable.get(sessionId);
  if (!ids) return;
  displayTable.delete(sessionId);
  // destroys are independent — run them in parallel (each takes ~9s:
  // server-side Xvfb/x11vnc teardown)
  await Promise.all([...ids].map((id) => destroyDisplay(sessionId, id)));
}

export const LxhLifecycle = async () => ({
  "tool.execute.before": async (input, output) => {
    if (!isLxhTool(input.tool)) return;
    output.args = {
      ...output.args,
      _meta: { session_id: input.sessionID },
    };
    activeSession = input.sessionID;
  },

  "tool.execute.after": async (input, output) => {
    if (!isCreateCall(input.tool) || !input.sessionID) return;
    const displayId = extractDisplayId(JSON.stringify(output));
    if (displayId) bindDisplay(input.sessionID, displayId);
  },

  event: async ({ event }) => {
    if (event.type === "session.created") {
      const info = event.properties?.info;
      if (!info || info.parentID) return; // subagent session — not a switch
      const prev = activeSession;
      activeSession = info.id;
      if (prev && prev !== info.id) await endSessionDisplays(prev);
      return;
    }
    if (event.type === "session.deleted") {
      const id = event.properties?.info?.id ?? event.properties?.sessionID;
      if (id) await endSessionDisplays(id);
    }
  },
});
