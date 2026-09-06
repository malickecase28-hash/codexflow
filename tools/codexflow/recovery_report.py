#!/usr/bin/env python3
"""Summarize durable CodexFlow recovery trajectories deterministically."""

from __future__ import annotations

import argparse
from collections import Counter
import json
from pathlib import Path
from typing import Any

EVENT_SCHEMA = "codexflow.recovery-event.v1"
REPORT_SCHEMA = "codexflow.recovery-report.v1"
PROFILE_ORDER = {
    "fast": 0,
    "balanced": 1,
    "deep": 2,
    "critical": 3,
}
MAX_LIMIT = 100_000


def load_events(path: Path, limit: int | None = None) -> list[dict[str, Any]]:
    if limit is not None and not 1 <= limit <= MAX_LIMIT:
        raise ValueError(f"limit must be between 1 and {MAX_LIMIT}")
    if not path.exists():
        return []

    events: list[dict[str, Any]] = []
    with path.open("r", encoding="utf-8") as handle:
        for line_number, line in enumerate(handle, 1):
            if not line.strip():
                continue
            try:
                event = json.loads(line)
            except json.JSONDecodeError as exc:
                raise ValueError(f"invalid JSON on line {line_number}: {exc}") from exc
            validate_event(event, line_number)
            events.append(event)

    if limit is not None:
        events = events[-limit:]
    return events


def validate_event(event: Any, line_number: int) -> None:
    if not isinstance(event, dict):
        raise ValueError(f"line {line_number}: recovery event must be an object")
    if event.get("schema") != EVENT_SCHEMA:
        raise ValueError(f"line {line_number}: unsupported recovery event schema")
    if not isinstance(event.get("recorded_at"), str):
        raise ValueError(f"line {line_number}: recorded_at must be a string")
    decision = event.get("decision")
    if not isinstance(decision, dict):
        raise ValueError(f"line {line_number}: decision must be an object")
    required = [
        "failure_class",
        "current_profile",
        "next_profile",
        "retry_allowed",
        "strategy_change",
        "additional_retrieval",
        "rollback_recommended",
        "human_approval",
        "verification_depth",
    ]
    missing = [key for key in required if key not in decision]
    if missing:
        raise ValueError(f"line {line_number}: missing decision fields {', '.join(missing)}")
    for profile_key in ["current_profile", "next_profile"]:
        profile = decision[profile_key]
        if profile not in PROFILE_ORDER:
            raise ValueError(f"line {line_number}: invalid {profile_key} {profile!r}")


def build_report(events: list[dict[str, Any]]) -> dict[str, Any]:
    failure_counts: Counter[str] = Counter()
    transition_counts: Counter[str] = Counter()
    verification_counts: Counter[str] = Counter()
    retry_allowed = 0
    retry_blocked = 0
    strategy_changes = 0
    retrieval_expansions = 0
    rollback_recommendations = 0
    human_gates = 0
    escalations = 0

    for event in events:
        decision = event["decision"]
        failure_counts[str(decision["failure_class"])] += 1
        current = str(decision["current_profile"])
        next_profile = str(decision["next_profile"])
        transition_counts[f"{current}->{next_profile}"] += 1
        verification_counts[str(decision["verification_depth"])] += 1

        if bool(decision["retry_allowed"]):
            retry_allowed += 1
        else:
            retry_blocked += 1
        strategy_changes += int(bool(decision["strategy_change"]))
        retrieval_expansions += int(bool(decision["additional_retrieval"]))
        rollback_recommendations += int(bool(decision["rollback_recommended"]))
        human_gates += int(bool(decision["human_approval"]))
        escalations += int(PROFILE_ORDER[next_profile] > PROFILE_ORDER[current])

    latest_recorded_at = events[-1]["recorded_at"] if events else None
    return {
        "schema": REPORT_SCHEMA,
        "records": len(events),
        "latest_recorded_at": latest_recorded_at,
        "failure_counts": dict(sorted(failure_counts.items())),
        "profile_transitions": dict(sorted(transition_counts.items())),
        "verification_depth_counts": dict(sorted(verification_counts.items())),
        "retry_allowed": retry_allowed,
        "retry_blocked": retry_blocked,
        "strategy_changes": strategy_changes,
        "retrieval_expansions": retrieval_expansions,
        "rollback_recommendations": rollback_recommendations,
        "human_gates": human_gates,
        "escalations": escalations,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--project-root",
        type=Path,
        default=Path.cwd(),
        help="Project root containing .codexflow/state/recovery-v1.jsonl",
    )
    parser.add_argument("--limit", type=int, default=None)
    args = parser.parse_args()

    path = args.project_root.resolve() / ".codexflow" / "state" / "recovery-v1.jsonl"
    events = load_events(path, args.limit)
    print(json.dumps(build_report(events), indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
