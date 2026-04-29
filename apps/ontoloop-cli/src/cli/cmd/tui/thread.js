import { cmd } from "../cmd.js";

export const ThreadCommand = cmd({
  name: "thread",
  describe: "start OntoLoop CLI thread mode",
  usage:
    "ontoloop-cli thread [--session <id>] [--url <url>] [--transport <kind>] [--jwt <token>] [--subject <id>] [--tenant <id>] [--ttl-ms <ms>]",
  async run(ctx) {
    const sessionID = ctx.flags.session || "cli:focus";
    const baseUrl = ctx.flags.url || "http://127.0.0.1:8787";
    return ctx.runThread({
      sessionID,
      baseUrl,
      attach: false,
      jwtToken: ctx.flags.jwt || undefined,
      transportKind: ctx.flags.transport || "cli",
      subject: ctx.flags.subject || undefined,
      tenantID: ctx.flags.tenant || undefined,
      ttlMs: ctx.flags["ttl-ms"] ? Number(ctx.flags["ttl-ms"]) : undefined,
    });
  },
});
