# CodexFlow Phase 2E — Delivery Plane

Phase 2E adds git worktree isolation and GitHub pull-request delivery.

## Worktrees

```powershell
codexflow delivery worktree-create --task reconnect_fix --base main
codexflow delivery worktree-list
codexflow delivery worktree-remove --task reconnect_fix --yes
```

Default task branches are:

```text
codexflow/task/<task>
```

and default worktrees are outside the repository:

```text
<repo-parent>\.codexflow-worktrees\<project>\<task>
```

## Pull requests

```powershell
codexflow delivery pr-create --title "Fix reconnect semantics" --draft
codexflow delivery pr-checks
codexflow delivery pr-checks --watch
codexflow delivery merge-check
```

Actual merge remains explicit:

```powershell
codexflow delivery merge --yes --method squash --delete-branch
```

Use `--auto` only when repository policy intentionally enables GitHub auto-merge.

The delivery plane requires the GitHub CLI for PR operations but uses ordinary
git for worktree management.
