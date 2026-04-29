import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const root = path.resolve(__dirname, "..");
const srcDir = path.join(root, "src");
const distDir = path.join(root, "dist");

async function rmrf(target) {
  await fs.rm(target, { recursive: true, force: true });
}

async function mkdirp(target) {
  await fs.mkdir(target, { recursive: true });
}

async function copyTree(from, to) {
  const entries = await fs.readdir(from, { withFileTypes: true });
  await mkdirp(to);
  for (const entry of entries) {
    const srcPath = path.join(from, entry.name);
    const dstPath = path.join(to, entry.name);
    if (entry.isDirectory()) {
      await copyTree(srcPath, dstPath);
      continue;
    }
    await fs.copyFile(srcPath, dstPath);
  }
}

async function writeBinShim() {
  const binDir = path.join(distDir, "bin");
  await mkdirp(binDir);
  const content = `#!/usr/bin/env node
import "../index.js";
`;
  const target = path.join(binDir, "ontoloop-cli");
  await fs.writeFile(target, content, "utf8");
  await fs.chmod(target, 0o755);
}

async function main() {
  await rmrf(distDir);
  await copyTree(srcDir, distDir);
  await writeBinShim();
  process.stdout.write("ontoloop-cli build complete\\n");
}

main().catch((error) => {
  process.stderr.write(`build failed: ${error?.stack || error}\\n`);
  process.exitCode = 1;
});
