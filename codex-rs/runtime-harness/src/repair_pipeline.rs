use crate::CriterionStatus;
use crate::VerificationEvidence;
use crate::WorkflowFailure;
use crate::WorkflowProgress;
use crate::WorkflowStateError;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairPipelineStage {
    Generate,
    Critique,
    Verify,
    Repair,
    Done,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateArtifact {
    pub id: String,
    pub generation: u32,
    pub summary: String,
    pub changed_files: Vec<PathBuf>,
}

impl CandidateArtifact {
    pub fn new(id: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            generation: 0,
            summary: summary.into(),
            changed_files: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CritiqueSeverity {
    Advisory,
    Blocking,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CritiqueFinding {
    pub id: String,
    pub severity: CritiqueSeverity,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CritiqueReport {
    pub candidate_id: String,
    pub findings: Vec<CritiqueFinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CriterionVerification {
    pub criterion_id: String,
    pub evidence: VerificationEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepairDirective {
    pub candidate_id: String,
    pub candidate_generation: u32,
    pub exact_failures: Vec<String>,
    pub critique_findings: Vec<CritiqueFinding>,
}

#[derive(Debug, thiserror::Error)]
pub enum RepairPipelineError {
    #[error("repair pipeline requires at least one acceptance criterion")]
    MissingAcceptanceCriteria,
    #[error("operation requires pipeline stage {expected:?}, found {actual:?}")]
    InvalidStage {
        expected: RepairPipelineStage,
        actual: RepairPipelineStage,
    },
    #[error("candidate id cannot be empty")]
    EmptyCandidateId,
    #[error("critique targets candidate '{found}', expected '{expected}'")]
    CritiqueCandidateMismatch { expected: String, found: String },
    #[error("verification batch omitted acceptance criteria: {0:?}")]
    MissingCriterionVerification(Vec<String>),
    #[error("verification batch contains duplicate criterion '{0}'")]
    DuplicateCriterionVerification(String),
    #[error(transparent)]
    Workflow(#[from] WorkflowStateError),
}

#[derive(Debug, Clone)]
pub struct GeneratorCriticRepairPipeline {
    pub progress: WorkflowProgress,
    pub stage: RepairPipelineStage,
    candidate: Option<CandidateArtifact>,
    critique: Option<CritiqueReport>,
    repair_cycles: u32,
    max_repair_cycles: u32,
}

impl GeneratorCriticRepairPipeline {
    pub fn new(
        progress: WorkflowProgress,
        max_repair_cycles: u32,
    ) -> Result<Self, RepairPipelineError> {
        if progress.criteria.is_empty() {
            return Err(RepairPipelineError::MissingAcceptanceCriteria);
        }
        Ok(Self {
            progress,
            stage: RepairPipelineStage::Generate,
            candidate: None,
            critique: None,
            repair_cycles: 0,
            max_repair_cycles,
        })
    }

    pub fn candidate(&self) -> Option<&CandidateArtifact> {
        self.candidate.as_ref()
    }

    pub fn critique(&self) -> Option<&CritiqueReport> {
        self.critique.as_ref()
    }

    pub fn repair_cycles(&self) -> u32 {
        self.repair_cycles
    }

    pub fn submit_candidate(
        &mut self,
        mut candidate: CandidateArtifact,
    ) -> Result<(), RepairPipelineError> {
        if self.stage != RepairPipelineStage::Generate {
            return Err(RepairPipelineError::InvalidStage {
                expected: RepairPipelineStage::Generate,
                actual: self.stage,
            });
        }
        validate_candidate(&candidate)?;
        candidate.generation = 0;
        self.progress.current_subtask = Some("independent critique".to_string());
        self.progress.next_action = Some("critique candidate in isolated context".to_string());
        self.progress.changed_files = candidate.changed_files.clone();
        self.candidate = Some(candidate);
        self.critique = None;
        self.stage = RepairPipelineStage::Critique;
        Ok(())
    }

    pub fn submit_critique(
        &mut self,
        report: CritiqueReport,
    ) -> Result<(), RepairPipelineError> {
        require_stage(self.stage, RepairPipelineStage::Critique)?;
        let candidate = self.candidate.as_ref().expect("candidate exists in critique stage");
        if report.candidate_id != candidate.id {
            return Err(RepairPipelineError::CritiqueCandidateMismatch {
                expected: candidate.id.clone(),
                found: report.candidate_id,
            });
        }
        self.critique = Some(report);
        self.progress.current_subtask = Some("independent verification".to_string());
        self.progress.next_action = Some("run every acceptance verifier".to_string());
        self.stage = RepairPipelineStage::Verify;
        Ok(())
    }

    /// Record a complete fresh verification batch for the current candidate.
    ///
    /// Every acceptance criterion must appear exactly once. This prevents a
    /// repaired candidate from inheriting a stale pass from an older generation.
    /// Critic findings are advisory inputs to repair; only verifier evidence can
    /// change acceptance-criterion status or advance the pipeline to Done.
    pub fn record_verification_batch(
        &mut self,
        batch: Vec<CriterionVerification>,
    ) -> Result<RepairPipelineStage, RepairPipelineError> {
        require_stage(self.stage, RepairPipelineStage::Verify)?;
        validate_verification_batch(&self.progress, &batch)?;

        for verification in batch {
            let failure_id = verification_failure_id(&verification.criterion_id);
            let passed = verification.evidence.passed;
            let detail = verification.evidence.detail.clone();
            self.progress
                .record_verification(&verification.criterion_id, verification.evidence)?;
            if passed {
                self.progress.resolve_failure(&failure_id);
            } else {
                self.progress.record_failure(WorkflowFailure {
                    id: failure_id,
                    exact_failure: detail,
                    blocking: true,
                });
            }
        }

        let all_passed = self
            .progress
            .criteria
            .values()
            .all(|criterion| criterion.status == CriterionStatus::Passed);
        if all_passed && self.progress.unresolved_failures.values().all(|failure| !failure.blocking) {
            self.progress.current_subtask = None;
            self.progress.next_action = None;
            self.stage = RepairPipelineStage::Done;
            return Ok(self.stage);
        }

        if self.repair_cycles >= self.max_repair_cycles {
            self.progress.current_subtask = None;
            self.progress.next_action = Some("escalate unresolved verification failures".to_string());
            self.stage = RepairPipelineStage::Blocked;
            return Ok(self.stage);
        }

        self.progress.current_subtask = Some("repair failed verification".to_string());
        self.progress.next_action = Some("repair exact verifier failures".to_string());
        self.stage = RepairPipelineStage::Repair;
        Ok(self.stage)
    }

    pub fn repair_directive(&self) -> Result<RepairDirective, RepairPipelineError> {
        require_stage(self.stage, RepairPipelineStage::Repair)?;
        let candidate = self.candidate.as_ref().expect("candidate exists in repair stage");
        let exact_failures = self
            .progress
            .unresolved_failures
            .values()
            .filter(|failure| failure.blocking)
            .map(|failure| failure.exact_failure.clone())
            .collect();
        let critique_findings = self
            .critique
            .as_ref()
            .map(|report| report.findings.clone())
            .unwrap_or_default();
        Ok(RepairDirective {
            candidate_id: candidate.id.clone(),
            candidate_generation: candidate.generation,
            exact_failures,
            critique_findings,
        })
    }

    pub fn submit_repaired_candidate(
        &mut self,
        action: impl Into<String>,
        mut candidate: CandidateArtifact,
    ) -> Result<(), RepairPipelineError> {
        require_stage(self.stage, RepairPipelineStage::Repair)?;
        validate_candidate(&candidate)?;
        let previous = self.candidate.as_ref().expect("candidate exists in repair stage");
        candidate.generation = previous.generation.saturating_add(1);
        self.repair_cycles = self.repair_cycles.saturating_add(1);
        let action = action.into();
        let failure_ids = self
            .progress
            .unresolved_failures
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for failure_id in failure_ids {
            self.progress
                .record_repair_attempt(failure_id, action.clone(), None);
        }

        self.progress.changed_files = candidate.changed_files.clone();
        self.progress.current_subtask = Some("independent critique".to_string());
        self.progress.next_action = Some("critique repaired candidate in fresh context".to_string());
        self.candidate = Some(candidate);
        self.critique = None;
        self.stage = RepairPipelineStage::Critique;
        Ok(())
    }
}

fn validate_candidate(candidate: &CandidateArtifact) -> Result<(), RepairPipelineError> {
    if candidate.id.trim().is_empty() {
        return Err(RepairPipelineError::EmptyCandidateId);
    }
    Ok(())
}

fn require_stage(
    actual: RepairPipelineStage,
    expected: RepairPipelineStage,
) -> Result<(), RepairPipelineError> {
    if actual == expected {
        Ok(())
    } else {
        Err(RepairPipelineError::InvalidStage { expected, actual })
    }
}

fn validate_verification_batch(
    progress: &WorkflowProgress,
    batch: &[CriterionVerification],
) -> Result<(), RepairPipelineError> {
    let mut by_id = BTreeMap::new();
    for verification in batch {
        if by_id
            .insert(verification.criterion_id.as_str(), ())
            .is_some()
        {
            return Err(RepairPipelineError::DuplicateCriterionVerification(
                verification.criterion_id.clone(),
            ));
        }
    }
    let missing = progress
        .criteria
        .keys()
        .filter(|id| !by_id.contains_key(id.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(RepairPipelineError::MissingCriterionVerification(missing))
    }
}

fn verification_failure_id(criterion_id: &str) -> String {
    format!("verification:{criterion_id}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AcceptanceCriterion;
    use crate::VerificationKind;

    fn progress() -> WorkflowProgress {
        let mut progress = WorkflowProgress::new("ship parser change");
        progress.add_criterion(AcceptanceCriterion::pending("unit", "unit tests pass"));
        progress.add_criterion(AcceptanceCriterion::pending("lint", "lint passes"));
        progress
    }

    fn evidence(passed: bool, detail: &str) -> VerificationEvidence {
        VerificationEvidence {
            verifier: "external".to_string(),
            kind: VerificationKind::Custom("test".to_string()),
            passed,
            detail: detail.to_string(),
            artifact: None,
        }
    }

    fn candidate() -> CandidateArtifact {
        CandidateArtifact::new("candidate-a", "parser implementation")
    }

    #[test]
    fn candidate_must_pass_through_critic_before_verification() {
        let mut pipeline = GeneratorCriticRepairPipeline::new(progress(), 2).unwrap();
        pipeline.submit_candidate(candidate()).unwrap();
        assert_eq!(pipeline.stage, RepairPipelineStage::Critique);
        assert!(matches!(
            pipeline.record_verification_batch(Vec::new()),
            Err(RepairPipelineError::InvalidStage { .. })
        ));
    }

    #[test]
    fn verification_batch_must_cover_every_acceptance_criterion() {
        let mut pipeline = GeneratorCriticRepairPipeline::new(progress(), 2).unwrap();
        pipeline.submit_candidate(candidate()).unwrap();
        pipeline
            .submit_critique(CritiqueReport {
                candidate_id: "candidate-a".to_string(),
                findings: Vec::new(),
            })
            .unwrap();

        assert!(matches!(
            pipeline.record_verification_batch(vec![CriterionVerification {
                criterion_id: "unit".to_string(),
                evidence: evidence(true, "unit pass"),
            }]),
            Err(RepairPipelineError::MissingCriterionVerification(_))
        ));
    }

    #[test]
    fn failed_verification_produces_exact_repair_directive() {
        let mut pipeline = GeneratorCriticRepairPipeline::new(progress(), 2).unwrap();
        pipeline.submit_candidate(candidate()).unwrap();
        pipeline
            .submit_critique(CritiqueReport {
                candidate_id: "candidate-a".to_string(),
                findings: vec![CritiqueFinding {
                    id: "c1".to_string(),
                    severity: CritiqueSeverity::Blocking,
                    message: "check error path".to_string(),
                }],
            })
            .unwrap();
        let stage = pipeline
            .record_verification_batch(vec![
                CriterionVerification {
                    criterion_id: "unit".to_string(),
                    evidence: evidence(false, "test_parser_error_path failed"),
                },
                CriterionVerification {
                    criterion_id: "lint".to_string(),
                    evidence: evidence(true, "lint pass"),
                },
            ])
            .unwrap();
        assert_eq!(stage, RepairPipelineStage::Repair);

        let directive = pipeline.repair_directive().unwrap();
        assert_eq!(directive.exact_failures, vec!["test_parser_error_path failed"]);
        assert_eq!(directive.critique_findings.len(), 1);
    }

    #[test]
    fn repaired_candidate_requires_fresh_full_verification() {
        let mut pipeline = GeneratorCriticRepairPipeline::new(progress(), 2).unwrap();
        pipeline.submit_candidate(candidate()).unwrap();
        pipeline
            .submit_critique(CritiqueReport {
                candidate_id: "candidate-a".to_string(),
                findings: Vec::new(),
            })
            .unwrap();
        pipeline
            .record_verification_batch(vec![
                CriterionVerification {
                    criterion_id: "unit".to_string(),
                    evidence: evidence(false, "unit failed"),
                },
                CriterionVerification {
                    criterion_id: "lint".to_string(),
                    evidence: evidence(true, "lint pass"),
                },
            ])
            .unwrap();
        pipeline
            .submit_repaired_candidate("fix unit failure", candidate())
            .unwrap();
        assert_eq!(pipeline.candidate().unwrap().generation, 1);
        pipeline
            .submit_critique(CritiqueReport {
                candidate_id: "candidate-a".to_string(),
                findings: Vec::new(),
            })
            .unwrap();

        assert!(matches!(
            pipeline.record_verification_batch(vec![CriterionVerification {
                criterion_id: "unit".to_string(),
                evidence: evidence(true, "unit now passes"),
            }]),
            Err(RepairPipelineError::MissingCriterionVerification(_))
        ));
    }

    #[test]
    fn only_external_verification_can_advance_to_done() {
        let mut pipeline = GeneratorCriticRepairPipeline::new(progress(), 2).unwrap();
        pipeline.submit_candidate(candidate()).unwrap();
        pipeline
            .submit_critique(CritiqueReport {
                candidate_id: "candidate-a".to_string(),
                findings: Vec::new(),
            })
            .unwrap();
        let stage = pipeline
            .record_verification_batch(vec![
                CriterionVerification {
                    criterion_id: "unit".to_string(),
                    evidence: evidence(true, "unit pass"),
                },
                CriterionVerification {
                    criterion_id: "lint".to_string(),
                    evidence: evidence(true, "lint pass"),
                },
            ])
            .unwrap();

        assert_eq!(stage, RepairPipelineStage::Done);
        assert!(pipeline.progress.completion().is_done());
    }

    #[test]
    fn exhausted_repair_budget_blocks_and_requests_escalation() {
        let mut pipeline = GeneratorCriticRepairPipeline::new(progress(), 0).unwrap();
        pipeline.submit_candidate(candidate()).unwrap();
        pipeline
            .submit_critique(CritiqueReport {
                candidate_id: "candidate-a".to_string(),
                findings: Vec::new(),
            })
            .unwrap();
        let stage = pipeline
            .record_verification_batch(vec![
                CriterionVerification {
                    criterion_id: "unit".to_string(),
                    evidence: evidence(false, "unit failed"),
                },
                CriterionVerification {
                    criterion_id: "lint".to_string(),
                    evidence: evidence(true, "lint pass"),
                },
            ])
            .unwrap();

        assert_eq!(stage, RepairPipelineStage::Blocked);
        assert_eq!(
            pipeline.progress.next_action.as_deref(),
            Some("escalate unresolved verification failures")
        );
    }
}
