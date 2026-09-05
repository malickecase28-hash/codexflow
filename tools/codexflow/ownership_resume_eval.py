#!/usr/bin/env python3
"""End-to-end acceptance test for mechanical task ownership and resume packets."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile


def run(cmd: list[str], *, cwd: Path, env: dict[str, str], check: bool = True) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(cmd, cwd=cwd, env=env, text=True, capture_output=True)
    if check and completed.returncode != 0:
        raise SystemExit(
            f"command failed ({completed.returncode}): {' '.join(cmd)}\nstdout:\n{completed.stdout}\nstderr:\n{completed.stderr}"
        )
    return completed


def run_json(cmd: list[str], *, cwd: Path, env: dict[str, str]):
    completed = run(cmd, cwd=cwd, env=env)
    try:
        return json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise SystemExit(f"invalid JSON from {' '.join(cmd)}: {error}\n{completed.stdout}") from error


def expect_fail(cmd: list[str], needle: str, *, cwd: Path, env: dict[str, str]) -> None:
    completed = run(cmd, cwd=cwd, env=env, check=False)
    if completed.returncode == 0:
        raise SystemExit(f"expected failure but command succeeded: {' '.join(cmd)}")
    combined = f"{completed.stdout}\n{completed.stderr}"
    if needle not in combined:
        raise SystemExit(
            f"failure did not contain {needle!r}: {' '.join(cmd)}\nstdout:\n{completed.stdout}\nstderr:\n{completed.stderr}"
        )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin", required=True, type=Path)
    args = parser.parse_args()
    binary = args.bin.resolve()
    if not binary.is_file():
        raise SystemExit(f"CodexFlow binary not found: {binary}")

    with tempfile.TemporaryDirectory(prefix="codexflow-ownership-eval-") as temp_dir:
        temp = Path(temp_dir)
        project_root = temp / "project"
        project_root.mkdir()
        run(["git", "init", "-q"], cwd=project_root, env=os.environ.copy())

        codex_home = temp / "codex-home"
        sqlite_home = temp / "sqlite-home"
        codex_home.mkdir()
        sqlite_home.mkdir()
        env = os.environ.copy()
        env["CODEX_HOME"] = str(codex_home)
        env["CODEX_SQLITE_HOME"] = str(sqlite_home)

        run(
            [str(binary), "project", "add", "runtime-eval", "--root", str(project_root)],
            cwd=project_root,
            env=env,
        )

        expect_fail(
            [
                str(binary),
                "runtime",
                "--project",
                "runtime-eval",
                "task-create",
                "--id",
                "direct-owner",
                "--title",
                "reject direct owner",
                "--assignee",
                "worker-a",
            ],
            "task-create --assignee is disabled",
            cwd=project_root,
            env=env,
        )

        run(
            [
                str(binary),
                "runtime",
                "--project",
                "runtime-eval",
                "task-create",
                "--id",
                "demo",
                "--title",
                "mechanical ownership demo",
                "--risk",
                "medium",
                "--acceptance",
                "ownership and resume evaluation passes",
            ],
            cwd=project_root,
            env=env,
        )

        run(
            [
                str(binary),
                "runtime",
                "--project",
                "runtime-eval",
                "agent-set",
                "--name",
                "worker-a",
                "--role",
                "flow_worker",
                "--status",
                "running",
                "--task",
                "demo",
            ],
            cwd=project_root,
            env=env,
        )

        expect_fail(
            [
                str(binary),
                "runtime",
                "--project",
                "runtime-eval",
                "agent-set",
                "--name",
                "worker-b",
                "--role",
                "flow_worker",
                "--status",
                "running",
                "--task",
                "demo",
            ],
            "held by worker-a",
            cwd=project_root,
            env=env,
        )

        run(
            [
                str(binary),
                "runtime",
                "--project",
                "runtime-eval",
                "agent-heartbeat",
                "--name",
                "worker-a",
                "--progress",
                "ownership active",
            ],
            cwd=project_root,
            env=env,
        )

        resume = run_json(
            [str(binary), "resume", "--project", "runtime-eval", "--task", "demo"],
            cwd=project_root,
            env=env,
        )
        assert resume["schema"] == "codexflow.resume-packet.v1"
        assert resume["assignee"] == "worker-a"
        assert resume["pending_acceptance"][0]["id"] == "ac-1"
        assert resume["next_action"].startswith("satisfy acceptance ac-1")

        expect_fail(
            [
                str(binary),
                "runtime",
                "--project",
                "runtime-eval",
                "task-set",
                "--id",
                "demo",
                "--assignee",
                "worker-b",
            ],
            "task-set --assignee is disabled",
            cwd=project_root,
            env=env,
        )

        expect_fail(
            [
                str(binary),
                "runtime",
                "--project",
                "runtime-eval",
                "task-set",
                "--id",
                "demo",
                "--status",
                "failed",
            ],
            "close or fail the assigned agent",
            cwd=project_root,
            env=env,
        )

        leases = run_json(
            [str(binary), "orchestrate", "--project", "runtime-eval", "lease", "list"],
            cwd=project_root,
            env=env,
        )
        task_lease = next(item for item in leases if item["scope"] == "task-demo")
        assert task_lease["owner"] == "worker-a"
        assert task_lease["expired"] is False

        run(
            [
                str(binary),
                "runtime",
                "--project",
                "runtime-eval",
                "agent-set",
                "--name",
                "worker-a",
                "--role",
                "flow_worker",
                "--status",
                "closed",
                "--task",
                "demo",
            ],
            cwd=project_root,
            env=env,
        )

        run(
            [
                str(binary),
                "runtime",
                "--project",
                "runtime-eval",
                "agent-set",
                "--name",
                "worker-b",
                "--role",
                "flow_worker",
                "--status",
                "running",
                "--task",
                "demo",
            ],
            cwd=project_root,
            env=env,
        )

        tasks = run_json(
            [str(binary), "runtime", "--project", "runtime-eval", "task-list"],
            cwd=project_root,
            env=env,
        )
        assert tasks["demo"]["assignee"] == "worker-b"

        run(
            [
                str(binary),
                "runtime",
                "--project",
                "runtime-eval",
                "task-evidence",
                "--id",
                "demo",
                "--criterion",
                "ac-1",
                "--status",
                "pass",
                "--evidence",
                "ownership_resume_eval.py passed",
            ],
            cwd=project_root,
            env=env,
        )
        run(
            [
                str(binary),
                "runtime",
                "--project",
                "runtime-eval",
                "task-set",
                "--id",
                "demo",
                "--status",
                "review",
            ],
            cwd=project_root,
            env=env,
        )
        run(
            [
                str(binary),
                "runtime",
                "--project",
                "runtime-eval",
                "task-complete",
                "--id",
                "demo",
                "--actor",
                "verifier",
            ],
            cwd=project_root,
            env=env,
        )

        leases = run_json(
            [str(binary), "orchestrate", "--project", "runtime-eval", "lease", "list"],
            cwd=project_root,
            env=env,
        )
        assert all(item["scope"] != "task-demo" for item in leases)

        agents = run_json(
            [str(binary), "runtime", "--project", "runtime-eval", "agent-list"],
            cwd=project_root,
            env=env,
        )
        assert agents["worker-b"]["status"] == "completed"
        assert agents["worker-b"]["task"] is None

        resume = run_json(
            [str(binary), "resume", "--project", "runtime-eval", "--task", "demo"],
            cwd=project_root,
            env=env,
        )
        assert resume["status"] == "done"
        assert resume["assignee"] is None
        assert resume["blockers"] == []
        assert resume["next_action"] == "task is complete; no implementation work remains"

        print(
            json.dumps(
                {
                    "schema": "codexflow.ownership-resume-eval.v1",
                    "status": "pass",
                    "project": "runtime-eval",
                    "task": "demo",
                    "checks": 12,
                },
                sort_keys=True,
            )
        )


if __name__ == "__main__":
    main()
