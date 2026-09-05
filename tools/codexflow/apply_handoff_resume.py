#!/usr/bin/env python3
"""Materialize structured durable handoffs on top of ownership/resume runtime."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
RUNTIME = ROOT / "codex-rs/cli/src/bin/codexflow/runtime.rs"
RESUME = ROOT / "codex-rs/cli/src/bin/codexflow/resume.rs"


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one match, found {count}")
    return text.replace(old, new, 1)


def replace_between(text: str, start: str, end: str, replacement: str, label: str) -> str:
    start_index = text.find(start)
    if start_index < 0:
        raise SystemExit(f"{label}: start marker not found")
    end_index = text.find(end, start_index + len(start))
    if end_index < 0:
        raise SystemExit(f"{label}: end marker not found")
    return text[:start_index] + replacement + text[end_index:]


def patch_runtime() -> None:
    text = RUNTIME.read_text()
    old_args = '''    HandoffAdd {
        #[arg(long)]
        task: String,
        #[arg(long = "from")]
        from_actor: String,
        #[arg(long = "to")]
        to_actor: String,
        #[arg(long)]
        summary: String,
        #[arg(long = "ref")]
        refs: Vec<String>,
    },
'''
    new_args = '''    HandoffAdd {
        #[arg(long)]
        task: String,
        #[arg(long = "from")]
        from_actor: String,
        #[arg(long = "to")]
        to_actor: String,
        #[arg(long)]
        summary: String,
        #[arg(long = "ref")]
        refs: Vec<String>,
        #[arg(long = "accomplished")]
        accomplished: Vec<String>,
        #[arg(long = "remaining")]
        remaining_work: Vec<String>,
        #[arg(long = "failure")]
        failures: Vec<String>,
        #[arg(long = "file")]
        relevant_files: Vec<String>,
        #[arg(long = "decision")]
        decisions: Vec<String>,
        #[arg(long)]
        rationale: Option<String>,
        #[arg(long = "restart-command")]
        restart_commands: Vec<String>,
        #[arg(long = "next-action")]
        next_action: Option<String>,
    },
'''
    text = replace_once(text, old_args, new_args, "handoff command args")

    old_record = '''struct HandoffRecord {
    from: String,
    to: String,
    summary: String,
    refs: Vec<String>,
    at: String,
}
'''
    new_record = '''struct HandoffRecord {
    from: String,
    to: String,
    summary: String,
    refs: Vec<String>,
    #[serde(default)]
    accomplished: Vec<String>,
    #[serde(default)]
    remaining_work: Vec<String>,
    #[serde(default)]
    failures: Vec<String>,
    #[serde(default)]
    relevant_files: Vec<String>,
    #[serde(default)]
    decisions: Vec<String>,
    #[serde(default)]
    rationale: Option<String>,
    #[serde(default)]
    restart_commands: Vec<String>,
    #[serde(default)]
    next_action: Option<String>,
    at: String,
}
'''
    text = replace_once(text, old_record, new_record, "handoff record")

    handler = '''        RuntimeCommand::HandoffAdd {
            task,
            from_actor,
            to_actor,
            summary,
            refs,
            accomplished,
            remaining_work,
            failures,
            relevant_files,
            decisions,
            rationale,
            restart_commands,
            next_action,
        } => with_locked_ledger(project_root, |ledger| {
            validate_id(&from_actor)?;
            validate_id(&to_actor)?;
            validate_handoff_text(&summary, "handoff summary")?;
            validate_handoff_items(&refs, "handoff ref")?;
            validate_handoff_items(&accomplished, "handoff accomplished item")?;
            validate_handoff_items(&remaining_work, "handoff remaining item")?;
            validate_handoff_items(&failures, "handoff failure item")?;
            validate_handoff_items(&relevant_files, "handoff file")?;
            validate_handoff_items(&decisions, "handoff decision")?;
            validate_handoff_items(&restart_commands, "handoff restart command")?;
            if let Some(value) = rationale.as_deref() {
                validate_handoff_text(value, "handoff rationale")?;
            }
            if let Some(value) = next_action.as_deref() {
                validate_handoff_text(value, "handoff next action")?;
            }
            let task_record = ledger
                .tasks
                .get_mut(&task)
                .with_context(|| format!("unknown task: {task}"))?;
            task_record.handoffs.push(HandoffRecord {
                from: from_actor.clone(),
                to: to_actor.clone(),
                summary,
                refs,
                accomplished,
                remaining_work,
                failures,
                relevant_files,
                decisions,
                rationale,
                restart_commands,
                next_action,
                at: now_iso(),
            });
            task_record.updated_at = now_iso();
            append_event(
                project_root,
                "handoff",
                &from_actor,
                Some(&task),
                &format!("handoff to {to_actor}"),
            )?;
            Ok(())
        }),
'''
    text = replace_between(
        text,
        '        RuntimeCommand::HandoffAdd {\n',
        '        RuntimeCommand::Snapshot => {\n',
        handler,
        "handoff handler",
    )

    validation = '''fn validate_handoff_text(value: &str, label: &str) -> Result<()> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 2048 {
        bail!("{label} must be 1 to 2048 characters");
    }
    Ok(())
}

fn validate_handoff_items(values: &[String], label: &str) -> Result<()> {
    if values.len() > 128 {
        bail!("{label} list cannot exceed 128 items");
    }
    for value in values {
        validate_handoff_text(value, label)?;
    }
    Ok(())
}

'''
    text = replace_once(
        text,
        'fn validate_agent_status(value: &str) -> Result<()> {\n',
        validation + 'fn validate_agent_status(value: &str) -> Result<()> {\n',
        "handoff validation helpers",
    )

    legacy_test = '''
    #[test]
    fn legacy_handoff_deserializes_with_structured_defaults() {
        let handoff: HandoffRecord = serde_json::from_value(serde_json::json!({
            "from": "worker-a",
            "to": "worker-b",
            "summary": "legacy",
            "refs": ["src/lib.rs"],
            "at": "2026-09-05T00:00:00Z"
        }))
        .expect("legacy handoff");
        assert!(handoff.accomplished.is_empty());
        assert!(handoff.remaining_work.is_empty());
        assert!(handoff.restart_commands.is_empty());
        assert!(handoff.next_action.is_none());
    }
'''
    marker = '\n    #[test]\n    fn completion_requires_passing_evidence() {'
    text = replace_once(
        text,
        marker,
        legacy_test + marker,
        "legacy handoff test",
    )
    RUNTIME.write_text(text)


def patch_resume() -> None:
    text = RESUME.read_text()
    text = replace_once(
        text,
        'use std::path::Path;\n\nconst RUNTIME_SCHEMA',
        'use std::path::Path;\n\n#[path = "handoff_context.rs"]\nmod handoff_context;\n\nconst RUNTIME_SCHEMA',
        "resume handoff module",
    )
    text = replace_once(
        text,
        '    latest_handoff: Option<Value>,\n    relevant_refs: Vec<String>,\n',
        '    latest_handoff: Option<Value>,\n    relevant_refs: Vec<String>,\n    handoff: handoff_context::HandoffContext,\n',
        "resume handoff field",
    )
    text = replace_once(
        text,
        '''    let relevant_refs = latest_handoff
        .as_ref()
        .and_then(|handoff| handoff.get("refs"))
        .map(|value| string_array(Some(value)))
        .unwrap_or_default();

    let status = optional_string(task.get("status"));
''',
        '''    let relevant_refs = latest_handoff
        .as_ref()
        .and_then(|handoff| handoff.get("refs"))
        .map(|value| string_array(Some(value)))
        .unwrap_or_default();
    let handoff = handoff_context::HandoffContext::from_latest(latest_handoff.as_ref());

    let status = optional_string(task.get("status"));
''',
        "resume handoff extraction",
    )
    text = replace_once(
        text,
        '''        &pending_acceptance,
        &blocking_gates,
    );
''',
        '''        &pending_acceptance,
        &blocking_gates,
        &handoff,
    );
''',
        "resume handoff action call",
    )
    text = replace_once(
        text,
        '''        latest_handoff,
        relevant_refs,
        dependencies,
''',
        '''        latest_handoff,
        relevant_refs,
        handoff,
        dependencies,
''',
        "resume handoff packet",
    )
    text = replace_once(
        text,
        '''    pending_acceptance: &[AcceptanceStatus],
    blocking_gates: &[BlockingGate],
) -> String {
''',
        '''    pending_acceptance: &[AcceptanceStatus],
    blocking_gates: &[BlockingGate],
    handoff: &handoff_context::HandoffContext,
) -> String {
''',
        "resume handoff action signature",
    )
    text = replace_once(
        text,
        '''    if status == Some("done") {
        return "task is complete; no implementation work remains".to_string();
    }
    format!("continue task {task_id} and collect deterministic verification evidence")
''',
        '''    if status == Some("done") {
        return "task is complete; no implementation work remains".to_string();
    }
    if let Some(next_action) = handoff.next_action_hint() {
        return next_action.to_string();
    }
    format!("continue task {task_id} and collect deterministic verification evidence")
''',
        "resume explicit handoff next action",
    )

    # Existing unit calls gain a default handoff. This intentionally modifies only
    # the two direct derive calls in the test module if future tests add more calls.
    text = text.replace(
        '            &blocking_gates,\n        );',
        '            &blocking_gates,\n            &handoff_context::HandoffContext::default(),\n        );',
    )
    RESUME.write_text(text)


def main() -> None:
    patch_runtime()
    patch_resume()
    print("structured handoff/resume transformations applied")


if __name__ == "__main__":
    main()
