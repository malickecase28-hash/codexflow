# Phase 2A architecture

```text
User
  |
  v
codexflow launcher
  |
  +-- existing codex binary on PATH
        |
        +-- root thread: GOD developer instructions
        |
        +-- native Codex multi-agent runtime
        |     +-- trinity_explorer
        |     +-- trinity_worker
        |     +-- trinity_verifier
        |     +-- trinity_reviewer
        |     +-- trinity_integrator
        |
        +-- normal Codex tools / skills / context hygiene

.codexflow/state/
  +-- ledger.json      atomic durable mission/task/gate mirror
  +-- events.jsonl     append-only local audit events
```

## Boundary

Codex owns thread execution, native multi-agent messaging, model/tool context, and
agent lifecycle. CodexFlow owns policy, role contracts, mission/task/gate state, and
future Engineering OS orchestration.

The boundary is deliberate. Native agent execution should not be reimplemented by a
second process manager unless a future provider cannot participate through Codex's
native agent runtime.

## Next source-level seams

Once the customized Codex fork is pushed into `malickecase28-hash/codexflow`, Phase
2B should connect the durable state layer directly to:

1. native `spawn_agent` completion/activity events;
2. agent status transitions and close/interrupt events;
3. per-thread token usage / turn completion;
4. tool-call repetition and error signals;
5. root thread startup so `codex` can enter GOD mode automatically for configured repos;
6. native mission resume and recovery.

The state schema in this package is intentionally small so those hooks can update it
without forcing a storage rewrite.
