use crate::RuntimeModelId;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelRole {
    Tiny,
    Default,
    Planner,
    Slow,
    Critic,
    Test,
    Vision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningLevel {
    Minimal,
    Low,
    Medium,
    High,
    ExtraHigh,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleModelTarget {
    pub model: RuntimeModelId,
    pub reasoning: Option<ReasoningLevel>,
}

impl RoleModelTarget {
    pub fn new(model: RuntimeModelId) -> Self {
        Self {
            model,
            reasoning: None,
        }
    }

    pub fn with_reasoning(mut self, reasoning: ReasoningLevel) -> Self {
        self.reasoning = Some(reasoning);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleRoute {
    pub primary: RoleModelTarget,
    #[serde(default)]
    pub escalation: Vec<RoleModelTarget>,
}

impl RoleRoute {
    pub fn new(primary: RoleModelTarget) -> Self {
        Self {
            primary,
            escalation: Vec::new(),
        }
    }

    pub fn with_escalation(
        mut self,
        targets: impl IntoIterator<Item = RoleModelTarget>,
    ) -> Self {
        self.escalation = targets.into_iter().collect();
        self
    }

    fn target_at(&self, level: usize) -> &RoleModelTarget {
        if level == 0 {
            return &self.primary;
        }
        self.escalation
            .get(level - 1)
            .unwrap_or_else(|| self.escalation.last().unwrap_or(&self.primary))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EscalationPolicy {
    /// Escalate one tier for each N verifier failures. Zero disables this trigger.
    pub failures_per_tier: u32,
    /// Escalate at least one tier when externally estimated confidence is below
    /// this value. Confidence is expressed as 0-100.
    pub minimum_confidence_percent: u8,
    pub escalate_on_ambiguity: bool,
    pub escalate_on_loop: bool,
}

impl Default for EscalationPolicy {
    fn default() -> Self {
        Self {
            failures_per_tier: 2,
            minimum_confidence_percent: 60,
            escalate_on_ambiguity: true,
            escalate_on_loop: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RoutingSignals {
    pub verifier_failures: u32,
    pub confidence_percent: Option<u8>,
    pub ambiguity_detected: bool,
    pub loop_detected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutingReason {
    pub code: &'static str,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleRoutingDecision {
    pub role: ModelRole,
    pub escalation_level: usize,
    pub target: RoleModelTarget,
    pub reasons: Vec<RoutingReason>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RoleRoutingError {
    #[error("no model route configured for role {0:?}")]
    MissingRole(ModelRole),
    #[error("confidence percent must be between 0 and 100, found {0}")]
    InvalidConfidence(u8),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RoleRouter {
    routes: BTreeMap<ModelRole, RoleRoute>,
}

impl RoleRouter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, role: ModelRole, route: RoleRoute) -> Option<RoleRoute> {
        self.routes.insert(role, route)
    }

    pub fn route(
        &self,
        role: ModelRole,
        signals: RoutingSignals,
        policy: EscalationPolicy,
    ) -> Result<RoleRoutingDecision, RoleRoutingError> {
        if let Some(confidence) = signals.confidence_percent
            && confidence > 100
        {
            return Err(RoleRoutingError::InvalidConfidence(confidence));
        }
        let route = self
            .routes
            .get(&role)
            .ok_or(RoleRoutingError::MissingRole(role))?;
        let mut reasons = Vec::new();
        let mut level = 0usize;

        if policy.failures_per_tier > 0 && signals.verifier_failures > 0 {
            let failure_level = signals
                .verifier_failures
                .div_ceil(policy.failures_per_tier) as usize;
            level = level.max(failure_level);
            reasons.push(RoutingReason {
                code: "verifier_failures",
                detail: format!(
                    "{} verifier failure(s) requested escalation level {}",
                    signals.verifier_failures, failure_level
                ),
            });
        }

        if let Some(confidence) = signals.confidence_percent
            && confidence < policy.minimum_confidence_percent
        {
            level = level.max(1);
            reasons.push(RoutingReason {
                code: "low_confidence",
                detail: format!(
                    "confidence {confidence}% is below policy minimum {}%",
                    policy.minimum_confidence_percent
                ),
            });
        }

        if signals.ambiguity_detected && policy.escalate_on_ambiguity {
            level = level.max(1);
            reasons.push(RoutingReason {
                code: "ambiguity",
                detail: "external ambiguity detector requested escalation".to_string(),
            });
        }

        if signals.loop_detected && policy.escalate_on_loop {
            level = level.max(1);
            reasons.push(RoutingReason {
                code: "loop",
                detail: "workflow loop detector requested escalation".to_string(),
            });
        }

        let max_level = route.escalation.len();
        let escalation_level = level.min(max_level);
        if level > max_level {
            reasons.push(RoutingReason {
                code: "escalation_capped",
                detail: format!(
                    "requested escalation level {level} capped at configured maximum {max_level}"
                ),
            });
        }

        Ok(RoleRoutingDecision {
            role,
            escalation_level,
            target: route.target_at(escalation_level).clone(),
            reasons,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProviderId;

    fn model(provider: ProviderId, name: &str) -> RoleModelTarget {
        RoleModelTarget::new(RuntimeModelId::new(provider, name).unwrap())
    }

    fn router() -> RoleRouter {
        let mut router = RoleRouter::new();
        router.insert(
            ModelRole::Planner,
            RoleRoute::new(model(ProviderId::Cursor, "qwen-local"))
                .with_escalation([
                    model(ProviderId::Cursor, "qwen-large")
                        .with_reasoning(ReasoningLevel::High),
                    model(ProviderId::OpenAi, "gpt-5.6")
                        .with_reasoning(ReasoningLevel::ExtraHigh),
                ]),
        );
        router
    }

    #[test]
    fn no_signal_uses_role_primary() {
        let decision = router()
            .route(
                ModelRole::Planner,
                RoutingSignals::default(),
                EscalationPolicy::default(),
            )
            .unwrap();

        assert_eq!(decision.escalation_level, 0);
        assert_eq!(decision.target.model.provider, ProviderId::Cursor);
        assert_eq!(decision.target.model.model, "qwen-local");
        assert!(decision.reasons.is_empty());
    }

    #[test]
    fn low_confidence_escalates_one_tier() {
        let decision = router()
            .route(
                ModelRole::Planner,
                RoutingSignals {
                    confidence_percent: Some(40),
                    ..Default::default()
                },
                EscalationPolicy::default(),
            )
            .unwrap();

        assert_eq!(decision.escalation_level, 1);
        assert_eq!(decision.target.model.model, "qwen-large");
        assert_eq!(decision.reasons[0].code, "low_confidence");
    }

    #[test]
    fn repeated_verifier_failures_step_through_stronger_targets() {
        let decision = router()
            .route(
                ModelRole::Planner,
                RoutingSignals {
                    verifier_failures: 3,
                    ..Default::default()
                },
                EscalationPolicy::default(),
            )
            .unwrap();

        assert_eq!(decision.escalation_level, 2);
        assert_eq!(decision.target.model.provider, ProviderId::OpenAi);
        assert_eq!(decision.target.model.model, "gpt-5.6");
    }

    #[test]
    fn escalation_is_capped_by_explicit_route() {
        let decision = router()
            .route(
                ModelRole::Planner,
                RoutingSignals {
                    verifier_failures: 99,
                    ..Default::default()
                },
                EscalationPolicy::default(),
            )
            .unwrap();

        assert_eq!(decision.escalation_level, 2);
        assert!(decision
            .reasons
            .iter()
            .any(|reason| reason.code == "escalation_capped"));
    }

    #[test]
    fn missing_role_is_not_silently_mapped_to_default() {
        assert_eq!(
            router()
                .route(
                    ModelRole::Vision,
                    RoutingSignals::default(),
                    EscalationPolicy::default(),
                )
                .unwrap_err(),
            RoleRoutingError::MissingRole(ModelRole::Vision)
        );
    }
}
