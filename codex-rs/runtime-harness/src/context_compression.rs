use std::collections::HashMap;
use std::fmt;

const CONTEXT_FORMAT_VERSION: u32 = 1;
const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextChunk {
    pub id: String,
    pub source: String,
    pub content: String,
    pub priority: u8,
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
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompressionPolicy {
    /// Maximum number of UTF-8 bytes in the rendered compact context.
    ///
    /// Compression is whole-chunk only: a selected chunk is never truncated.
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
}

impl fmt::Display for ContextCompressionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyChunkId => f.write_str("context chunk id cannot be empty"),
            Self::DuplicateChunkId(id) => write!(f, "duplicate context chunk id '{id}'"),
            Self::UnknownChunkId(id) => write!(f, "unknown context chunk id '{id}'"),
        }
    }
}

impl std::error::Error for ContextCompressionError {}

/// Deterministically compact context while retaining an exact in-memory archive.
///
/// Chunks are selected by descending priority, with original order as the
/// deterministic tie-breaker. Selected chunks are rendered back in original
/// order so surrounding context remains readable. Omitted chunks remain
/// addressable by id through [`CompressedContext::rehydrate`] and the complete
/// original bundle can be reconstructed with [`CompressedContext::rehydrate_all`].
pub fn compress_context(
    chunks: Vec<ContextChunk>,
    policy: CompressionPolicy,
) -> Result<CompressedContext, ContextCompressionError> {
    validate_chunks(&chunks)?;

    let input_bytes = chunks.iter().map(rendered_chunk_len).sum();
    let mut ranked: Vec<usize> = (0..chunks.len()).collect();
    ranked.sort_by_key(|&index| (std::cmp::Reverse(chunks[index].priority), index));

    let mut retained = vec![false; chunks.len()];
    let mut used_bytes = 0usize;

    for index in ranked {
        let separator_bytes = if used_bytes > 0 { 2 } else { 0 };
        let candidate_bytes = rendered_chunk_len(&chunks[index]) + separator_bytes;
        if used_bytes.saturating_add(candidate_bytes) <= policy.max_bytes {
            retained[index] = true;
            used_bytes += candidate_bytes;
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

    let rendered = chunks
        .iter()
        .zip(&retained)
        .filter(|(_, keep)| **keep)
        .map(|(chunk, _)| render_chunk(chunk))
        .collect::<Vec<_>>()
        .join("\n\n");

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

fn fingerprint_chunks(chunks: &[ContextChunk]) -> String {
    let mut hash = FNV_OFFSET_BASIS;
    hash_bytes(&mut hash, &CONTEXT_FORMAT_VERSION.to_le_bytes());
    for chunk in chunks {
        hash_field(&mut hash, chunk.id.as_bytes());
        hash_field(&mut hash, chunk.source.as_bytes());
        hash_field(&mut hash, chunk.content.as_bytes());
        hash_bytes(&mut hash, &[chunk.priority]);
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
        let first = compress_context(chunks.clone(), CompressionPolicy::new(single_b_budget)).unwrap();
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
        let budget = rendered_chunk_len(&chunks[0]) + rendered_chunk_len(&chunks[2]) + 2;
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
    fn exact_rehydration_round_trips_omitted_context() {
        let chunks = vec![
            chunk("system", "never drop", 255),
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
    fn zero_budget_keeps_archive_reversible() {
        let chunks = vec![chunk("a", "alpha", 1)];
        let compressed = compress_context(chunks.clone(), CompressionPolicy::new(0)).unwrap();

        assert!(compressed.rendered().is_empty());
        assert_eq!(compressed.retained_ids(), &[]);
        assert_eq!(compressed.omitted_ids(), &["a".to_string()]);
        assert_eq!(compressed.rehydrate_all(), chunks);
    }

    #[test]
    fn fingerprint_changes_when_content_changes() {
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

        assert_ne!(first.fingerprint(), second.fingerprint());
    }
}
