# CodexFlow Phase 2C — Supervision and Build-Cost Control

Phase 2C adds durable runtime state, agent/task budgets, circuit-breaker
observation, and project-aware build-cost management.

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
codexflow runtime supervise
codexflow runtime supervise --apply
```

The breaker can block a stuck worker in durable state. It does not yet kill the
native Codex thread or delete work.

## Prebuilt release

`.github/workflows/codexflow-prebuilt-windows.yml` performs the expensive Windows
release link in GitHub Actions and uploads `codex.exe` and `codexflow.exe`
together.

Use `install-prebuilt.ps1` to download a release, verify SHA-256, install it, and
create a user launcher without changing stock `codex` PATH resolution.
