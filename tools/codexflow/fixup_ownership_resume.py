#!/usr/bin/env python3
"""Small post-materialization fixes kept separate so each generated invariant is explicit."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
RUNTIME = ROOT / "codex-rs/cli/src/bin/codexflow/runtime.rs"
LEASE = ROOT / "codex-rs/cli/src/bin/codexflow/lease.rs"


def replace_once(path: Path, old: str, new: str, label: str) -> None:
    text = path.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one match, found {count}")
    path.write_text(text.replace(old, new, 1))


def main() -> None:
    replace_once(
        LEASE,
        "use std::time::Duration;\n",
        "use std::time::Duration;\nuse std::time::Instant;\n",
        "lease Instant import",
    )
    replace_once(
        RUNTIME,
        "            let new_task = task.as_deref();\n",
        "            let new_task_owned = task.clone();\n            let new_task = new_task_owned.as_deref();\n",
        "runtime task borrow lifetime",
    )
    print("ownership/resume post-materialization fixes applied")


if __name__ == "__main__":
    main()
