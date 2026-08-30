# CodexFlow Phase 2B — Native Project Management

Phase 2B makes CodexFlow project-agnostic.

TrinityR is the first managed project, not a special project type.

## Design

Codex already has a SQLite-backed project subsystem used by the app-server.
Phase 2B exposes that same state to the CLI harness instead of creating another
project registry.

A project contains:

- stable project id
- display name
- one or more ordered absolute roots
- metadata
- persisted thread assignments

The native `codexflow` binary lives in the existing `codex-cli` Cargo package.

## Commands

```powershell
codexflow project add TrinityR --root F:\TrinityR
codexflow project list
codexflow project current
codexflow project show TrinityR
codexflow project rename TrinityR TrinityR-Engine
codexflow project root-add TrinityR D:\shared-contracts
codexflow project root-remove TrinityR D:\shared-contracts
codexflow project delete TrinityR --yes
```

Launch the current project:

```powershell
cd F:\TrinityR
codexflow
```

Or launch a named project from anywhere:

```powershell
codexflow run TrinityR
```

Forward Codex arguments after `--`:

```powershell
codexflow run TrinityR -- --model gpt-5.6
```

## Session assignment

The launcher sets `CODEXFLOW_PROJECT_ID`.

The Phase 2B installer adds a small TUI bridge that forwards that value to
native `thread/start.projectId`, so new CodexFlow sessions are assigned to the
same project table used by the Codex app-server.

Ordinary `codex` sessions do not set the variable and are unchanged.

## Generic roles

Phase 2A role names were Trinity-specific. Phase 2B replaces them for new
CodexFlow installations with:

- flow_explorer
- flow_worker
- flow_verifier
- flow_reviewer
- flow_integrator

Project-specific departments are not generic roles. They will be selected by
the later orchestration layer from each project's capabilities.

## Multi-root projects

A project can own more than one root. Current-project resolution chooses the
most specific registered root containing the current directory.

Overlapping equal-specificity project roots are rejected as ambiguous.

## State ownership

Do not create a second JSON project registry.

Codex project membership remains in Codex SQLite. Repository-local
`.codexflow/` state may later hold task/gate execution state, but the project
catalog itself belongs to the native Codex state store.
