import { createHash } from "node:crypto";
import { mkdir, readFile, rename, rm, writeFile, chmod } from "node:fs/promises";
import https from "node:https";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { binaryAssetName, resolveTarget, vendorBinaryPath } from "../lib/platform.mjs";

const packageRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const packageJson = JSON.parse(await readFile(path.join(packageRoot, "package.json"), "utf8"));

if (process.env.CODEXFLOW_SKIP_DOWNLOAD === "1") {
  process.exit(0);
}

const releaseTag = process.env.CODEXFLOW_RELEASE_TAG || releaseTagForVersion(packageJson.version);
const releaseBaseUrl =
  process.env.CODEXFLOW_RELEASE_BASE_URL ||
  `https://github.com/malickecase28-hash/codexflow/releases/download/${releaseTag}`;
const resolved = resolveTarget();
const checksumManifest = parseChecksums(
  (await download(`${releaseBaseUrl}/checksums.txt`)).toString("utf8"),
);

for (const binaryName of ["codexflow", "codexflow-supervisor"]) {
  const asset = binaryAssetName(binaryName, resolved);
  const expected = checksumManifest.get(asset);
  if (!expected) {
    throw new Error(`Release ${releaseTag} is missing a checksum for ${asset}`);
  }

  const bytes = await download(`${releaseBaseUrl}/${asset}`);
  const actual = createHash("sha256").update(bytes).digest("hex");
  if (actual !== expected) {
    throw new Error(
      `SHA-256 mismatch for ${asset}: expected ${expected}, received ${actual}`,
    );
  }

  const destination = vendorBinaryPath(packageRoot, binaryName, resolved);
  await installAtomically(destination, bytes);
}

console.log(`CodexFlow ${packageJson.version} installed for ${resolved.target}`);

function releaseTagForVersion(version) {
  if (!version || version === "0.0.0-dev") {
    throw new Error(
      "Development installs require CODEXFLOW_RELEASE_TAG or a versioned GitHub Release tarball.",
    );
  }
  return `codexflow-v${version}`;
}

function parseChecksums(text) {
  const checksums = new Map();
  for (const rawLine of text.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line) {
      continue;
    }
    const match = line.match(/^([a-fA-F0-9]{64})\s+\*?(.+)$/);
    if (!match) {
      throw new Error(`Invalid checksums.txt line: ${rawLine}`);
    }
    checksums.set(match[2], match[1].toLowerCase());
  }
  return checksums;
}

async function installAtomically(destination, bytes) {
  const directory = path.dirname(destination);
  await mkdir(directory, { recursive: true });
  const temporary = `${destination}.tmp-${process.pid}-${Date.now()}`;
  await writeFile(temporary, bytes, { mode: 0o755 });
  if (process.platform !== "win32") {
    await chmod(temporary, 0o755);
  }
  await rm(destination, { force: true });
  await rename(temporary, destination);
}

function download(url, redirects = 0) {
  if (redirects > 8) {
    return Promise.reject(new Error(`Too many redirects while downloading ${url}`));
  }

  return new Promise((resolve, reject) => {
    const request = https.get(
      url,
      {
        headers: {
          Accept: "application/octet-stream",
          "User-Agent": "codexflow-npm-installer",
        },
      },
      (response) => {
        const status = response.statusCode ?? 0;
        const location = response.headers.location;
        if (status >= 300 && status < 400 && location) {
          response.resume();
          const redirected = new URL(location, url).toString();
          resolve(download(redirected, redirects + 1));
          return;
        }
        if (status !== 200) {
          response.resume();
          reject(new Error(`Download failed (${status}) for ${url}`));
          return;
        }

        const chunks = [];
        response.on("data", (chunk) => chunks.push(chunk));
        response.on("end", () => resolve(Buffer.concat(chunks)));
        response.on("error", reject);
      },
    );
    request.on("error", reject);
  });
}
