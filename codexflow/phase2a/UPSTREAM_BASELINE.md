# Upstream baseline and integration seams

The user's current Codex context-hygiene build was pinned to:

```text
dde85b435b16994f956bce08e5fb796ed94c27fd
```

At that revision, native Multi-Agent V2 is already present. Relevant source seams:

```text
codex-rs/core/src/agent/control/spawn.rs
codex-rs/core/src/agent/role.rs
codex-rs/core/src/session/multi_agents.rs
codex-rs/core/src/tools/handlers/multi_agents_spec.rs
codex-rs/core/src/tools/handlers/multi_agents_v2/spawn.rs
```

The multi-agent tool surface includes named spawned tasks, role selection, context-fork
control, agent messaging/follow-up, wait/list/interrupt/close lifecycle, and persisted
agent metadata. Codex role files can supply role-scoped developer instructions.

On 2026-08-30 the public OpenAI Codex repository had advanced beyond that baseline.
Do not blindly apply source patches from a newer revision to the user's fork. Phase 2B
should diff the pushed `codexflow` source first and integrate against its exact tree.

## Phase 2B targets

1. Add a CodexFlow root-mode startup seam that can activate GOD behavior for an opted-in repo without requiring the wrapper profile.
2. Mirror native agent spawn/status/close events into the CodexFlow durable ledger automatically.
3. Record per-agent/turn token usage and implement a steer -> constrain -> stop breaker.
4. Add failure recovery for orphaned/errored native agents.
5. Decide, from measured behavior, whether native agent workspace isolation is sufficient; add Git worktree allocation only where it materially improves correctness.
6. Expose a compact status view in the TUI.
7. Only after the runtime is stable, add Engineering OS department routing and PR/CI/merge policy.
