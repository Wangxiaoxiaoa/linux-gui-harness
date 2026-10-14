/**
 * pi extension — lxh display lifecycle, literal contract:
 *
 * 1. create-hook: `tool_call` grabs the session id (and injects it so the
 *    daemon attributes the create); `tool_result` of the matching
 *    lxh_display_create call binds session id → display id in the local
 *    hash table below (correlated via toolCallId/tool_use_id).
 * 2. end-hook: `session_shutdown` — pi fires it on quit, reload, and
 *    in-process session switches (/new, /fork, /clone, /resume) — looks
 *    up the table for the dying session's display ids and closes each one
 *    directly over the daemon socket.
 */
import net from "node:net";

const socketPath = () =>
  process.env.LXH_SOCKET_PATH ??
  `${process.env.XDG_RUNTIME_DIR ?? "/tmp"}/linux-x11-harness.sock`;

// The hash table: session id → Set of display ids it created.
const displayTable = new Map<string, Set<string>>();
// toolCallId → session id, for correlating lxh_display_create results.
const pendingCreates = new Map<string, string>();
let currentSessionId: string | null = null;

// pi exposes MCP tools as "mcp__<server>__<tool>" (codemode) or bare —
// accept any convention whose tail is an lxh tool name.
const isLxhTool = (name: unknown) => /(^|[^a-z0-9])lxh_[a-z0-9_]+$/.test(String(name ?? ""));
const isCreateCall = (name: unknown) => /lxh_display_create$/.test(String(name ?? ""));

// display_id survives content-array wrapping and JSON escaping alike.
const extractDisplayId = (text: unknown) =>
  String(text ?? "").match(/display_id\\?":\\?"([^"\\]+)/)?.[1] ?? null;

function bindDisplay(sessionId: string, displayId: string) {
  let set = displayTable.get(sessionId);
  if (!set) displayTable.set(sessionId, (set = new Set<string>()));
  set.add(displayId);
}

/** Direct daemon-socket call (dsk): close one display. */
function destroyDisplay(sessionId: string, displayId: string): Promise<void> {
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
async function endSessionDisplays(sessionId: string) {
  const ids = displayTable.get(sessionId);
  if (!ids) return;
  displayTable.delete(sessionId);
  // destroys are independent — run them in parallel (each takes ~9s:
  // server-side Xvfb/x11vnc teardown)
  await Promise.all([...ids].map((id) => destroyDisplay(sessionId, id)));
}

interface ToolCallEvent {
  toolName: string;
  toolCallId?: string;
  input: Record<string, unknown>;
}

interface ExtensionCtx {
  sessionManager?: { getSessionId(): string };
}

export default function (pi: {
  on: (
    event: "tool_call" | "tool_result" | "session_start" | "session_shutdown",
    handler: (event: Record<string, unknown>, ctx: ExtensionCtx) => void | Promise<void>,
  ) => void;
}) {
  const sessionId = (ctx: ExtensionCtx): string | null =>
    ctx?.sessionManager?.getSessionId?.() ?? currentSessionId;

  pi.on("session_start", (_event, ctx) => {
    currentSessionId = ctx?.sessionManager?.getSessionId?.() ?? currentSessionId;
  });

  // create-hook, first half: grab the session id and remember the call.
  pi.on("tool_call", (event, ctx) => {
    const ev = event as unknown as ToolCallEvent;
    if (!isLxhTool(ev.toolName)) return;
    const sid = sessionId(ctx);
    if (sid) currentSessionId = sid;
    const meta = (ev.input._meta as Record<string, unknown> | undefined) ?? {};
    if (sid) ev.input._meta = { ...meta, session_id: sid };
    if (isCreateCall(ev.toolName) && ev.toolCallId && sid) {
      pendingCreates.set(ev.toolCallId, sid);
    }
  });

  // create-hook, second half: bind display id once the result arrives.
  pi.on("tool_result", (event, ctx) => {
    const ev = event as unknown as { tool_use_id?: string; content?: unknown };
    const sid = (ev.tool_use_id && pendingCreates.get(ev.tool_use_id)) || null;
    if (!sid) return;
    pendingCreates.delete(ev.tool_use_id as string);
    void ctx;
    const displayId = extractDisplayId(JSON.stringify(ev.content ?? ev));
    if (displayId) bindDisplay(sid, displayId);
  });

  // end-hook: fires on quit / reload / session switch.
  pi.on("session_shutdown", async (_event, ctx) => {
    const sid = sessionId(ctx);
    currentSessionId = null;
    if (!sid) return;
    await endSessionDisplays(sid);
  });
}
