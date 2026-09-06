use crate::VerificationEvidence;
use crate::VerificationKind;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use tokio::process::Command;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessVariant {
    pub name: String,
    pub features: BTreeMap<String, bool>,
    pub parameters: BTreeMap<String, String>,
}

impl HarnessVariant {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            features: BTreeMap::new(),
            parameters: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AblationMatrix {
    pub baseline: HarnessVariant,
    pub variants: Vec<HarnessVariant>,
}

impl AblationMatrix {
    /// Build deterministic one-factor-at-a-time variants from a baseline.
    pub fn one_at_a_time(
        baseline: HarnessVariant,
        feature_values: impl IntoIterator<Item = (String, bool)>,
    ) -> Self {
        let variants = feature_values
            .into_iter()
            .map(|(feature, value)| {
                let mut variant = baseline.clone();
                variant.name = format!("{}__{}={value}", baseline.name, feature);
                variant.features.insert(feature, value);
                variant
            })
            .collect();
        Self { baseline, variants }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InspectEvaluationPlan {
    pub tasks: Vec<String>,
    pub model: Option<String>,
    pub task_args: BTreeMap<String, String>,
    pub extra_args: Vec<String>,
}

impl InspectEvaluationPlan {
    pub fn new(tasks: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            tasks: tasks.into_iter().map(Into::into).collect(),
            model: None,
            task_args: BTreeMap::new(),
            extra_args: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InspectTaskResult {
    pub task: Option<String>,
    pub status: String,
    pub log_location: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InspectEvaluationEvidence {
    pub passed: bool,
    pub run_id: Option<String>,
    pub tasks: Vec<InspectTaskResult>,
    pub records: Vec<Value>,
    pub stderr: String,
}

impl InspectEvaluationEvidence {
    pub fn workflow_evidence(&self) -> VerificationEvidence {
        let failed = self.tasks.iter().filter(|task| task.status != "success").count();
        VerificationEvidence {
            verifier: "inspect-ai".to_string(),
            kind: VerificationKind::Custom("inspect_ai_evaluation".to_string()),
            passed: self.passed,
            detail: format!("{} task(s), {failed} non-success", self.tasks.len()),
            artifact: self
                .tasks
                .iter()
                .find_map(|task| task.log_location.clone()),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum InspectEvaluationError {
    #[error("Inspect evaluation requires at least one task")]
    EmptyTasks,
    #[error("Inspect evaluation adapter does not accept detached runs")]
    DetachedRunUnsupported,
    #[error("failed to execute Inspect command {args:?}: {source}")]
    CommandIo {
        args: Vec<String>,
        #[source]
        source: std::io::Error,
    },
    #[error("Inspect command failed: {args:?}; stderr: {stderr}")]
    CommandFailed { args: Vec<String>, stderr: String },
    #[error("Inspect emitted malformed JSON line: {line}: {source}")]
    InvalidJson {
        line: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("Inspect exited without a terminal done record")]
    MissingDoneRecord,
}

#[derive(Debug, Clone)]
pub struct InspectCliRunner {
    executable: PathBuf,
    working_directory: PathBuf,
}

impl InspectCliRunner {
    pub fn new(executable: impl Into<PathBuf>, working_directory: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            working_directory: working_directory.into(),
        }
    }

    pub async fn run(
        &self,
        plan: &InspectEvaluationPlan,
    ) -> Result<InspectEvaluationEvidence, InspectEvaluationError> {
        validate_plan(plan)?;
        let args = inspect_args(plan);
        let output = Command::new(&self.executable)
            .args(&args)
            .current_dir(&self.working_directory)
            .output()
            .await
            .map_err(|source| InspectEvaluationError::CommandIo {
                args: args.clone(),
                source,
            })?;
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        if !output.status.success() {
            return Err(InspectEvaluationError::CommandFailed { args, stderr });
        }
        parse_inspect_output(&stdout, stderr)
    }
}

fn validate_plan(plan: &InspectEvaluationPlan) -> Result<(), InspectEvaluationError> {
    if plan.tasks.is_empty() || plan.tasks.iter().any(|task| task.trim().is_empty()) {
        return Err(InspectEvaluationError::EmptyTasks);
    }
    if plan.extra_args.iter().any(|arg| {
        arg == "--detach" || arg.starts_with("--detach=") || arg == "--no-detach"
    }) {
        return Err(InspectEvaluationError::DetachedRunUnsupported);
    }
    Ok(())
}

fn inspect_args(plan: &InspectEvaluationPlan) -> Vec<String> {
    let mut args = vec!["eval".to_string(), "--json".to_string()];
    args.extend(plan.tasks.iter().cloned());
    if let Some(model) = &plan.model {
        args.push("--model".to_string());
        args.push(model.clone());
    }
    for (key, value) in &plan.task_args {
        args.push("-T".to_string());
        args.push(format!("{key}={value}"));
    }
    args.extend(plan.extra_args.iter().cloned());
    args
}

fn parse_inspect_output(
    stdout: &str,
    stderr: String,
) -> Result<InspectEvaluationEvidence, InspectEvaluationError> {
    let mut records = Vec::new();
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        records.push(
            serde_json::from_str::<Value>(line).map_err(|source| {
                InspectEvaluationError::InvalidJson {
                    line: line.to_string(),
                    source,
                }
            })?,
        );
    }

    let done = records
        .iter()
        .rev()
        .find(|record| record.get("event").and_then(Value::as_str) == Some("done"))
        .ok_or(InspectEvaluationError::MissingDoneRecord)?;
    let run_id = done
        .get("run_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let tasks = done
        .get("logs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|entry| InspectTaskResult {
            task: entry.get("task").and_then(Value::as_str).map(str::to_string),
            status: entry
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string(),
            log_location: entry
                .get("location")
                .and_then(Value::as_str)
                .map(PathBuf::from),
        })
        .collect::<Vec<_>>();
    let passed = !tasks.is_empty() && tasks.iter().all(|task| task.status == "success");

    Ok(InspectEvaluationEvidence {
        passed,
        run_id,
        tasks,
        records,
        stderr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_at_a_time_ablation_changes_only_requested_feature() {
        let mut baseline = HarnessVariant::new("baseline");
        baseline.features.insert("compression".to_string(), true);
        baseline.features.insert("governance".to_string(), true);
        let matrix = AblationMatrix::one_at_a_time(
            baseline.clone(),
            [("compression".to_string(), false), ("governance".to_string(), false)],
        );

        assert_eq!(matrix.baseline, baseline);
        assert_eq!(matrix.variants.len(), 2);
        assert_eq!(matrix.variants[0].features["compression"], false);
        assert_eq!(matrix.variants[0].features["governance"], true);
        assert_eq!(matrix.variants[1].features["compression"], true);
        assert_eq!(matrix.variants[1].features["governance"], false);
    }

    #[test]
    fn inspect_args_are_deterministic() {
        let mut plan = InspectEvaluationPlan::new(["evals/coding.py"]);
        plan.model = Some("openai/gpt-5".to_string());
        plan.task_args.insert("b".to_string(), "two".to_string());
        plan.task_args.insert("a".to_string(), "one".to_string());

        assert_eq!(
            inspect_args(&plan),
            vec![
                "eval",
                "--json",
                "evals/coding.py",
                "--model",
                "openai/gpt-5",
                "-T",
                "a=one",
                "-T",
                "b=two"
            ]
        );
    }

    #[test]
    fn task_error_is_not_mistaken_for_successful_cli_exit() {
        let stdout = concat!(
            "{\"event\":\"launch\",\"run_id\":\"run-1\"}\n",
            "{\"event\":\"done\",\"run_id\":\"run-1\",\"logs\":[",
            "{\"task\":\"coding\",\"status\":\"error\",\"location\":\"logs/fail.eval\"}]}\n"
        );
        let evidence = parse_inspect_output(stdout, String::new()).unwrap();

        assert!(!evidence.passed);
        assert_eq!(evidence.tasks[0].status, "error");
        assert_eq!(
            evidence.tasks[0].log_location,
            Some(PathBuf::from("logs/fail.eval"))
        );
    }

    #[test]
    fn all_successful_tasks_produce_passing_workflow_evidence() {
        let stdout = concat!(
            "{\"event\":\"launch\",\"run_id\":\"run-2\"}\n",
            "{\"event\":\"done\",\"run_id\":\"run-2\",\"logs\":[",
            "{\"task\":\"a\",\"status\":\"success\",\"location\":\"logs/a.eval\"},",
            "{\"task\":\"b\",\"status\":\"success\",\"location\":\"logs/b.eval\"}]}\n"
        );
        let evidence = parse_inspect_output(stdout, String::new()).unwrap();
        let workflow = evidence.workflow_evidence();

        assert!(evidence.passed);
        assert!(workflow.passed);
        assert_eq!(evidence.run_id.as_deref(), Some("run-2"));
    }

    #[test]
    fn missing_done_record_is_failure() {
        let stdout = "{\"event\":\"launch\",\"run_id\":\"run-3\"}\n";
        assert!(matches!(
            parse_inspect_output(stdout, String::new()),
            Err(InspectEvaluationError::MissingDoneRecord)
        ));
    }

    #[test]
    fn detached_runs_are_rejected_by_synchronous_evidence_adapter() {
        let mut plan = InspectEvaluationPlan::new(["evals/coding.py"]);
        plan.extra_args.push("--detach".to_string());
        assert!(matches!(
            validate_plan(&plan),
            Err(InspectEvaluationError::DetachedRunUnsupported)
        ));
    }
}
