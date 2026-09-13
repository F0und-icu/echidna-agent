//! Fixed pre-Receipt Blob publication. Never converts persistence into Tool failure.

use super::{AgentLoop, AgentLoopError};
use crate::domain::tools::{ToolExecutionOutcome, validate_captures};
use crate::ports::{artifact::ArtifactStore, model::ModelProvider, tool::ToolExecutor};

pub(super) type OutputPublisher<'a> =
    dyn Fn(&mut ToolExecutionOutcome) -> Result<(), AgentLoopError> + 'a;

impl<'provider, 'tools, Provider: ModelProvider + ?Sized, Tools: ToolExecutor + ?Sized>
    AgentLoop<'provider, 'tools, Provider, Tools>
{
    /// Uses the configured project's existing Blob Port before any Receipt.
    pub fn with_output_store<S>(mut self, store: S) -> Self
    where
        S: ArtifactStore + 'tools,
        S::Error: std::fmt::Display,
    {
        self.output_publisher = Some(Box::new(move |outcome| publish(&store, outcome)));
        self
    }
}

fn publish<S: ArtifactStore>(
    store: &S,
    outcome: &mut ToolExecutionOutcome,
) -> Result<(), AgentLoopError>
where
    S::Error: std::fmt::Display,
{
    if outcome.pending_captures.is_empty() {
        outcome.receipt.validate_output_captures(&outcome.result)?;
        return Ok(());
    }
    if !outcome.receipt.output_captures.is_empty()
        || outcome.result.output.get("output_captures").is_some()
    {
        return Err(AgentLoopError::ToolOutputPublication {
            message: "capture references already assigned".to_owned(),
        });
    }
    let mut captures = Vec::with_capacity(outcome.pending_captures.len());
    for capture in &outcome.pending_captures {
        let blob = store.publish_bytes(&capture.bytes).map_err(|error| {
            AgentLoopError::ToolOutputPublication {
                message: error.to_string(),
            }
        })?;
        if blob.size_bytes != capture.bytes.len() as u64 {
            return Err(AgentLoopError::ToolOutputPublication {
                message: "published size mismatch".to_owned(),
            });
        }
        captures.push(capture.published(blob.blob_id));
    }
    validate_captures(&captures)?;
    let output = outcome.result.output.as_object_mut().ok_or_else(|| {
        AgentLoopError::ToolOutputPublication {
            message: "capture result must be an object".to_owned(),
        }
    })?;
    output.insert(
        "output_captures".to_owned(),
        serde_json::to_value(&captures).expect("typed capture serialization"),
    );
    outcome.receipt.output_captures = captures;
    outcome.pending_captures.clear();
    outcome.receipt.validate_output_captures(&outcome.result)?;
    Ok(())
}
