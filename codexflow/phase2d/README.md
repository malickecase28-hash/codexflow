# CodexFlow Phase 2D — Engineering OS Orchestration

Phase 2D turns project capabilities into deterministic execution plans.

The planner reads only a compact department manifest and installed skill names.
It does not stuff all skill descriptions into the root model context.

## Initialize

```powershell
codexflow orchestrate init --preset engineering
```

For a project that should define every department itself:

```powershell
codexflow orchestrate init --preset minimal
```

The manifest is:

```text
<project>\.codexflow\orchestration.json
```

## Plan

```powershell
codexflow orchestrate plan --task "fix broker reconnect semantics" --task-id reconnect_fix --apply
```

The plan returns:

- risk
- topology
- selected departments
- selected skills
- missing skills
- required roles
- blocking gates
- independent review requirement

`--apply` seeds the Phase 2C runtime ledger with the task and pending blocking
gates.

## Compatibility

If `.codexflow/orchestration.json` does not exist but
`docs/maintenance/departments.json` does, CodexFlow imports department skill
lists into the built-in engineering routing profiles.

This keeps TrinityR compatible while remaining project-agnostic.
