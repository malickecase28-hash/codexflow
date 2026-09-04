#!/usr/bin/env node
import { createHash } from "node:crypto";
import { createReadStream, createWriteStream } from "node:fs";
import { promises as fs } from "node:fs";
import http from "node:http";
import https from "node:https";
import path from "node:path";
import { pipeline } from "node:stream/promises";
import { fileURLToPath } from "node:url";

const REPOSITORY = "malickecase28-hash/codexflow";
const DEFAULT_TAG = "codexflow-edge";
const BINARIES = [
  "codex",
  "codexflow",
  "codexflow-supervisor",
  "codex-code-mode-host",
];
const MAX_REDIRECTS = 8;

const __filename = fileURLToPath(import.meta.url);
const packageRoot = path.resolve(path.dirname(__filename), "..");

function platformSpec() {
  if (process.platform === "linux" && process.arch === "x64") {
    return { key: "linux-x64", assetSuffix: "linux-x64", extension: "" };
  }
  if (process.platform === "win32" && process.arch === "x64") {
    return { key: "win32-x64", assetSuffix: "win32-x64", extension: ".exe" };
  }
  throw new Error(
    `Unsupported CodexFlow platform: ${process.platform} (${process.arch}). ` +
      "Available GitHub edge binaries: Linux x64 and Windows x64.",
  );
}

function releaseBaseUrl() {
  if (process.env.CODEXFLOW_RELEASE_BASE_URL) {
    return process.env.CODEXFLOW_RELEASE_BASE_URL.replace(/\/$/, "");
  }
  const tag = process.env.CODEXFLOW_RELEASE_TAG || DEFAULT_TAG;
  return `https://github.com/${REPOSITORY}/releases/download/${encodeURIComponent(tag)}`;
}

async function download(url, destination, redirects = 0) {
  if (redirects > MAX_REDIRECTS) {
    throw new Error(`Too many redirects while downloading ${url}`);
  }
  const client = url.startsWith("https:") ? https : http;
  await new Promise((resolve, reject) => {
    const request = client.get(
      url,
      { headers: { "user-agent": "codexflow-github-installer" } },
      async (response) => {
        const status = response.statusCode || 0;
        if (status >= 300 && status < 400 && response.headers.location) {
          response.resume();
          try {
            const next = new URL(response.headers.location, url).toString();
            await download(next, destination, redirects + 1);
            resolve();
          } catch (error) {
            reject(error);
          }
          return;
        }
        if (status !== 200) {
          response.resume();
          reject(new Error(`Download failed (${status}) for ${url}`));
          return;
        }
        try {
          await pipeline(response, createWriteStream(destination));
          resolve();
        } catch (error) {
          reject(error);
        }
      },
    );
    request.setTimeout(60_000, () => {
      request.destroy(new Error(`Download timed out for ${url}`));
    });
    request.once("error", reject);
  });
}

async function copyOrDownload(assetName, destination) {
  const assetDir = process.env.CODEXFLOW_ASSET_DIR;
  if (assetDir) {
    await fs.copyFile(path.join(assetDir, assetName), destination);
    return;
  }
  await download(`${releaseBaseUrl()}/${encodeURIComponent(assetName)}`, destination);
}

function parseChecksums(text) {
  const checksums = new Map();
  for (const rawLine of text.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line) {
      continue;
    }
    const match = /^([0-9a-fA-F]{64})\s+\*?(.+)$/.exec(line);
    if (!match) {
      throw new Error(`Invalid SHA256SUMS line: ${rawLine}`);
    }
    checksums.set(match[2].trim(), match[1].toLowerCase());
  }
  return checksums;
}

async function sha256(filePath) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(filePath)) {
    hash.update(chunk);
  }
  return hash.digest("hex");
}

async function main() {
  const spec = platformSpec();
  const vendorRoot = path.join(packageRoot, "vendor");
  const finalDir = path.join(vendorRoot, spec.key);
  const tempDir = path.join(vendorRoot, `.install-${process.pid}-${Date.now()}`);
  const tempBin = path.join(tempDir, "bin");
  await fs.mkdir(tempBin, { recursive: true });

  try {
    const checksumPath = path.join(tempDir, "SHA256SUMS.txt");
    await copyOrDownload("SHA256SUMS.txt", checksumPath);
    const checksums = parseChecksums(await fs.readFile(checksumPath, "utf8"));

    for (const binary of BINARIES) {
      const assetName = `${binary}-${spec.assetSuffix}${spec.extension}`;
      const expected = checksums.get(assetName);
      if (!expected) {
        throw new Error(`Release checksum manifest is missing ${assetName}`);
      }
      const downloaded = path.join(tempBin, assetName);
      await copyOrDownload(assetName, downloaded);
      const actual = await sha256(downloaded);
      if (actual !== expected) {
        throw new Error(
          `SHA-256 mismatch for ${assetName}: expected ${expected}, got ${actual}`,
        );
      }
      const installed = path.join(tempBin, `${binary}${spec.extension}`);
      await fs.rename(downloaded, installed);
      if (process.platform !== "win32") {
        await fs.chmod(installed, 0o755);
      }
    }

    await fs.mkdir(vendorRoot, { recursive: true });
    await fs.rm(finalDir, { recursive: true, force: true });
    await fs.rename(tempDir, finalDir);
    process.stdout.write(
      `Installed precompiled CodexFlow ${spec.key} binaries from ` +
        `${process.env.CODEXFLOW_ASSET_DIR ? "local CI assets" : releaseBaseUrl()}.\n`,
    );
  } catch (error) {
    await fs.rm(tempDir, { recursive: true, force: true });
    throw error;
  }
}

main().catch((error) => {
  console.error(`codexflow-github install failed: ${error.message}`);
  process.exit(1);
});
