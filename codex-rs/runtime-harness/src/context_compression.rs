use std::collections::HashMap;
use std::fmt;

const CONTEXT_FORMAT_VERSION: u32 = 2;
const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;
const CHUNK_SEPARATOR_BYTES: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextRetention {
    Compressible,
    Pinned,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextChunk {
    pub id: String,
    pub source: String,
    pub content: String,
    pub priority: u8,
    pub retention: ContextRetention,
}

impl ContextChunk {
    pub fn new(
        id: impl Into<String>,
        source: impl Into<String>,
        content: impl Into<String>,
        priority: u8,
    ) -> Result<Self, ContextCompressionError> {
        let id = id.into();
        if id.trim().is_empty() {
            return Err(ContextCompressionError::EmptyChunkId);
        }
        Ok(Self {
            id,
            source: source.into(),
            content: content.into(),
            priority,
            retention: ContextRetention::Compressible,
        })
    }

    pub fn pinned(mut self) -> Self {
        self.retention = ContextRetention::Pinned;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompressionPolicy {
    /// Maximum number of UTF-8 bytes in the rendered compact context.
    ///
    /// Compression is whole-chunk only: a selected chunk is never truncated.
    /// Pinned chunks are mandatory and cause compression to fail if they cannot
    /// fit rather than silently dropping an invariant.
    pub max_bytes: usize,
}

impl CompressionPolicy {
    pub const fn new(max_bytes: usize) -> Self {
        Self { max_bytes }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompressionStats {
    pub input_chunks: usize,
    pub retained_chunks: usize,
    pub omitted_chunks: usize,
    pub input_bytes: usize,
    pub rendered_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompressedContext {
    rendered: String,
    retained_ids: Vec<String>,
    omitted_ids: Vec<String>,
    archive: Vec<ContextChunk>,
    fingerprint: String,
    stats: CompressionStats,
}

impl CompressedContext {
    pub fn rendered(&self) -> &str {
        &self.rendered
    }

    pub fn retained_ids(&self) -> &[String] {
        &self.retained_ids
    }

    pub fn omitted_ids(&self) -> &[String] {
        &self.omitted_ids
    }

    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    pub const fn stats(&self) -> CompressionStats {
        self.stats
    }

    /// Restore every source chunk in its original order.
    pub fn rehydrate_all(&self) -> Vec<ContextChunk> {
        self.archive.clone()
    }

    /// Restore selected source chunks in the caller's requested order.
    pub fn rehydrate<'a, I>(&self, ids: I) -> Result<Vec<ContextChunk>, ContextCompressionError>
    where
        I: IntoIterator<Item = &'a str>,
    {
        let by_id: HashMap<&str, &ContextChunk> = self
            .archive
            .iter()
            .map(|chunk| (chunk.id.as_str(), chunk))
            .collect();

        ids.into_iter()
            .map(|id| {
                by_id
                    .get(id)
                    .cloned()
                    .cloned()
                    .ok_or_else(|| ContextCompressionError::UnknownChunkId(id.to_string()))
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextCompressionError {
    EmptyChunkId,
    DuplicateChunkId(String),
    UnknownChunkId(String),
    PinnedContextExceedsBudget { required: usize, budget: usize },
}

impl fmt::Display for ContextCompressionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyChunkId => f.write_str("context chunk id cannot be empty"),
            Self::DuplicateChunkId(id) => write!(f, "duplicate context chunk id '{id}'"),
            Self::UnknownChunkId(id) => write!(f, "unknown context chunk id '{id}'"),
            Self::PinnedContextExceedsBudget { required, budget } => write!(
                f,
                "pinned context requires {required} bytes but the context budget is {budget} bytes"
            ),
        }
    }
}

impl std::error::Error for ContextCompressionError {}

/// Deterministically compact context while retaining an exact in-memory archive.
///
/// Pinned chunks are always retained or the operation fails. Remaining chunks
/// are selected by descending priority, with original order as the deterministic
/// tie-breaker. Selected chunks render back in original order so surrounding
/// context remains readable. Omitted chunks remain addressable by id through
/// [`CompressedContext::rehydrate`] and the complete original bundle can be
/// reconstructed with [`CompressedContext::rehydrate_all`].
pub fn compress_context(
    chunks: Vec<ContextChunk>,
    policy: CompressionPolicy,
) -> Result<CompressedContext, ContextCompressionError> {
    validate_chunks(&chunks)?;

    let input_bytes = chunks.iter().map(rendered_chunk_len).sum();
    let mut retained = chunks
        .iter()
        .map(|chunk| chunk.retention == ContextRetention::Pinned)
        .collect::<Vec<_>>();
    let pinned_bytes = rendered_selected_len(&chunks, &retained);
    if pinned_bytes > policy.max_bytes {
        return Err(ContextCompressionError::PinnedContextExceedsBudget {
            required: pinned_bytes,
            budget: policy.max_bytes,
        });
    }

    let mut used_bytes = pinned_bytes;
    let mut retained_count = retained.iter().filter(|keep| **keep).count();
    let mut ranked = chunks
        .iter()
        .enumerate()
        .filter(|(_, chunk)| chunk.retention == ContextRetention::Compressible)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    ranked.sort_by_key(|&index| (std::cmp::Reverse(chunks[index].priority), index));

    for index in ranked {
        let separator_bytes = if retained_count > 0 {
            CHUNK_SEPARATOR_BYTES
        } else {
            0
        };
        let candidate_bytes = rendered_chunk_len(&chunks[index]) + separator_bytes;
        if used_bytes.saturating_add(candidate_bytes) <= policy.max_bytes {
            retained[index] = true;
            used_bytes += candidate_bytes;
            retained_count += 1;
        }
    }

    let retained_ids = chunks
        .iter()
        .zip(&retained)
        .filter(|(_, keep)| **keep)
        .map(|(chunk, _)| chunk.id.clone())
        .collect::<Vec<_>>();
    let omitted_ids = chunks
        .iter()
        .zip(&retained)
        .filter(|(_, keep)| !**keep)
        .map(|(chunk, _)| chunk.id.clone())
        .collect::<Vec<_>>();

    let rendered = render_selected(&chunks, &retained);
    debug_assert_eq!(used_bytes, rendered.len());

    let stats = CompressionStats {
        input_chunks: chunks.len(),
        retained_chunks: retained_ids.len(),
        omitted_chunks: omitted_ids.len(),
        input_bytes,
        rendered_bytes: rendered.len(),
    };

    Ok(CompressedContext {
        fingerprint: fingerprint_chunks(&chunks),
        rendered,
        retained_ids,
        omitted_ids,
        archive: chunks,
        stats,
    })
}

fn validate_chunks(chunks: &[ContextChunk]) -> Result<(), ContextCompressionError> {
    let mut seen = HashMap::with_capacity(chunks.len());
    for chunk in chunks {
        if chunk.id.trim().is_empty() {
            return Err(ContextCompressionError::EmptyChunkId);
        }
        if seen.insert(chunk.id.as_str(), ()).is_some() {
            return Err(ContextCompressionError::DuplicateChunkId(chunk.id.clone()));
        }
    }
    Ok(())
}

fn render_chunk(chunk: &ContextChunk) -> String {
    format!(
        "[[codexflow-context id={} source={}]]\n{}\n[[/codexflow-context]]",
        chunk.id, chunk.source, chunk.content
    )
}

fn rendered_chunk_len(chunk: &ContextChunk) -> usize {
    render_chunk(chunk).len()
}

fn render_selected(chunks: &[ContextChunk], retained: &[bool]) -> String {
    chunks
        .iter()
        .zip(retained)
        .filter(|(_, keep)| **keep)
        .map(|(chunk, _)| render_chunk(chunk))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn rendered_selected_len(chunks: &[ContextChunk], retained: &[bool]) -> usize {
    let selected = chunks
        .iter()
        .zip(retained)
        .filter(|(_, keep)| **keep)
        .map(|(chunk, _)| rendered_chunk_len(chunk))
        .collect::<Vec<_>>();
    selected.iter().sum::<usize>()
        + selected.len().saturating_sub(1) * CHUNK_SEPARATOR_BYTES
}

fn fingerprint_chunks(chunks: &[ContextChunk]) -> String {
    let mut hash = FNV_OFFSET_BASIS;
    hash_bytes(&mut hash, &CONTEXT_FORMAT_VERSION.to_le_bytes());
    for chunk in chunks {
        hash_field(&mut hash, chunk.id.as_bytes());
        hash_field(&mut hash, chunk.source.as_bytes());
        hash_field(&mut hash, chunk.content.as_bytes());
        hash_bytes(&mut hash, &[chunk.priority]);
        let retention = match chunk.retention {
            ContextRetention::Compressible => 0,
            ContextRetention::Pinned => 1,
        };
        hash_bytes(&mut hash, &[retention]);
    }
    format!("{hash:016x}")
}

fn hash_field(hash: &mut u64, value: &[u8]) {
    hash_bytes(hash, &(value.len() as u64).to_le_bytes());
    hash_bytes(hash, value);
}

fn hash_bytes(hash: &mut u64, value: &[u8]) {
    for byte in value {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(FNV_PRIME);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(id: &str, content: &str, priority: u8) -> ContextChunk {
        ContextChunk::new(id, "test", content, priority).unwrap()
    }

    #[test]
    fn compression_is_deterministic_and_budget_bounded() {
        let chunks = vec![
            chunk("a", "alpha", 1),
            chunk("b", "beta", 10),
            chunk("c", "gamma", 5),
        ];
        let single_b_budget = rendered_chunk_len(&chunks[1]);
        let first =
            compress_context(chunks.clone(), CompressionPolicy::new(single_b_budget)).unwrap();
        let second = compress_context(chunks, CompressionPolicy::new(single_b_budget)).unwrap();

        assert_eq!(first, second);
        assert_eq!(first.retained_ids(), &["b".to_string()]);
        assert!(first.stats().rendered_bytes <= single_b_budget);
    }

    #[test]
    fn retained_chunks_render_in_original_order() {
        let chunks = vec![
            chunk("a", "alpha", 5),
            chunk("b", "beta", 1),
            chunk("c", "gamma", 9),
        ];
        let budget = rendered_chunk_len(&chunks[0])
            + rendered_chunk_len(&chunks[2])
            + CHUNK_SEPARATOR_BYTES;
        let compressed = compress_context(chunks, CompressionPolicy::new(budget)).unwrap();

        assert_eq!(
            compressed.retained_ids(),
            &["a".to_string(), "c".to_string()]
        );
        assert!(
            compressed.rendered().find("id=a").unwrap()
                < compressed.rendered().find("id=c").unwrap()
        );
    }

    #[test]
    fn pinned_chunks_are_never_dropped_for_higher_priority_optional_context() {
        let pinned = chunk("system", "required invariant", 1).pinned();
        let optional = chunk("tool", "high priority tool output", 255);
        let budget = rendered_chunk_len(&pinned);
        let compressed = compress_context(
            vec![pinned.clone(), optional],
            CompressionPolicy::new(budget),
        )
        .unwrap();

        assert_eq!(compressed.retained_ids(), &["system".to_string()]);
        assert!(compressed.rendered().contains("required invariant"));
        assert_eq!(compressed.rehydrate(["system"].into_iter()).unwrap(), vec![pinned]);
    }

    #[test]
    fn pinned_context_over_budget_fails_instead_of_violating_invariant() {
        let pinned = chunk("system", "required invariant", 1).pinned();
        let required = rendered_chunk_len(&pinned);
        let error = compress_context(
            vec![pinned],
            CompressionPolicy::new(required.saturating_sub(1)),
        )
        .unwrap_err();

        assert_eq!(
            error,
            ContextCompressionError::PinnedContextExceedsBudget {
                required,
                budget: required.saturating_sub(1),
            }
        );
    }

    #[test]
    fn exact_rehydration_round_trips_omitted_context() {
        let chunks = vec![
            chunk("system", "never drop", 255).pinned(),
            chunk("tool", "large tool output", 1),
        ];
        let compressed = compress_context(
            chunks.clone(),
            CompressionPolicy::new(rendered_chunk_len(&chunks[0])),
        )
        .unwrap();

        assert_eq!(compressed.omitted_ids(), &["tool".to_string()]);
        assert_eq!(compressed.rehydrate_all(), chunks);
        assert_eq!(
            compressed.rehydrate(["tool"].into_iter()).unwrap(),
            vec![chunk("tool", "large tool output", 1)]
        );
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let error = compress_context(
            vec![chunk("same", "one", 1), chunk("same", "two", 2)],
            CompressionPolicy::new(1_000),
        )
        .unwrap_err();

        assert_eq!(
            error,
            ContextCompressionError::DuplicateChunkId("same".to_string())
        );
    }

    #[test]
    fn zero_budget_keeps_optional_archive_reversible() {
        let chunks = vec![chunk("a", "alpha", 1)];
        let compressed = compress_context(chunks.clone(), CompressionPolicy::new(0)).unwrap();

        assert!(compressed.rendered().is_empty());
        assert_eq!(compressed.retained_ids(), &[]);
        assert_eq!(compressed.omitted_ids(), &["a".to_string()]);
        assert_eq!(compressed.rehydrate_all(), chunks);
    }

    #[test]
    fn fingerprint_changes_when_content_or_retention_changes() {
        let first = compress_context(
            vec![chunk("a", "alpha", 1)],
            CompressionPolicy::new(1_000),
        )
        .unwrap();
        let second = compress_context(
            vec![chunk("a", "beta", 1)],
            CompressionPolicy::new(1_000),
        )
        .unwrap();
        let third = compress_context(
            vec![chunk("a", "alpha", 1).pinned()],
            CompressionPolicy::new(1_000),
        )
        .unwrap();

        assert_ne!(first.fingerprint(), second.fingerprint());
        assert_ne!(first.fingerprint(), third.fingerprint());
    }
}
