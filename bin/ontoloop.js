#!/usr/bin/env node
// OntoLoop — sovereign AI harness
// Usage: ontoloop [--message "prompt"] [tui|focus|mcp|...]
const { spawn } = require("child_process");
const path = require("path");
const fs = require("fs");

const ONTOLOOP_DIR = path.resolve(__dirname, "..");

function rustBinary() {
  const ext = process.platform === "win32" ? ".exe" : "";
  const release = path.join(ONTOLOOP_DIR, "target", "release", `ontoloop${ext}`);
  const debug = path.join(ONTOLOOP_DIR, "target", "debug", `ontoloop${ext}`);
  if (fs.existsSync(release)) return release;
  if (fs.existsSync(debug)) return debug;
  // cargo install path: the binary IS ontoloop, called via npm link
  const cargoInstall = path.join(ONTOLOOP_DIR, `ontoloop${ext}`);
  if (fs.existsSync(cargoInstall)) return cargoInstall;
  return null;
}

async function main() {
  const args = process.argv.slice(2);
  const hasMessage = args.includes("--message");

  let bin = rustBinary();
  if (!bin) {
    // If npm-linked, try the cargo-installed binary
    const cargoBin = process.env.CARGO_HOME
      ? path.join(process.env.CARGO_HOME, "bin", "ontoloop")
      : null;
    if (cargoBin) {
      const ext = process.platform === "win32" ? ".exe" : "";
      const full = cargoBin + ext;
      if (fs.existsSync(full)) bin = full;
    }
  }

  if (!bin) {
    console.error("[ontoloop] Rust binary not found.");
    console.error("  Install: cargo install --git https://github.com/anomalyco/ontoloop-ai");
    console.error("  Or build: cd ontoloop && cargo build");
    process.exit(1);
  }

  const env = { ...process.env };

  if (hasMessage) {
    // CLI mode: pass --tenant for auto-identity
    const child = spawn(bin, [
      "--tenant", "ontoloop-user",
      "--principal", "ontoloop-user",
      "--policy", "full-access",
      "--session", "cli-" + Date.now(),
      ...args,
    ], { env, stdio: "inherit" });
    child.on("exit", (code) => process.exit(code || 0));
  } else if (args.length > 0) {
    // Subcommand mode: ontoloop tui / focus / mcp / ...
    const child = spawn(bin, args, { env, stdio: "inherit" });
    child.on("exit", (code) => process.exit(code || 0));
  } else {
    // Default: launch TUI
    const child = spawn(bin, ["tui"], { env, stdio: "inherit" });
    child.on("exit", (code) => process.exit(code || 0));
  }
}

main().catch((err) => {
  console.error("[ontoloop]", err.message);
  process.exit(1);
});
