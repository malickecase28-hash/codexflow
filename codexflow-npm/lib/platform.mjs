import path from "node:path";

const TARGETS = new Map([
  ["linux:x64", { target: "x86_64-unknown-linux-gnu", exeSuffix: "" }],
  ["linux:arm64", { target: "aarch64-unknown-linux-gnu", exeSuffix: "" }],
  ["darwin:x64", { target: "x86_64-apple-darwin", exeSuffix: "" }],
  ["darwin:arm64", { target: "aarch64-apple-darwin", exeSuffix: "" }],
  ["win32:x64", { target: "x86_64-pc-windows-msvc", exeSuffix: ".exe" }],
  ["win32:arm64", { target: "aarch64-pc-windows-msvc", exeSuffix: ".exe" }],
]);

export function resolveTarget({ platform = process.platform, arch = process.arch } = {}) {
  const key = `${platform}:${arch}`;
  const resolved = TARGETS.get(key);
  if (!resolved) {
    const supported = [...TARGETS.keys()].sort().join(", ");
    throw new Error(
      `Unsupported CodexFlow platform ${platform}/${arch}. Supported runtime pairs: ${supported}`,
    );
  }
  return { platform, arch, ...resolved };
}

export function binaryAssetName(binaryName, resolved = resolveTarget()) {
  return `${binaryName}-${resolved.target}${resolved.exeSuffix}`;
}

export function vendorBinaryPath(packageRoot, binaryName, resolved = resolveTarget()) {
  return path.join(
    packageRoot,
    "vendor",
    resolved.target,
    "bin",
    `${binaryName}${resolved.exeSuffix}`,
  );
}

export const supportedRuntimePairs = Object.freeze([...TARGETS.keys()].sort());
