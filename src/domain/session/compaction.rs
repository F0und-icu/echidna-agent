//! Reference-only Session Summary and Compaction fact data.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::domain::canonical_json::canonical_blake3_hash;
use crate::domain::events::{EventSequence, EventType};
use crate::domain::history::{ConversationHistory, retain_recent_complete_history};
use crate::domain::model::ContentItem;

use super::{COMPACTION_CREATED_EVENT_TYPE, SessionError};

mod evidence;
mod summary;

pub use evidence::{CompactionValidationContext, EvidenceSpine, ResearchStreamIdentity};
pub use summary::{NarrativeSummary, SessionSummary, SessionSummaryDraft};

pub(super) const CURRENT_SESSION_SUMMARY_SCHEMA_VERSION: u16 = 2;

/// One append-only Compaction fact. Original Session History remains durable.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionCompaction {
    schema_version: u16,
    summary: SessionSummary,
    source_history_item_count: usize,
    source_event_sequence: EventSequence,
    retained_from_normalized_history_index: usize,
    retained_history: Vec<ContentItem>,
}

impl SessionCompaction {
    pub(super) fn from_history(
        history: &ConversationHistory,
        summary: SessionSummary,
        source_event_sequence: EventSequence,
        requested_recent_items: usize,
    ) -> Result<Self, SessionError> {
        let (retained_from_normalized_history_index, retained_history) =
            retain_recent_complete_history(history.items(), requested_recent_items)?;
        Ok(Self {
            schema_version: summary.schema_version(),
            summary,
            source_history_item_count: history.items().len(),
            source_event_sequence,
            retained_from_normalized_history_index,
            retained_history,
        })
    }

    pub fn summary(&self) -> &SessionSummary {
        &self.summary
    }

    pub const fn source_history_item_count(&self) -> usize {
        self.source_history_item_count
    }

    pub const fn source_event_sequence(&self) -> EventSequence {
        self.source_event_sequence
    }

    pub const fn retained_from_normalized_history_index(&self) -> usize {
        self.retained_from_normalized_history_index
    }

    pub fn retained_history(&self) -> &[ContentItem] {
        &self.retained_history
    }

    pub(super) fn validate_shape(&self) -> Result<(), SessionError> {
        if self.schema_version != self.summary.schema_version()
            || !matches!(
                self.schema_version,
                1 | CURRENT_SESSION_SUMMARY_SCHEMA_VERSION
            )
        {
            return Err(SessionError::UnsupportedPayloadSchema {
                event_type: EventType::new(COMPACTION_CREATED_EVENT_TYPE),
                schema_version: self.schema_version,
            });
        }
        self.summary.validate_shape()?;
        ConversationHistory::from_complete_content_items(&self.retained_history)?;
        Ok(())
    }

    pub(crate) fn validate_references(
        &self,
        context: &CompactionValidationContext<'_>,
    ) -> Result<(), SessionError> {
        self.summary.validate_references(context)
    }

    pub(crate) fn fingerprint(&self) -> Result<String, SessionError> {
        fingerprint_value(self)
    }
}

pub(super) fn fingerprint_value(value: &impl Serialize) -> Result<String, SessionError> {
    canonical_blake3_hash(value).map_err(|error| SessionError::PayloadEncode {
        event_type: EventType::new(COMPACTION_CREATED_EVENT_TYPE),
        message: error.to_string(),
    })
}

pub(super) fn validate_unique_ids<'a>(
    field: &'static str,
    values: impl IntoIterator<Item = &'a str>,
) -> Result<(), SessionError> {
    let mut seen = BTreeSet::new();
    for value in values {
        require_text(field, value)?;
        if !seen.insert(value) {
            return Err(SessionError::CompactionSummaryEntryDuplicate {
                field,
                value: value.to_owned(),
            });
        }
    }
    Ok(())
}

pub(super) fn require_text(field: &'static str, value: &str) -> Result<(), SessionError> {
    if value.trim().is_empty() {
        Err(SessionError::CompactionSummaryFieldRequired { field })
    } else {
        Ok(())
    }
}
