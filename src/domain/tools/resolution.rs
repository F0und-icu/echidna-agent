//! Pure immutable Tool exposure resolution for one Sampling Step.

use std::collections::BTreeSet;

use serde::Serialize;
use serde_json::Value;

use super::{
    ResolvedToolPlanFingerprint, ToolContractError, ToolExecutionPlan, ToolExposure,
    ToolExposurePlan, ToolName, ToolResult, ToolResultStatus, ToolSpec, fingerprint,
};

/// Deferred Tools activated by successful `tool_search` calls in the current
/// Turn. AgentLoop owns the lifetime; the immutable Tool catalog never does.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeferredActivationSet {
    names: BTreeSet<ToolName>,
}

impl DeferredActivationSet {
    pub fn names(&self) -> &BTreeSet<ToolName> {
        &self.names
    }

    /// Applies only names that are Deferred candidates in the Step which
    /// produced the successful search result. Hidden and unknown names can
    /// therefore never be promoted through model-controlled output.
    pub fn apply_search_result(
        &mut self,
        result: &ToolResult,
        source_plan: &ResolvedToolPlan,
    ) -> Result<(), ToolContractError> {
        if result.status != ToolResultStatus::Succeeded
            || result.tool_name.as_str() != "tool_search"
        {
            return Ok(());
        }
        let matches = result
            .output
            .get("matches")
            .and_then(Value::as_array)
            .ok_or(ToolContractError::InvalidToolSearchResult)?;
        let candidates = source_plan
            .deferred_specs
            .iter()
            .map(|spec| spec.name.clone())
            .collect::<BTreeSet<_>>();
        for entry in matches {
            let name = entry
                .get("name")
                .and_then(Value::as_str)
                .ok_or(ToolContractError::InvalidToolSearchResult)?;
            let name = ToolName::new(name);
            if candidates.contains(&name) {
                self.names.insert(name);
            }
        }
        Ok(())
    }
}

/// One immutable, ordered resolution of the canonical Tool catalog for a
/// Sampling Step. It is the only authority for both advertised schemas and
/// model-originated dispatch eligibility.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ResolvedToolPlan {
    fingerprint: ResolvedToolPlanFingerprint,
    advertised_specs: Vec<ToolSpec>,
    deferred_specs: Vec<ToolSpec>,
    resolved_specs: Vec<ToolSpec>,
}

impl ResolvedToolPlan {
    pub fn resolve(
        catalog: impl IntoIterator<Item = ToolSpec>,
        exposure_plan: &ToolExposurePlan,
        activations: &DeferredActivationSet,
    ) -> Result<Self, ToolContractError> {
        let mut catalog = catalog.into_iter().collect::<Vec<_>>();
        catalog.sort_by(|left, right| left.name.cmp(&right.name));
        if catalog.windows(2).any(|pair| pair[0].name == pair[1].name) {
            return Err(ToolContractError::DuplicateToolName);
        }
        if let Some(name) = exposure_plan.overrides().keys().find(|name| {
            catalog
                .binary_search_by(|spec| spec.name.cmp(name))
                .is_err()
        }) {
            return Err(ToolContractError::UnknownToolExposure {
                tool_name: name.to_string(),
            });
        }

        let mut resolved_specs = Vec::with_capacity(catalog.len());
        for canonical in catalog {
            let mut resolved = exposure_plan.apply_to_spec(canonical);
            if activations.names.contains(&resolved.name) {
                if resolved.exposure != ToolExposure::Deferred {
                    return Err(ToolContractError::InvalidDeferredActivation {
                        tool_name: resolved.name.to_string(),
                    });
                }
                resolved.exposure = ToolExposure::Direct;
            }
            resolved_specs.push(resolved);
        }
        let advertised_specs = resolved_specs
            .iter()
            .filter(|spec| spec.exposure == ToolExposure::Direct)
            .cloned()
            .collect::<Vec<_>>();
        let deferred_specs = resolved_specs
            .iter()
            .filter(|spec| spec.exposure == ToolExposure::Deferred)
            .cloned()
            .collect::<Vec<_>>();
        let fingerprint = ResolvedToolPlanFingerprint::new(fingerprint(&resolved_specs)?);
        Ok(Self {
            fingerprint,
            advertised_specs,
            deferred_specs,
            resolved_specs,
        })
    }

    pub fn fingerprint(&self) -> &ResolvedToolPlanFingerprint {
        &self.fingerprint
    }

    pub fn advertised_specs(&self) -> &[ToolSpec] {
        &self.advertised_specs
    }

    pub fn deferred_specs(&self) -> &[ToolSpec] {
        &self.deferred_specs
    }

    pub fn resolved_specs(&self) -> &[ToolSpec] {
        &self.resolved_specs
    }

    pub fn advertised_spec(&self, name: &ToolName) -> Option<&ToolSpec> {
        self.advertised_specs
            .binary_search_by(|spec| spec.name.cmp(name))
            .ok()
            .map(|index| &self.advertised_specs[index])
    }

    pub fn validates_execution_plan(&self, plan: &ToolExecutionPlan) -> bool {
        plan.resolved_tool_plan_fingerprint.as_ref() == Some(&self.fingerprint)
            && self
                .advertised_spec(&plan.call.name)
                .is_some_and(|spec| plan.matches_spec(spec))
    }
}
