// Minimal event contract aligned with OntoLoop CLI frontend v1.
export const TuiEvent = {
  PromptAppend: "tui.prompt.append",
  CommandExecute: "tui.command.execute",
  SessionSelect: "tui.session.select",
  ToastShow: "tui.toast.show",
};

export const FrontendRuntimeEvent = {
  READY: "ready",
  STATE_SNAPSHOT: "state_snapshot",
  ASSISTANT_DELTA: "assistant_delta",
  TOOL_STARTED: "tool_started",
  TOOL_COMPLETED: "tool_completed",
  PERMISSION_ASKED: "permission_asked",
  SESSION_IDLE: "session_idle",
  ERROR: "error",
};
