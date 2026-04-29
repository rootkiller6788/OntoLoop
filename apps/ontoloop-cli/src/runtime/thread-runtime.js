import { OntoLoopClient } from "../adapters/ontoloop-client.js";
import { FrontendRuntimeEvent } from "../cli/cmd/tui/event.js";
import { UI } from "../cli/ui.js";

function renderEvent(event) {
  switch (event.type) {
    case FrontendRuntimeEvent.READY:
      UI.println("[ready]", JSON.stringify(event.payload));
      return;
    case FrontendRuntimeEvent.STATE_SNAPSHOT:
      UI.println("[state]", JSON.stringify(event.payload));
      return;
    case FrontendRuntimeEvent.ASSISTANT_DELTA:
      UI.println("[assistant]", event.payload?.delta || "");
      return;
    case FrontendRuntimeEvent.TOOL_STARTED:
      UI.println("[tool_started]", JSON.stringify(event.payload));
      return;
    case FrontendRuntimeEvent.TOOL_COMPLETED:
      UI.println("[tool_completed]", JSON.stringify(event.payload));
      return;
    case FrontendRuntimeEvent.PERMISSION_ASKED:
      UI.println("[permission]", JSON.stringify(event.payload));
      return;
    case FrontendRuntimeEvent.SESSION_IDLE:
      UI.println("[idle]");
      return;
    case FrontendRuntimeEvent.ERROR:
      UI.error(`[error] ${event.payload?.message || "unknown error"}`);
      return;
    default:
      UI.println("[event]", JSON.stringify(event));
  }
}

export async function runThreadMode({
  sessionID,
  baseUrl,
  attach,
  jwtToken,
  transportKind,
  subject,
  tenantID,
  ttlMs,
}) {
  const client = new OntoLoopClient({ baseUrl });
  await client.createSession({ sessionID });
  await client.attachSession({
    sessionID,
    transportKind: transportKind || "cli",
    jwtToken,
    subject,
    tenantID,
    ttlMs: ttlMs || 3600000,
  });
  UI.println(
    attach ? "Attached OntoLoop CLI frontend:" : "Started OntoLoop CLI frontend:",
    `session=${sessionID}`,
    `url=${baseUrl}`,
    `transport=${transportKind || "cli"}`,
    jwtToken ? "mode=jwt" : "mode=local",
  );

  for await (const event of client.eventStream({ sessionID })) {
    renderEvent(event);
  }
}
