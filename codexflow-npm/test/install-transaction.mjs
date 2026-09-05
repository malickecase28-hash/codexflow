import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import {
  access,
  chmod,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath, pathToFileURL } from "node:url";

import { installTransaction } from "../lib/install-transaction.mjs";
import { binaryAssetName, resolveTarget, vendorBinaryPath } from "../lib/platform.mjs";

const packageRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const binaryNames = [
  "codex",
  "codexflow",
  "codexflow-supervisor",
  "codex-code-mode-host",
];

await verifyCommitRollback();
await verifyRealPostinstallWithoutRust();
console.log("CodexFlow transactional installer tests passed");

async function verifyCommitRollback() {
  const root = await mkdtemp(path.join(os.tmpdir(), "codexflow-transaction-"));
  try {
    const files = [];
    for (const [index, name] of binaryNames.entries()) {
      const destination = path.join(root, "bin", name);
      await mkdir(path.dirname(destination), { recursive: true });
      await writeFile(destination, `old-${name}`);
      files.push({
        asset: `${name}-fixture`,
        destination,
        bytes: Buffer.from(`new-${index}-${name}`),
      });
    }

    await assert.rejects(
      installTransaction(files, {
        afterInstall: async ({ index }) => {
          if (index === 1) {
            throw new Error("injected commit failure");
          }
        },
      }),
      /injected commit failure/,
    );

    for (const name of binaryNames) {
      assert.equal(
        await readFile(path.join(root, "bin", name), "utf8"),
        `old-${name}`,
        `rollback failed for ${name}`,
      );
    }

    await installTransaction(files);
    for (const [index, name] of binaryNames.entries()) {
      assert.equal(
        await readFile(path.join(root, "bin", name), "utf8"),
        `new-${index}-${name}`,
        `transaction did not commit ${name}`,
      );
    }
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}

async function verifyRealPostinstallWithoutRust() {
  const fixtureRoot = await mkdtemp(path.join(os.tmpdir(), "codexflow-postinstall-"));
  const releaseRoot = path.join(fixtureRoot, "release");
  const fakeBin = path.join(fixtureRoot, "fake-bin");
  const compilerMarker = path.join(fixtureRoot, "compiler-invoked");
  const vendorRoot = path.join(packageRoot, "vendor");
  const resolved = resolveTarget();
  await mkdir(releaseRoot, { recursive: true });
  await mkdir(fakeBin, { recursive: true });
  await installCompilerTraps(fakeBin, compilerMarker);
  await rm(vendorRoot, { recursive: true, force: true });

  try {
    const assets = [];
    for (const [index, binaryName] of binaryNames.entries()) {
      const asset = binaryAssetName(binaryName, resolved);
      const bytes = Buffer.from(`fixture-${index}-${asset}\n`);
      assets.push({ asset, bytes, binaryName });
      await writeFile(path.join(releaseRoot, asset), bytes);
    }
    await writeChecksums(releaseRoot, assets);
    await seedVendor(resolved, "before-success");

    const success = runPostinstall(releaseRoot, fakeBin);
    assert.equal(
      success.status,
      0,
      `postinstall fixture failed: ${success.stderr || success.stdout}`,
    );
    for (const { binaryName, bytes } of assets) {
      assert.deepEqual(
        await readFile(vendorBinaryPath(packageRoot, binaryName, resolved)),
        bytes,
        `verified asset was not installed for ${binaryName}`,
      );
    }

    await seedVendor(resolved, "before-checksum-failure");
    const corruptAssets = assets.map((entry, index) => ({
      ...entry,
      checksumOverride: index === 2 ? "0".repeat(64) : undefined,
    }));
    await writeChecksums(releaseRoot, corruptAssets);
    const failed = runPostinstall(releaseRoot, fakeBin);
    assert.notEqual(failed.status, 0, "corrupt checksum unexpectedly installed");
    assert.match(`${failed.stdout}\n${failed.stderr}`, /SHA-256 mismatch/);
    for (const { binaryName } of assets) {
      assert.equal(
        await readFile(vendorBinaryPath(packageRoot, binaryName, resolved), "utf8"),
        `before-checksum-failure-${binaryName}`,
        `checksum failure partially replaced ${binaryName}`,
      );
    }

    await assert.rejects(access(compilerMarker), /ENOENT|no such file/i);
  } finally {
    await rm(vendorRoot, { recursive: true, force: true });
    await rm(fixtureRoot, { recursive: true, force: true });
  }
}

async function writeChecksums(root, assets) {
  const lines = assets.map(({ asset, bytes, checksumOverride }) => {
    const checksum =
      checksumOverride ?? createHash("sha256").update(bytes).digest("hex");
    return `${checksum}  ${asset}`;
  });
  await writeFile(path.join(root, "checksums.txt"), `${lines.join("\n")}\n`);
}

async function seedVendor(resolved, prefix) {
  for (const binaryName of binaryNames) {
    const destination = vendorBinaryPath(packageRoot, binaryName, resolved);
    await mkdir(path.dirname(destination), { recursive: true });
    await writeFile(destination, `${prefix}-${binaryName}`);
    if (process.platform !== "win32") {
      await chmod(destination, 0o755);
    }
  }
}

function runPostinstall(releaseRoot, fakeBin) {
  const separator = process.platform === "win32" ? ";" : ":";
  const env = {
    ...process.env,
    CODEXFLOW_RELEASE_TAG: "codexflow-test-fixture",
    CODEXFLOW_RELEASE_BASE_URL: pathToFileURL(releaseRoot).toString(),
    PATH: `${fakeBin}${separator}${process.env.PATH ?? ""}`,
  };
  return spawnSync(process.execPath, [path.join(packageRoot, "scripts", "postinstall.mjs")], {
    cwd: packageRoot,
    env,
    encoding: "utf8",
  });
}

async function installCompilerTraps(fakeBin, marker) {
  if (process.platform === "win32") {
    const body = `@echo off\r\necho compiler-invoked>"${marker}"\r\nexit /b 97\r\n`;
    await writeFile(path.join(fakeBin, "cargo.cmd"), body);
    await writeFile(path.join(fakeBin, "rustc.cmd"), body);
    return;
  }
  const body = `#!/bin/sh\nprintf compiler-invoked > '${marker.replaceAll("'", "'\\''")}'\nexit 97\n`;
  for (const name of ["cargo", "rustc"]) {
    const target = path.join(fakeBin, name);
    await writeFile(target, body, { mode: 0o755 });
    await chmod(target, 0o755);
  }
}
