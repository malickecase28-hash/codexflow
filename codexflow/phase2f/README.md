# CodexFlow Phase 2F — Continuous Caretaker

Phase 2F closes the first runtime/orchestration delivery loop.

## Policy

```powershell
codexflow caretaker init
codexflow caretaker show
```

Policy lives at:

```text
<project>\.codexflow\caretaker.json
```

Defaults are conservative:

- auto-fix disabled
- only low/medium findings can be queued
- high-risk conditions are surfaced, not patched
- one maintenance change per PR

## Scan

```powershell
codexflow caretaker scan
codexflow caretaker scan --json
```

Current deterministic signals include:

- dirty starting worktree
- unusually large tracked source files
- concentrated TODO/FIXME debt
- repeated non-common basenames that warrant semantic-duplication review
- missing Rust build-cost policy
- missing orchestration policy

These are candidates, not automatic proof of defects.

## Queue

```powershell
codexflow caretaker queue
codexflow caretaker queue --apply
```

`--apply` seeds eligible findings into the runtime ledger. Medium-risk findings
receive a pending independent-review gate.

## Scheduled GitHub mode

```powershell
codexflow caretaker workflow-install
```

The installed workflow is opt-in. Set repository variable:

```text
CODEXFLOW_CARETAKER_ENABLED=true
```

For AI patch generation also set:

```text
CODEXFLOW_CARETAKER_AUTOFIX=true
OPENAI_API_KEY=<repository secret>
```

The workflow selects at most one low-risk candidate and opens a draft PR. It
does not merge automatically.
