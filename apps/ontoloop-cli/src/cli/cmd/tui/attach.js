import { cmd } from "../cmd.js";

export const AttachCommand = cmd({
  name: "attach",
  describe: "attach to a running OntoLoop endpoint",
  usage:
    "ontoloop-cli attach <url> [--session <id>] [--jwt <token>] [--transport <kind>] [--subject <id>] [--tenant <id>] [--ttl-ms <ms>]",
  async run(ctx) {
    const url = ctx.argv[0];
    if (!url) {
      throw new Error("attach requires <url>");
    }
    const sessionID = ctx.flags.session || "cli:focus";
    return ctx.runThread({
      sessionID,
      baseUrl: url,
      attach: true,
      jwtToken: ctx.flags.jwt || undefined,
      transportKind: ctx.flags.transport || "cli",
      subject: ctx.flags.subject || undefined,
      tenantID: ctx.flags.tenant || undefined,
      ttlMs: ctx.flags["ttl-ms"] ? Number(ctx.flags["ttl-ms"]) : undefined,
    });
  },
});
