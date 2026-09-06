# Native Codex Wait Runtime Seams

## Finding

Codex already performs event-driven waiting internally. The token-wasting behavior is primarily caused by bounded waits returning to the model on timeout, after which the model may call `wait_agent` again.

### Multi-agent V1

`codex-rs/core/src/tools/handlers/multi_agents/wait.rs` obtains a Tokio `watch::Receiver<AgentStatus>` from `AgentControl::subscribe_status` and awaits `status_rx.changed()`. This is already push-driven for agent status transitions.

### Multi-agent V2

`codex-rs/core/src/tools/handlers/multi_agents_v2/wait.rs` subscribes to `InputQueueActivity` and awaits `activity_rx.changed()`. Mailbox messages/final notifications and steered user input therefore already wake the wait without status polling.

The V2 handler wraps that event wait in `timeout_at(...)`. When the timeout expires, a tool result returns to the model. Repeated model calls to `wait_agent` are the waste we need to eliminate.

## Phase 3C implementation strategy

### Step 1 — compatibility event-suspend mode

Add a backwards-compatible wait mode rather than changing vanilla Codex defaults globally.

Conceptual schema:

```json
{
  "timeout_ms": 30000,
  "until_event": false
}
```

When `until_event=true`:

- do not create a polling interval;
- do not wake the model on a periodic timeout;
- await the existing mailbox/status watch receiver;
- remain interruptible by user steer/session cancellation/runtime shutdown;
- trace sleeping duration and wake source.

Vanilla Codex keeps `until_event=false` unless explicitly requested. CodexFlow's workflow state can choose event suspension automatically when no runnable critical-path work remains.

Do not encode indefinite wait as `timeout_ms=0`; current code uses timeout bounds and zero has ambiguous/error semantics in V1. A named mode is clearer and safer.

### Step 2 — workflow-owned suspension

The final design should not require GOD to remember to call `wait_agent`.

```text
GoalLedger has unfinished criteria
        |
Runnable task graph empty
        |
Dependencies have event sources
        |
        v
WorkflowEngine -> BLOCKED_WAITING
        |
register subscriptions
        |
yield active model turn
        |
external event
        |
resume turn with compact event envelope
```

This moves waiting below the model entirely.

### Step 3 — generalize beyond agents

Use the same `AwaitSpec` / EventBus for:

- native agent mailbox/final status;
- build/test job completion;
- process exit;
- GitHub checks and PR review events;
- filesystem watchers;
- service-agent results;
- scheduled timers;
- user decisions.

Long-running tools should return a job handle and completion event rather than requiring status polling.

## Cancellation and ownership

An event-suspended interactive turn remains user-interruptible. A protected service-agent wait additionally passes the Phase 3B LifecycleAuthorityGate; ordinary GOD/task agents cannot terminate a `user_managed` service run.

## Tests required

1. V2 `until_event` sleeps through intervals longer than the old default timeout without returning a model-visible timeout result.
2. Mailbox activity wakes exactly once.
3. Final child-agent notification wakes exactly once.
4. Steered user input interrupts immediately.
5. Session cancellation/shutdown does not leak the waiter.
6. Event arriving during registration does not produce a lost wakeup.
7. Duplicate notification does not create duplicate workflow resumes.
8. Vanilla bounded wait semantics remain unchanged.
9. Benchmark trace records zero model polling calls while suspended.

## Implementation boundary

Do not add a second polling scheduler around `wait_agent`. Reuse the existing Tokio watch-based notification paths and extend them into CodexFlow's durable EventBus/WorkflowEngine.
