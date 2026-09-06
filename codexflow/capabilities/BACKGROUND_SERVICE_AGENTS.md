# Protected Background Service Agents

## Goal

CodexFlow should support useful autonomous work without opening extra terminals or keeping an interactive GOD session alive. These workers are **service agents**, not ordinary task subagents.

Typical uses:

- post-merge / post-CI self-heal
- PR health and review follow-up
- repository caretaker scans
- dependency and security scans
- build/regression investigation
- code-quality simplification sweeps
- benchmark runs
- scheduled project audits

The service plane must remain safe, bounded, quiet, and cheap when idle.

## Agent classes

CodexFlow distinguishes lifecycle ownership from functional role.

| Class | Owner | Typical lifetime | Can GOD close it? | UI |
| --- | --- | --- | --- | --- |
| interactive root / GOD | user | terminal session | n/a | foreground |
| task agent | GOD/task | minutes | yes | normal agent UI |
| reviewer/verifier | GOD/task | minutes | yes after completion | normal agent UI |
| **service agent** | **user/supervisor** | recurring/persistent definition; ephemeral runs | **no** | background services UI |

A service definition is durable. An individual run may be a fresh thread to preserve context hygiene.

## Why this cannot be a prompt rule

Current multi-agent `close_agent` handling accepts a target and directly delegates closure to `AgentControl`; it has no user-owned protection class. CodexFlow therefore needs an authoritative lifecycle check before close/terminate/resume mutation.

The invariant is:

```text
service_agent.protection = user_managed

model/tool close request
        |
        v
LifecycleAuthorityGate
        |
        +-- user/session authority token? -> allow
        |
        +-- otherwise -> deny
```

GOD can read status and consume results but cannot stop, delete, reconfigure, or steal ownership of a protected service.

## Runtime architecture

```text
                       user configuration
                              |
                              v
                  CodexFlow Service Registry
                              |
                  +-----------+-----------+
                  |                       |
             trigger index           policy/budgets
                  |                       |
                  +-----------+-----------+
                              |
                              v
                   CodexFlow Supervisor
                 (small, no LLM while idle)
                              |
              +---------------+----------------+
              |               |                |
          schedule          event bus       manual run
              |               |                |
              +---------------+----------------+
                              |
                         create run
                              |
                              v
                     isolated worktree
                              |
                              v
                    headless Codex thread
                              |
                   model + capability profile
                              |
                              v
                 verify / review / package result
                              |
                              v
                         Project Inbox
                              |
                    terminal opens / GOD starts
                              |
                              v
                      compact notification
```

The supervisor does not keep model context resident between runs. It stores state and wakes a fresh bounded agent when a trigger fires.

## Cross-platform supervisor

OpenAI's current `codex-app-server-daemon` is Unix-only, so CodexFlow should not make its service plane depend on that lifecycle implementation.

CodexFlow should add a small cross-platform supervisor process with platform adapters:

- Windows: user-level startup / Task Scheduler registration, no administrator requirement by default.
- Linux: user systemd service where available, otherwise foreground/service-manager adapter.
- macOS: user LaunchAgent adapter.

The scheduling and job semantics live in shared Rust; platform code only starts/stops the supervisor.

The supervisor should use app-server/thread APIs for headless model work rather than PTY scraping.

## Durable service definition

A service definition contains at minimum:

```json
{
  "id": "self-heal-ci",
  "enabled": true,
  "project_id": "<project>",
  "role": "flow_worker",
  "protection": "user_managed",
  "trigger": {
    "kind": "event",
    "events": ["ci.failed"]
  },
  "model": {
    "provider": "openai",
    "model": "user-selected",
    "reasoning_effort": "medium"
  },
  "capabilities": [
    "process.debug",
    "policy.minimal_change",
    "completion.evidence_gate",
    "review.engine"
  ],
  "permissions": {
    "filesystem": "workspace-write",
    "network": "project-policy",
    "delivery": "draft_pr_only"
  },
  "budgets": {
    "max_runs_per_day": 4,
    "max_attempts_per_fingerprint": 2,
    "max_tokens_per_run": 80000,
    "cooldown_minutes": 60
  },
  "notifications": {
    "on_success": true,
    "on_failure": true,
    "on_blocked": true
  }
}
```

Model choice is a service property. A user can deliberately run cheap/fast models for routine maintenance and stronger models for security, architecture, or difficult remediation.

## Trigger types

### Time

- cron-like schedule
- interval
- local daypart
- one-shot time

### Project events

- repository changed
- branch updated
- build failed
- test failed
- lint/static analysis failed
- CI failed
- PR opened / review requested / review changed
- dependency manifest changed
- release/tag created
- security advisory discovered
- caretaker finding created

### State transitions

- task blocked for too long
- repeated agent failure fingerprint
- merge completed
- deployment completed
- regression evidence discovered

### Manual

- `run now`
- one-off background job created from TUI

Triggers are deterministic. They do not require a model call just to decide whether a configured schedule or event fired.

## Self-heal flow

Self-heal is retroactive repair, not uncontrolled autonomous mutation.

```text
failure/finding
     |
     v
fingerprint + deduplicate
     |
     +-- existing active repair? -> attach evidence / stop
     |
     v
fresh service run
     |
     v
reproduce
     |
     +-- cannot reproduce -> report only
     |
     v
smallest repair
     |
     v
focused verification
     |
     v
fresh independent review
     |
     +-- blocked -> inbox report
     |
     v
draft PR
     |
     v
human review / normal merge gate
```

Default self-heal authority stops at a draft PR. A service must never silently convert `human_review_required` into merge permission.

## Loop prevention

Background repair needs stronger circuit breakers than interactive work.

Each finding receives a stable fingerprint from project id, trigger type, failing check/error identity, relevant path/symbol, and base revision.

The supervisor enforces:

- duplicate active-run suppression
- cooldown after failure
- maximum attempts per fingerprint
- maximum runs per service/day
- token/cost ceiling
- wall-clock ceiling
- maximum concurrent service runs/project
- no repair of a repair branch unless explicitly allowed
- no repeated PR creation for the same unresolved fingerprint
- automatic pause on repeated no-progress outcomes

A paused service requires user action or a configured recovery rule.

## Isolation

Read-only services may run against a clean snapshot.

Any service allowed to modify files gets its own worktree and branch by default. It never edits the user's active worktree.

Suggested branch namespace:

```text
codexflow/service/<service-id>/<date>/<fingerprint>
```

Service worktrees are recorded in the project state and are not owned by interactive agents.

## Result inbox

Service results should not inject full transcripts into GOD context.

Persist a compact result envelope:

```json
{
  "service_id": "self-heal-ci",
  "run_id": "<uuid>",
  "status": "completed",
  "project_id": "<project>",
  "summary": "Reproduced reconnect test failure and opened a draft repair PR.",
  "artifacts": {
    "branch": "...",
    "pr": 123,
    "report": "..."
  },
  "evidence": ["focused test passed", "independent review passed"],
  "findings": [],
  "metrics_ref": "<benchmark/telemetry id>"
}
```

On next interactive startup:

```text
Background work: 3 new results
› self-heal-ci      draft PR ready
  dependency-scout advisory found
  benchmark-nightly complete
```

GOD receives only selected result envelopes. Full run history remains retrievable on demand.

## User controls

CodexFlow TUI should expose a `Background Services` view using the existing list/overlay primitives.

Actions:

- enable / disable
- run now
- pause
- resume
- inspect last run
- inspect history
- change model
- change reasoning effort
- change capability set
- change permissions
- change schedule/trigger
- change budgets
- acknowledge result
- terminate current run
- delete service definition

Only user-originated UI/CLI operations can mutate a `user_managed` service definition or terminate its active run.

## Capability restrictions

A service does not inherit every installed capability.

It receives an allowlisted capability profile. For example, a PR watcher normally needs Git/GitHub, review, CI status, and notification capabilities; it does not need browser automation, deployment credentials, or quant research modules.

Capability selection is cached and static per service unless the user edits it.

## Suggested built-in presets

### PR Guardian

Trigger: PR/check/review events.

Behavior: monitor checks and review changes, classify actionable failures, optionally prepare bounded fixes, report to GOD. No merge by default.

### Self-Heal CI

Trigger: CI failure on configured branches.

Behavior: reproduce, repair, verify, fresh review, draft PR.

### Quality Sweeper

Trigger: schedule or post-merge.

Behavior: search for bounded complexity/dead-surface findings; at most one repair candidate per run.

### Dependency Scout

Trigger: schedule or manifest change.

Behavior: advisories, source/license drift, low-risk update candidates. No dependency update without project policy.

### Benchmark Runner

Trigger: schedule, release candidate, harness change, or explicit run.

Behavior: execute the benchmark corpus and write comparison evidence. It never modifies production project code.

## Relationship to caretaker

Caretaker becomes one producer of service jobs rather than one monolithic always-running agent.

```text
Caretaker scanner -> finding -> policy -> optional service run
```

This keeps deterministic scanning cheap and only spends model tokens when a finding merits investigation or repair.
