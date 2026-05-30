use kubio_core::{Decision, DecisionReason, RouteState};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyDecision {
    pub decision: Decision,
    pub reasons: Vec<DecisionReason>,
    pub route_state: RouteState,
    pub score: i16,
}

impl PolicyDecision {
    pub fn new(
        decision: Decision,
        mut reasons: Vec<DecisionReason>,
        route_state: RouteState,
        score: i16,
    ) -> Self {
        if reasons.is_empty() {
            reasons.push(DecisionReason::PolicyError);
        }
        Self {
            decision,
            reasons,
            route_state,
            score,
        }
    }

    pub fn protected(&self) -> bool {
        self.decision == Decision::Protect
    }
}

pub(crate) fn observe_reasons(mut reasons: Vec<DecisionReason>) -> Vec<DecisionReason> {
    if !reasons.contains(&DecisionReason::InsufficientShadowValidations) {
        reasons.push(DecisionReason::InsufficientShadowValidations);
    }
    reasons
}
