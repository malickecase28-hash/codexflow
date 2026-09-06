use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCapability {
    Read,
    Write,
    Execute,
    Network,
    Browser,
    Mcp,
    Custom(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCost {
    Cheap,
    Standard,
    Expensive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolTier {
    Hot,
    Cold,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDescriptor {
    pub name: String,
    pub capabilities: BTreeSet<ToolCapability>,
    pub cost: ToolCost,
    pub tier: ToolTier,
}

impl ToolDescriptor {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            capabilities: BTreeSet::new(),
            cost: ToolCost::Standard,
            tier: ToolTier::Hot,
        }
    }

    pub fn with_capabilities(
        mut self,
        capabilities: impl IntoIterator<Item = ToolCapability>,
    ) -> Self {
        self.capabilities = capabilities.into_iter().collect();
        self
    }

    pub fn with_cost(mut self, cost: ToolCost) -> Self {
        self.cost = cost;
        self
    }

    pub fn with_tier(mut self, tier: ToolTier) -> Self {
        self.tier = tier;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolPolicyRequest {
    pub role: String,
    pub tool: ToolDescriptor,
    pub working_directory: Option<PathBuf>,
    #[serde(default)]
    pub input: Value,
}

impl ToolPolicyRequest {
    pub fn new(role: impl Into<String>, tool: ToolDescriptor) -> Self {
        Self {
            role: role.into(),
            tool,
            working_directory: None,
            input: Value::Null,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolPolicyDecision {
    Abstain,
    Allow,
    Confirm { reason: String },
    Deny { reason: String },
}

pub trait ToolPolicy: Send + Sync {
    fn policy_id(&self) -> &str;
    fn evaluate(&self, request: &ToolPolicyRequest) -> ToolPolicyDecision;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultToolDecision {
    Allow,
    Confirm,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectiveToolDecision {
    Allow,
    Confirm { reason: String },
    Deny { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolPolicyEvidence {
    pub policy: String,
    pub decision: ToolPolicyDecision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolPolicyVerdict {
    pub decision: EffectiveToolDecision,
    pub evidence: Vec<ToolPolicyEvidence>,
}

pub struct ToolPolicyChain {
    policies: Vec<Box<dyn ToolPolicy>>,
    default: DefaultToolDecision,
}

impl ToolPolicyChain {
    pub fn new(default: DefaultToolDecision) -> Self {
        Self {
            policies: Vec::new(),
            default,
        }
    }

    pub fn fail_closed() -> Self {
        Self::new(DefaultToolDecision::Deny)
    }

    pub fn push<P>(&mut self, policy: P)
    where
        P: ToolPolicy + 'static,
    {
        self.policies.push(Box::new(policy));
    }

    /// Evaluate every policy and compose them deterministically.
    ///
    /// Deny dominates Confirm, Confirm dominates Allow, and Abstain contributes
    /// evidence without changing the effective decision. If every policy
    /// abstains, the chain's explicit default is used.
    pub fn evaluate(&self, request: &ToolPolicyRequest) -> ToolPolicyVerdict {
        let mut evidence = Vec::with_capacity(self.policies.len());
        let mut first_deny = None;
        let mut first_confirm = None;
        let mut saw_allow = false;

        for policy in &self.policies {
            let decision = policy.evaluate(request);
            match &decision {
                ToolPolicyDecision::Deny { reason } if first_deny.is_none() => {
                    first_deny = Some(reason.clone());
                }
                ToolPolicyDecision::Confirm { reason } if first_confirm.is_none() => {
                    first_confirm = Some(reason.clone());
                }
                ToolPolicyDecision::Allow => saw_allow = true,
                ToolPolicyDecision::Abstain
                | ToolPolicyDecision::Confirm { .. }
                | ToolPolicyDecision::Deny { .. } => {}
            }
            evidence.push(ToolPolicyEvidence {
                policy: policy.policy_id().to_string(),
                decision,
            });
        }

        let decision = if let Some(reason) = first_deny {
            EffectiveToolDecision::Deny { reason }
        } else if let Some(reason) = first_confirm {
            EffectiveToolDecision::Confirm { reason }
        } else if saw_allow {
            EffectiveToolDecision::Allow
        } else {
            match self.default {
                DefaultToolDecision::Allow => EffectiveToolDecision::Allow,
                DefaultToolDecision::Confirm => EffectiveToolDecision::Confirm {
                    reason: "tool policy requires confirmation by default".to_string(),
                },
                DefaultToolDecision::Deny => EffectiveToolDecision::Deny {
                    reason: "tool policy denied by default".to_string(),
                },
            }
        };

        ToolPolicyVerdict { decision, evidence }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSelectionContext {
    pub role: String,
    pub include_cold_tools: bool,
    pub max_cost: ToolCost,
}

impl ToolSelectionContext {
    pub fn new(role: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            include_cold_tools: false,
            max_cost: ToolCost::Standard,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSelectionPlan {
    pub exposed: Vec<ToolDescriptor>,
    pub deferred: Vec<ToolDescriptor>,
    pub denied: Vec<ToolDescriptor>,
}

/// Apply progressive disclosure before a model sees tool schemas.
///
/// This is intentionally not an MCP registry. The caller supplies the tools it
/// already owns; the policy spine only decides which of those tools are exposed
/// for the current role/workflow state.
pub fn select_tools(
    tools: impl IntoIterator<Item = ToolDescriptor>,
    context: &ToolSelectionContext,
    policies: &ToolPolicyChain,
) -> ToolSelectionPlan {
    let mut exposed = Vec::new();
    let mut deferred = Vec::new();
    let mut denied = Vec::new();

    for tool in tools {
        let request = ToolPolicyRequest::new(context.role.clone(), tool.clone());
        let verdict = policies.evaluate(&request);
        if matches!(verdict.decision, EffectiveToolDecision::Deny { .. }) {
            denied.push(tool);
            continue;
        }

        if tool.cost > context.max_cost
            || matches!(tool.tier, ToolTier::Cold) && !context.include_cold_tools
        {
            deferred.push(tool);
        } else {
            exposed.push(tool);
        }
    }

    ToolSelectionPlan {
        exposed,
        deferred,
        denied,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StaticPolicy {
        id: &'static str,
        decision: ToolPolicyDecision,
    }

    impl ToolPolicy for StaticPolicy {
        fn policy_id(&self) -> &str {
            self.id
        }

        fn evaluate(&self, _request: &ToolPolicyRequest) -> ToolPolicyDecision {
            self.decision.clone()
        }
    }

    struct WorkerNoWrite;

    impl ToolPolicy for WorkerNoWrite {
        fn policy_id(&self) -> &str {
            "worker-no-write"
        }

        fn evaluate(&self, request: &ToolPolicyRequest) -> ToolPolicyDecision {
            if request.role == "worker"
                && request.tool.capabilities.contains(&ToolCapability::Write)
            {
                ToolPolicyDecision::Deny {
                    reason: "worker role cannot write".to_string(),
                }
            } else {
                ToolPolicyDecision::Allow
            }
        }
    }

    #[test]
    fn deny_dominates_confirmation_and_allow() {
        let mut chain = ToolPolicyChain::fail_closed();
        chain.push(StaticPolicy {
            id: "allow",
            decision: ToolPolicyDecision::Allow,
        });
        chain.push(StaticPolicy {
            id: "confirm",
            decision: ToolPolicyDecision::Confirm {
                reason: "confirm network".to_string(),
            },
        });
        chain.push(StaticPolicy {
            id: "deny",
            decision: ToolPolicyDecision::Deny {
                reason: "hard policy".to_string(),
            },
        });

        let verdict = chain.evaluate(&ToolPolicyRequest::new(
            "worker",
            ToolDescriptor::new("bash"),
        ));

        assert_eq!(
            verdict.decision,
            EffectiveToolDecision::Deny {
                reason: "hard policy".to_string()
            }
        );
        assert_eq!(verdict.evidence.len(), 3);
    }

    #[test]
    fn all_abstain_uses_explicit_fail_closed_default() {
        let mut chain = ToolPolicyChain::fail_closed();
        chain.push(StaticPolicy {
            id: "opa-unconfigured",
            decision: ToolPolicyDecision::Abstain,
        });

        let verdict = chain.evaluate(&ToolPolicyRequest::new(
            "worker",
            ToolDescriptor::new("bash"),
        ));

        assert!(matches!(
            verdict.decision,
            EffectiveToolDecision::Deny { .. }
        ));
    }

    #[test]
    fn role_policy_can_forbid_write_tools() {
        let mut chain = ToolPolicyChain::new(DefaultToolDecision::Allow);
        chain.push(WorkerNoWrite);
        let edit = ToolDescriptor::new("edit")
            .with_capabilities([ToolCapability::Read, ToolCapability::Write]);

        let verdict = chain.evaluate(&ToolPolicyRequest::new("worker", edit));

        assert_eq!(
            verdict.decision,
            EffectiveToolDecision::Deny {
                reason: "worker role cannot write".to_string()
            }
        );
    }

    #[test]
    fn progressive_disclosure_defers_cold_and_expensive_tools() {
        let policies = ToolPolicyChain::new(DefaultToolDecision::Allow);
        let hot = ToolDescriptor::new("read").with_cost(ToolCost::Cheap);
        let cold = ToolDescriptor::new("toolhive-discover")
            .with_tier(ToolTier::Cold)
            .with_cost(ToolCost::Cheap);
        let expensive = ToolDescriptor::new("browser")
            .with_cost(ToolCost::Expensive)
            .with_capabilities([ToolCapability::Browser]);

        let plan = select_tools(
            vec![hot.clone(), cold.clone(), expensive.clone()],
            &ToolSelectionContext::new("default"),
            &policies,
        );

        assert_eq!(plan.exposed, vec![hot]);
        assert_eq!(plan.deferred, vec![cold, expensive]);
        assert!(plan.denied.is_empty());
    }

    #[test]
    fn denied_tools_are_never_exposed_even_when_hot() {
        let mut policies = ToolPolicyChain::new(DefaultToolDecision::Allow);
        policies.push(WorkerNoWrite);
        let edit = ToolDescriptor::new("edit").with_capabilities([ToolCapability::Write]);

        let plan = select_tools(
            vec![edit.clone()],
            &ToolSelectionContext::new("worker"),
            &policies,
        );

        assert!(plan.exposed.is_empty());
        assert!(plan.deferred.is_empty());
        assert_eq!(plan.denied, vec![edit]);
    }
}
