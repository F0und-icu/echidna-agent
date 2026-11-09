//! Bounded Research navigation and on-demand expansion over the strict reducer.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::domain::research::{
    EvidenceLink, EvidenceRef, ExecutionReceiptId, ResearchRecordRef, ResearchReferenceCatalog,
    ResearchState,
};
use crate::domain::retrieval::ScopedTaskRanker;

const MAX_CANDIDATES: usize = 256;
const MAX_DEPENDENCIES: usize = 512;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResearchTaskCandidate {
    pub(crate) reference: String,
    pub(crate) excerpt: String,
    pub(crate) truncated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResearchTaskUnit {
    pub(crate) key: String,
    pub(crate) reference: String,
    pub(crate) value: Value,
    pub(crate) dependencies: Vec<String>,
}

fn reference(key: &str, source_hash: &str) -> String {
    format!("research:{key}:{source_hash}")
}

fn key_from_reference<'a>(reference: &'a str, source_hash: &str) -> Result<&'a str, String> {
    let (key, revision) = reference
        .strip_prefix("research:")
        .and_then(|value| value.rsplit_once(':'))
        .ok_or_else(|| "invalid Research task reference".to_owned())?;
    if revision != source_hash {
        return Err("Research task reference has a different Project revision".to_owned());
    }
    Ok(key)
}

fn excerpt(text: &str, maximum: usize) -> &str {
    let mut end = text.len().min(maximum);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn research_key(reference: &ResearchRecordRef) -> String {
    match reference {
        ResearchRecordRef::Hypothesis(id) => format!("hypothesis:{id}"),
        ResearchRecordRef::Experiment(id) => format!("experiment:{id}"),
        ResearchRecordRef::Finding(id) => format!("finding:{id}"),
    }
}

fn evidence_dependencies(links: &[EvidenceLink]) -> Vec<String> {
    links
        .iter()
        .filter_map(|link| match &link.evidence {
            EvidenceRef::Experiment(id) => Some(format!("experiment:{id}")),
            EvidenceRef::Artifact(_) | EvidenceRef::ExecutionReceipt(_) => None,
        })
        .collect()
}

fn receipts<'a>(
    ids: impl IntoIterator<Item = &'a ExecutionReceiptId>,
    references: &ResearchReferenceCatalog,
) -> Result<Vec<Value>, String> {
    ids.into_iter()
        .map(|id| {
            references
                .execution_receipt(id)
                .map(|receipt| json!({"receipt_id":id, "receipt":receipt}))
                .ok_or_else(|| format!("Research Receipt `{id}` is absent"))
        })
        .collect()
}

fn evidence_receipts(
    links: &[EvidenceLink],
    references: &ResearchReferenceCatalog,
) -> Result<Vec<Value>, String> {
    receipts(
        links.iter().filter_map(|link| match &link.evidence {
            EvidenceRef::ExecutionReceipt(id) => Some(id),
            EvidenceRef::Artifact(_) | EvidenceRef::Experiment(_) => None,
        }),
        references,
    )
}

fn visit_navigation(
    state: &ResearchState,
    mut visit: impl FnMut(String, String) -> Result<(), String>,
) -> Result<(), String> {
    for record in state.retrieval_target_snapshot_records() {
        visit(
            format!("target_snapshot:{}", record.id),
            format!(
                "target snapshot {} {} {} {}",
                record.id,
                excerpt(&record.target_id, 256),
                excerpt(&record.locator, 256),
                excerpt(&record.version, 128)
            ),
        )?;
    }
    for record in state.hypothesis_records() {
        visit(
            format!("hypothesis:{}", record.id),
            format!(
                "hypothesis {} {} {} {}",
                record.id,
                excerpt(&record.title, 256),
                excerpt(&record.target_id, 256),
                excerpt(&record.attack_surface, 256)
            ),
        )?;
    }
    for record in state.retrieval_experiment_records() {
        visit(
            format!("experiment:{}", record.id),
            format!(
                "experiment {} hypothesis {} target {} {} {}",
                record.id,
                record.hypothesis_id,
                record.target_snapshot_id,
                excerpt(&record.goal, 256),
                excerpt(&record.notes, 256)
            ),
        )?;
    }
    for record in state.finding_records() {
        visit(
            format!("finding:{}", record.id),
            format!(
                "finding {} {} {} {}",
                record.id,
                excerpt(&record.title, 256),
                excerpt(&record.target_id, 256),
                excerpt(&record.impact, 256)
            ),
        )?;
    }
    for record in state.retrieval_next_action_records() {
        visit(
            format!("next_action:{}", record.id),
            format!("next action {} {}", record.id, excerpt(&record.action, 512)),
        )?;
    }
    Ok(())
}

fn full_unit(
    state: &ResearchState,
    references: &ResearchReferenceCatalog,
    source_hash: &str,
    key: &str,
) -> Result<ResearchTaskUnit, String> {
    let (kind, id) = key
        .split_once(':')
        .ok_or_else(|| "invalid Research task key".to_owned())?;
    let (value, mut dependencies) = match kind {
        "target_snapshot" => {
            let record = state
                .retrieval_target_snapshot_records()
                .find(|record| record.id.as_str() == id)
                .ok_or_else(|| format!("unknown Research task key `{key}`"))?;
            (
                json!({"kind":"target_snapshot", "record":record}),
                Vec::new(),
            )
        }
        "hypothesis" => {
            let record = state
                .hypothesis_records()
                .find(|record| record.id.as_str() == id)
                .ok_or_else(|| format!("unknown Research task key `{key}`"))?;
            let mut dependencies = evidence_dependencies(&record.evidence_links);
            dependencies.extend(
                record
                    .last_action
                    .iter()
                    .chain(&record.next_action)
                    .map(|id| format!("next_action:{id}")),
            );
            (
                json!({"kind":"hypothesis", "record":record, "receipts":evidence_receipts(&record.evidence_links, references)?}),
                dependencies,
            )
        }
        "experiment" => {
            let record = state
                .retrieval_experiment_records()
                .find(|record| record.id.as_str() == id)
                .ok_or_else(|| format!("unknown Research task key `{key}`"))?;
            let capsule = state
                .capsule_for_experiment(record)
                .ok_or_else(|| format!("Experiment `{}` has no Capsule", record.id))?;
            (
                json!({"kind":"experiment", "record":record, "capsule":capsule, "receipts":receipts(&capsule.execution_receipt_ids, references)?}),
                vec![
                    format!("hypothesis:{}", record.hypothesis_id),
                    format!("target_snapshot:{}", record.target_snapshot_id),
                ],
            )
        }
        "finding" => {
            let record = state
                .finding_records()
                .find(|record| record.id.as_str() == id)
                .ok_or_else(|| format!("unknown Research task key `{key}`"))?;
            let mut dependencies = evidence_dependencies(&record.evidence_links);
            dependencies.extend(
                record
                    .target_snapshot_ids
                    .iter()
                    .map(|id| format!("target_snapshot:{id}")),
            );
            (
                json!({"kind":"finding", "record":record, "receipts":evidence_receipts(&record.evidence_links, references)?}),
                dependencies,
            )
        }
        "next_action" => {
            let record = state
                .retrieval_next_action_records()
                .find(|record| record.id.as_str() == id)
                .ok_or_else(|| format!("unknown Research task key `{key}`"))?;
            (
                json!({"kind":"next_action", "record":record}),
                vec![research_key(&record.research_id)],
            )
        }
        _ => return Err(format!("unknown Research task kind `{kind}`")),
    };
    if matches!(kind, "hypothesis" | "experiment" | "finding") {
        dependencies.extend(
            state
                .retrieval_next_action_records()
                .filter(|record| {
                    research_key(&record.research_id) == key
                        && !matches!(
                            record.status,
                            crate::domain::research::NextActionStatus::Done
                        )
                })
                .map(|record| format!("next_action:{}", record.id)),
        );
    }
    dependencies.sort();
    dependencies.dedup();
    Ok(ResearchTaskUnit {
        key: key.to_owned(),
        reference: reference(key, source_hash),
        value,
        dependencies,
    })
}

pub(crate) fn candidates(
    state: &ResearchState,
    references: &ResearchReferenceCatalog,
    source_hash: &str,
    project_id: &str,
    query: &str,
    expanded: Option<&crate::domain::task_context::TaskSelectionDecision>,
) -> Result<(Vec<ResearchTaskCandidate>, usize), String> {
    let mut ranker = ScopedTaskRanker::new(project_id, query).map_err(|error| error.to_string())?;
    let mut total = 0usize;
    visit_navigation(state, |key, text| {
        total += 1;
        ranker
            .push(&reference(&key, source_hash), &text)
            .map_err(|error| error.to_string())
    })?;
    let pinned = expanded
        .into_iter()
        .flat_map(|decision| &decision.required_context_refs)
        .filter(|item| item.starts_with("research:"))
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut chosen = pinned.clone();
    for item in ranker.finish() {
        if chosen.len() >= MAX_CANDIDATES {
            break;
        }
        chosen.insert(item);
    }
    let mut found = BTreeMap::new();
    visit_navigation(state, |key, text| {
        let id = reference(&key, source_hash);
        if chosen.contains(&id) {
            let (text, truncated) = if pinned.contains(&id) {
                let rendered = full_unit(state, references, source_hash, &key)?
                    .value
                    .to_string();
                (excerpt(&rendered, 4096).to_owned(), rendered.len() > 4096)
            } else {
                (text, true)
            };
            found.insert(
                id.clone(),
                ResearchTaskCandidate {
                    reference: id,
                    excerpt: text,
                    truncated,
                },
            );
        }
        Ok(())
    })?;
    if pinned.iter().any(|id| !found.contains_key(id)) {
        return Err("expanded Research reference is absent".to_owned());
    }
    let omitted = total.saturating_sub(found.len());
    Ok((found.into_values().collect(), omitted))
}

pub(crate) fn resolve_selection(
    state: &ResearchState,
    references: &ResearchReferenceCatalog,
    source_hash: &str,
    selected: &[String],
) -> Result<Vec<ResearchTaskUnit>, String> {
    let mut pending = selected
        .iter()
        .map(|item| key_from_reference(item, source_hash).map(str::to_owned))
        .collect::<Result<Vec<_>, _>>()?;
    let mut units = BTreeMap::new();
    while let Some(key) = pending.pop() {
        if units.contains_key(&key) {
            continue;
        }
        if units.len() == MAX_DEPENDENCIES {
            return Err("Research dependency closure exceeds the bounded source plan".to_owned());
        }
        let unit = full_unit(state, references, source_hash, &key)?;
        pending.extend(unit.dependencies.iter().cloned());
        units.insert(key, unit);
    }
    Ok(units.into_values().collect())
}

pub(crate) fn close_research_selection<'a>(
    units: &'a [ResearchTaskUnit],
    selected: &[String],
) -> Result<Vec<&'a ResearchTaskUnit>, String> {
    let by_key = units
        .iter()
        .map(|unit| (unit.key.as_str(), unit))
        .collect::<BTreeMap<_, _>>();
    let by_reference = units
        .iter()
        .map(|unit| (unit.reference.as_str(), unit))
        .collect::<BTreeMap<_, _>>();
    if by_key.len() != units.len() || by_reference.len() != units.len() {
        return Err("duplicate Research task unit".to_owned());
    }
    let mut pending = selected
        .iter()
        .map(|reference| {
            by_reference
                .get(reference.as_str())
                .copied()
                .ok_or_else(|| format!("unknown Research task reference `{reference}`"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut closed = BTreeSet::new();
    while let Some(current) = pending.pop() {
        if !closed.insert(current.key.as_str()) {
            continue;
        }
        for dependency in &current.dependencies {
            pending.push(
                *by_key
                    .get(dependency.as_str())
                    .ok_or_else(|| format!("Research dependency `{dependency}` is absent"))?,
            );
        }
    }
    Ok(units
        .iter()
        .filter(|unit| closed.contains(unit.key.as_str()))
        .collect())
}
