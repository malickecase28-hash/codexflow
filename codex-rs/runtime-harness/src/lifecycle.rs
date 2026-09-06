use crate::CompressedContext;
use crate::CompressionPolicy;
use crate::ContextChunk;
use crate::ContextCompressionError;
use crate::ToolDescriptor;
use crate::ToolPolicyChain;
use crate::ToolSelectionContext;
use crate::ToolSelectionPlan;
use crate::compress_context;
use crate::select_tools;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeforeModelContext {
    pub chunks: Vec<ContextChunk>,
}

impl BeforeModelContext {
    pub fn new(chunks: Vec<ContextChunk>) -> Self {
        Self { chunks }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecyclePolicyError {
    pub policy: String,
    pub message: String,
}

impl LifecyclePolicyError {
    pub fn new(policy: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            policy: policy.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for LifecyclePolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "lifecycle policy {} failed: {}", self.policy, self.message)
    }
}

impl std::error::Error for LifecyclePolicyError {}

/// Ordered policy seam executed before context is rendered for a model.
///
/// Implementations can inject task state or retrieved memories, deduplicate or
/// replace tool-result chunks, and mark invariants as pinned. The policy bus
/// performs final deterministic compression after every policy has run.
pub trait BeforeModelPolicy: Send + Sync {
    fn policy_id(&self) -> &str;

    fn apply(&self, context: &mut BeforeModelContext) -> Result<(), LifecyclePolicyError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecyclePolicyEvidence {
    pub policy: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeforeModelOutcome {
    pub context: CompressedContext,
    pub evidence: Vec<LifecyclePolicyEvidence>,
}

#[derive(Debug, thiserror::Error)]
pub enum BeforeModelError {
    #[error(transparent)]
    Policy(#[from] LifecyclePolicyError),
    #[error(transparent)]
    Compression(#[from] ContextCompressionError),
}

pub struct LifecyclePolicyBus {
    before_model: Vec<Box<dyn BeforeModelPolicy>>,
    compression: CompressionPolicy,
    tool_policies: ToolPolicyChain,
}

impl LifecyclePolicyBus {
    pub fn new(compression: CompressionPolicy, tool_policies: ToolPolicyChain) -> Self {
        Self {
            before_model: Vec::new(),
            compression,
            tool_policies,
        }
    }

    pub fn push_before_model<P>(&mut self, policy: P)
    where
        P: BeforeModelPolicy + 'static,
    {
        self.before_model.push(Box::new(policy));
    }

    /// Apply ordered context policies and then enforce the configured context
    /// budget. Pinned chunks are guaranteed by `compress_context` or this call
    /// fails before any model invocation can occur.
    pub fn before_model(
        &self,
        chunks: Vec<ContextChunk>,
    ) -> Result<BeforeModelOutcome, BeforeModelError> {
        let mut context = BeforeModelContext::new(chunks);
        let mut evidence = Vec::with_capacity(self.before_model.len());
        for policy in &self.before_model {
            policy.apply(&mut context)?;
            evidence.push(LifecyclePolicyEvidence {
                policy: policy.policy_id().to_string(),
            });
        }

        Ok(BeforeModelOutcome {
            context: compress_context(context.chunks, self.compression)?,
            evidence,
        })
    }

    /// Apply hard policy and progressive disclosure before tool schemas are
    /// exposed to a model. Denied tools are never returned in `exposed` or
    /// `deferred`; cold/expensive tools remain discoverable without consuming
    /// the hot-tool schema budget.
    pub fn before_tool_selection(
        &self,
        tools: impl IntoIterator<Item = ToolDescriptor>,
        context: &ToolSelectionContext,
    ) -> ToolSelectionPlan {
        select_tools(tools, context, &self.tool_policies)
    }

    pub fn tool_policies(&self) -> &ToolPolicyChain {
        &self.tool_policies
    }

    pub fn tool_policies_mut(&mut self) -> &mut ToolPolicyChain {
        &mut self.tool_policies
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DefaultToolDecision;
    use crate::ToolCapability;
    use crate::ToolCost;
    use crate::ToolPolicy;
    use crate::ToolPolicyDecision;
    use crate::ToolPolicyRequest;
    use crate::ToolTier;

    struct InjectPinnedTaskState;

    impl BeforeModelPolicy for InjectPinnedTaskState {
        fn policy_id(&self) -> &str {
            "task-state"
        }

        fn apply(&self, context: &mut BeforeModelContext) -> Result<(), LifecyclePolicyError> {
            context.chunks.push(
                ContextChunk::new("task-state", "runtime", "must finish tests", 255)
                    .unwrap()
                    .pinned(),
            );
            Ok(())
        }
    }

    struct DenyWorkerWrite;

    impl ToolPolicy for DenyWorkerWrite {
        fn policy_id(&self) -> &str {
            "deny-worker-write"
        }

        fn evaluate(&self, request: &ToolPolicyRequest) -> ToolPolicyDecision {
            if request.role == "worker"
                && request.tool.capabilities.contains(&ToolCapability::Write)
            {
                ToolPolicyDecision::Deny {
                    reason: "worker cannot write".to_string(),
                }
            } else {
                ToolPolicyDecision::Allow
            }
        }
    }

    fn chunk(id: &str, content: &str, priority: u8) -> ContextChunk {
        ContextChunk::new(id, "test", content, priority).unwrap()
    }

    #[test]
    fn before_model_applies_policies_before_budget_enforcement() {
        let task = chunk("task-state", "must finish tests", 255).pinned();
        let budget = format!(
            "[[codexflow-context id={} source={}]]\n{}\n[[/codexflow-context]]",
            task.id, task.source, task.content
        )
        .len();
        let mut bus = LifecyclePolicyBus::new(
            CompressionPolicy::new(budget),
            ToolPolicyChain::new(DefaultToolDecision::Allow),
        );
        bus.push_before_model(InjectPinnedTaskState);

        let outcome = bus
            .before_model(vec![chunk("tool-output", "optional", 1)])
            .unwrap();

        assert_eq!(outcome.evidence.len(), 1);
        assert_eq!(outcome.evidence[0].policy, "task-state");
        assert_eq!(outcome.context.retained_ids(), &["task-state".to_string()]);
        assert_eq!(outcome.context.omitted_ids(), &["tool-output".to_string()]);
    }

    #[test]
    fn before_model_fails_when_policy_injected_invariant_cannot_fit() {
        let mut bus = LifecyclePolicyBus::new(
            CompressionPolicy::new(1),
            ToolPolicyChain::new(DefaultToolDecision::Allow),
        );
        bus.push_before_model(InjectPinnedTaskState);

        assert!(matches!(
            bus.before_model(Vec::new()),
            Err(BeforeModelError::Compression(
                ContextCompressionError::PinnedContextExceedsBudget { .. }
            ))
        ));
    }

    #[test]
    fn before_tool_selection_combines_hard_policy_and_progressive_disclosure() {
        let mut policies = ToolPolicyChain::new(DefaultToolDecision::Allow);
        policies.push(DenyWorkerWrite);
        let bus = LifecyclePolicyBus::new(CompressionPolicy::new(10_000), policies);

        let read = ToolDescriptor::new("read").with_cost(ToolCost::Cheap);
        let write = ToolDescriptor::new("edit").with_capabilities([ToolCapability::Write]);
        let cold = ToolDescriptor::new("toolhive-discover")
            .with_tier(ToolTier::Cold)
            .with_cost(ToolCost::Cheap);

        let plan = bus.before_tool_selection(
            vec![read.clone(), write.clone(), cold.clone()],
            &ToolSelectionContext::new("worker"),
        );

        assert_eq!(plan.exposed, vec![read]);
        assert_eq!(plan.deferred, vec![cold]);
        assert_eq!(plan.denied, vec![write]);
    }
}
