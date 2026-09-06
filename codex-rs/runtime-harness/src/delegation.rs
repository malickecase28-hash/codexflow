use crate::AcceptanceCriterion;
use crate::CompressedContext;
use crate::CompressionPolicy;
use crate::ContextChunk;
use crate::ContextCompressionError;
use crate::EscalationPolicy;
use crate::ModelRole;
use crate::RoleRouter;
use crate::RoleRoutingDecision;
use crate::RoleRoutingError;
use crate::RoutingSignals;
use crate::ToolCost;
use crate::ToolDescriptor;
use crate::ToolPolicyChain;
use crate::ToolSelectionContext;
use crate::ToolSelectionPlan;
use crate::select_tools;
use std::collections::BTreeSet;

#[derive(Debug, Clone)]
pub struct DelegationTask {
    pub id: String,
    pub objective: String,
    pub role: ModelRole,
    pub system_prompt: String,
    /// Purpose-built context only. The parent transcript is intentionally not a
    /// field on this type so callers cannot accidentally inherit it wholesale.
    pub context: Vec<ContextChunk>,
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    pub requested_mcp_servers: Vec<String>,
}

impl DelegationTask {
    pub fn new(
        id: impl Into<String>,
        objective: impl Into<String>,
        role: ModelRole,
        system_prompt: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            objective: objective.into(),
            role,
            system_prompt: system_prompt.into(),
            context: Vec::new(),
            acceptance_criteria: Vec::new(),
            requested_mcp_servers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DelegationPolicy {
    pub max_context_bytes: usize,
    pub max_exposed_tools: usize,
    pub max_tool_cost: ToolCost,
    pub include_cold_tools: bool,
    pub allowed_mcp_servers: BTreeSet<String>,
    /// Tool names capable of recursively delegating. These are always removed
    /// from a worker plan even if another policy would otherwise allow them.
    pub recursive_delegation_tools: BTreeSet<String>,
}

impl Default for DelegationPolicy {
    fn default() -> Self {
        Self {
            max_context_bytes: 32 * 1024,
            max_exposed_tools: 8,
            max_tool_cost: ToolCost::Standard,
            include_cold_tools: false,
            allowed_mcp_servers: BTreeSet::new(),
            recursive_delegation_tools: ["spawn_agent", "spawn_agents"]
                .into_iter()
                .map(str::to_string)
                .collect(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DelegationPlan {
    pub task_id: String,
    pub objective: String,
    pub system_prompt: String,
    pub model: RoleRoutingDecision,
    pub context: CompressedContext,
    pub tools: ToolSelectionPlan,
    pub mcp_servers: Vec<String>,
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    pub recursive_delegation_allowed: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum DelegationError {
    #[error("delegation task id cannot be empty")]
    EmptyTaskId,
    #[error("delegation objective cannot be empty")]
    EmptyObjective,
    #[error("delegation system prompt cannot be empty")]
    EmptySystemPrompt,
    #[error("delegation task must define at least one acceptance criterion")]
    MissingAcceptanceCriteria,
    #[error("delegation requested MCP server '{0}' that policy did not allow")]
    McpServerNotAllowed(String),
    #[error(transparent)]
    Context(#[from] ContextCompressionError),
    #[error(transparent)]
    Routing(#[from] RoleRoutingError),
}

pub fn prepare_delegation(
    task: DelegationTask,
    available_tools: impl IntoIterator<Item = ToolDescriptor>,
    tool_policies: &ToolPolicyChain,
    role_router: &RoleRouter,
    routing_signals: RoutingSignals,
    escalation_policy: EscalationPolicy,
    delegation_policy: &DelegationPolicy,
) -> Result<DelegationPlan, DelegationError> {
    validate_task(&task, delegation_policy)?;

    let model = role_router.route(task.role, routing_signals, escalation_policy)?;
    let context = crate::compress_context(
        task.context,
        CompressionPolicy::new(delegation_policy.max_context_bytes),
    )?;

    let mut recursive_denied = Vec::new();
    let mut candidate_tools = Vec::new();
    for tool in available_tools {
        if delegation_policy
            .recursive_delegation_tools
            .contains(&tool.name)
        {
            recursive_denied.push(tool);
        } else {
            candidate_tools.push(tool);
        }
    }

    let mut selection_context = ToolSelectionContext::new(model_role_label(task.role));
    selection_context.include_cold_tools = delegation_policy.include_cold_tools;
    selection_context.max_cost = delegation_policy.max_tool_cost;
    let mut tools = select_tools(candidate_tools, &selection_context, tool_policies);
    tools.denied.extend(recursive_denied);

    if tools.exposed.len() > delegation_policy.max_exposed_tools {
        let overflow = tools.exposed.split_off(delegation_policy.max_exposed_tools);
        tools.deferred.extend(overflow);
    }

    Ok(DelegationPlan {
        task_id: task.id,
        objective: task.objective,
        system_prompt: task.system_prompt,
        model,
        context,
        tools,
        mcp_servers: task.requested_mcp_servers,
        acceptance_criteria: task.acceptance_criteria,
        recursive_delegation_allowed: false,
    })
}

fn validate_task(
    task: &DelegationTask,
    policy: &DelegationPolicy,
) -> Result<(), DelegationError> {
    if task.id.trim().is_empty() {
        return Err(DelegationError::EmptyTaskId);
    }
    if task.objective.trim().is_empty() {
        return Err(DelegationError::EmptyObjective);
    }
    if task.system_prompt.trim().is_empty() {
        return Err(DelegationError::EmptySystemPrompt);
    }
    if task.acceptance_criteria.is_empty() {
        return Err(DelegationError::MissingAcceptanceCriteria);
    }
    for server in &task.requested_mcp_servers {
        if !policy.allowed_mcp_servers.contains(server) {
            return Err(DelegationError::McpServerNotAllowed(server.clone()));
        }
    }
    Ok(())
}

fn model_role_label(role: ModelRole) -> &'static str {
    match role {
        ModelRole::Tiny => "tiny",
        ModelRole::Default => "default",
        ModelRole::Planner => "planner",
        ModelRole::Slow => "slow",
        ModelRole::Critic => "critic",
        ModelRole::Test => "test",
        ModelRole::Vision => "vision",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DefaultToolDecision;
    use crate::ProviderId;
    use crate::RoleModelTarget;
    use crate::RoleRoute;
    use crate::RuntimeModelId;
    use crate::ToolCapability;

    fn role_router() -> RoleRouter {
        let mut router = RoleRouter::new();
        router.insert(
            ModelRole::Critic,
            RoleRoute::new(RoleModelTarget::new(
                RuntimeModelId::new(ProviderId::Cursor, "critic-local").unwrap(),
            )),
        );
        router
    }

    fn task() -> DelegationTask {
        let mut task = DelegationTask::new(
            "review-parser",
            "Review parser changes independently",
            ModelRole::Critic,
            "You are an independent code critic. Do not edit source.",
        );
        task.context = vec![
            ContextChunk::new("diff", "git-diff", "parser diff", 10)
                .unwrap()
                .pinned(),
            ContextChunk::new("notes", "supervisor", "implementation notes", 1).unwrap(),
        ];
        task.acceptance_criteria.push(AcceptanceCriterion::pending(
            "review",
            "Report concrete correctness findings",
        ));
        task
    }

    #[test]
    fn worker_plan_never_exposes_recursive_spawn_tools() {
        let tools = vec![
            ToolDescriptor::new("read").with_capabilities([ToolCapability::Read]),
            ToolDescriptor::new("spawn_agent"),
        ];
        let plan = prepare_delegation(
            task(),
            tools,
            &ToolPolicyChain::new(DefaultToolDecision::Allow),
            &role_router(),
            RoutingSignals::default(),
            EscalationPolicy::default(),
            &DelegationPolicy::default(),
        )
        .unwrap();

        assert!(!plan.recursive_delegation_allowed);
        assert_eq!(plan.tools.exposed.len(), 1);
        assert_eq!(plan.tools.exposed[0].name, "read");
        assert!(plan
            .tools
            .denied
            .iter()
            .any(|tool| tool.name == "spawn_agent"));
    }

    #[test]
    fn unapproved_mcp_server_fails_closed() {
        let mut task = task();
        task.requested_mcp_servers.push("serena".to_string());

        assert!(matches!(
            prepare_delegation(
                task,
                Vec::<ToolDescriptor>::new(),
                &ToolPolicyChain::new(DefaultToolDecision::Allow),
                &role_router(),
                RoutingSignals::default(),
                EscalationPolicy::default(),
                &DelegationPolicy::default(),
            ),
            Err(DelegationError::McpServerNotAllowed(server)) if server == "serena"
        ));
    }

    #[test]
    fn approved_mcp_server_is_scoped_to_worker_plan() {
        let mut task = task();
        task.requested_mcp_servers.push("serena".to_string());
        let mut policy = DelegationPolicy::default();
        policy.allowed_mcp_servers.insert("serena".to_string());

        let plan = prepare_delegation(
            task,
            Vec::<ToolDescriptor>::new(),
            &ToolPolicyChain::new(DefaultToolDecision::Allow),
            &role_router(),
            RoutingSignals::default(),
            EscalationPolicy::default(),
            &policy,
        )
        .unwrap();

        assert_eq!(plan.mcp_servers, vec!["serena".to_string()]);
    }

    #[test]
    fn tool_budget_defers_overflow_instead_of_exposing_everything() {
        let tools = (0..5)
            .map(|index| ToolDescriptor::new(format!("tool-{index}")))
            .collect::<Vec<_>>();
        let policy = DelegationPolicy {
            max_exposed_tools: 2,
            ..Default::default()
        };

        let plan = prepare_delegation(
            task(),
            tools,
            &ToolPolicyChain::new(DefaultToolDecision::Allow),
            &role_router(),
            RoutingSignals::default(),
            EscalationPolicy::default(),
            &policy,
        )
        .unwrap();

        assert_eq!(plan.tools.exposed.len(), 2);
        assert_eq!(plan.tools.deferred.len(), 3);
    }

    #[test]
    fn acceptance_criteria_are_mandatory() {
        let mut task = task();
        task.acceptance_criteria.clear();

        assert!(matches!(
            prepare_delegation(
                task,
                Vec::<ToolDescriptor>::new(),
                &ToolPolicyChain::new(DefaultToolDecision::Allow),
                &role_router(),
                RoutingSignals::default(),
                EscalationPolicy::default(),
                &DelegationPolicy::default(),
            ),
            Err(DelegationError::MissingAcceptanceCriteria)
        ));
    }
}
