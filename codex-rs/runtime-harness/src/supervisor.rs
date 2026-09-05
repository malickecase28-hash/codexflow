use crate::types::ProviderId;
use crate::types::RuntimeEvent;
use crate::types::RuntimeEventSink;
use crate::types::RuntimeSessionId;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupervisedSession {
    pub generation: u64,
    pub provider: ProviderId,
    pub session_id: RuntimeSessionId,
}

#[derive(Clone)]
pub struct RuntimeSessionSupervisor {
    generation: Arc<AtomicU64>,
}

impl Default for RuntimeSessionSupervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeSessionSupervisor {
    pub fn new() -> Self {
        Self {
            generation: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Start ownership of a runtime session. Every transition advances a generation,
    /// allowing delayed events from a dead/replaced provider process to be discarded.
    pub fn begin(&self, provider: ProviderId, session_id: RuntimeSessionId) -> SupervisedSession {
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        SupervisedSession {
            generation,
            provider,
            session_id,
        }
    }

    /// Invalidate every event sink created for the current generation.
    pub fn invalidate(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::AcqRel) + 1
    }

    pub fn current_generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    pub fn guarded_sink(
        &self,
        session: &SupervisedSession,
        downstream: Arc<dyn RuntimeEventSink>,
    ) -> Arc<dyn RuntimeEventSink> {
        Arc::new(GenerationGuardedSink {
            active_generation: Arc::clone(&self.generation),
            generation: session.generation,
            downstream,
        })
    }
}

struct GenerationGuardedSink {
    active_generation: Arc<AtomicU64>,
    generation: u64,
    downstream: Arc<dyn RuntimeEventSink>,
}

impl RuntimeEventSink for GenerationGuardedSink {
    fn emit(&self, event: RuntimeEvent) {
        if self.active_generation.load(Ordering::Acquire) == self.generation {
            self.downstream.emit(event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Collector(Mutex<Vec<RuntimeEvent>>);

    impl RuntimeEventSink for Collector {
        fn emit(&self, event: RuntimeEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    #[test]
    fn stale_provider_events_are_discarded_after_replacement() {
        let supervisor = RuntimeSessionSupervisor::new();
        let collector = Arc::new(Collector::default());
        let first = supervisor.begin(ProviderId::Cursor, RuntimeSessionId("s1".into()));
        let first_sink = supervisor.guarded_sink(&first, collector.clone());
        first_sink.emit(RuntimeEvent::AgentMessageChunk {
            text: "accepted".into(),
        });

        let second = supervisor.begin(ProviderId::Cursor, RuntimeSessionId("s2".into()));
        let second_sink = supervisor.guarded_sink(&second, collector.clone());
        first_sink.emit(RuntimeEvent::AgentMessageChunk {
            text: "stale".into(),
        });
        second_sink.emit(RuntimeEvent::AgentMessageChunk {
            text: "current".into(),
        });

        let events = collector.0.lock().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0],
            RuntimeEvent::AgentMessageChunk {
                text: "accepted".into()
            }
        );
        assert_eq!(
            events[1],
            RuntimeEvent::AgentMessageChunk {
                text: "current".into()
            }
        );
    }

    #[test]
    fn invalidation_silences_current_provider_sink() {
        let supervisor = RuntimeSessionSupervisor::new();
        let collector = Arc::new(Collector::default());
        let session = supervisor.begin(ProviderId::Cursor, RuntimeSessionId("s1".into()));
        let sink = supervisor.guarded_sink(&session, collector.clone());
        supervisor.invalidate();
        sink.emit(RuntimeEvent::ProviderError {
            message: "late crash".into(),
        });
        assert!(collector.0.lock().unwrap().is_empty());
    }
}
