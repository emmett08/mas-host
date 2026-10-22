//! Compute conservative fixture handling while preserving external execution certainty.

use crate::{Assessment, Category, Completion, Limits, Result, Source};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HandlingProfile {
    SyntheticFixture,
    Protected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionCertainty {
    NotSent,
    KnownRefusal,
    KnownCompleted,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Disposition {
    PermitSynthetic,
    Withhold,
    Hold,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotificationRoute {
    DataOwner,
    SecurityResponse,
    ServiceOperations,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Reason {
    ExpectedAuthorisedUse,
    PolicyUnavailable,
    AccessDenied,
    ProhibitedFinding,
    SuspectedExposure,
    IncompleteInspection,
    UnqualifiedProtectedProfile,
}

/// Values must be derived by trusted host configuration/current authority.
/// This fixture type grants no rights, persists no intent and sends no messages.
#[derive(Clone, Copy, Debug)]
pub struct HandlingContext {
    pub profile: HandlingProfile,
    pub policy_available: bool,
    pub authorised: bool,
    pub findings_permitted: bool,
    pub suspected_exposure: bool,
    pub execution: ExecutionCertainty,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Decision {
    pub disposition: Disposition,
    pub reason: Reason,
    pub route: Option<NotificationRoute>,
    pub execution: ExecutionCertainty,
}

pub fn decide(
    assessment: &Assessment,
    source: &Source,
    context: HandlingContext,
    limits: Limits,
) -> Result<Decision> {
    assessment.validate(source, limits)?;
    let credential = assessment
        .findings()
        .iter()
        .any(|finding| finding.categories.contains(&Category::Credential));
    let (disposition, reason, route) = if context.suspected_exposure {
        (
            Disposition::Withhold,
            Reason::SuspectedExposure,
            Some(NotificationRoute::SecurityResponse),
        )
    } else if !context.authorised {
        (Disposition::Withhold, Reason::AccessDenied, None)
    } else if !context.policy_available {
        (
            Disposition::Hold,
            Reason::PolicyUnavailable,
            Some(NotificationRoute::ServiceOperations),
        )
    } else if !assessment.findings().is_empty() && !context.findings_permitted {
        (
            Disposition::Withhold,
            Reason::ProhibitedFinding,
            Some(if credential {
                NotificationRoute::SecurityResponse
            } else {
                NotificationRoute::DataOwner
            }),
        )
    } else if assessment.completion() != Completion::Complete {
        (
            Disposition::Hold,
            Reason::IncompleteInspection,
            Some(NotificationRoute::ServiceOperations),
        )
    } else if context.profile == HandlingProfile::Protected {
        (
            Disposition::Hold,
            Reason::UnqualifiedProtectedProfile,
            Some(NotificationRoute::ServiceOperations),
        )
    } else {
        (
            Disposition::PermitSynthetic,
            Reason::ExpectedAuthorisedUse,
            None,
        )
    };
    Ok(Decision {
        disposition,
        reason,
        route,
        execution: context.execution,
    })
}
