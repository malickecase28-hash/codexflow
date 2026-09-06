import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import https from "node:https";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { installTransaction } from "../lib/install-transaction.mjs";
import { binaryAssetName, resolveTarget, vendorBinaryPath } from "../lib/platform.mjs";

const packageRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const packageJson = JSON.parse(await readFile(path.join(packageRoot, "package.json"), "utf8"));
const BINARY_NAMES = [
  "codex",
  "codexflow",
  "codexflow-supervisor",
  "codex-code-mode-host",
];
const MAX_CHECKSUM_BYTES = 4 * 1024 * 1024;
const MAX_BINARY_BYTES = 1024 * 1024 * 1024;

if (process.env.CODEXFLOW_SKIP_DOWNLOAD === "1") {
  process.exit(0);
}

const releaseTag =
  process.env.CODEXFLOW_RELEASE_TAG ||
  packageJson.codexflowReleaseTag ||
  releaseTagForVersion(packageJson.version);
const releaseBaseUrl =
  process.env.CODEXFLOW_RELEASE_BASE_URL ||
  `https://github.com/malickecase28-hash/codexflow/releases/download/${releaseTag}`;
const resolved = resolveTarget();
const checksumManifest = parseChecksums(
  (
    await download(`${releaseBaseUrl}/checksums.txt`, {
      maxBytes: MAX_CHECKSUM_BYTES,
    })
  ).toString("utf8"),
);

const prepared = [];
for (const binaryName of BINARY_NAMES) {
  const asset = binaryAssetName(binaryName, resolved);
  const expected = checksumManifest.get(asset);
  if (!expected) {
    throw new Error(`Release ${releaseTag} is missing a checksum for ${asset}`);
  }

  const bytes = await download(`${releaseBaseUrl}/${asset}`, {
    maxBytes: MAX_BINARY_BYTES,
  });
  const actual = createHash("sha256").update(bytes).digest("hex");
  if (actual !== expected) {
    throw new Error(
      `SHA-256 mismatch for ${asset}: expected ${expected}, received ${actual}`,
    );
  }

  prepared.push({
    asset,
    bytes,
    destination: vendorBinaryPath(packageRoot, binaryName, resolved),
  });
}

await installTransaction(prepared);
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
    const asset = match[2];
    if (checksums.has(asset)) {
      throw new Error(`Duplicate checksum entry for ${asset}`);
    }
    checksums.set(asset, match[1].toLowerCase());
  }
  return checksums;
}

async function download(url, { redirects = 0, maxBytes = MAX_BINARY_BYTES } = {}) {
  if (redirects > 8) {
    throw new Error(`Too many redirects while downloading ${url}`);
  }

  const parsed = new URL(url);
  if (parsed.protocol === "file:") {
    const bytes = await readFile(fileURLToPath(parsed));
    if (bytes.length > maxBytes) {
      throw new Error(`Download exceeds ${maxBytes} bytes for ${url}`);
    }
    return bytes;
  }
  if (parsed.protocol !== "https:") {
    throw new Error(`Unsupported download protocol ${parsed.protocol} for ${url}`);
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
          resolve(download(redirected, { redirects: redirects + 1, maxBytes }));
          return;
        }
        if (status !== 200) {
          response.resume();
          reject(new Error(`Download failed (${status}) for ${url}`));
          return;
        }

        const declaredLength = Number(response.headers["content-length"] ?? 0);
        if (Number.isFinite(declaredLength) && declaredLength > maxBytes) {
          response.resume();
          reject(new Error(`Download exceeds ${maxBytes} bytes for ${url}`));
          return;
        }

        const chunks = [];
        let received = 0;
        response.on("data", (chunk) => {
          received += chunk.length;
          if (received > maxBytes) {
            request.destroy(new Error(`Download exceeds ${maxBytes} bytes for ${url}`));
            return;
          }
          chunks.push(chunk);
        });
        response.on("end", () => resolve(Buffer.concat(chunks)));
        response.on("error", reject);
      },
    );
    request.setTimeout(60_000, () => {
      request.destroy(new Error(`Download timed out for ${url}`));
    });
    request.on("error", reject);
  });
}
