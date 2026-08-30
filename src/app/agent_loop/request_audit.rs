//! Fixed RequestAudit attempt construction for the one AgentLoop.

use crate::domain::context::{BaselineAssessment, ContextDeliveryMap};
use crate::domain::request_audit::{
    RequestAuditAttempt, RequestAuditDelivery, RequestAuditRunContext,
};
use crate::domain::sampling::SamplingStepPlan;
use crate::ports::model::{ModelProvider, ModelRequest};
use crate::ports::tool::ToolExecutor;

use super::{AgentLoop, AgentLoopError};

/// The one fixed, opt-in audit stage for an AgentLoop Provider attempt. This
/// is a named callback shape, not a general Hook or runtime registry.
pub(super) type RequestAuditStage<'a> = (
    &'a RequestAuditRunContext,
    &'a mut dyn FnMut(RequestAuditAttempt) -> Result<(), AgentLoopError>,
);

impl<'provider, 'tools, Provider, Tools> AgentLoop<'provider, 'tools, Provider, Tools>
where
    Provider: ModelProvider + ?Sized,
    Tools: ToolExecutor + ?Sized,
{
    pub(super) fn record_request_audit_attempt(
        &self,
        audit: &mut Option<RequestAuditStage<'_>>,
        sampling_step: &SamplingStepPlan,
        full_input: &crate::domain::model::ModelInputIR,
        request: &ModelRequest,
        delivery_map: &ContextDeliveryMap,
        assessment: &BaselineAssessment,
    ) -> Result<(), AgentLoopError> {
        let Some((context, record_attempt)) = audit.as_mut() else {
            return Ok(());
        };
        let delivery = match (request.is_incremental(), assessment) {
            (false, BaselineAssessment::FullRequest { reason }) => {
                RequestAuditDelivery::full(*reason)
            }
            (true, BaselineAssessment::AppendOnlyCandidate { .. }) => {
                RequestAuditDelivery::incremental_append_only()
            }
            _ => return Err(AgentLoopError::RequestAuditDeliveryMismatch),
        };
        let attempt = RequestAuditAttempt::new(
            context,
            self.model.clone(),
            self.profile.clone(),
            sampling_step.capabilities().clone(),
            full_input.clone(),
            request.input().clone(),
            delivery,
        )?
        .bind_sampling_step(sampling_step, delivery_map.clone())?;
        record_attempt(attempt)
    }
}
