#!/usr/bin/env python3
"""Post-materialization fixes for ownership lifetime and terminal-state invariants."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
RUNTIME = ROOT / "codex-rs/cli/src/bin/codexflow/runtime.rs"
LEASE = ROOT / "codex-rs/cli/src/bin/codexflow/lease.rs"


def replace_once(path: Path, old: str, new: str, label: str) -> None:
    text = path.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one match, found {count}")
    path.write_text(text.replace(old, new, 1))


def replace_between(path: Path, start: str, end: str, replacement: str, label: str) -> None:
    text = path.read_text()
    start_index = text.find(start)
    if start_index < 0:
        raise SystemExit(f"{label}: start marker not found")
    end_index = text.find(end, start_index + len(start))
    if end_index < 0:
        raise SystemExit(f"{label}: end marker not found")
    path.write_text(text[:start_index] + replacement + text[end_index:])


def main() -> None:
    replace_once(
        LEASE,
        "use std::time::Duration;\n",
        "use std::time::Duration;\nuse std::time::Instant;\n",
        "lease Instant import",
    )
    replace_once(
        RUNTIME,
        "            let new_task = task.as_deref();\n",
        "            let new_task_owned = task.clone();\n            let new_task = new_task_owned.as_deref();\n",
        "runtime task borrow lifetime",
    )

    task_set = '''        RuntimeCommand::TaskSet {
            id,
            status,
            risk,
            assignee,
            budget_tokens,
        } => with_locked_ledger(project_root, |ledger| {
            if assignee.is_some() {
                bail!("task-set --assignee is disabled; assign ownership with runtime agent-set");
            }
            let task = ledger
                .tasks
                .get_mut(&id)
                .with_context(|| format!("unknown task: {id}"))?;
            if let Some(status) = status {
                validate_task_status(&status)?;
                if status == "blocked_waiting" {
                    bail!("use task-wait so blocked_waiting records the await id");
                }
                if status == "done" {
                    bail!("use task-complete so completion criteria and gates are checked");
                }
                if matches!(status.as_str(), "failed" | "cancelled") && task.assignee.is_some() {
                    bail!(
                        "close or fail the assigned agent with runtime agent-set before marking task {id} {status}"
                    );
                }
                task.status = status;
                task.waiting_on = None;
            }
            if let Some(risk) = risk {
                validate_risk(&risk)?;
                task.risk = risk;
            }
            if budget_tokens.is_some() {
                task.budget_tokens = budget_tokens;
            }
            task.updated_at = now_iso();
            println!("{}", serde_json::to_string_pretty(task)?);
            Ok(())
        }),
'''
    replace_between(
        RUNTIME,
        "        RuntimeCommand::TaskSet {\n",
        "        RuntimeCommand::TaskAcceptanceAdd {\n",
        task_set,
        "runtime final task-set",
    )

    task_complete = '''        RuntimeCommand::TaskComplete { id, actor } => {
            validate_id(&actor)?;
            let owner = with_locked_ledger(project_root, |ledger| {
                let blockers = completion_blockers(ledger, &id)?;
                let check = CompletionCheck {
                    task: id.clone(),
                    ready: blockers.is_empty(),
                    blockers,
                };
                if !check.ready {
                    println!("{}", serde_json::to_string_pretty(&check)?);
                    bail!("task {id} is not ready for completion");
                }
                let owner = {
                    let task = ledger
                        .tasks
                        .get_mut(&id)
                        .with_context(|| format!("unknown task: {id}"))?;
                    let owner = task.assignee.take();
                    task.status = "done".to_string();
                    task.waiting_on = None;
                    task.updated_at = now_iso();
                    println!("{}", serde_json::to_string_pretty(task)?);
                    owner
                };
                if let Some(owner_name) = owner.as_deref()
                    && let Some(agent) = ledger.agents.get_mut(owner_name)
                    && agent.task.as_deref() == Some(id.as_str())
                {
                    agent.status = "completed".to_string();
                    agent.task = None;
                    agent.updated_at = now_iso();
                }
                append_event(
                    project_root,
                    "task.completed",
                    &actor,
                    Some(&id),
                    "completion gate passed",
                )?;
                Ok(owner)
            })?;
            if let Some(owner) = owner {
                lease::release_task_if_owned(project_root, &id, &owner)
                    .with_context(|| format!("task {id} completed but its ownership lease could not be released"))?;
            }
            Ok(())
        },
'''
    replace_between(
        RUNTIME,
        "        RuntimeCommand::TaskComplete { id, actor } =>",
        "        RuntimeCommand::TaskWait { id, await_id } =>",
        task_complete,
        "runtime final task-complete",
    )
    print("ownership/resume terminal invariants applied")


if __name__ == "__main__":
    main()
