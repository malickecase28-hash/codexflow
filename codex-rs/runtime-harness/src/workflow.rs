use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CriterionStatus {
    Pending,
    Passed,
    Failed,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationKind {
    Syntax,
    TypeCheck,
    Lint,
    UnitTest,
    IntegrationTest,
    EndToEnd,
    Browser,
    Screenshot,
    Compiler,
    Runtime,
    Schema,
    Artifact,
    StaticCheck,
    Custom(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationEvidence {
    pub verifier: String,
    pub kind: VerificationKind,
    pub passed: bool,
    pub detail: String,
    pub artifact: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptanceCriterion {
    pub id: String,
    pub description: String,
    pub status: CriterionStatus,
    pub evidence: Vec<VerificationEvidence>,
}

impl AcceptanceCriterion {
    pub fn pending(id: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            description: description.into(),
            status: CriterionStatus::Pending,
            evidence: Vec::new(),
        }
    }

    pub fn record(&mut self, evidence: VerificationEvidence) {
        self.status = if evidence.passed {
            CriterionStatus::Passed
        } else {
            CriterionStatus::Failed
        };
        self.evidence.push(evidence);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowFailure {
    pub id: String,
    pub exact_failure: String,
    pub blocking: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepairAttempt {
    pub failure_id: String,
    pub attempt: u32,
    pub action: String,
    pub verification: Option<VerificationEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowProgress {
    pub goal: String,
    pub current_subtask: Option<String>,
    pub criteria: BTreeMap<String, AcceptanceCriterion>,
    pub completed_tasks: Vec<String>,
    pub failed_tasks: Vec<String>,
    pub blocked_tasks: Vec<String>,
    pub next_action: Option<String>,
    pub changed_files: Vec<PathBuf>,
    pub open_questions: Vec<String>,
    pub discovered_constraints: Vec<String>,
    pub important_commands: Vec<String>,
    pub environment_status: Vec<String>,
    pub unresolved_failures: BTreeMap<String, WorkflowFailure>,
    pub repair_attempts: Vec<RepairAttempt>,
}

impl WorkflowProgress {
    pub fn new(goal: impl Into<String>) -> Self {
        Self {
            goal: goal.into(),
            current_subtask: None,
            criteria: BTreeMap::new(),
            completed_tasks: Vec::new(),
            failed_tasks: Vec::new(),
            blocked_tasks: Vec::new(),
            next_action: None,
            changed_files: Vec::new(),
            open_questions: Vec::new(),
            discovered_constraints: Vec::new(),
            important_commands: Vec::new(),
            environment_status: Vec::new(),
            unresolved_failures: BTreeMap::new(),
            repair_attempts: Vec::new(),
        }
    }

    pub fn add_criterion(&mut self, criterion: AcceptanceCriterion) -> bool {
        self.criteria.insert(criterion.id.clone(), criterion).is_none()
    }

    pub fn record_verification(
        &mut self,
        criterion_id: &str,
        evidence: VerificationEvidence,
    ) -> Result<(), WorkflowStateError> {
        let criterion = self
            .criteria
            .get_mut(criterion_id)
            .ok_or_else(|| WorkflowStateError::UnknownCriterion(criterion_id.to_string()))?;
        criterion.record(evidence);
        Ok(())
    }

    pub fn record_failure(&mut self, failure: WorkflowFailure) {
        self.unresolved_failures
            .insert(failure.id.clone(), failure);
    }

    pub fn resolve_failure(&mut self, failure_id: &str) -> bool {
        self.unresolved_failures.remove(failure_id).is_some()
    }

    pub fn record_repair_attempt(
        &mut self,
        failure_id: impl Into<String>,
        action: impl Into<String>,
        verification: Option<VerificationEvidence>,
    ) -> RepairAttempt {
        let failure_id = failure_id.into();
        let attempt = self
            .repair_attempts
            .iter()
            .filter(|entry| entry.failure_id == failure_id)
            .count()
            .saturating_add(1) as u32;
        let entry = RepairAttempt {
            failure_id,
            attempt,
            action: action.into(),
            verification,
        };
        self.repair_attempts.push(entry.clone());
        entry
    }

    pub fn completion(&self) -> CompletionGate {
        let mut blocking_reasons = Vec::new();

        if self.criteria.is_empty() {
            blocking_reasons.push("no acceptance criteria are defined".to_string());
        }
        for criterion in self.criteria.values() {
            if criterion.status != CriterionStatus::Passed {
                blocking_reasons.push(format!(
                    "acceptance criterion {} is {:?}",
                    criterion.id, criterion.status
                ));
            } else if criterion.evidence.is_empty() {
                blocking_reasons.push(format!(
                    "acceptance criterion {} has no verification evidence",
                    criterion.id
                ));
            } else if !criterion.evidence.iter().any(|evidence| evidence.passed) {
                blocking_reasons.push(format!(
                    "acceptance criterion {} has no passing evidence",
                    criterion.id
                ));
            }
        }

        for failure in self.unresolved_failures.values() {
            if failure.blocking {
                blocking_reasons.push(format!(
                    "blocking failure {} remains unresolved: {}",
                    failure.id, failure.exact_failure
                ));
            }
        }
        if !self.blocked_tasks.is_empty() {
            blocking_reasons.push(format!(
                "{} blocked task(s) remain",
                self.blocked_tasks.len()
            ));
        }
        if !self.failed_tasks.is_empty() {
            blocking_reasons.push(format!(
                "{} failed task(s) remain",
                self.failed_tasks.len()
            ));
        }
        if self.current_subtask.is_some() {
            blocking_reasons.push("a subtask is still active".to_string());
        }
        if self.next_action.is_some() {
            blocking_reasons.push("a next action is still recorded".to_string());
        }

        if blocking_reasons.is_empty() {
            CompletionGate::Done
        } else {
            CompletionGate::Blocked { blocking_reasons }
        }
    }

    pub fn progress_percentage(&self) -> u8 {
        if self.criteria.is_empty() {
            return 0;
        }
        let passed = self
            .criteria
            .values()
            .filter(|criterion| criterion.status == CriterionStatus::Passed)
            .count();
        ((passed * 100) / self.criteria.len()) as u8
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionGate {
    Done,
    Blocked { blocking_reasons: Vec<String> },
}

impl CompletionGate {
    pub const fn is_done(&self) -> bool {
        matches!(self, Self::Done)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorkflowStateError {
    #[error("unknown acceptance criterion '{0}'")]
    UnknownCriterion(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowFingerprint {
    pub action: String,
    pub state: String,
}

impl WorkflowFingerprint {
    pub fn new(action: impl Into<String>, state: impl Into<String>) -> Self {
        Self {
            action: action.into(),
            state: state.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LoopDetector {
    window: VecDeque<WorkflowFingerprint>,
    max_window: usize,
    repeat_threshold: usize,
}

impl LoopDetector {
    pub fn new(max_window: usize, repeat_threshold: usize) -> Self {
        Self {
            window: VecDeque::with_capacity(max_window),
            max_window,
            repeat_threshold: repeat_threshold.max(2),
        }
    }

    /// Record an externally computed action/state fingerprint and return true
    /// once the exact same action in the exact same state repeats enough times.
    /// This avoids guessing whether similar-looking but progressing actions are a
    /// loop; callers decide what constitutes equivalent state.
    pub fn record(&mut self, fingerprint: WorkflowFingerprint) -> bool {
        if self.max_window == 0 {
            return false;
        }
        let repeats = self
            .window
            .iter()
            .filter(|entry| **entry == fingerprint)
            .count()
            .saturating_add(1);
        self.window.push_back(fingerprint);
        while self.window.len() > self.max_window {
            self.window.pop_front();
        }
        repeats >= self.repeat_threshold
    }

    pub fn clear(&mut self) {
        self.window.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evidence(passed: bool) -> VerificationEvidence {
        VerificationEvidence {
            verifier: "cargo test".to_string(),
            kind: VerificationKind::UnitTest,
            passed,
            detail: if passed {
                "all tests passed".to_string()
            } else {
                "one test failed".to_string()
            },
            artifact: None,
        }
    }

    #[test]
    fn completion_is_impossible_without_acceptance_criteria() {
        let progress = WorkflowProgress::new("build feature");

        assert!(!progress.completion().is_done());
        assert_eq!(progress.progress_percentage(), 0);
    }

    #[test]
    fn criterion_requires_external_passing_evidence() {
        let mut progress = WorkflowProgress::new("build feature");
        progress.add_criterion(AcceptanceCriterion::pending("tests", "tests pass"));
        progress
            .record_verification("tests", evidence(false))
            .unwrap();

        assert!(!progress.completion().is_done());
        assert_eq!(progress.progress_percentage(), 0);

        progress
            .record_verification("tests", evidence(true))
            .unwrap();
        assert!(progress.completion().is_done());
        assert_eq!(progress.progress_percentage(), 100);
    }

    #[test]
    fn unresolved_blocking_failure_prevents_done() {
        let mut progress = WorkflowProgress::new("build feature");
        progress.add_criterion(AcceptanceCriterion::pending("tests", "tests pass"));
        progress
            .record_verification("tests", evidence(true))
            .unwrap();
        progress.record_failure(WorkflowFailure {
            id: "windows".to_string(),
            exact_failure: "link failed".to_string(),
            blocking: true,
        });

        assert!(!progress.completion().is_done());
        assert!(progress.resolve_failure("windows"));
        assert!(progress.completion().is_done());
    }

    #[test]
    fn repair_attempts_are_numbered_per_failure() {
        let mut progress = WorkflowProgress::new("repair feature");
        let first = progress.record_repair_attempt("lint", "apply lint fix", None);
        let second = progress.record_repair_attempt("lint", "apply second fix", Some(evidence(true)));
        let other = progress.record_repair_attempt("test", "repair test", None);

        assert_eq!(first.attempt, 1);
        assert_eq!(second.attempt, 2);
        assert_eq!(other.attempt, 1);
    }

    #[test]
    fn loop_detector_only_trips_on_repeated_action_and_state() {
        let mut detector = LoopDetector::new(5, 3);
        assert!(!detector.record(WorkflowFingerprint::new("cargo test", "sha-a")));
        assert!(!detector.record(WorkflowFingerprint::new("cargo test", "sha-b")));
        assert!(!detector.record(WorkflowFingerprint::new("cargo test", "sha-a")));
        assert!(detector.record(WorkflowFingerprint::new("cargo test", "sha-a")));
    }

    #[test]
    fn active_or_pending_work_prevents_premature_done() {
        let mut progress = WorkflowProgress::new("build feature");
        progress.add_criterion(AcceptanceCriterion::pending("tests", "tests pass"));
        progress
            .record_verification("tests", evidence(true))
            .unwrap();
        progress.current_subtask = Some("inspect diff".to_string());
        progress.next_action = Some("run final review".to_string());

        let CompletionGate::Blocked { blocking_reasons } = progress.completion() else {
            panic!("completion should remain blocked");
        };
        assert!(blocking_reasons.iter().any(|reason| reason.contains("subtask")));
        assert!(blocking_reasons.iter().any(|reason| reason.contains("next action")));
    }
}
