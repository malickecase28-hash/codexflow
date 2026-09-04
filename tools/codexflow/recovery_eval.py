#!/usr/bin/env python3
"""End-to-end regression checks for CodexFlow failure-aware routing."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile

FAILURES = {
    "retrieval": {
        "retry_allowed": True,
        "additional_retrieval": True,
        "human_approval": False,
    },
    "tool_selection": {
        "retry_allowed": True,
        "strategy_change": True,
        "human_approval": False,
    },
    "invalid_arguments": {
        "retry_allowed": True,
        "strategy_change": False,
        "human_approval": False,
    },
    "missing_dependency": {
        "retry_allowed": False,
        "human_approval": False,
    },
    "context_insufficiency": {
        "retry_allowed": True,
        "additional_retrieval": True,
        "human_approval": False,
    },
    "reasoning": {
        "retry_allowed": True,
        "strategy_change": True,
        "human_approval": False,
    },
    "test": {
        "retry_allowed": True,
        "rollback_recommended": True,
        "human_approval": False,
    },
    "permission": {
        "retry_allowed": False,
        "strategy_change": False,
        "human_approval": True,
    },
    "timeout": {
        "retry_allowed": True,
        "strategy_change": True,
        "human_approval": False,
    },
    "ambiguous_requirement": {
        "retry_allowed": False,
        "strategy_change": False,
        "human_approval": True,
    },
}


def run(command: list[str], env: dict[str, str], cwd: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        cwd=cwd,
        env=env,
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )


def recover(
    binary: Path,
    env: dict[str, str],
    root: Path,
    failure: str,
    *,
    attempt: int = 2,
    profile: str = "fast",
) -> dict[str, object]:
    completed = run(
        [
            str(binary),
            "route",
            "--project",
            "recovery-eval",
            "recover",
            "--failure",
            failure,
            "--attempt",
            str(attempt),
            "--profile",
            profile,
            "--detail",
            f"synthetic {failure} regression case",
        ],
        env,
        root,
    )
    return json.loads(completed.stdout)


def history(binary: Path, env: dict[str, str], root: Path) -> list[dict[str, object]]:
    completed = run(
        [
            str(binary),
            "route",
            "--project",
            "recovery-eval",
            "history",
            "--limit",
            "100",
        ],
        env,
        root,
    )
    value = json.loads(completed.stdout)
    if not isinstance(value, list):
        raise AssertionError(f"route history returned {type(value).__name__}, expected list")
    return value


def assert_equal(actual: object, expected: object, label: str) -> None:
    if actual != expected:
        raise AssertionError(f"{label}: expected {expected!r}, got {actual!r}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin", required=True, type=Path, help="Path to the codexflow binary")
    args = parser.parse_args()
    binary = args.bin.resolve()
    if not binary.is_file():
        raise SystemExit(f"codexflow binary not found: {binary}")

    with tempfile.TemporaryDirectory(prefix="codexflow-recovery-eval-") as temp_dir:
        temp = Path(temp_dir)
        root = temp / "project"
        root.mkdir()
        run(["git", "init", "-q"], os.environ.copy(), root)

        home = temp / "codex-home"
        sqlite_home = temp / "sqlite-home"
        home.mkdir()
        sqlite_home.mkdir()
        env = os.environ.copy()
        env["CODEX_HOME"] = str(home)
        env["CODEX_SQLITE_HOME"] = str(sqlite_home)

        run(
            [str(binary), "project", "add", "recovery-eval", "--root", str(root)],
            env,
            root,
        )

        results: dict[str, dict[str, object]] = {}
        for failure, expectations in FAILURES.items():
            decision = recover(binary, env, root, failure)
            results[failure] = decision
            assert_equal(decision.get("schema"), "codexflow.recovery-decision.v1", f"{failure}.schema")
            assert_equal(decision.get("failure_class"), failure, f"{failure}.failure_class")
            assert_equal(decision.get("attempt"), 2, f"{failure}.attempt")
            assert_equal(decision.get("current_profile"), "fast", f"{failure}.current_profile")
            assert_equal(decision.get("preserve_failure_evidence"), True, f"{failure}.preserve_failure_evidence")
            for key, expected in expectations.items():
                assert_equal(decision.get(key), expected, f"{failure}.{key}")

            if decision.get("retry_allowed"):
                assert_equal(decision.get("next_profile"), "balanced", f"{failure}.next_profile")
                assert_equal(decision.get("verification_depth"), "standard", f"{failure}.verification_depth")
            else:
                assert_equal(decision.get("next_profile"), "fast", f"{failure}.next_profile")
                assert_equal(decision.get("verification_depth"), "focused", f"{failure}.verification_depth")

        critical = recover(binary, env, root, "reasoning", attempt=2, profile="deep")
        assert_equal(critical.get("next_profile"), "critical", "critical.next_profile")
        assert_equal(critical.get("verification_depth"), "exhaustive", "critical.verification_depth")
        assert_equal(critical.get("human_approval"), True, "critical.human_approval")
        assert_equal(critical.get("strategy_change"), True, "critical.strategy_change")

        durable = history(binary, env, root)
        assert_equal(len(durable), len(FAILURES) + 1, "history.count")
        for index, event in enumerate(durable):
            assert_equal(event.get("schema"), "codexflow.recovery-event.v1", f"history[{index}].schema")
            if not isinstance(event.get("recorded_at"), str):
                raise AssertionError(f"history[{index}].recorded_at is not a string")
            if not isinstance(event.get("decision"), dict):
                raise AssertionError(f"history[{index}].decision is not an object")
        final_decision = durable[-1]["decision"]
        assert isinstance(final_decision, dict)
        assert_equal(final_decision.get("failure_class"), "reasoning", "history.last.failure_class")
        assert_equal(final_decision.get("next_profile"), "critical", "history.last.next_profile")

        invalid = subprocess.run(
            [
                str(binary),
                "route",
                "--project",
                "recovery-eval",
                "recover",
                "--failure",
                "unknown_failure",
            ],
            cwd=root,
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        if invalid.returncode == 0:
            raise AssertionError("unknown failure class unexpectedly succeeded")
        assert_equal(len(history(binary, env, root)), len(durable), "history.after_invalid_count")

        summary = {
            "schema": "codexflow.recovery-eval.v1",
            "cases": len(results) + 3,
            "passed": len(results) + 3,
            "failure_classes": sorted(results),
            "critical_escalation_verified": True,
            "durable_history_verified": True,
            "invalid_failure_rejected_without_record": True,
        }
        print(json.dumps(summary, indent=2, sort_keys=True))

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
