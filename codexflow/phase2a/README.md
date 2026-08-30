# CodexFlow Native Hive — Phase 2A

This package is the first runtime layer below the Trinity Engineering OS orchestrator.
It uses **Codex's native multi-agent engine** for agent execution and adds a small,
durable control-plane contract around it.

It deliberately does **not** replace `codex.exe`, create a second PTY farm, modify
`~/.codex/config.toml`, push branches, open PRs, merge code, or deploy anything.
Those are later layers.

## What this build provides

- a named Codex profile, `codexflow`, whose root session acts as GOD / supervisor;
- native Codex multi-agent execution enabled explicitly;
- five scoped agent roles: explorer, worker, verifier, reviewer, integrator;
- an adaptive topology policy so trivial work does not wake a fleet;
- independent-review rules for non-trivial code changes;
- a local durable task/gate/agent/handoff ledger under `.codexflow/state/`;
- atomic state updates plus an append-only local event log;
- install, doctor and uninstall scripts for Windows;
- no modification of the stock/custom Codex executable.

The runtime state directory is gitignored by design. It is execution state, not
source code. Project-level `.codexflow/config.json` remains visible so later phases
can add repo-specific orchestration policy without serializing every agent event into Git.

## Why native Codex agents

The pinned Codex baseline used by the context-hygiene pack already contains native
multi-agent V2 support: named agent tasks, isolated thread contexts, role-specific
configuration, messaging/follow-up, waiting/listing/interrupt lifecycle, and persisted
agent metadata. Duplicating that with another process supervisor would add auth,
PTY, hook, state and failure modes without improving the core workflow.

CodexFlow therefore treats Codex as the **execution plane** and adds the Engineering OS
as the **policy/control plane**.

## Install on Windows

From the extracted package:

```powershell
Set-ExecutionPolicy -Scope Process Bypass
.\scripts\install.ps1
```

The installer creates:

```text
%USERPROFILE%\.codexflow\current\
%USERPROFILE%\.codexflow\bin\codexflow.cmd
%CODEX_HOME%\codexflow.config.toml
%CODEX_HOME%\agents\trinity_*.toml
```

If `CODEX_HOME` is not set, `%USERPROFILE%\.codex` is used.

The installer adds `%USERPROFILE%\.codexflow\bin` to your **user** PATH. Open a new
terminal after installation.

It does not edit `%USERPROFILE%\.codex\config.toml`.

## Initialize a repository

In TrinityR, or any trusted Git repository:

```powershell
cd C:\path\to\TrinityR
codexflow init
codexflow doctor
```

You should see PASS for Codex, Python, Git, the CodexFlow profile, five roles,
the project runtime, and ledger.

## Launch GOD

```powershell
codexflow
```

or explicitly:

```powershell
codexflow launch
```

The wrapper executes your **existing `codex` on PATH** with the `codexflow` profile, explicitly enables `multi_agent`, and applies the GOD developer instructions again at runtime precedence. The runtime override is deliberate: a repository-level `developer_instructions` value cannot silently turn GOD mode off. Other project configuration remains authoritative.

So if your context-hygiene Codex binary is first on PATH, that is the binary the
GOD and its native subagents use.

You can forward ordinary Codex arguments after `launch`:

```powershell
codexflow launch -- --model gpt-5.6
```

## First local test

Start GOD and ask:

```text
Inspect this repository and make one harmless documentation-only change that fixes
an actual issue you find. Use the CodexFlow workflow. Do not push or commit.
```

Expected behavior:

1. GOD classifies the task.
2. For a small change, it should avoid an unnecessary fleet.
3. It records a task in `.codexflow/state/ledger.json` when the work is non-trivial.
4. If it delegates, it uses a `trinity_*` role.
5. It verifies before declaring completion.
6. It does not push, merge, or deploy.

Then inspect:

```powershell
codexflow snapshot
Get-Content .codexflow\state\events.jsonl
```

## Heavier test

Ask:

```text
Audit the reconnect subsystem for duplicated logic and correctness risks. If a
non-trivial fix is justified, have a bounded worker implement it and a fresh
independent reviewer inspect the finished diff. Do not push or merge.
```

Expected topology:

```text
GOD
 ├─ trinity_explorer, if investigation can run independently
 ├─ trinity_worker
 ├─ trinity_verifier, when test/reproduction evidence is useful
 └─ trinity_reviewer with fresh context
```

The GOD prompt specifically prefers `fork_turns="none"` for independent review so the
reviewer receives explicit evidence rather than inheriting the implementer's reasoning.

## State commands

Examples:

```powershell
codexflow task create --id reconnect_fix --title "Fix reconnect semantics" --risk high
codexflow task set --id reconnect_fix --status doing --assignee worker_reconnect
codexflow gate set --task reconnect_fix --name independent_review --status block --risk high --reviewer reviewer_1 --finding "review pending"
codexflow handoff add --task reconnect_fix --from worker_reconnect --to reviewer_1 --summary "Implementation ready" --ref src\reconnect.rs
codexflow snapshot
```

The model is instructed to use these commands instead of hand-editing the ledger.

## What Phase 2A does not yet enforce

The following require integration against your pushed customized Codex source and are
intentionally deferred rather than faked at prompt level:

- automatic mirroring of native agent lifecycle events into the ledger;
- hard per-agent token/cost budgets;
- repeated-tool/error-storm circuit breaker enforcement;
- external Git worktree allocation where native workspace isolation is insufficient;
- restart/resume recovery policy for a whole GOD mission;
- PR/CI/merge/deploy automation;
- Engineering OS department selection and blocking gates.

Those belong in Phase 2B/3 after `codexflow` contains your actual Codex fork. The
current package establishes the contracts those source-level hooks will update.

## Rollback

```powershell
.\scripts\uninstall.ps1
```

This removes the profile, prefixed roles and launcher. It leaves each repository's
`.codexflow/` state untouched for audit/recovery.

## Validation

From the package root:

```powershell
python -m unittest discover -s tests -v
python -m py_compile codexflow.py
```

## Upstream design references

This build is based on current/pinned OpenAI Codex multi-agent and role mechanisms,
plus selected control-plane ideas from Munder Difflin (mail/task separation, GOD
supervision, bounded handoffs, and explicit safety controls). No Munder source code is
vendored in this package.

- OpenAI Codex: https://github.com/openai/codex
- Munder Difflin: https://github.com/chaitanyagiri/munder-difflin
