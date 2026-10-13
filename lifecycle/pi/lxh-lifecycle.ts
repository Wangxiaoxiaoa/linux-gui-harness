/**
 * pi extension: attribute harness display calls to this session.
 *
 * Injects pi's session id into every lxh_* tool call so the harness
 * tracks display ownership by the official session id.
 */
export default function (pi: {
  on: (event: "tool_call", handler: (event: { toolName: string; input: Record<string, unknown> }, ctx: { sessionManager: { getSessionId(): string } }) => void) => void;
}) {
  pi.on("tool_call", (event, ctx) => {
    if (!event.toolName.startsWith("lxh_")) return;
    const meta = (event.input._meta as Record<string, unknown> | undefined) ?? {};
    event.input._meta = { ...meta, session_id: ctx.sessionManager.getSessionId() };
  });
}
