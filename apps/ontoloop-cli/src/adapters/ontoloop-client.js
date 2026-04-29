import fs from "node:fs";
import path from "node:path";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { FrontendRuntimeEvent } from "../cli/cmd/tui/event.js";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const DEFAULT_WORKSPACE_ROOT = path.resolve(__dirname, "..", "..", "..", "..");

function now() {
  return Date.now();
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function safeJsonParse(text, fallback = {}) {
  try {
    return JSON.parse(text);
  } catch {
    return fallback;
  }
}

function chooseAutocogInvocation(workspaceRoot) {
  const explicit = process.env.ONTOLOOP_AUTOCOG_BIN;
  if (explicit && explicit.trim().length > 0) {
    return { bin: explicit, fixedArgs: [] };
  }

  const debugExe = path.join(workspaceRoot, "target", "debug", "ontoloop.exe");
  if (fs.existsSync(debugExe)) {
    return { bin: debugExe, fixedArgs: [] };
  }

  return {
    bin: "cargo",
    fixedArgs: ["run", "--manifest-path", path.join(workspaceRoot, "Cargo.toml"), "--"],
  };
}

export class OntoLoopClient {
  constructor({
    baseUrl = "http://127.0.0.1:8787",
    workspaceRoot = DEFAULT_WORKSPACE_ROOT,
    pollIntervalMs = 800,
  } = {}) {
    this.baseUrl = baseUrl;
    this.workspaceRoot = workspaceRoot;
    this.pollIntervalMs = pollIntervalMs;
    this.pendingPermissionRequests = new Map();
    this.syntheticEventQueue = [];
    this.lastSequenceBySession = new Map();
    this.attachStateBySession = new Map();
    this.invocation = chooseAutocogInvocation(this.workspaceRoot);
  }

  async runAutocog(args, { allowNonZero = false } = {}) {
    const finalArgs = [...this.invocation.fixedArgs, ...args];
    const child = spawn(this.invocation.bin, finalArgs, {
      cwd: this.workspaceRoot,
      stdio: ["ignore", "pipe", "pipe"],
      env: process.env,
      windowsHide: true,
    });

    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (chunk) => {
      stdout += chunk.toString();
    });
    child.stderr.on("data", (chunk) => {
      stderr += chunk.toString();
    });

    const exitCode = await new Promise((resolve, reject) => {
      child.on("error", reject);
      child.on("close", (code) => resolve(code ?? 0));
    });

    if (!allowNonZero && exitCode !== 0) {
      const error = new Error(
        `autocog command failed (${this.invocation.bin} ${finalArgs.join(" ")}): ${stderr || stdout}`,
      );
      error.exitCode = exitCode;
      throw error;
    }

    return { exitCode, stdout: stdout.trim(), stderr: stderr.trim() };
  }

  enqueueSynthetic(event) {
    this.syntheticEventQueue.push(event);
  }

  async createSession({ sessionID }) {
    const result = await this.runAutocog(["--session", sessionID, "system", "status"]);
    const parsed = safeJsonParse(result.stdout, { raw: result.stdout });
    this.enqueueSynthetic({
      type: FrontendRuntimeEvent.READY,
      sessionID,
      sequence: 1,
      emittedAt: now(),
      payload: { state: parsed },
    });
    this.lastSequenceBySession.set(sessionID, Math.max(this.lastSequenceBySession.get(sessionID) || 0, 1));
    return {
      sessionID,
      baseUrl: this.baseUrl,
      createdAt: now(),
      status: parsed,
    };
  }

  async attachSession({
    sessionID,
    transportKind = "cli",
    jwtToken,
    subject,
    tenantID,
    ttlMs = 3600000,
  }) {
    const args = ["--session", sessionID, "frontend", "attach", "--transport-kind", transportKind];
    if (jwtToken) {
      args.push("--jwt", jwtToken);
    }
    if (subject) {
      args.push("--subject", subject);
    }
    if (tenantID) {
      args.push("--tenant-id", tenantID);
    }
    if (Number.isFinite(Number(ttlMs))) {
      args.push("--ttl-ms", String(ttlMs));
    }
    const result = await this.runAutocog(args);
    const parsed = safeJsonParse(result.stdout, { raw: result.stdout });
    this.attachStateBySession.set(sessionID, {
      transportKind,
      jwtToken: jwtToken || null,
      subject: subject || null,
      tenantID: tenantID || null,
      ttlMs,
      lastAttachedAt: now(),
    });
    return parsed;
  }

  async prompt({ sessionID, text }) {
    const traceID = `trace:${sessionID}:frontend:${now()}`;
    const result = await this.runAutocog([
      "--session",
      sessionID,
      "frontend",
      "prompt",
      "--trace-id",
      traceID,
      "--content",
      text,
    ]);
    const parsed = safeJsonParse(result.stdout, {});
    const events = Array.isArray(parsed.events) ? parsed.events : [];
    for (const event of events) {
      const sequence = Number(event.sequence || 0);
      if (sequence > 0) {
        this.lastSequenceBySession.set(sessionID, sequence);
      }
    }
    return {
      accepted: true,
      sessionID,
      queuedAt: now(),
      traceID,
      output: parsed.response || result.stdout,
    };
  }

  async command({ sessionID, command }) {
    const trimmed = String(command || "").trim();
    const tokens = trimmed.startsWith("/") ? trimmed.slice(1).trim().split(/\s+/) : trimmed.split(/\s+/);
    const primary = (tokens[0] || "").toLowerCase();

    let args;
    if (primary === "status") {
      args = ["--session", sessionID, "system", "status"];
    } else if (primary === "health") {
      args = ["--session", sessionID, "system", "health"];
    } else if (primary === "bridge") {
      args = ["--session", sessionID, "bridge", "status"];
    } else if (primary === "trigger" && (tokens[1] || "").toLowerCase() === "list") {
      args = ["--session", sessionID, "trigger", "list"];
    } else if (primary === "permission" && (tokens[1] || "").toLowerCase() === "mode") {
      args = ["--session", sessionID, "system", "permission-mode"];
    } else {
      // generic passthrough for "/command ..."
      const passthrough = trimmed.startsWith("/") ? trimmed.slice(1).trim().split(/\s+/) : tokens;
      args = ["--session", sessionID, ...passthrough];
    }

    const result = await this.runAutocog(args, { allowNonZero: true });
    const isError = result.exitCode !== 0;
    const payloadText = result.stdout || result.stderr || "(no output)";

    const nextSequence = (this.lastSequenceBySession.get(sessionID) || 0) + 1;
    const emittedAt = now();
    this.enqueueSynthetic({
      type: FrontendRuntimeEvent.ASSISTANT_DELTA,
      sessionID,
      sequence: nextSequence,
      emittedAt,
      payload: {
        turn_id: `turn:${nextSequence}`,
        delta: payloadText,
      },
    });
    if (isError) {
      this.enqueueSynthetic({
        type: FrontendRuntimeEvent.ERROR,
        sessionID,
        sequence: nextSequence + 1,
        emittedAt: emittedAt + 1,
        payload: {
          code: "command_failed",
          message: payloadText,
        },
      });
      this.lastSequenceBySession.set(sessionID, nextSequence + 1);
    } else {
      this.enqueueSynthetic({
        type: FrontendRuntimeEvent.SESSION_IDLE,
        sessionID,
        sequence: nextSequence + 1,
        emittedAt: emittedAt + 1,
        payload: {},
      });
      this.lastSequenceBySession.set(sessionID, nextSequence + 1);
    }

    return {
      accepted: !isError,
      sessionID,
      command: trimmed,
      queuedAt: emittedAt,
      output: payloadText,
      exitCode: result.exitCode,
    };
  }

  // D3 adapter supports reply tracking even before dedicated backend endpoint.
  async permissionReply({ requestID, reply }) {
    this.pendingPermissionRequests.set(requestID, {
      requestID,
      reply,
      acceptedAt: now(),
    });
    return this.pendingPermissionRequests.get(requestID);
  }

  async *eventStream({ sessionID, signal, reconnect = true } = {}) {
    let consecutiveFailures = 0;
    while (!signal?.aborted) {
      while (this.syntheticEventQueue.length > 0) {
        const event = this.syntheticEventQueue.shift();
        if (event.sessionID === sessionID) {
          yield event;
        }
      }

      const result = await this.runAutocog(
        ["--session", sessionID, "frontend", "events", "--format", "json", "--limit", "100"],
        { allowNonZero: true },
      );

      if (result.exitCode !== 0) {
        consecutiveFailures += 1;
        yield {
          type: FrontendRuntimeEvent.ERROR,
          sessionID,
          sequence: (this.lastSequenceBySession.get(sessionID) || 0) + 1,
          emittedAt: now(),
          payload: {
            code: "event_poll_failed",
            message: result.stderr || result.stdout || "event polling failed",
          },
        };
        if (reconnect && consecutiveFailures >= 3) {
          const attachState = this.attachStateBySession.get(sessionID);
          if (attachState) {
            try {
              await this.attachSession({
                sessionID,
                transportKind: attachState.transportKind,
                jwtToken: attachState.jwtToken || undefined,
                subject: attachState.subject || undefined,
                tenantID: attachState.tenantID || undefined,
                ttlMs: attachState.ttlMs,
              });
              yield {
                type: FrontendRuntimeEvent.STATE_SNAPSHOT,
                sessionID,
                sequence: (this.lastSequenceBySession.get(sessionID) || 0) + 1,
                emittedAt: now(),
                payload: { status: "reconnected", reason: "event_poll_failed_retry" },
              };
              consecutiveFailures = 0;
            } catch (reconnectError) {
              yield {
                type: FrontendRuntimeEvent.ERROR,
                sessionID,
                sequence: (this.lastSequenceBySession.get(sessionID) || 0) + 1,
                emittedAt: now(),
                payload: {
                  code: "event_reconnect_failed",
                  message: reconnectError?.message || String(reconnectError),
                },
              };
            }
          }
        }
        await sleep(this.pollIntervalMs);
        continue;
      }
      consecutiveFailures = 0;

      const parsed = safeJsonParse(result.stdout, { events: [] });
      const events = Array.isArray(parsed.events) ? parsed.events : [];
      const lastSeen = this.lastSequenceBySession.get(sessionID) || 0;

      for (const event of events) {
        const sequence = Number(event.sequence || 0);
        if (!Number.isFinite(sequence) || sequence <= lastSeen) {
          continue;
        }
        this.lastSequenceBySession.set(sessionID, sequence);
        yield {
          type: String(event.event_type || "").toLowerCase(),
          sessionID: event.session_id || sessionID,
          sequence,
          emittedAt: Number(event.emitted_at_ms || now()),
          payload: event.payload || {},
        };
      }

      await sleep(this.pollIntervalMs);
    }
  }
}
