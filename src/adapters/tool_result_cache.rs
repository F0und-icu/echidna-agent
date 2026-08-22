//! Project-isolated, content-addressed derived Tool Result Cache.
//!
//! The cache intentionally stores only successful deterministic read-only
//! outcomes whose concrete mutable input was verified by the Tool adapter.
//! Every failure here degrades to a miss: Event/Receipt facts remain outside
//! this directory and authoritative.

mod session;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::adapters::project_event_store::{
    Project, ProjectEventStoreError, commit_new_durable_file, sync_directory,
};
use crate::domain::events::StreamId;
use crate::domain::project::ProjectId;
use crate::domain::tools::{
    ToolExecutionOutcome, ToolExecutionPlan, ToolResultCacheKey, ToolResultStatus,
};
use session::{ReceiptEventReference, SessionToolCacheAudit};

const CACHE_SCHEMA_VERSION: u16 = 1;
const CACHE_DIRECTORY: &str = "cache";
const TOOL_RESULTS_DIRECTORY: &str = "tool-results";

/// The one local persistence owner for derived Tool results. It has no cache
/// trait because the only runtime consumer is the fixed Tool Registry.
pub(crate) struct LocalToolResultCache {
    project_id: ProjectId,
    control_directory: PathBuf,
    session_audit: SessionToolCacheAudit,
}

impl LocalToolResultCache {
    /// An ordinary non-initialized temporary directory simply has no project
    /// cache. Invalid/corrupt project control facts are also treated as a
    /// safe cache miss; they never alter tool execution semantics.
    pub(crate) fn bind(
        project_root: &Path,
        session_id: StreamId,
    ) -> Result<Self, ProjectEventStoreError> {
        let project = Project::discover(project_root)?;
        Ok(Self {
            project_id: project.project_id().clone(),
            control_directory: project.root().join(".cyber-agent"),
            session_audit: SessionToolCacheAudit::new(project, session_id)?,
        })
    }

    pub(crate) fn lookup(&self, plan: &ToolExecutionPlan) -> Option<ToolExecutionOutcome> {
        let identity = self.identity_for(plan)?;
        let key = cache_key(&identity)?;
        let path = self.entry_path(&key);
        let bytes = match regular_file_bytes(&path) {
            Ok(Some(bytes)) => bytes,
            Ok(None) | Err(()) => return None,
        };
        let entry: ToolResultCacheEntry = match serde_json::from_slice(&bytes) {
            Ok(entry) => entry,
            Err(_) => {
                remove_corrupt_cache_file(&path);
                return None;
            }
        };
        if entry.cache_schema_version != CACHE_SCHEMA_VERSION
            || entry.project_id != self.project_id
            || entry.cache_key != key
            || entry.identity != identity
            || entry.source.result.tool_name != plan.call.name
            || entry.source.receipt.status != ToolResultStatus::Succeeded
            || entry.source.result.status != ToolResultStatus::Succeeded
            || entry.source.receipt.cache_source.is_some()
            || source_checksum(&entry.source).as_deref() != Some(entry.source_checksum.as_str())
            // No Artifact Store is available at this cache boundary to prove
            // referenced Blobs still exist. Future Artifact-producing cache
            // consumers must add that validation before this guard is relaxed.
            || !entry.source.receipt.output_artifact_ids.is_empty()
            || entry
                .source
                .receipt
                .cache_validation
                .as_ref()
                .is_none_or(|validation| !validation.is_verified_for(plan))
            || !self
                .session_audit
                .source_is_valid(&entry.source_receipt_event, &entry.source)
        {
            remove_corrupt_cache_file(&path);
            return None;
        }
        let outcome = ToolExecutionOutcome::cached_for(
            plan.clone(),
            &entry.source,
            key,
            entry.source_receipt_event.stream_id.clone(),
            entry.source_receipt_event.event_id.clone(),
        );
        self.session_audit
            .record_hit(&outcome.receipt)
            .then_some(outcome)
    }

    pub(crate) fn store(&self, plan: &ToolExecutionPlan, source: &ToolExecutionOutcome) {
        let Some(identity) = self.identity_for(plan) else {
            return;
        };
        let Some(key) = cache_key(&identity) else {
            return;
        };
        if source.receipt.status != ToolResultStatus::Succeeded
            || source.result.status != ToolResultStatus::Succeeded
            || source.receipt.execution_id != plan.execution_id
            || source.receipt.plan_fingerprint != plan.plan_fingerprint
            || source.result.call_id != plan.call.call_id
            || source.result.tool_name != plan.call.name
            || source.receipt.cache_source.is_some()
            || !source.receipt.output_artifact_ids.is_empty()
            || source
                .receipt
                .cache_validation
                .as_ref()
                .is_none_or(|validation| !validation.is_verified_for(plan))
        {
            return;
        }
        let Some(source_checksum) = source_checksum(source) else {
            return;
        };
        let path = self.entry_path(&key);
        if regular_file_bytes(&path).is_ok_and(|entry| entry.is_some()) {
            return;
        }
        let Some(source_receipt_event) = self.session_audit.record_source(source) else {
            return;
        };
        let entry = ToolResultCacheEntry {
            cache_schema_version: CACHE_SCHEMA_VERSION,
            project_id: self.project_id.clone(),
            cache_key: key.clone(),
            identity,
            source_checksum,
            source_receipt_event,
            source: source.clone(),
        };
        let Ok(bytes) = serde_json::to_vec(&entry) else {
            return;
        };
        let Ok(directory) = self.ensure_directory() else {
            return;
        };
        let path = directory.join(format!("{}.json", key.as_str()));
        if regular_file_bytes(&path).is_ok_and(|entry| entry.is_some()) {
            return;
        }
        let _ = commit_new_durable_file(&path, &bytes);
    }

    fn entry_path(&self, key: &ToolResultCacheKey) -> PathBuf {
        self.control_directory
            .join(CACHE_DIRECTORY)
            .join(TOOL_RESULTS_DIRECTORY)
            .join(format!("{}.json", key.as_str()))
    }

    fn identity_for(&self, plan: &ToolExecutionPlan) -> Option<ToolResultCacheIdentity> {
        if !plan.result_cache_eligible() {
            return None;
        }
        Some(ToolResultCacheIdentity {
            cache_schema_version: CACHE_SCHEMA_VERSION,
            project_id: self.project_id.clone(),
            tool_name: plan.call.name.clone(),
            tool_version: plan.tool_version.clone(),
            input_fingerprint: plan.input_fingerprint.clone(),
            // The authoritative Research JSONL bytes are this Tool's entire
            // observable environment. TargetSnapshot is not present on the
            // fixed M2-S5 plan and therefore remains explicitly unbound.
            environment_fingerprint: Some(plan.cache_resource_fingerprint.clone()?),
            // No target fact is available on the fixed M2-S5 plan; do not
            // invent one for a cache identity.
            target_snapshot_id: None,
        })
    }

    fn ensure_directory(&self) -> Result<PathBuf, ()> {
        ensure_directory(&self.control_directory)?;
        let cache = self.control_directory.join(CACHE_DIRECTORY);
        ensure_directory(&cache)?;
        let results = cache.join(TOOL_RESULTS_DIRECTORY);
        ensure_directory(&results)?;
        Ok(results)
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ToolResultCacheEntry {
    cache_schema_version: u16,
    project_id: ProjectId,
    cache_key: ToolResultCacheKey,
    identity: ToolResultCacheIdentity,
    source_checksum: String,
    source_receipt_event: ReceiptEventReference,
    source: ToolExecutionOutcome,
}

#[derive(Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ToolResultCacheIdentity {
    cache_schema_version: u16,
    project_id: ProjectId,
    tool_name: crate::domain::tools::ToolName,
    tool_version: String,
    input_fingerprint: String,
    target_snapshot_id: Option<String>,
    environment_fingerprint: Option<String>,
}

fn cache_key(identity: &ToolResultCacheIdentity) -> Option<ToolResultCacheKey> {
    serde_json::to_vec(identity)
        .ok()
        .map(|bytes| ToolResultCacheKey::new(blake3::hash(&bytes).to_hex().to_string()))
}

fn source_checksum(source: &ToolExecutionOutcome) -> Option<String> {
    serde_json::to_vec(source)
        .ok()
        .map(|bytes| blake3::hash(&bytes).to_hex().to_string())
}

fn regular_file_bytes(path: &Path) -> Result<Option<Vec<u8>>, ()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => fs::read(path).map(Some).map_err(|_| ()),
        Ok(_) => Err(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(()),
    }
}

fn remove_corrupt_cache_file(path: &Path) {
    if matches!(fs::symlink_metadata(path), Ok(metadata) if metadata.file_type().is_file()) {
        let _ = fs::remove_file(path);
    }
}

fn ensure_directory(path: &Path) -> Result<(), ()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => Ok(()),
        Ok(_) => Err(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|_| ())?;
            let parent = path.parent().ok_or(())?;
            sync_directory(parent).map_err(|_| ())
        }
        Err(_) => Err(()),
    }
}
