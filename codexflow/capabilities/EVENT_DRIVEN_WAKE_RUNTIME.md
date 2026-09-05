# Event-Driven Wake Runtime

## Purpose

CodexFlow must not spend model tokens polling for work that has not completed yet. Waiting is a runtime state, not a reasoning task.

The target behavior is:

```text
agent reaches WAITING
        |
        v
register subscriptions + timeout
        |
        v
persist continuation state
        |
        v
suspend model execution
        |
        |   no model calls
        |   no polling tool calls
        |   no repeated status prompts
        v
external event arrives
        |
        v
EventBus writes durable event
        |
        v
WakeRouter resolves subscribers
        |
        v
resume only affected agent/service
```

This is inspired by Google Antigravity's trigger model, especially the separation between inline hooks and long-lived asynchronous triggers that can push messages back into an agent connection. CodexFlow should take the push/wakeup seam, but avoid making periodic polling the normal implementation. File events, process exits, child-agent completion, CI webhooks, GitHub events and OS notifications should wake waiters directly.

## Core invariant

**No model call is permitted solely to ask whether an unchanged external condition is now complete.**

If CodexFlow can represent the awaited condition as an event source, the agent must yield instead of polling.

Polling remains a last-resort adapter only for upstream systems that expose no event/watch/webhook mechanism. Even then the polling occurs in deterministic supervisor code, not in a model turn, and uses backoff, deduplication and a configured maximum rate.

## Agent wait primitive

CodexFlow adds an internal `AwaitSpec` concept:

```text
wait_id
owner_thread_id
project_id
condition
subscriptions[]
timeout_at
resume_policy
dedupe_key
continuation_ref
created_at
```

An agent does not receive a `wait` tool that encourages repeated model decisions. Instead, a workflow transition can resolve to `WAITING` and return control to the runtime.

Examples:

```text
WAIT child.agent.completed(<thread>)
WAIT process.exited(<pid/job>)
WAIT ci.check.completed(<repo, sha, check>)
WAIT pr.review.changed(<repo, pr>)
WAIT file.changed(<path>)
WAIT build.completed(<job>)
WAIT service.result(<run-id>)
WAIT timer(<deadline>)
```

## Event envelope

Every wakeable event uses a small canonical envelope:

```json
{
  "event_id": "uuid",
  "type": "ci.check.completed",
  "project_id": "project-uuid",
  "source": "github",
  "subject": "repo@sha/check-name",
  "occurred_at": "timestamp",
  "dedupe_key": "stable-key",
  "payload_ref": "event-store-reference"
}
```

Large payloads are not injected into the waiting thread. The wake message contains only the event summary plus references required for targeted retrieval.

## Event sources

### Native push/watch sources

Prefer these whenever available:

- child-agent lifecycle notifications
- process exit notifications
- filesystem watcher APIs
- GitHub webhooks / check-run events
- app-server thread/turn completion events
- build-runner completion events
- supervisor service completion events
- socket/queue notifications
- explicit user actions
- OS timers

### Polling adapters

Some third-party systems expose only status APIs. In those cases CodexFlow may run a deterministic watcher with:

- exponential or policy-defined backoff
- ETag/revision-aware requests where available
- state-change deduplication
- a maximum polling budget
- no model context
- automatic conversion into an EventBus event only when state changes

The model never pays tokens for these checks.

## Child-agent completion

This fixes a common multi-agent failure mode:

```text
BAD
GOD -> wait_agent
GOD -> model wakes
GOD -> wait_agent
GOD -> model wakes
GOD -> wait_agent
```

Instead:

```text
GOD delegates
     |
     v
workflow has no runnable work
     |
     v
WAIT agent.completed(worker-id)
     |
     v
GOD thread suspended
     |
worker finishes
     |
AgentControl emits completion event
     |
WakeRouter resumes GOD exactly once
```

The result envelope is delivered with the wake event, so GOD does not first spend another turn asking what happened.

## Goal integration

Goals remain important because they prevent premature completion, but a goal with blocked prerequisites must distinguish `BLOCKED_WAITING` from `ACTIVE_WORK`.

```text
ACTIVE_WORK
  agent has runnable next action

BLOCKED_WAITING
  completion criteria remain open
  no runnable action exists until event E
  thread may sleep without violating goal persistence

DONE
  evidence gates satisfied
```

The goal engine therefore prevents both premature stopping and wasteful polling.

## Service-agent integration

Protected background service definitions do not need a resident model session while waiting.

```text
service definition
      |
      v
Supervisor trigger subscription
      |
      | zero model tokens while idle
      v
event fires
      |
      v
fresh bounded service run
      |
      v
result -> project inbox
```

If the terminal is closed, interactive GOD is not kept alive merely to receive an event. The supervisor persists the event/result. On next CodexFlow startup, GOD receives a compact inbox projection.

## Delivery while terminal is open

When an interactive thread is suspended and its terminal is still open, the runtime may resume it automatically after an event arrives. The user should see a small status transition such as:

```text
waiting: CI check "windows"...

[CI completed: failed]
resuming debugger
```

There should be no artificial assistant message saying it is still waiting.

## Delivery while terminal is closed

If the owning interactive session no longer exists:

1. persist the event;
2. mark the wait as `wake_pending`;
3. do not create a model session automatically unless project/user policy explicitly allows that workflow to become a service run;
4. surface the pending wake in the project inbox on next startup.

This prevents accidental background spending by ordinary interactive tasks.

## Protected ownership

Service-agent waits inherit the service's `user_managed` lifecycle protection. Task-agent waits inherit normal GOD/task ownership.

A GOD or task agent cannot terminate a protected service merely because it is waiting. Lifecycle authority is checked below the model.

## Debounce and event storms

The event bus must prevent a file watcher, CI provider or webhook from creating a model-call storm.

Per subscription support:

- debounce window
- coalescing strategy
- dedupe key
- edge-triggered vs level-triggered semantics
- maximum wakes per interval
- cooldown

Example:

```text
147 filesystem events from cargo build
        |
        v
coalesce by watched logical resource
        |
        v
one `source.changed` wake
```

## Lost wakeup protection

Registration and suspension must be atomic from the workflow's perspective:

```text
1 persist AwaitSpec
2 establish event cursor/subscription
3 check whether matching event already occurred since cursor
4 mark thread suspended
```

On resume/restart, unresolved waits are reconstructed from durable state. Event IDs and cursors make delivery idempotent.

## Timeouts

A timeout is itself an event produced by the supervisor timer wheel. It does not require periodic model turns.

Timeout policy may:

- wake the same agent for recovery;
- escalate to a service/reviewer;
- mark a task blocked;
- notify the user;
- cancel an external job if authority allows.

## Event-aware tool contracts

Long-running tools should return a job handle rather than block the model loop or encourage polling:

```json
{
  "status": "started",
  "job_id": "job-123",
  "completion_event": "build.completed:job-123"
}
```

The workflow engine then subscribes and yields.

This contract applies to builds, long tests, CI, browser recordings, large scans, background downloads and service jobs.

## Metrics

The benchmark layer records:

- `model_poll_calls`: target 0 for event-capable jobs
- `runtime_poll_calls`: expected 0 for push-capable providers
- sleeping wall time
- wake count
- duplicate events suppressed
- event-to-resume latency
- tokens avoided by sleeping
- false/spurious wakes
- lost/replayed wake events

A Phase 3 benchmark should compare vanilla Codex polling behavior against CodexFlow event waiting for identical long-running jobs.

## Implementation seams in Codex

The first Rust implementation should connect to existing native seams rather than build a second agent transport:

1. native multi-agent status/completion events -> EventBus;
2. app-server/thread lifecycle -> EventBus;
3. Phase 2C runtime ledger -> durable waits/events;
4. Phase 3B Supervisor -> timers/webhooks/file watchers/service events;
5. Goal/task state machine -> `BLOCKED_WAITING` transition;
6. TUI -> waiting/sleeping status and inbox projection.

## Non-goals

- Do not keep a model process alive solely because an event may occur.
- Do not wake all agents for every project event.
- Do not turn every file change into a model turn.
- Do not replace a provider's real webhook/watch API with polling for convenience.
- Do not let an LLM decide whether a deterministic event condition fired.

## Acceptance criteria

The first implementation is complete when:

- a GOD waiting for a child completion makes zero intervening model calls;
- a long build/test tool can return a job handle and later wake the owning workflow;
- a service agent can remain idle indefinitely with zero model tokens;
- terminal closure does not lose service results or pending interactive wake events;
- duplicate events produce at most one logical wake;
- protected service waits cannot be terminated by ordinary agent authority;
- tracing and benchmark evidence can show tokens/time saved versus polling.
