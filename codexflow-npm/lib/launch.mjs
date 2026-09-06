import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { resolveTarget, vendorBinaryPath } from "./platform.mjs";

const packageRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

export async function launch(binaryName) {
  const resolved = resolveTarget();
  const binaryPath = vendorBinaryPath(packageRoot, binaryName, resolved);
  if (!existsSync(binaryPath)) {
    throw new Error(
      [
        `Missing prebuilt CodexFlow binary: ${binaryPath}`,
        `Target: ${resolved.target}`,
        "Reinstall the CodexFlow npm tarball from the matching GitHub Release.",
        "No local Rust compilation is required or attempted by this package.",
      ].join("\n"),
    );
  }

  const env = {
    ...process.env,
    CODEXFLOW_MANAGED_BY_NPM: "1",
    CODEXFLOW_MANAGED_PACKAGE_ROOT: packageRoot,
  };
  const child = spawn(binaryPath, process.argv.slice(2), {
    stdio: "inherit",
    env,
  });

  child.on("error", (error) => {
    console.error(error);
    process.exit(1);
  });

  const forwardSignal = (signal) => {
    if (child.killed) {
      return;
    }
    try {
      child.kill(signal);
    } catch {
      // The child may have exited between the signal and this handler.
    }
  };

  for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
    process.on(signal, () => forwardSignal(signal));
  }

  const result = await new Promise((resolve) => {
    child.on("exit", (code, signal) => {
      if (signal) {
        resolve({ type: "signal", signal });
      } else {
        resolve({ type: "code", code: code ?? 1 });
      }
    });
  });

  if (result.type === "signal") {
    process.kill(process.pid, result.signal);
    return;
  }
  process.exit(result.code);
}
