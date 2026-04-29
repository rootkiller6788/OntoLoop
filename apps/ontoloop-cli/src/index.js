#!/usr/bin/env node
import { AttachCommand } from "./cli/cmd/tui/attach.js";
import { ThreadCommand } from "./cli/cmd/tui/thread.js";
import { UI } from "./cli/ui.js";
import { runThreadMode } from "./runtime/thread-runtime.js";

function parseArgv(argv) {
  const [command, ...rest] = argv;
  const args = [];
  const flags = {};
  for (let i = 0; i < rest.length; i += 1) {
    const token = rest[i];
    if (!token.startsWith("--")) {
      args.push(token);
      continue;
    }
    const key = token.slice(2);
    const next = rest[i + 1];
    if (!next || next.startsWith("--")) {
      flags[key] = true;
      continue;
    }
    flags[key] = next;
    i += 1;
  }
  return { command: command || "thread", args, flags };
}

function usage() {
  return [
    "ontoloop-cli minimal frontend",
    "",
    "Commands:",
    "  ontoloop-cli thread [--session <id>] [--url <url>] [--transport <kind>] [--jwt <token>]",
    "  ontoloop-cli attach <url> [--session <id>] [--transport <kind>] [--jwt <token>]",
  ].join("\n");
}

async function main() {
  const parsed = parseArgv(process.argv.slice(2));
  const commandMap = new Map([
    [ThreadCommand.name, ThreadCommand],
    [AttachCommand.name, AttachCommand],
  ]);
  const selected = commandMap.get(parsed.command);
  if (!selected) {
    UI.error(`unknown command: ${parsed.command}`);
    UI.println(usage());
    process.exitCode = 1;
    return;
  }

  try {
    await selected.run({
      argv: parsed.args,
      flags: parsed.flags,
      runThread: runThreadMode,
    });
  } catch (error) {
    UI.error(error?.message || String(error));
    process.exitCode = 1;
  }
}

await main();
