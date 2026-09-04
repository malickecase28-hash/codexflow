import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const packageRoot = path.resolve(path.dirname(__filename), "..");

function platformKey() {
  if (process.platform === "linux" && process.arch === "x64") {
    return "linux-x64";
  }
  if (process.platform === "win32" && process.arch === "x64") {
    return "win32-x64";
  }
  throw new Error(
    `Unsupported CodexFlow platform: ${process.platform} (${process.arch}). ` +
      "The GitHub edge package currently provides Linux x64 and Windows x64 binaries.",
  );
}

export async function runNative(binaryName) {
  const executableName =
    process.platform === "win32" ? `${binaryName}.exe` : binaryName;
  const binaryPath = path.join(
    packageRoot,
    "vendor",
    platformKey(),
    "bin",
    executableName,
  );

  if (!existsSync(binaryPath)) {
    throw new Error(
      `Missing precompiled CodexFlow binary at ${binaryPath}. ` +
        "Reinstall the package so its postinstall step can fetch the GitHub Release assets.",
    );
  }

  const child = spawn(binaryPath, process.argv.slice(2), {
    stdio: "inherit",
    env: process.env,
  });

  const result = await new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("exit", (code, signal) => resolve({ code, signal }));
  });

  if (result.signal) {
    try {
      process.kill(process.pid, result.signal);
    } catch {
      process.exit(1);
    }
    return;
  }
  process.exit(result.code ?? 1);
}
