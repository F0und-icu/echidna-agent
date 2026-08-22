//! One live-process Resource lease owner for Jobs and PTY sessions.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::domain::tools::{
    JobId, PtySessionId, ResourceKey, ToolEffect, ToolExecutionPlan, ToolParallelism,
    scheduling_conflicts,
};

/// A concrete long-lived process identity. It is deliberately limited to the
/// two process lifecycles that currently retain Resource leases; it is not a
/// general-purpose scheduling or lifecycle framework.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum ActiveProcessId {
    Job(JobId),
    Pty(PtySessionId),
}

#[derive(Clone)]
pub(super) struct ActiveProcessLeases {
    entries: Arc<Mutex<BTreeMap<ActiveProcessId, ActiveProcessLease>>>,
}

struct ActiveProcessLease {
    resource_keys: Vec<ResourceKey>,
}

/// RAII ownership of one live-process lease. Keeping this guard with process
/// setup and then its supervisor closes cancellation, spawn-failure and panic
/// paths without duplicating release logic in Job and PTY adapters.
pub(super) struct ActiveProcessLeaseGuard {
    leases: ActiveProcessLeases,
    process_id: ActiveProcessId,
}

impl ActiveProcessLeases {
    pub(super) fn new() -> Self {
        Self {
            entries: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    /// Acquires the Resource lease retained from process launch through its
    /// stable terminal publication. The Domain conflict predicate remains the
    /// sole owner of Resource/Effect/Parallelism semantics.
    pub(super) fn acquire(
        &self,
        process_id: ActiveProcessId,
        mut resource_keys: Vec<ResourceKey>,
    ) -> Result<ActiveProcessLeaseGuard, ActiveProcessId> {
        resource_keys.sort();
        resource_keys.dedup();
        let mut entries = lock_entries(&self.entries);
        if let Some(conflict) = entries.iter().find_map(|(active_id, active)| {
            scheduling_conflicts(
                ToolParallelism::ResourceLocked,
                ToolEffect::Interactive,
                &resource_keys,
                ToolParallelism::ResourceLocked,
                ToolEffect::Interactive,
                &active.resource_keys,
            )
            .then(|| active_id.clone())
        }) {
            return Err(conflict);
        }
        entries.insert(process_id.clone(), ActiveProcessLease { resource_keys });
        Ok(ActiveProcessLeaseGuard {
            leases: self.clone(),
            process_id,
        })
    }

    pub(super) fn conflicting(&self, plan: &ToolExecutionPlan) -> Option<ActiveProcessId> {
        let entries = lock_entries(&self.entries);
        entries.iter().find_map(|(active_id, active)| {
            scheduling_conflicts(
                plan.parallelism,
                plan.effect,
                &plan.resolved_resource_keys,
                ToolParallelism::ResourceLocked,
                ToolEffect::Interactive,
                &active.resource_keys,
            )
            .then(|| active_id.clone())
        })
    }
}

impl Drop for ActiveProcessLeaseGuard {
    fn drop(&mut self) {
        lock_entries(&self.leases.entries).remove(&self.process_id);
    }
}

fn lock_entries(
    entries: &Arc<Mutex<BTreeMap<ActiveProcessId, ActiveProcessLease>>>,
) -> MutexGuard<'_, BTreeMap<ActiveProcessId, ActiveProcessLease>> {
    entries.lock().unwrap_or_else(|error| error.into_inner())
}
