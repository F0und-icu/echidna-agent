mod support;

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use cyber_agent::adapters::fragment_cache::LocalFragmentCache;
use cyber_agent::adapters::project_event_store::Project;
use cyber_agent::app::context_compilation::{FragmentCacheDisposition, resolve_context_fragment};
use cyber_agent::domain::artifacts::ArtifactId;
use cyber_agent::domain::context::{
    ContextBudget, ContextFragment, ContextFragmentCacheKey, ContextFragmentContent,
    ContextFragmentId, ContextFragmentPriority, ContextFragmentSource, ContextPlanner,
    ContextPlannerVersions, ContextSourceFingerprint, FragmentCachePolicy,
};
use cyber_agent::domain::model::{CapabilityMappingVersion, ContentItem, ModelCapabilities};
use cyber_agent::ports::context::{
    ContextFragmentCache, ContextFragmentCacheError, ContextFragmentCacheOperation,
};
use serde_json::{Value, json};

use support::TemporaryDirectory;

fn initialized_project(label: &str) -> (TemporaryDirectory, Project) {
    let root = TemporaryDirectory::new(label);
    let project = Project::initialize(root.path()).expect("temporary project should initialize");
    (root, project)
}

fn capabilities() -> ModelCapabilities {
    ModelCapabilities {
        text_input: true,
        image_input: false,
        streaming: true,
        tool_call: true,
        structured_output: true,
        incremental_input: true,
        usage_reporting: true,
        context_window: 256,
        capability_mapping_version: CapabilityMappingVersion::new(1),
    }
}

fn versions() -> ContextPlannerVersions {
    ContextPlannerVersions::new("planner:1", "estimator:1", "budget-policy:1")
}

fn budget() -> ContextBudget {
    ContextBudget::new(128, 16, 8).expect("fixture budget should be valid")
}

fn fragment(
    id: &str,
    source: ContextFragmentSource,
    policy: FragmentCachePolicy,
) -> ContextFragment {
    let content = match source {
        ContextFragmentSource::BasePrompt
        | ContextFragmentSource::PersonalPreferences
        | ContextFragmentSource::ProjectPrompt
        | ContextFragmentSource::ModePrompt
        | ContextFragmentSource::DomainPrompt
        | ContextFragmentSource::PhasePrompt => ContextFragmentContent::Instructions {
            text: format!("instruction:{id}"),
        },
        ContextFragmentSource::ResearchWorldState
        | ContextFragmentSource::ProjectSnapshot
        | ContextFragmentSource::ProjectMemoryCore
        | ContextFragmentSource::RetrievedMemory
        | ContextFragmentSource::SessionSummary
        | ContextFragmentSource::RecentHistory
        | ContextFragmentSource::UserInput
        | ContextFragmentSource::TurnLocalUpdates => ContextFragmentContent::ContentItems {
            items: vec![ContentItem::user_text(format!("content:{id}"))],
        },
    };
    ContextFragment::new(
        ContextFragmentId::new(id).expect("fixture ID should be valid"),
        source,
        ContextFragmentPriority::Normal,
        policy,
        content,
    )
    .expect("fixture fragment should be valid")
}

fn key_for(
    fragment: &ContextFragment,
    source: &str,
    versions: &ContextPlannerVersions,
    capabilities: &ModelCapabilities,
    budget: ContextBudget,
) -> ContextFragmentCacheKey {
    ContextPlanner::new(versions.clone())
        .fragment_cache_key(
            fragment.id().clone(),
            fragment.source(),
            fragment.priority(),
            fragment.cache_policy(),
            ContextSourceFingerprint::from_source(&json!({ "source": source }))
                .expect("fixture source should fingerprint"),
            fragment.content_hash().clone(),
            capabilities,
            budget,
        )
        .expect("fixture cache key should build")
}

fn default_key(fragment: &ContextFragment) -> ContextFragmentCacheKey {
    key_for(fragment, "source:1", &versions(), &capabilities(), budget())
}

fn entry_path(project: &Project, key: &ContextFragmentCacheKey) -> PathBuf {
    project
        .root()
        .join(".cyber-agent/cache/context-fragments")
        .join(format!("{}.json", key.as_str()))
}

fn assert_renderer_runs(
    cache: &LocalFragmentCache,
    key: &ContextFragmentCacheKey,
    fragment: &ContextFragment,
) {
    let renders = AtomicUsize::new(0);
    let resolved = resolve_context_fragment(Some(cache), key, || {
        renders.fetch_add(1, Ordering::SeqCst);
        Ok(fragment.clone())
    })
    .expect("invalid cache entry should rebuild from the renderer");
    assert_eq!(resolved.fragment, *fragment);
    assert_eq!(renders.load(Ordering::SeqCst), 1);
}

struct MismatchedHitCache {
    fragment: ContextFragment,
    lookup_calls: AtomicUsize,
    store_calls: AtomicUsize,
}

impl ContextFragmentCache for MismatchedHitCache {
    fn lookup(
        &self,
        _key: &ContextFragmentCacheKey,
    ) -> Result<Option<ContextFragment>, ContextFragmentCacheError> {
        self.lookup_calls.fetch_add(1, Ordering::SeqCst);
        Ok(Some(self.fragment.clone()))
    }

    fn store(
        &self,
        _key: &ContextFragmentCacheKey,
        _fragment: &ContextFragment,
    ) -> Result<(), ContextFragmentCacheError> {
        self.store_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn tamper_entry(
    cache: &LocalFragmentCache,
    project: &Project,
    key: &ContextFragmentCacheKey,
    fragment: &ContextFragment,
    mutate: impl FnOnce(&mut Value),
) {
    cache
        .store(key, fragment)
        .expect("fixture cache entry should persist");
    let path = entry_path(project, key);
    let mut entry: Value =
        serde_json::from_slice(&fs::read(&path).expect("fixture cache entry should be readable"))
            .expect("fixture cache entry should be JSON");
    mutate(&mut entry);
    fs::write(
        &path,
        serde_json::to_vec(&entry).expect("tampered entry should encode"),
    )
    .expect("tampered entry should write");
    assert_renderer_runs(cache, key, fragment);
}

#[test]
fn content_addressed_hit_skips_the_renderer_after_the_first_store() {
    let (_root, project) = initialized_project("fragment-cache-hit");
    let cache = LocalFragmentCache::for_project(&project);
    let fragment = fragment(
        "base",
        ContextFragmentSource::BasePrompt,
        FragmentCachePolicy::ContentAddressed,
    );
    let key = default_key(&fragment);
    let renders = AtomicUsize::new(0);

    let first = resolve_context_fragment(Some(&cache), &key, || {
        renders.fetch_add(1, Ordering::SeqCst);
        Ok(fragment.clone())
    })
    .expect("initial cache miss should render");
    let second = resolve_context_fragment(Some(&cache), &key, || {
        renders.fetch_add(1, Ordering::SeqCst);
        Ok(fragment.clone())
    })
    .expect("cache hit should load");

    assert_eq!(first.fragment, fragment);
    assert_eq!(second.fragment, fragment);
    assert_eq!(renders.load(Ordering::SeqCst), 1);
    assert!(entry_path(&project, &key).is_file());
}

#[test]
fn source_fingerprint_change_is_a_distinct_fragment_cache_miss() {
    let (_root, project) = initialized_project("fragment-cache-source");
    let cache = LocalFragmentCache::for_project(&project);
    let fragment = fragment(
        "memory",
        ContextFragmentSource::RetrievedMemory,
        FragmentCachePolicy::ContentAddressed,
    );
    let versions = versions();
    let capabilities = capabilities();
    let first = key_for(&fragment, "source:one", &versions, &capabilities, budget());
    let second = key_for(&fragment, "source:two", &versions, &capabilities, budget());
    let changed_content = ContextFragment::new(
        fragment.id().clone(),
        fragment.source(),
        fragment.priority(),
        fragment.cache_policy(),
        ContextFragmentContent::ContentItems {
            items: vec![ContentItem::user_text("changed rendered content")],
        },
    )
    .expect("changed fixture Fragment should be valid");
    let changed_content_key = key_for(
        &changed_content,
        "source:one",
        &versions,
        &capabilities,
        budget(),
    );
    let renders = AtomicUsize::new(0);

    for (key, rendered) in [
        (&first, &fragment),
        (&second, &fragment),
        (&changed_content_key, &changed_content),
    ] {
        resolve_context_fragment(Some(&cache), key, || {
            renders.fetch_add(1, Ordering::SeqCst);
            Ok(rendered.clone())
        })
        .expect("different source fingerprints should be renderable");
    }

    assert_ne!(first, second);
    assert_ne!(
        first, changed_content_key,
        "the final Context content hash is part of the cache identity"
    );
    assert_eq!(renders.load(Ordering::SeqCst), 3);
}

#[test]
fn planner_versions_capabilities_and_complete_budget_all_invalidate_cache_identity() {
    let (_root, project) = initialized_project("fragment-cache-key-material");
    let cache = LocalFragmentCache::for_project(&project);
    let fragment = fragment(
        "snapshot",
        ContextFragmentSource::ProjectSnapshot,
        FragmentCachePolicy::ContentAddressed,
    );
    let baseline_versions = versions();
    let baseline_capabilities = capabilities();
    let baseline_budget = budget();
    let baseline = key_for(
        &fragment,
        "source:stable",
        &baseline_versions,
        &baseline_capabilities,
        baseline_budget,
    );
    let changed_planner = key_for(
        &fragment,
        "source:stable",
        &ContextPlannerVersions::new("planner:2", "estimator:1", "budget-policy:1"),
        &baseline_capabilities,
        baseline_budget,
    );
    let changed_estimator = key_for(
        &fragment,
        "source:stable",
        &ContextPlannerVersions::new("planner:1", "estimator:2", "budget-policy:1"),
        &baseline_capabilities,
        baseline_budget,
    );
    let changed_budget_policy = key_for(
        &fragment,
        "source:stable",
        &ContextPlannerVersions::new("planner:1", "estimator:1", "budget-policy:2"),
        &baseline_capabilities,
        baseline_budget,
    );
    let mut mapping_changed_capabilities = baseline_capabilities.clone();
    mapping_changed_capabilities.capability_mapping_version = CapabilityMappingVersion::new(2);
    let changed_capability_mapping = key_for(
        &fragment,
        "source:stable",
        &baseline_versions,
        &mapping_changed_capabilities,
        baseline_budget,
    );
    let mut changed_capabilities = baseline_capabilities.clone();
    changed_capabilities.image_input = true;
    let changed_capability = key_for(
        &fragment,
        "source:stable",
        &baseline_versions,
        &changed_capabilities,
        baseline_budget,
    );
    let changed_budget = key_for(
        &fragment,
        "source:stable",
        &baseline_versions,
        &baseline_capabilities,
        ContextBudget::new(129, 17, 8).expect("changed budget should be valid"),
    );
    let keys = [
        &baseline,
        &changed_planner,
        &changed_estimator,
        &changed_budget_policy,
        &changed_capability_mapping,
        &changed_capability,
        &changed_budget,
    ];
    let renders = AtomicUsize::new(0);

    for key in keys {
        if key != &baseline {
            assert_ne!(baseline, *key, "each changed semantic key must differ");
        }
        resolve_context_fragment(Some(&cache), key, || {
            renders.fetch_add(1, Ordering::SeqCst);
            Ok(fragment.clone())
        })
        .expect("each changed cache identity should render");
    }

    assert_eq!(renders.load(Ordering::SeqCst), 7);
}

#[test]
fn malformed_incompatible_or_mismatched_entries_are_safe_misses() {
    let (_root, project) = initialized_project("fragment-cache-invalid-entry");
    let cache = LocalFragmentCache::for_project(&project);
    let fragment = fragment(
        "history",
        ContextFragmentSource::RecentHistory,
        FragmentCachePolicy::ContentAddressed,
    );
    let key = default_key(&fragment);
    let path = entry_path(&project, &key);

    cache
        .store(&key, &fragment)
        .expect("fixture cache entry should persist");
    fs::write(&path, b"{").expect("corrupt fixture should write");
    assert_renderer_runs(&cache, &key, &fragment);

    cache
        .store(&key, &fragment)
        .expect("fixture cache entry should persist");
    fs::write(&path, b"{\"cache_schema_version\":1").expect("truncated fixture should write");
    assert_renderer_runs(&cache, &key, &fragment);

    tamper_entry(&cache, &project, &key, &fragment, |entry| {
        entry["cache_schema_version"] = json!(1);
    });
    tamper_entry(&cache, &project, &key, &fragment, |entry| {
        entry["cache_key"] = json!("a-different-key");
    });
    tamper_entry(&cache, &project, &key, &fragment, |entry| {
        entry["project_id"] = json!("project-from-another-root");
    });
    tamper_entry(&cache, &project, &key, &fragment, |entry| {
        entry["fragment"]["content_hash"] = json!("0".repeat(64));
    });
    tamper_entry(&cache, &project, &key, &fragment, |entry| {
        entry["unknown_field"] = json!(true);
    });
    tamper_entry(&cache, &project, &key, &fragment, |entry| {
        entry["fragment"]["content"]["unknown_field"] = json!(true);
    });
}

#[test]
fn persisted_token_estimate_tampering_is_a_safe_miss() {
    let (_root, project) = initialized_project("fragment-cache-token-tamper");
    let cache = LocalFragmentCache::for_project(&project);
    let fragment = fragment(
        "history",
        ContextFragmentSource::RecentHistory,
        FragmentCachePolicy::ContentAddressed,
    );
    let key = default_key(&fragment);

    tamper_entry(&cache, &project, &key, &fragment, |entry| {
        entry["fragment"]["token_estimate"] = json!(1);
    });
}

#[test]
fn payload_checksum_binds_the_complete_persisted_fragment() {
    let (_root, project) = initialized_project("fragment-cache-payload-checksum");
    let cache = LocalFragmentCache::for_project(&project);
    let fragment = fragment(
        "history",
        ContextFragmentSource::RecentHistory,
        FragmentCachePolicy::ContentAddressed,
    );
    let replacement = ContextFragment::new(
        ContextFragmentId::new("history").unwrap(),
        ContextFragmentSource::RecentHistory,
        ContextFragmentPriority::Normal,
        FragmentCachePolicy::ContentAddressed,
        ContextFragmentContent::ContentItems {
            items: vec![ContentItem::user_text(
                "self-consistent but unbound tampering",
            )],
        },
    )
    .unwrap();
    let key = default_key(&fragment);

    tamper_entry(&cache, &project, &key, &fragment, |entry| {
        entry["fragment"]["content"] = serde_json::to_value(replacement.content()).unwrap();
        entry["fragment"]["content_hash"] = json!(replacement.content_hash().as_str());
        entry["fragment"]["token_estimate"] = json!(replacement.token_estimate());
    });
}

#[test]
fn mismatched_port_hit_is_rendered_and_stored_as_a_miss() {
    let expected = fragment(
        "base",
        ContextFragmentSource::BasePrompt,
        FragmentCachePolicy::ContentAddressed,
    );
    let stale = fragment(
        "different-base",
        ContextFragmentSource::BasePrompt,
        FragmentCachePolicy::ContentAddressed,
    );
    let cache = MismatchedHitCache {
        fragment: stale,
        lookup_calls: AtomicUsize::new(0),
        store_calls: AtomicUsize::new(0),
    };
    let key = default_key(&expected);
    let renders = AtomicUsize::new(0);

    let resolved = resolve_context_fragment(Some(&cache), &key, || {
        renders.fetch_add(1, Ordering::SeqCst);
        Ok(expected.clone())
    })
    .expect("a mismatched Port hit must safely fall through to rendering");

    assert_eq!(resolved.fragment, expected);
    assert_eq!(resolved.cache_disposition, FragmentCacheDisposition::Miss);
    assert_eq!(renders.load(Ordering::SeqCst), 1);
    assert_eq!(cache.lookup_calls.load(Ordering::SeqCst), 1);
    assert_eq!(cache.store_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn deletion_and_project_identity_mismatch_rebuild_without_cross_project_reuse() {
    let (_first_root, first_project) = initialized_project("fragment-cache-project-one");
    let (_second_root, second_project) = initialized_project("fragment-cache-project-two");
    let first_cache = LocalFragmentCache::for_project(&first_project);
    let second_cache = LocalFragmentCache::for_project(&second_project);
    let fragment = fragment(
        "user",
        ContextFragmentSource::UserInput,
        FragmentCachePolicy::ContentAddressed,
    );
    let key = default_key(&fragment);
    let first_path = entry_path(&first_project, &key);
    let second_path = entry_path(&second_project, &key);

    first_cache
        .store(&key, &fragment)
        .expect("first project should persist its cache entry");
    fs::remove_file(&first_path).expect("cache entry should be removable");
    assert_renderer_runs(&first_cache, &key, &fragment);

    fs::create_dir_all(
        second_path
            .parent()
            .expect("cache entry path should have a parent"),
    )
    .expect("second cache directory should be creatable");
    fs::copy(&first_path, &second_path).expect("fixture should copy a foreign cache entry");
    assert_eq!(second_cache.lookup(&key).unwrap(), None);
    assert_renderer_runs(&second_cache, &key, &fragment);
}

#[test]
fn forged_invalid_fragment_channel_or_instruction_artifact_fallback_is_a_safe_miss() {
    let (_root, project) = initialized_project("fragment-cache-forged-fragment");
    let cache = LocalFragmentCache::for_project(&project);
    let content_fragment = fragment(
        "snapshot",
        ContextFragmentSource::ProjectSnapshot,
        FragmentCachePolicy::ContentAddressed,
    );
    let content_key = default_key(&content_fragment);
    tamper_entry(&cache, &project, &content_key, &content_fragment, |entry| {
        entry["fragment"]["source"] = json!("base_prompt");
    });

    let instruction_fragment = fragment(
        "base",
        ContextFragmentSource::BasePrompt,
        FragmentCachePolicy::ContentAddressed,
    );
    let instruction_key = default_key(&instruction_fragment);
    tamper_entry(
        &cache,
        &project,
        &instruction_key,
        &instruction_fragment,
        |entry| {
            entry["fragment"]["artifact_reference"] = json!(ArtifactId::new("artifact:forged"));
        },
    );
}

#[test]
fn do_not_cache_bypasses_lookup_and_store() {
    let (_root, project) = initialized_project("fragment-cache-do-not-cache");
    let cache = LocalFragmentCache::for_project(&project);
    let fragment = fragment(
        "volatile",
        ContextFragmentSource::RetrievedMemory,
        FragmentCachePolicy::DoNotCache,
    );
    let key = default_key(&fragment);
    let renders = AtomicUsize::new(0);

    for _ in 0..2 {
        resolve_context_fragment(Some(&cache), &key, || {
            renders.fetch_add(1, Ordering::SeqCst);
            Ok(fragment.clone())
        })
        .expect("do-not-cache renderer should succeed");
    }

    assert_eq!(renders.load(Ordering::SeqCst), 2);
    assert!(!entry_path(&project, &key).exists());
    assert!(
        !project
            .root()
            .join(".cyber-agent/cache/context-fragments")
            .exists()
    );
}

#[test]
fn cache_directory_regular_file_is_a_typed_filesystem_error() {
    let (_root, project) = initialized_project("fragment-cache-directory-error");
    let cache = LocalFragmentCache::for_project(&project);
    let fragment = fragment(
        "memory",
        ContextFragmentSource::RetrievedMemory,
        FragmentCachePolicy::ContentAddressed,
    );
    let key = default_key(&fragment);
    let path = project.root().join(".cyber-agent/cache/context-fragments");
    fs::create_dir_all(
        path.parent()
            .expect("fragment cache directory should have a parent"),
    )
    .expect("cache parent should be creatable");
    fs::write(&path, b"not a directory").expect("cache path fixture should write");

    assert!(matches!(
        cache.lookup(&key),
        Err(ContextFragmentCacheError {
            operation: ContextFragmentCacheOperation::Lookup,
            message,
        }) if message.contains(&path.display().to_string())
    ));
}

#[test]
fn store_rejects_a_fragment_that_does_not_match_the_key_identity() {
    let (_root, project) = initialized_project("fragment-cache-store-mismatch");
    let cache = LocalFragmentCache::for_project(&project);
    let base = fragment(
        "base",
        ContextFragmentSource::BasePrompt,
        FragmentCachePolicy::ContentAddressed,
    );
    let user = fragment(
        "user",
        ContextFragmentSource::UserInput,
        FragmentCachePolicy::ContentAddressed,
    );

    assert!(matches!(
        cache.store(&default_key(&base), &user),
        Err(ContextFragmentCacheError {
            operation: ContextFragmentCacheOperation::Store,
            message,
        }) if message.contains("does not match cache key")
    ));
}
