use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

const DURABLE_STEP_FORMAT_VERSION: u32 = 1;
const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SideEffectClass {
    ReadOnly,
    IdempotentWrite,
    NonIdempotentWrite,
    Irreversible,
}

impl SideEffectClass {
    pub const fn retry_safe_after_interruption(self) -> bool {
        matches!(self, Self::ReadOnly | Self::IdempotentWrite)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurableStepIntent {
    pub step_id: String,
    pub idempotency_key: String,
    pub operation: String,
    pub side_effect: SideEffectClass,
    pub input_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DurableStepEventKind {
    Prepared { intent: DurableStepIntent },
    Committed { output_fingerprint: Option<String> },
    Failed { exact_failure: String, retryable: bool },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurableStepEvent {
    pub format_version: u32,
    pub sequence: u64,
    pub step_id: String,
    pub event: DurableStepEventKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DurableStepState {
    Pending { intent: DurableStepIntent },
    Committed {
        intent: DurableStepIntent,
        output_fingerprint: Option<String>,
    },
    Failed {
        intent: DurableStepIntent,
        exact_failure: String,
        retryable: bool,
    },
}

impl DurableStepState {
    pub fn intent(&self) -> &DurableStepIntent {
        match self {
            Self::Pending { intent }
            | Self::Committed { intent, .. }
            | Self::Failed { intent, .. } => intent,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryDisposition {
    RetrySafe,
    RequiresReview,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableRecoveryAction {
    pub step_id: String,
    pub disposition: RecoveryDisposition,
    pub operation: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurableResumeToken {
    pub format_version: u32,
    pub last_sequence: u64,
    pub journal_fingerprint: String,
}

#[derive(Debug, thiserror::Error)]
pub enum DurableStepError {
    #[error("durable step id cannot be empty")]
    EmptyStepId,
    #[error("durable step idempotency key cannot be empty")]
    EmptyIdempotencyKey,
    #[error("durable step operation cannot be empty")]
    EmptyOperation,
    #[error("durable step input fingerprint cannot be empty")]
    EmptyInputFingerprint,
    #[error("idempotency key '{key}' already belongs to step '{step_id}'")]
    DuplicateIdempotencyKey { key: String, step_id: String },
    #[error("durable step '{0}' is unknown")]
    UnknownStep(String),
    #[error("durable step '{step_id}' cannot transition from {state} to {transition}")]
    InvalidTransition {
        step_id: String,
        state: &'static str,
        transition: &'static str,
    },
    #[error("unsupported durable-step journal version {found}; supported version is {supported}")]
    UnsupportedVersion { found: u32, supported: u32 },
    #[error("durable-step journal I/O failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid durable-step journal line {line}: {source}")]
    InvalidLine {
        line: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("durable-step serialization failed: {0}")]
    Serialize(#[from] serde_json::Error),
}

#[derive(Debug)]
pub struct DurableStepJournal {
    path: PathBuf,
    events: Vec<DurableStepEvent>,
    states: BTreeMap<String, DurableStepState>,
    idempotency_keys: BTreeMap<String, String>,
    next_sequence: u64,
}

impl DurableStepJournal {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, DurableStepError> {
        let path = path.into();
        let events = load_events(&path)?;
        let (states, idempotency_keys) = replay_events(&events)?;
        let next_sequence = events
            .iter()
            .map(|event| event.sequence)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        Ok(Self {
            path,
            events,
            states,
            idempotency_keys,
            next_sequence,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn state(&self, step_id: &str) -> Option<&DurableStepState> {
        self.states.get(step_id)
    }

    pub fn prepare(&mut self, intent: DurableStepIntent) -> Result<(), DurableStepError> {
        validate_intent(&intent)?;
        if let Some(step_id) = self.idempotency_keys.get(&intent.idempotency_key) {
            return Err(DurableStepError::DuplicateIdempotencyKey {
                key: intent.idempotency_key,
                step_id: step_id.clone(),
            });
        }
        let event = DurableStepEvent {
            format_version: DURABLE_STEP_FORMAT_VERSION,
            sequence: self.next_sequence,
            step_id: intent.step_id.clone(),
            event: DurableStepEventKind::Prepared {
                intent: intent.clone(),
            },
        };
        self.append_event(event)?;
        self.idempotency_keys
            .insert(intent.idempotency_key.clone(), intent.step_id.clone());
        self.states
            .insert(intent.step_id.clone(), DurableStepState::Pending { intent });
        Ok(())
    }

    pub fn commit(
        &mut self,
        step_id: &str,
        output_fingerprint: Option<String>,
    ) -> Result<(), DurableStepError> {
        let state = self
            .states
            .get(step_id)
            .cloned()
            .ok_or_else(|| DurableStepError::UnknownStep(step_id.to_string()))?;
        let intent = match state {
            DurableStepState::Pending { intent }
            | DurableStepState::Failed {
                intent,
                retryable: true,
                ..
            } => intent,
            DurableStepState::Committed { .. } => {
                return Err(DurableStepError::InvalidTransition {
                    step_id: step_id.to_string(),
                    state: "committed",
                    transition: "commit",
                });
            }
            DurableStepState::Failed {
                retryable: false, ..
            } => {
                return Err(DurableStepError::InvalidTransition {
                    step_id: step_id.to_string(),
                    state: "failed_non_retryable",
                    transition: "commit",
                });
            }
        };
        let event = DurableStepEvent {
            format_version: DURABLE_STEP_FORMAT_VERSION,
            sequence: self.next_sequence,
            step_id: step_id.to_string(),
            event: DurableStepEventKind::Committed {
                output_fingerprint: output_fingerprint.clone(),
            },
        };
        self.append_event(event)?;
        self.states.insert(
            step_id.to_string(),
            DurableStepState::Committed {
                intent,
                output_fingerprint,
            },
        );
        Ok(())
    }

    pub fn fail(
        &mut self,
        step_id: &str,
        exact_failure: impl Into<String>,
        retryable: bool,
    ) -> Result<(), DurableStepError> {
        let state = self
            .states
            .get(step_id)
            .cloned()
            .ok_or_else(|| DurableStepError::UnknownStep(step_id.to_string()))?;
        let intent = match state {
            DurableStepState::Pending { intent }
            | DurableStepState::Failed {
                intent,
                retryable: true,
                ..
            } => intent,
            DurableStepState::Committed { .. } => {
                return Err(DurableStepError::InvalidTransition {
                    step_id: step_id.to_string(),
                    state: "committed",
                    transition: "fail",
                });
            }
            DurableStepState::Failed {
                retryable: false, ..
            } => {
                return Err(DurableStepError::InvalidTransition {
                    step_id: step_id.to_string(),
                    state: "failed_non_retryable",
                    transition: "fail",
                });
            }
        };
        let exact_failure = exact_failure.into();
        let event = DurableStepEvent {
            format_version: DURABLE_STEP_FORMAT_VERSION,
            sequence: self.next_sequence,
            step_id: step_id.to_string(),
            event: DurableStepEventKind::Failed {
                exact_failure: exact_failure.clone(),
                retryable,
            },
        };
        self.append_event(event)?;
        self.states.insert(
            step_id.to_string(),
            DurableStepState::Failed {
                intent,
                exact_failure,
                retryable,
            },
        );
        Ok(())
    }

    pub fn recovery_actions(&self) -> Vec<DurableRecoveryAction> {
        self.states
            .iter()
            .filter_map(|(step_id, state)| match state {
                DurableStepState::Committed { .. } => None,
                DurableStepState::Pending { intent } => {
                    Some(recovery_for_pending(step_id, intent))
                }
                DurableStepState::Failed {
                    intent,
                    exact_failure,
                    retryable,
                } => Some(recovery_for_failure(
                    step_id,
                    intent,
                    exact_failure,
                    *retryable,
                )),
            })
            .collect()
    }

    pub fn resume_token(&self) -> Result<DurableResumeToken, DurableStepError> {
        let bytes = serde_json::to_vec(&self.events)?;
        Ok(DurableResumeToken {
            format_version: DURABLE_STEP_FORMAT_VERSION,
            last_sequence: self.next_sequence.saturating_sub(1),
            journal_fingerprint: fnv_hex(&bytes),
        })
    }

    fn append_event(&mut self, event: DurableStepEvent) -> Result<(), DurableStepError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|source| DurableStepError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|source| DurableStepError::Io {
                path: self.path.clone(),
                source,
            })?;
        serde_json::to_writer(&mut file, &event)?;
        file.write_all(b"\n").map_err(|source| DurableStepError::Io {
            path: self.path.clone(),
            source,
        })?;
        file.sync_all().map_err(|source| DurableStepError::Io {
            path: self.path.clone(),
            source,
        })?;
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.events.push(event);
        Ok(())
    }
}

fn validate_intent(intent: &DurableStepIntent) -> Result<(), DurableStepError> {
    if intent.step_id.trim().is_empty() {
        return Err(DurableStepError::EmptyStepId);
    }
    if intent.idempotency_key.trim().is_empty() {
        return Err(DurableStepError::EmptyIdempotencyKey);
    }
    if intent.operation.trim().is_empty() {
        return Err(DurableStepError::EmptyOperation);
    }
    if intent.input_fingerprint.trim().is_empty() {
        return Err(DurableStepError::EmptyInputFingerprint);
    }
    Ok(())
}

fn load_events(path: &Path) -> Result<Vec<DurableStepEvent>, DurableStepError> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(DurableStepError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let mut events = Vec::new();
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|source| DurableStepError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if line.trim().is_empty() {
            continue;
        }
        let event: DurableStepEvent =
            serde_json::from_str(&line).map_err(|source| DurableStepError::InvalidLine {
                line: index + 1,
                source,
            })?;
        if event.format_version != DURABLE_STEP_FORMAT_VERSION {
            return Err(DurableStepError::UnsupportedVersion {
                found: event.format_version,
                supported: DURABLE_STEP_FORMAT_VERSION,
            });
        }
        events.push(event);
    }
    Ok(events)
}

fn replay_events(
    events: &[DurableStepEvent],
) -> Result<(BTreeMap<String, DurableStepState>, BTreeMap<String, String>), DurableStepError> {
    let mut states = BTreeMap::new();
    let mut keys = BTreeMap::new();
    for event in events {
        match &event.event {
            DurableStepEventKind::Prepared { intent } => {
                validate_intent(intent)?;
                if let Some(existing) = keys.get(&intent.idempotency_key) {
                    return Err(DurableStepError::DuplicateIdempotencyKey {
                        key: intent.idempotency_key.clone(),
                        step_id: existing.clone(),
                    });
                }
                keys.insert(intent.idempotency_key.clone(), intent.step_id.clone());
                states.insert(
                    intent.step_id.clone(),
                    DurableStepState::Pending {
                        intent: intent.clone(),
                    },
                );
            }
            DurableStepEventKind::Committed { output_fingerprint } => {
                let Some(state) = states.get(&event.step_id).cloned() else {
                    return Err(DurableStepError::UnknownStep(event.step_id.clone()));
                };
                let intent = state.intent().clone();
                states.insert(
                    event.step_id.clone(),
                    DurableStepState::Committed {
                        intent,
                        output_fingerprint: output_fingerprint.clone(),
                    },
                );
            }
            DurableStepEventKind::Failed {
                exact_failure,
                retryable,
            } => {
                let Some(state) = states.get(&event.step_id).cloned() else {
                    return Err(DurableStepError::UnknownStep(event.step_id.clone()));
                };
                let intent = state.intent().clone();
                states.insert(
                    event.step_id.clone(),
                    DurableStepState::Failed {
                        intent,
                        exact_failure: exact_failure.clone(),
                        retryable: *retryable,
                    },
                );
            }
        }
    }
    Ok((states, keys))
}

fn recovery_for_pending(step_id: &str, intent: &DurableStepIntent) -> DurableRecoveryAction {
    if intent.side_effect.retry_safe_after_interruption() {
        DurableRecoveryAction {
            step_id: step_id.to_string(),
            disposition: RecoveryDisposition::RetrySafe,
            operation: intent.operation.clone(),
            reason: "prepared step has no terminal event and side effects are retry-safe"
                .to_string(),
        }
    } else {
        DurableRecoveryAction {
            step_id: step_id.to_string(),
            disposition: RecoveryDisposition::RequiresReview,
            operation: intent.operation.clone(),
            reason: "prepared step has no terminal event and may already have produced a non-idempotent side effect"
                .to_string(),
        }
    }
}

fn recovery_for_failure(
    step_id: &str,
    intent: &DurableStepIntent,
    exact_failure: &str,
    retryable: bool,
) -> DurableRecoveryAction {
    let retry_safe = retryable && intent.side_effect.retry_safe_after_interruption();
    DurableRecoveryAction {
        step_id: step_id.to_string(),
        disposition: if retry_safe {
            RecoveryDisposition::RetrySafe
        } else {
            RecoveryDisposition::RequiresReview
        },
        operation: intent.operation.clone(),
        reason: if retry_safe {
            format!("retryable failure with retry-safe side effects: {exact_failure}")
        } else {
            format!("automatic retry is unsafe or disallowed: {exact_failure}")
        },
    }
}

fn fnv_hex(bytes: &[u8]) -> String {
    let mut hash = FNV_OFFSET_BASIS;
    for byte in DURABLE_STEP_FORMAT_VERSION
        .to_le_bytes()
        .into_iter()
        .chain(bytes.iter().copied())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn intent(id: &str, key: &str, side_effect: SideEffectClass) -> DurableStepIntent {
        DurableStepIntent {
            step_id: id.to_string(),
            idempotency_key: key.to_string(),
            operation: format!("operation {id}"),
            side_effect,
            input_fingerprint: "input-v1".to_string(),
        }
    }

    #[test]
    fn interrupted_idempotent_step_is_retry_safe_after_reopen() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("steps.jsonl");
        let mut journal = DurableStepJournal::open(&path).unwrap();
        journal
            .prepare(intent("write-cache", "cache-key", SideEffectClass::IdempotentWrite))
            .unwrap();
        drop(journal);

        let journal = DurableStepJournal::open(&path).unwrap();
        let actions = journal.recovery_actions();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].disposition, RecoveryDisposition::RetrySafe);
    }

    #[test]
    fn interrupted_irreversible_step_requires_review() {
        let directory = tempdir().unwrap();
        let mut journal = DurableStepJournal::open(directory.path().join("steps.jsonl")).unwrap();
        journal
            .prepare(intent("publish", "release-1", SideEffectClass::Irreversible))
            .unwrap();
        let actions = journal.recovery_actions();
        assert_eq!(actions[0].disposition, RecoveryDisposition::RequiresReview);
    }

    #[test]
    fn committed_idempotency_key_cannot_be_prepared_again() {
        let directory = tempdir().unwrap();
        let mut journal = DurableStepJournal::open(directory.path().join("steps.jsonl")).unwrap();
        journal
            .prepare(intent("upload", "artifact-abc", SideEffectClass::IdempotentWrite))
            .unwrap();
        journal.commit("upload", Some("output-v1".to_string())).unwrap();

        assert!(matches!(
            journal.prepare(intent(
                "upload-again",
                "artifact-abc",
                SideEffectClass::IdempotentWrite
            )),
            Err(DurableStepError::DuplicateIdempotencyKey { .. })
        ));
        assert!(journal.recovery_actions().is_empty());
    }

    #[test]
    fn retryable_failure_requires_safe_side_effect_class() {
        let directory = tempdir().unwrap();
        let mut journal = DurableStepJournal::open(directory.path().join("steps.jsonl")).unwrap();
        journal
            .prepare(intent("query", "query-1", SideEffectClass::ReadOnly))
            .unwrap();
        journal.fail("query", "network reset", true).unwrap();
        assert_eq!(
            journal.recovery_actions()[0].disposition,
            RecoveryDisposition::RetrySafe
        );
    }

    #[test]
    fn resume_token_changes_as_journal_advances() {
        let directory = tempdir().unwrap();
        let mut journal = DurableStepJournal::open(directory.path().join("steps.jsonl")).unwrap();
        let empty = journal.resume_token().unwrap();
        journal
            .prepare(intent("read", "read-1", SideEffectClass::ReadOnly))
            .unwrap();
        let prepared = journal.resume_token().unwrap();
        journal.commit("read", None).unwrap();
        let committed = journal.resume_token().unwrap();

        assert_ne!(empty.journal_fingerprint, prepared.journal_fingerprint);
        assert_ne!(prepared.journal_fingerprint, committed.journal_fingerprint);
        assert_eq!(committed.last_sequence, 2);
    }
}
