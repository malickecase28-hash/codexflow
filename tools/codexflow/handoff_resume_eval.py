#!/usr/bin/env python3
"""Black-box acceptance for structured durable handoffs and resume precedence."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile


def run(cmd: list[str], *, cwd: Path, env: dict[str, str]) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(cmd, cwd=cwd, env=env, text=True, capture_output=True)
    if completed.returncode != 0:
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


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin", required=True, type=Path)
    args = parser.parse_args()
    binary = args.bin.resolve()

    with tempfile.TemporaryDirectory(prefix="codexflow-handoff-eval-") as temp_dir:
        temp = Path(temp_dir)
        root = temp / "project"
        root.mkdir()
        run(["git", "init", "-q"], cwd=root, env=os.environ.copy())
        env = os.environ.copy()
        env["CODEX_HOME"] = str(temp / "codex-home")
        env["CODEX_SQLITE_HOME"] = str(temp / "sqlite-home")
        Path(env["CODEX_HOME"]).mkdir()
        Path(env["CODEX_SQLITE_HOME"]).mkdir()

        run([str(binary), "project", "add", "handoff-eval", "--root", str(root)], cwd=root, env=env)
        run(
            [
                str(binary), "runtime", "--project", "handoff-eval", "task-create",
                "--id", "handoff-task", "--title", "durable restart packet", "--risk", "medium",
            ],
            cwd=root,
            env=env,
        )
        run(
            [
                str(binary), "runtime", "--project", "handoff-eval", "handoff-add",
                "--task", "handoff-task",
                "--from", "worker-a",
                "--to", "worker-b",
                "--summary", "ownership path compiled and package transaction validated",
                "--ref", "CI run 33934379932",
                "--accomplished", "six-host package transaction passed",
                "--accomplished", "ownership binaries compiled",
                "--remaining", "publish six-target edge release",
                "--failure", "first cold release run was cancelled before assembly",
                "--file", "codex-rs/cli/src/bin/codexflow/runtime.rs",
                "--file", ".github/workflows/codexflow-release.yml",
                "--decision", "task authority changes only through lease-aware agent-set",
                "--rationale", "fresh contexts must not infer authority from prose",
                "--restart-command", "cargo test -p codex-cli --bin codexflow",
                "--restart-command", "python3 tools/codexflow/ownership_resume_eval.py --bin target/debug/codexflow",
                "--next-action", "publish the cached six-target edge release",
            ],
            cwd=root,
            env=env,
        )

        packet = run_json(
            [str(binary), "resume", "--project", "handoff-eval", "--task", "handoff-task"],
            cwd=root,
            env=env,
        )
        assert packet["schema"] == "codexflow.resume-packet.v1"
        assert packet["relevant_refs"] == ["CI run 33934379932"]
        handoff = packet["handoff"]
        assert handoff["accomplished"] == [
            "six-host package transaction passed",
            "ownership binaries compiled",
        ]
        assert handoff["remaining_work"] == ["publish six-target edge release"]
        assert handoff["failures"] == ["first cold release run was cancelled before assembly"]
        assert handoff["relevant_files"] == [
            "codex-rs/cli/src/bin/codexflow/runtime.rs",
            ".github/workflows/codexflow-release.yml",
        ]
        assert handoff["decisions"] == [
            "task authority changes only through lease-aware agent-set"
        ]
        assert handoff["rationale"] == "fresh contexts must not infer authority from prose"
        assert len(handoff["restart_commands"]) == 2
        assert packet["next_action"] == "publish the cached six-target edge release"

        run(
            [
                str(binary), "runtime", "--project", "handoff-eval", "task-acceptance-add",
                "--id", "handoff-task", "--criterion", "release", "--text", "edge release is published",
            ],
            cwd=root,
            env=env,
        )
        blocked = run_json(
            [str(binary), "resume", "--project", "handoff-eval", "--task", "handoff-task"],
            cwd=root,
            env=env,
        )
        assert blocked["next_action"] == "satisfy acceptance release: edge release is published"
        assert any("acceptance release is pending" in item for item in blocked["blockers"])

        print(json.dumps({
            "schema": "codexflow.handoff-resume-eval.v1",
            "status": "pass",
            "checks": 13,
        }, sort_keys=True))


if __name__ == "__main__":
    main()
