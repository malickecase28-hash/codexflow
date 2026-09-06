use crate::ContextChunk;
use serde::Deserialize;
use serde::Serialize;
use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryTier {
    Hot,
    Warm,
    Cold,
    Archive,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    Working,
    Episodic,
    Semantic,
    Procedural,
    Decision,
    Assumption,
    OpenQuestion,
    LearnedFailure,
    Custom(String),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "value")]
pub enum MemoryScope {
    Global,
    User,
    Project,
    Task(String),
    Custom(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryRecordDraft {
    pub id: String,
    pub kind: MemoryKind,
    pub tier: MemoryTier,
    pub scope: MemoryScope,
    pub content: String,
    #[serde(default)]
    pub tags: BTreeSet<String>,
    pub source: String,
    /// Stable semantic key for conflict detection, such as
    /// `project.rust_version` or `architecture.auth_boundary`.
    pub fact_key: Option<String>,
    pub artifact: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryRecord {
    pub sequence: u64,
    #[serde(flatten)]
    pub draft: MemoryRecordDraft,
}

impl MemoryRecord {
    pub fn id(&self) -> &str {
        &self.draft.id
    }

    pub fn content(&self) -> &str {
        &self.draft.content
    }
}

#[derive(Debug, Clone)]
pub struct MemoryQuery {
    pub terms: Vec<String>,
    pub tiers: BTreeSet<MemoryTier>,
    pub kinds: BTreeSet<MemoryKind>,
    pub scopes: BTreeSet<MemoryScope>,
    pub max_results: usize,
}

impl Default for MemoryQuery {
    fn default() -> Self {
        Self {
            terms: Vec::new(),
            tiers: [MemoryTier::Hot, MemoryTier::Warm]
                .into_iter()
                .collect(),
            kinds: BTreeSet::new(),
            scopes: BTreeSet::new(),
            max_results: 12,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryHit {
    pub record: MemoryRecord,
    pub score: i32,
    pub matched_terms: Vec<String>,
}

impl MemoryHit {
    pub fn to_context_chunk(&self) -> Result<ContextChunk, crate::ContextCompressionError> {
        let priority = self.score.clamp(0, u8::MAX as i32) as u8;
        let chunk = ContextChunk::new(
            format!("memory:{}", self.record.id()),
            format!("memory:{:?}", self.record.draft.kind),
            self.record.content().to_string(),
            priority,
        )?;
        Ok(if self.record.draft.tier == MemoryTier::Hot {
            chunk.pinned()
        } else {
            chunk
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryConflict {
    pub fact_key: String,
    pub record_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryRetrieval {
    pub hits: Vec<MemoryHit>,
    pub conflicts: Vec<MemoryConflict>,
}

#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    #[error("memory id cannot be empty")]
    EmptyId,
    #[error("memory content cannot be empty")]
    EmptyContent,
    #[error("memory source cannot be empty")]
    EmptySource,
    #[error("duplicate memory id '{0}'")]
    DuplicateId(String),
    #[error("memory journal I/O failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid memory journal line {line}: {source}")]
    InvalidLine {
        line: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("memory serialization failed: {0}")]
    Serialize(#[from] serde_json::Error),
}

#[derive(Debug)]
pub struct JsonlMemoryJournal {
    path: PathBuf,
    records: Vec<MemoryRecord>,
    ids: BTreeSet<String>,
    next_sequence: u64,
}

impl JsonlMemoryJournal {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, MemoryError> {
        let path = path.into();
        let records = load_records(&path)?;
        let ids = records
            .iter()
            .map(|record| record.id().to_string())
            .collect::<BTreeSet<_>>();
        let next_sequence = records
            .iter()
            .map(|record| record.sequence)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        Ok(Self {
            path,
            records,
            ids,
            next_sequence,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn records(&self) -> &[MemoryRecord] {
        &self.records
    }

    pub fn append(&mut self, draft: MemoryRecordDraft) -> Result<MemoryRecord, MemoryError> {
        validate_draft(&draft)?;
        if self.ids.contains(&draft.id) {
            return Err(MemoryError::DuplicateId(draft.id));
        }
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|source| MemoryError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }

        let record = MemoryRecord {
            sequence: self.next_sequence,
            draft,
        };
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|source| MemoryError::Io {
                path: self.path.clone(),
                source,
            })?;
        serde_json::to_writer(&mut file, &record)?;
        file.write_all(b"\n").map_err(|source| MemoryError::Io {
            path: self.path.clone(),
            source,
        })?;
        file.sync_all().map_err(|source| MemoryError::Io {
            path: self.path.clone(),
            source,
        })?;

        self.next_sequence = self.next_sequence.saturating_add(1);
        self.ids.insert(record.id().to_string());
        self.records.push(record.clone());
        Ok(record)
    }

    pub fn retrieve(&self, query: &MemoryQuery) -> MemoryRetrieval {
        let terms = normalize_terms(&query.terms);
        let mut hits = self
            .records
            .iter()
            .filter(|record| query.tiers.is_empty() || query.tiers.contains(&record.draft.tier))
            .filter(|record| query.kinds.is_empty() || query.kinds.contains(&record.draft.kind))
            .filter(|record| query.scopes.is_empty() || query.scopes.contains(&record.draft.scope))
            .filter_map(|record| score_record(record, &terms))
            .collect::<Vec<_>>();
        hits.sort_by_key(|hit| (Reverse(hit.score), Reverse(hit.record.sequence), hit.record.id().to_string()));
        hits.truncate(query.max_results);
        let conflicts = detect_conflicts(&hits);
        MemoryRetrieval { hits, conflicts }
    }
}

fn validate_draft(draft: &MemoryRecordDraft) -> Result<(), MemoryError> {
    if draft.id.trim().is_empty() {
        return Err(MemoryError::EmptyId);
    }
    if draft.content.trim().is_empty() {
        return Err(MemoryError::EmptyContent);
    }
    if draft.source.trim().is_empty() {
        return Err(MemoryError::EmptySource);
    }
    Ok(())
}

fn load_records(path: &Path) -> Result<Vec<MemoryRecord>, MemoryError> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(MemoryError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let reader = BufReader::new(file);
    let mut records = Vec::new();
    for (index, line) in reader.lines().enumerate() {
        let line = line.map_err(|source| MemoryError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if line.trim().is_empty() {
            continue;
        }
        let record = serde_json::from_str(&line).map_err(|source| MemoryError::InvalidLine {
            line: index + 1,
            source,
        })?;
        records.push(record);
    }
    Ok(records)
}

fn normalize_terms(terms: &[String]) -> Vec<String> {
    terms
        .iter()
        .map(|term| term.trim().to_ascii_lowercase())
        .filter(|term| !term.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn score_record(record: &MemoryRecord, terms: &[String]) -> Option<MemoryHit> {
    let content = record.content().to_ascii_lowercase();
    let source = record.draft.source.to_ascii_lowercase();
    let tags = record
        .draft
        .tags
        .iter()
        .map(|tag| tag.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let mut matched_terms = Vec::new();
    let mut score = tier_score(record.draft.tier);

    for term in terms {
        let mut matched = false;
        if tags.contains(term) {
            score += 30;
            matched = true;
        }
        if content.contains(term) {
            score += 20;
            matched = true;
        }
        if source.contains(term) {
            score += 8;
            matched = true;
        }
        if record
            .draft
            .fact_key
            .as_ref()
            .is_some_and(|key| key.to_ascii_lowercase().contains(term))
        {
            score += 24;
            matched = true;
        }
        if matched {
            matched_terms.push(term.clone());
        }
    }

    if !terms.is_empty() && matched_terms.is_empty() {
        return None;
    }
    Some(MemoryHit {
        record: record.clone(),
        score,
        matched_terms,
    })
}

const fn tier_score(tier: MemoryTier) -> i32 {
    match tier {
        MemoryTier::Hot => 40,
        MemoryTier::Warm => 24,
        MemoryTier::Cold => 8,
        MemoryTier::Archive => 0,
    }
}

fn detect_conflicts(hits: &[MemoryHit]) -> Vec<MemoryConflict> {
    let mut by_key: BTreeMap<&str, BTreeMap<&str, Vec<&str>>> = BTreeMap::new();
    for hit in hits {
        let Some(key) = hit.record.draft.fact_key.as_deref() else {
            continue;
        };
        by_key
            .entry(key)
            .or_default()
            .entry(hit.record.content())
            .or_default()
            .push(hit.record.id());
    }
    by_key
        .into_iter()
        .filter(|(_, values)| values.len() > 1)
        .map(|(fact_key, values)| MemoryConflict {
            fact_key: fact_key.to_string(),
            record_ids: values
                .into_values()
                .flatten()
                .map(str::to_string)
                .collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn draft(id: &str, content: &str, tier: MemoryTier) -> MemoryRecordDraft {
        MemoryRecordDraft {
            id: id.to_string(),
            kind: MemoryKind::Semantic,
            tier,
            scope: MemoryScope::Project,
            content: content.to_string(),
            tags: BTreeSet::new(),
            source: "test".to_string(),
            fact_key: None,
            artifact: None,
        }
    }

    #[test]
    fn journal_round_trip_preserves_records_and_sequence() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("memory.jsonl");
        let mut journal = JsonlMemoryJournal::open(&path).unwrap();
        journal
            .append(draft("one", "first fact", MemoryTier::Warm))
            .unwrap();
        journal
            .append(draft("two", "second fact", MemoryTier::Cold))
            .unwrap();
        drop(journal);

        let journal = JsonlMemoryJournal::open(&path).unwrap();
        assert_eq!(journal.records().len(), 2);
        assert_eq!(journal.records()[0].sequence, 1);
        assert_eq!(journal.records()[1].sequence, 2);
    }

    #[test]
    fn retrieval_prefers_hot_tagged_relevant_memory() {
        let directory = tempdir().unwrap();
        let mut journal = JsonlMemoryJournal::open(directory.path().join("memory.jsonl")).unwrap();
        let mut hot = draft("hot", "auth boundary uses broker", MemoryTier::Hot);
        hot.tags.insert("auth".to_string());
        journal.append(hot).unwrap();
        journal
            .append(draft("warm", "auth test notes", MemoryTier::Warm))
            .unwrap();

        let retrieval = journal.retrieve(&MemoryQuery {
            terms: vec!["auth".to_string()],
            ..Default::default()
        });
        assert_eq!(retrieval.hits[0].record.id(), "hot");
    }

    #[test]
    fn hot_memory_becomes_pinned_context() {
        let hit = MemoryHit {
            record: MemoryRecord {
                sequence: 1,
                draft: draft("invariant", "never cross provider auth", MemoryTier::Hot),
            },
            score: 80,
            matched_terms: Vec::new(),
        };
        let chunk = hit.to_context_chunk().unwrap();
        assert_eq!(chunk.retention, crate::ContextRetention::Pinned);
    }

    #[test]
    fn same_fact_key_with_different_values_is_reported_as_conflict() {
        let directory = tempdir().unwrap();
        let mut journal = JsonlMemoryJournal::open(directory.path().join("memory.jsonl")).unwrap();
        let mut first = draft("rust-old", "Rust 1.90", MemoryTier::Warm);
        first.fact_key = Some("project.rust_version".to_string());
        let mut second = draft("rust-new", "Rust 1.98", MemoryTier::Warm);
        second.fact_key = Some("project.rust_version".to_string());
        journal.append(first).unwrap();
        journal.append(second).unwrap();

        let retrieval = journal.retrieve(&MemoryQuery {
            terms: vec!["rust".to_string()],
            ..Default::default()
        });
        assert_eq!(retrieval.conflicts.len(), 1);
        assert_eq!(retrieval.conflicts[0].fact_key, "project.rust_version");
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let directory = tempdir().unwrap();
        let mut journal = JsonlMemoryJournal::open(directory.path().join("memory.jsonl")).unwrap();
        journal
            .append(draft("same", "one", MemoryTier::Warm))
            .unwrap();
        assert!(matches!(
            journal.append(draft("same", "two", MemoryTier::Warm)),
            Err(MemoryError::DuplicateId(id)) if id == "same"
        ));
    }
}
