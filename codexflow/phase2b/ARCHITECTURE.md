# Phase 2B architecture

```text
                    CodexFlow Harness
                           |
                   Project Manager
                           |
              +------------+-------------+
              |                          |
        project catalog             active resolver
        Codex SQLite                cwd -> project
              |                          |
              +------------+-------------+
                           |
                         GOD
                           |
                 native Codex agents
                           |
              project-specific policies
```

## Separation of project concepts

Codex already has two related concepts:

1. Config-time active project: the git repository/worktree/cwd used for
   configuration and trust resolution.
2. Product project entity: a durable SQLite object grouping roots and threads.

CodexFlow bridges them but does not conflate them.

The current working directory determines which durable CodexFlow project is
active. Codex's existing config loader still resolves AGENTS.md and project
configuration normally inside that root.

## Later phases

Phase 2C can attach agent lifecycle, token budgets, circuit breakers, and task
state to the resolved project id.

The department orchestrator can then consume:

```text
project
 -> capabilities
 -> risk policy
 -> department router
 -> execution topology
 -> agents
 -> gates
```

This avoids any TrinityR hard-coding in the harness.
