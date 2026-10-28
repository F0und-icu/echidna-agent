//! Exact, independently verifiable Session and Research compaction sources.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::canonical_json::canonical_blake3_hash;
use crate::domain::events::{EventEnvelope, EventSequence, StreamId};
use crate::domain::project::ProjectId;
use crate::domain::research::{ResearchReducer, ResearchReferenceCatalog, ResearchState};
use crate::domain::session::{SessionError, SessionReducer, SessionState};

use super::ResearchStreamIdentity;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionSourceIdentity {
    project_id: ProjectId,
    stream_id: StreamId,
    watermark: EventSequence,
    stream_hash: String,
}

impl SessionSourceIdentity {
    pub(crate) fn from_events(
        project_id: ProjectId,
        events: &[EventEnvelope<Value>],
    ) -> Result<(Self, SessionState), SessionError> {
        let state = SessionReducer.reduce(events.iter().cloned())?;
        let last = events
            .last()
            .ok_or(SessionError::CompactionSourceSequenceMissing)?;
        let stream_id = state
            .stream_id()
            .ok_or(SessionError::CompactionSourceSequenceMissing)?
            .clone();
        let stream_hash = canonical_blake3_hash(&events).map_err(|error| {
            SessionError::CompactionSourceInvalid {
                message: error.to_string(),
            }
        })?;
        Ok((
            Self {
                project_id,
                stream_id,
                watermark: last.sequence,
                stream_hash,
            },
            state,
        ))
    }

    pub(crate) fn project_id(&self) -> &ProjectId {
        &self.project_id
    }
    pub(crate) fn stream_id(&self) -> &StreamId {
        &self.stream_id
    }
    pub(crate) fn watermark(&self) -> EventSequence {
        self.watermark
    }
    pub(crate) fn stream_hash(&self) -> &str {
        &self.stream_hash
    }

    pub(crate) fn validate(&self) -> Result<(), SessionError> {
        if self.project_id.as_str().trim().is_empty()
            || self
                .stream_id
                .as_str()
                .strip_prefix("session:")
                .is_none_or(str::is_empty)
            || self.watermark.get() == 0
            || !is_hash(&self.stream_hash)
        {
            return Err(SessionError::CompactionSourceInvalid {
                message: "invalid Session source identity".to_owned(),
            });
        }
        Ok(())
    }
}

/// Empty is a verified empty Research stream, not a failed or missing read.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ResearchSourceIdentity {
    Empty {
        project_id: ProjectId,
        stream_hash: String,
    },
    Nonempty {
        source: ResearchStreamIdentity,
    },
}

impl ResearchSourceIdentity {
    pub(crate) fn from_events(
        project_id: ProjectId,
        events: &[EventEnvelope<Value>],
    ) -> Result<Self, SessionError> {
        if events.is_empty() {
            let stream_hash = canonical_blake3_hash(&events).map_err(|error| {
                SessionError::CompactionSourceInvalid {
                    message: error.to_string(),
                }
            })?;
            Ok(Self::Empty {
                project_id,
                stream_hash,
            })
        } else {
            Ok(Self::Nonempty {
                source: ResearchStreamIdentity::from_events(project_id, events)?,
            })
        }
    }

    pub(crate) fn validate(&self, project_id: &ProjectId) -> Result<(), SessionError> {
        match self {
            Self::Empty {
                project_id: actual,
                stream_hash,
            } => {
                let empty: &[EventEnvelope<Value>] = &[];
                let expected = canonical_blake3_hash(&empty).map_err(|error| {
                    SessionError::CompactionSourceInvalid {
                        message: error.to_string(),
                    }
                })?;
                if actual != project_id || stream_hash != &expected {
                    return Err(SessionError::CompactionSourceInvalid {
                        message: "invalid empty Research source".to_owned(),
                    });
                }
            }
            Self::Nonempty { source } if source.project_id() == project_id => source.validate()?,
            Self::Nonempty { .. } => {
                return Err(SessionError::CompactionSourceInvalid {
                    message: "Research source belongs to another Project".to_owned(),
                });
            }
        }
        Ok(())
    }
}

/// Proof that this exact Research source was reduced against the caller's
/// strictly rebuilt real Session Receipt catalog, including the empty case.
pub(crate) struct ValidatedResearchSource {
    identity: ResearchSourceIdentity,
    state: ResearchState,
}

impl ValidatedResearchSource {
    pub(crate) fn from_events(
        project_id: ProjectId,
        events: &[EventEnvelope<Value>],
        references: &ResearchReferenceCatalog,
    ) -> Result<Self, SessionError> {
        let state = ResearchReducer
            .reduce_with_references(events.iter().cloned(), references)
            .map_err(|error| SessionError::CompactionSourceInvalid {
                message: error.to_string(),
            })?;
        let identity = ResearchSourceIdentity::from_events(project_id, events)?;
        Ok(Self { identity, state })
    }

    pub(crate) fn identity(&self) -> &ResearchSourceIdentity {
        &self.identity
    }
    pub(crate) fn state(&self) -> &ResearchState {
        &self.state
    }
}

pub(super) fn is_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}
