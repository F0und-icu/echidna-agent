//! Complete serialized ToolSpecs captured from a9cfcbd before M06 refactoring.
//! These hashes cover names, descriptions, schemas, versions, effects and limits.

use crate::adapters::tools::ToolRegistry;
use crate::domain::agent_definition::AgentId;
use crate::domain::events::StreamId;
use crate::domain::memory::MemoryScope;
use crate::domain::tools::{ExecutionId, ToolCall, ToolCallId};
use crate::ports::tool::{ToolExecutor, ToolExecutorError};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[test]
fn complete_catalogs_preserve_pre_refactor_contracts() {
    let mut cases = Vec::new();
    for window in [32768, 272000, 1048576] {
        for (verified, automatic) in [(false, false), (true, false), (true, true)] {
            cases.push((
                format!("project-window-{window}-verified-{verified}-automatic-{automatic}"),
                ToolRegistry::for_project("/fixture/catalog")
                    .with_context_window(window)
                    .with_input_verification(verified)
                    .with_automatic_verification(automatic),
            ));
        }
    }
    for research in [false, true] {
        for memory in [false, true] {
            let scope = memory.then(|| MemoryScope::Agent {
                agent_id: AgentId::new("catalog-fixture").unwrap(),
            });
            cases.push((
                format!("agent-research-{research}-memory-{memory}"),
                ToolRegistry::for_agent(
                    "/fixture/catalog",
                    StreamId::new("catalog-session"),
                    scope,
                    research,
                ),
            ));
        }
    }
    let expected: BTreeMap<String, Value> =
        serde_json::from_str(include_str!("../fixtures/tools/catalog-baseline.json")).unwrap();
    let actual: BTreeMap<String, Value> = cases
        .into_iter()
        .map(|(label, registry)| {
            let catalog = registry.catalog();
            let bytes = serde_json::to_vec(&catalog).unwrap();
            (
                label,
                json!({"sha256": format!("{:x}", Sha256::digest(&bytes)), "count": catalog.len()}),
            )
        })
        .collect();
    assert_eq!(actual, expected);
}

#[test]
fn every_catalog_entry_resolves_its_own_argument_contract() {
    let registry = ToolRegistry::for_project("/fixture/catalog");
    let catalog = registry.catalog();
    assert!(catalog.windows(2).all(|pair| pair[0].name < pair[1].name));
    for spec in catalog {
        let call = ToolCall::new(
            ToolCallId::new("catalog-invalid-input"),
            spec.name.clone(),
            Value::Null,
        );
        let error = registry
            .plan(ExecutionId::new("catalog-plan"), call)
            .unwrap_err();
        assert!(
            matches!(error, ToolExecutorError::InvalidArguments { tool_name, .. } if tool_name == spec.name.as_str()),
            "{} must dispatch to its own argument normalizer",
            spec.name
        );
    }
}
