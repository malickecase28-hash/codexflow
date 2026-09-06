#!/usr/bin/env python3
"""End-to-end regression checks for CodexFlow recovery routing, reports, and replay."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

FAILURES = {
    "retrieval": {"retry_allowed": True, "additional_retrieval": True},
    "tool_selection": {"retry_allowed": True, "strategy_change": True},
    "invalid_arguments": {"retry_allowed": True, "strategy_change": False},
    "missing_dependency": {"retry_allowed": False},
    "context_insufficiency": {"retry_allowed": True, "additional_retrieval": True},
    "reasoning": {"retry_allowed": True, "strategy_change": True},
    "test": {"retry_allowed": True, "rollback_recommended": True},
    "permission": {"retry_allowed": False, "human_approval": True, "strategy_change": False},
    "timeout": {"retry_allowed": True, "strategy_change": True},
    "ambiguous_requirement": {"retry_allowed": False, "human_approval": True, "strategy_change": False},
}


def invoke(binary: Path, root: Path, env: dict[str, str], *args: str) -> object:
    completed = subprocess.run(
        [str(binary), *args],
        cwd=root,
        env=env,
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    return json.loads(completed.stdout)


def route(binary: Path, root: Path, env: dict[str, str], *args: str) -> object:
    return invoke(binary, root, env, "route", "--project", "recovery-eval", *args)


def recover(
    binary: Path,
    root: Path,
    env: dict[str, str],
    failure: str,
    *,
    attempt: int = 2,
    profile: str = "fast",
) -> dict[str, object]:
    value = route(
        binary,
        root,
        env,
        "recover",
        "--failure",
        failure,
        "--attempt",
        str(attempt),
        "--profile",
        profile,
        "--detail",
        f"synthetic {failure} regression case",
    )
    if not isinstance(value, dict):
        raise AssertionError("route recover did not return an object")
    return value


def assert_equal(actual: object, expected: object, label: str) -> None:
    if actual != expected:
        raise AssertionError(f"{label}: expected {expected!r}, got {actual!r}")


def independent_report(root: Path, env: dict[str, str]) -> dict[str, object]:
    script = Path(__file__).resolve().with_name("recovery_report.py")
    completed = subprocess.run(
        [sys.executable, str(script), "--project-root", str(root)],
        cwd=root,
        env=env,
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    value = json.loads(completed.stdout)
    if not isinstance(value, dict):
        raise AssertionError("independent recovery report did not return an object")
    return value


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin", required=True, type=Path)
    args = parser.parse_args()
    binary = args.bin.resolve()
    if not binary.is_file():
        raise SystemExit(f"codexflow binary not found: {binary}")

    with tempfile.TemporaryDirectory(prefix="codexflow-recovery-eval-") as temp_dir:
        temp = Path(temp_dir)
        root = temp / "project"
        root.mkdir()
        subprocess.run(["git", "init", "-q"], cwd=root, check=True)

        env = os.environ.copy()
        env["CODEX_HOME"] = str(temp / "codex-home")
        env["CODEX_SQLITE_HOME"] = str(temp / "sqlite-home")
        Path(env["CODEX_HOME"]).mkdir()
        Path(env["CODEX_SQLITE_HOME"]).mkdir()

        subprocess.run(
            [str(binary), "project", "add", "recovery-eval", "--root", str(root)],
            cwd=root,
            env=env,
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )

        for failure, expectations in FAILURES.items():
            decision = recover(binary, root, env, failure)
            assert_equal(decision.get("schema"), "codexflow.recovery-decision.v1", f"{failure}.schema")
            assert_equal(decision.get("failure_class"), failure, f"{failure}.failure_class")
            assert_equal(decision.get("attempt"), 2, f"{failure}.attempt")
            assert_equal(decision.get("current_profile"), "fast", f"{failure}.profile")
            assert_equal(decision.get("preserve_failure_evidence"), True, f"{failure}.preserve")
            for key, expected in expectations.items():
                assert_equal(decision.get(key), expected, f"{failure}.{key}")
            if decision.get("retry_allowed"):
                assert_equal(decision.get("next_profile"), "balanced", f"{failure}.next_profile")
                assert_equal(decision.get("verification_depth"), "standard", f"{failure}.verification")
            else:
                assert_equal(decision.get("next_profile"), "fast", f"{failure}.next_profile")
                assert_equal(decision.get("verification_depth"), "focused", f"{failure}.verification")

        critical = recover(binary, root, env, "reasoning", attempt=2, profile="deep")
        assert_equal(critical.get("next_profile"), "critical", "critical.next_profile")
        assert_equal(critical.get("verification_depth"), "exhaustive", "critical.verification")
        assert_equal(critical.get("human_approval"), True, "critical.human_approval")

        history = route(binary, root, env, "history", "--limit", "100")
        if not isinstance(history, list):
            raise AssertionError("route history did not return a list")
        assert_equal(len(history), 11, "history.count")
        final_decision = history[-1].get("decision")
        if not isinstance(final_decision, dict):
            raise AssertionError("final history decision is not an object")

        native_report = route(binary, root, env, "report", "--limit", "100")
        if not isinstance(native_report, dict):
            raise AssertionError("route report did not return an object")
        independent = independent_report(root, env)
        assert_equal(native_report, independent, "report.native_matches_independent")
        assert_equal(native_report.get("records"), 11, "report.records")
        assert_equal(native_report.get("retry_allowed"), 8, "report.retry_allowed")
        assert_equal(native_report.get("retry_blocked"), 3, "report.retry_blocked")
        assert_equal(native_report.get("escalations"), 8, "report.escalations")
        assert_equal(native_report.get("human_gates"), 3, "report.human_gates")
        assert_equal(
            native_report.get("profile_transitions"),
            {"deep->critical": 1, "fast->balanced": 7, "fast->fast": 3},
            "report.profile_transitions",
        )

        replay_same = route(binary, root, env, "replay", "--offset", "0")
        if not isinstance(replay_same, dict):
            raise AssertionError("route replay did not return an object")
        assert_equal(replay_same.get("schema"), "codexflow.recovery-replay.v1", "replay.schema")
        assert_equal(replay_same.get("matches_current_policy"), True, "replay.matches")
        assert_equal(replay_same.get("changed_fields"), [], "replay.changed_fields")
        assert_equal(replay_same.get("history_index"), 10, "replay.history_index")
        assert_equal(replay_same.get("recorded_decision"), final_decision, "replay.recorded")

        routing_path = root / ".codexflow" / "routing.json"
        routing_path.write_text(
            json.dumps(
                {
                    "schema": "codexflow.routing.v1",
                    "escalation_failure_threshold": 5,
                },
                indent=2,
                sort_keys=True,
            ),
            encoding="utf-8",
        )
        replay_drift = route(binary, root, env, "replay", "--offset", "0")
        if not isinstance(replay_drift, dict):
            raise AssertionError("route replay drift did not return an object")
        assert_equal(replay_drift.get("matches_current_policy"), False, "replay.drift.matches")
        assert_equal(
            replay_drift.get("changed_fields"),
            ["human_approval", "next_profile", "strategy_change", "verification_depth"],
            "replay.drift.changed_fields",
        )

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
        post_invalid = route(binary, root, env, "history", "--limit", "100")
        assert_equal(len(post_invalid), 11, "history.after_invalid")

        bad_offset = subprocess.run(
            [
                str(binary),
                "route",
                "--project",
                "recovery-eval",
                "replay",
                "--offset",
                "11",
            ],
            cwd=root,
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        if bad_offset.returncode == 0:
            raise AssertionError("out-of-range replay unexpectedly succeeded")

        print(
            json.dumps(
                {
                    "schema": "codexflow.recovery-eval.v2",
                    "passed": True,
                    "failure_classes": sorted(FAILURES),
                    "durable_history_verified": True,
                    "native_report_matches_independent_verifier": True,
                    "unchanged_policy_replay_verified": True,
                    "policy_drift_replay_verified": True,
                    "invalid_failure_rejected_without_record": True,
                    "invalid_replay_offset_rejected": True,
                },
                indent=2,
                sort_keys=True,
            )
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
