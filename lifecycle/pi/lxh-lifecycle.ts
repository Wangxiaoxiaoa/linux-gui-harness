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
 *    directly over the daemon socket, with two reason-specific overrides:
 *      - "fork":  pi is handing control to a forked session (the event's
 *                 targetSessionFile names it) — stash the display ids to
 *                 /tmp instead of destroying, and re-own them to the new
 *                 session on its session_start.
 *      - "reload": the same session continues with a fresh extension
 *                 runtime — keep the displays untouched.
 */
import net from "node:net";
import fs from "node:fs";

const socketPath = () =>
  process.env.LXH_SOCKET_PATH ??
  `${process.env.XDG_RUNTIME_DIR ?? "/tmp"}/linux-x11-harness.sock`;

// The hash table: session id → Set of display ids it created.
const displayTable = new Map<string, Set<string>>();
// toolCallId → session id, for correlating lxh_display_create results.
const pendingCreates = new Map<string, string>();
// Displays stashed by a fork shutdown, waiting for the forked session's
// session_start: { from, ids, markerFile }.
let pendingInheritance: { from: string; ids: string[]; markerFile: string } | null = null;
let currentSessionId: string | null = null;

const stashFile = (targetSessionFile: string) =>
  `/tmp/lxh-inherit-${encodeURIComponent(targetSessionFile)}.json`;

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

/** One tools/call over the daemon socket; resolves with the raw response. */
function daemonCall(name: string, args: Record<string, unknown>): Promise<any> {
  return new Promise((resolve) => {
    const sock = net.connect(socketPath());
    let done = false;
    const finish = (value: any) => {
      if (done) return;
      done = true;
      sock.destroy();
      resolve(value);
    };
    sock.setTimeout(15000, () => finish(undefined));
    sock.on("error", () => finish(undefined));
    sock.on("connect", () => {
      sock.write(
        JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/call", params: { name, arguments: args } }) +
          "\n",
      );
    });
    let buf = "";
    sock.on("data", (chunk: Buffer) => {
      buf += chunk;
      const nl = buf.indexOf("\n");
      if (nl === -1) return;
      try {
        finish(JSON.parse(buf.slice(0, nl)));
      } catch {
        finish(undefined);
      }
    });
  });
}

/** End-hook: look up the table, close every display bound to the session. */
async function endSessionDisplays(sessionId: string) {
  const ids = displayTable.get(sessionId);
  if (!ids) return;
  displayTable.delete(sessionId);
  // destroys are independent — run them in parallel (each takes ~tens of
  // ms: server-side Xvfb/x11vnc teardown)
  await Promise.all(
    [...ids].map((id) => daemonCall("lxh_display_destroy", { display_id: id, _meta: { session_id: sessionId } })),
  );
}

/** Fork inheritance: hand the stashed displays to the new session. */
async function inheritDisplays(newSessionId: string, markerFile: string) {
  if (!pendingInheritance) return;
  const { from, ids } = pendingInheritance;
  pendingInheritance = null;
  try {
    fs.unlinkSync(markerFile);
  } catch {
    /* already gone */
  }
  for (const id of ids) {
    // Re-own first (only the old owner may), then track under the new id.
    await daemonCall("lxh_display_reown", {
      display_id: id,
      to: newSessionId,
      _meta: { session_id: from },
    });
    bindDisplay(newSessionId, id);
  }
}

interface ToolCallEvent {
  toolName: string;
  toolCallId?: string;
  input: Record<string, unknown>;
}

interface ExtensionCtx {
  sessionManager?: { getSessionId(): string; getSessionFile?(): string };
}

export default function (pi: {
  on: (
    event: "tool_call" | "tool_result" | "session_start" | "session_shutdown",
    handler: (event: Record<string, unknown>, ctx: ExtensionCtx) => void | Promise<void>,
  ) => void;
}) {
  const sessionId = (ctx: ExtensionCtx): string | null =>
    ctx?.sessionManager?.getSessionId?.() ?? currentSessionId;

  pi.on("session_start", async (_event, ctx) => {
    currentSessionId = ctx?.sessionManager?.getSessionId?.() ?? currentSessionId;
    const file = ctx?.sessionManager?.getSessionFile?.();
    const marker = file ? stashFile(file) : null;
    if (marker && fs.existsSync(marker)) {
      try {
        pendingInheritance = JSON.parse(fs.readFileSync(marker, "utf8"));
      } catch {
        pendingInheritance = null;
      }
    }
    const sid = currentSessionId;
    if (sid && pendingInheritance && marker) {
      currentSessionId = sid;
      await inheritDisplays(sid, marker);
    }
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

  // end-hook: fires on quit / reload / session switch — reason decides.
  pi.on("session_shutdown", async (event, ctx) => {
    const reason = String((event as any)?.reason ?? "");
    const sid = sessionId(ctx);
    currentSessionId = null;

    if (reason === "reload") return; // same session continues — keep displays

    if (reason === "fork" && sid) {
      const ids = displayTable.get(sid);
      if (ids && ids.size > 0) {
        const target = String((event as any)?.targetSessionFile ?? "");
        const markerFile = target ? stashFile(target) : stashFile(`fork-${Date.now()}`);
        try {
          fs.writeFileSync(
            markerFile,
            JSON.stringify({ from: sid, ids: [...ids] } satisfies { from: string; ids: string[] }),
          );
          displayTable.delete(sid);
          pendingInheritance = null;
          return; // the forked session's session_start claims the stash
        } catch {
          /* fall through to destroy */
        }
      }
    }

    if (sid) await endSessionDisplays(sid);
  });
}
