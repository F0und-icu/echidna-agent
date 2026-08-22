//! Typed Resource Key values and their one pure conflict predicate.

use serde::{Deserialize, Serialize};

use super::{ToolEffect, ToolParallelism};

/// A resolved scheduling target. Adapter code resolves paths and external
/// identifiers before constructing this provider-neutral value; the App layer
/// only compares typed keys and never reparses adapter arguments.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ResourceKey {
    File(String),
    Device(String),
    Pty(String),
    Output(String),
    GlobalSerial,
}

impl ResourceKey {
    pub fn file(path: impl Into<String>) -> Self {
        Self::File(path.into())
    }

    pub fn device(serial: impl Into<String>) -> Self {
        Self::Device(serial.into())
    }

    pub fn pty(session: impl Into<String>) -> Self {
        Self::Pty(session.into())
    }

    pub fn output(path: impl Into<String>) -> Self {
        Self::Output(path.into())
    }
}

/// The App owns scheduling across Tool Calls. An Adapter may reuse this
/// predicate only to honor explicit serial/parallel command-batch semantics;
/// it must not define another Resource Key conflict rule.
pub(crate) fn scheduling_conflicts(
    candidate_parallelism: ToolParallelism,
    candidate_effect: ToolEffect,
    candidate_keys: &[ResourceKey],
    active_parallelism: ToolParallelism,
    active_effect: ToolEffect,
    active_keys: &[ResourceKey],
) -> bool {
    if requires_global_serial(candidate_parallelism, candidate_effect, candidate_keys)
        || requires_global_serial(active_parallelism, active_effect, active_keys)
    {
        return true;
    }
    if candidate_keys
        .iter()
        .any(|key| key.is_exclusive_external_resource() && active_keys.contains(key))
    {
        return true;
    }
    if candidate_parallelism == ToolParallelism::Parallel
        && active_parallelism == ToolParallelism::Parallel
    {
        return false;
    }
    candidate_keys.iter().any(|key| active_keys.contains(key))
}

impl ResourceKey {
    fn is_exclusive_external_resource(&self) -> bool {
        matches!(self, Self::Device(_) | Self::Pty(_) | Self::Output(_))
    }
}

fn requires_global_serial(
    parallelism: ToolParallelism,
    effect: ToolEffect,
    resource_keys: &[ResourceKey],
) -> bool {
    parallelism == ToolParallelism::Serial
        || resource_keys.contains(&ResourceKey::GlobalSerial)
        || (resource_keys.is_empty()
            && (parallelism == ToolParallelism::ResourceLocked || effect != ToolEffect::ReadOnly))
}
