import assert from "node:assert/strict";
import { copyFile, mkdir, rm, chmod } from "node:fs/promises";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

import { resolveTarget, supportedRuntimePairs, vendorBinaryPath } from "../lib/platform.mjs";

const packageRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

assert.equal(resolveTarget({ platform: "linux", arch: "x64" }).target, "x86_64-unknown-linux-gnu");
assert.equal(resolveTarget({ platform: "linux", arch: "arm64" }).target, "aarch64-unknown-linux-gnu");
assert.equal(resolveTarget({ platform: "darwin", arch: "x64" }).target, "x86_64-apple-darwin");
assert.equal(resolveTarget({ platform: "darwin", arch: "arm64" }).target, "aarch64-apple-darwin");
assert.equal(resolveTarget({ platform: "win32", arch: "x64" }).target, "x86_64-pc-windows-msvc");
assert.equal(resolveTarget({ platform: "win32", arch: "arm64" }).target, "aarch64-pc-windows-msvc");
assert.throws(() => resolveTarget({ platform: "freebsd", arch: "x64" }), /Unsupported CodexFlow platform/);
assert.deepEqual(supportedRuntimePairs, [
  "darwin:arm64",
  "darwin:x64",
  "linux:arm64",
  "linux:x64",
  "win32:arm64",
  "win32:x64",
]);

const resolved = resolveTarget();
const vendorRoot = path.join(packageRoot, "vendor");
await rm(vendorRoot, { recursive: true, force: true });

try {
  for (const binaryName of ["codexflow", "codexflow-supervisor"]) {
    const destination = vendorBinaryPath(packageRoot, binaryName, resolved);
    await mkdir(path.dirname(destination), { recursive: true });
    await copyFile(process.execPath, destination);
    if (process.platform !== "win32") {
      await chmod(destination, 0o755);
    }

    const launcher = path.join(packageRoot, "bin", `${binaryName}.js`);
    const completed = spawnSync(process.execPath, [launcher, "--version"], {
      cwd: packageRoot,
      env: process.env,
      encoding: "utf8",
    });
    assert.equal(
      completed.status,
      0,
      `${binaryName} launcher failed: ${completed.stderr || completed.stdout}`,
    );
  }
} finally {
  await rm(vendorRoot, { recursive: true, force: true });
}

console.log("CodexFlow npm launcher smoke test passed");
