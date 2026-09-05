# CodexFlow Phase 2C — Supervision and Build-Cost Control

Phase 2C adds durable runtime state, agent/task budgets, circuit-breaker
observation, and project-aware build-cost management. Later phases extend this
same runtime and release path; they do not create a second installer.

## Local development rule

Do not run a cold release build after every harness change.

Use:

```powershell
.\codexflow\phase2c\scripts\source-check.ps1 -TargetDir F:\codexflow-target
```

This performs format validation and `cargo check` only. Add `-RunTests` when the
phase specifically needs the binary unit tests.

Do not delete the target directory between phases.

## Build ladder

```text
cargo check
focused cargo test
development build
release build
```

CodexFlow exposes the ladder through:

```powershell
codexflow build doctor
codexflow build configure --target-dir F:\codexflow-target
codexflow build check -- -p codex-cli --bin codexflow
codexflow build test -- -p codex-cli --bin codexflow
codexflow build dev -- -p codex-cli --bin codexflow
codexflow build release --yes -- -p codex-cli --bin codexflow
```

Release builds require explicit confirmation by default.

## Runtime

Representative commands:

```powershell
codexflow runtime init
codexflow runtime task-create --id parser_fix --title "Fix parser" --risk medium
codexflow runtime agent-set --name worker_1 --role flow_worker --status running --task parser_fix
codexflow runtime agent-heartbeat --name worker_1 --progress "tests-running"
codexflow runtime agent-action --name worker_1 --action "cargo-check"
codexflow runtime agent-tokens --name worker_1 --add 12000
codexflow runtime task-wait --id parser_fix --await worker_1_done
codexflow runtime task-wake --id parser_fix
codexflow runtime supervise
codexflow runtime supervise --apply
```

`blocked_waiting` is a durable non-terminal state. A task in that state remains
incomplete while the model is suspended waiting for its named event.

The breaker can block a stuck worker in durable state. It does not silently kill
the native Codex thread or delete work.

## Prebuilt release

`.github/workflows/codexflow-prebuilt-windows.yml` performs the expensive Windows
release link in GitHub Actions. A release is a runtime bundle, not one executable.
The Windows bundle currently requires all of:

```text
codex.exe
codexflow.exe
codex-code-mode-host.exe
codexflow-supervisor.exe
```

All required binaries are built from one source revision. The workflow refuses
to publish an incomplete bundle and smoke-tests the executable entry points before
archiving it.

`install-prebuilt.ps1` downloads the release, verifies SHA-256, expands it into a
versioned candidate directory, validates the complete runtime bundle, runs setup,
and only then atomically updates `current.txt`. The former current release is
retained in `previous.txt`; the live runtime is never overwritten in place.

Default Windows layout when `F:` exists:

```text
F:\CodexFlow\
  current.txt
  previous.txt
  releases\
    <release-id>\
      bin\
        codex.exe
        codexflow.exe
        codex-code-mode-host.exe
        codexflow-supervisor.exe
```

The user launcher remains under `~\.codexflow\bin` so stock `codex` PATH
resolution is not changed.

To roll back after a bad promoted release:

```powershell
.\codexflow\phase2c\scripts\rollback-prebuilt.ps1
```

Rollback validates the previous complete runtime before swapping the current and
previous pointers.
