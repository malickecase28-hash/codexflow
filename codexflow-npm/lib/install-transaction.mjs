import { randomUUID } from "node:crypto";
import { chmod, mkdir, rename, rm, writeFile } from "node:fs/promises";
import path from "node:path";

/**
 * Install a set of already-verified native files as one logical transaction.
 *
 * All temporary files are fully staged before any destination is replaced. Existing
 * destinations are renamed to unique backups, and every committed replacement is
 * rolled back in reverse order if any later replacement fails.
 *
 * `afterInstall` is an internal fault-injection seam used only by package tests.
 */
export async function installTransaction(files, { afterInstall } = {}) {
  const token = `${process.pid}-${randomUUID()}`;
  const stages = [];

  try {
    for (const file of files) {
      const directory = path.dirname(file.destination);
      await mkdir(directory, { recursive: true });
      const temporary = `${file.destination}.tmp-${token}`;
      const backup = `${file.destination}.bak-${token}`;
      await writeFile(temporary, file.bytes, { mode: 0o755 });
      if (process.platform !== "win32") {
        await chmod(temporary, 0o755);
      }
      stages.push({
        asset: file.asset,
        destination: file.destination,
        temporary,
        backup,
        hadExisting: false,
        installed: false,
      });
    }

    for (const [index, stage] of stages.entries()) {
      try {
        await rename(stage.destination, stage.backup);
        stage.hadExisting = true;
      } catch (error) {
        if (error?.code !== "ENOENT") {
          throw error;
        }
      }
      await rename(stage.temporary, stage.destination);
      stage.installed = true;
      if (afterInstall) {
        await afterInstall({ index, stage });
      }
    }
  } catch (installError) {
    const rollbackErrors = [];
    for (const stage of [...stages].reverse()) {
      try {
        if (stage.installed) {
          await rm(stage.destination, { force: true });
        }
        if (stage.hadExisting) {
          await rename(stage.backup, stage.destination);
        }
      } catch (rollbackError) {
        rollbackErrors.push(
          new Error(`Rollback failed for ${stage.asset}: ${rollbackError.message}`, {
            cause: rollbackError,
          }),
        );
      }
      try {
        await rm(stage.temporary, { force: true });
      } catch (cleanupError) {
        rollbackErrors.push(
          new Error(`Temporary cleanup failed for ${stage.asset}: ${cleanupError.message}`, {
            cause: cleanupError,
          }),
        );
      }
    }
    if (rollbackErrors.length > 0) {
      throw new AggregateError(
        [installError, ...rollbackErrors],
        "CodexFlow installation failed and rollback was incomplete",
      );
    }
    throw installError;
  }

  for (const stage of stages) {
    await rm(stage.backup, { force: true }).catch((error) => {
      console.warn(`CodexFlow warning: could not remove backup ${stage.backup}: ${error.message}`);
    });
    await rm(stage.temporary, { force: true }).catch((error) => {
      console.warn(
        `CodexFlow warning: could not remove temporary ${stage.temporary}: ${error.message}`,
      );
    });
  }
}
