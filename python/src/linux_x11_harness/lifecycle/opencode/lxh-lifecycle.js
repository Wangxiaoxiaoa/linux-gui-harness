/**
 * opencode plugin: attribute harness display calls to this session.
 *
 * Injects opencode's session id into every lxh_* tool call so the
 * harness tracks display ownership by the official session id.
 */
export const LxhLifecycle = async () => ({
  "tool.execute.before": async (input, output) => {
    if (!input.tool.startsWith("lxh_")) return;
    output.args = {
      ...output.args,
      _meta: { session_id: input.sessionID },
    };
  },
});
