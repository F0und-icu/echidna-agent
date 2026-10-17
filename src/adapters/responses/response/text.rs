//! Response-local final-text reconciliation, never a History or retry owner.

use serde_json::Value;

use crate::domain::model::ModelEvent;
use crate::ports::model::ModelProviderError;

use super::protocol_error;

#[derive(Default)]
pub(super) struct TextOutput {
    parts: Vec<Part>,
    frontier: Option<(usize, usize)>,
    unindexed_barrier: bool,
}

struct Part {
    position: Option<(usize, usize)>,
    item_id: Option<String>,
    text: String,
    finalized: bool,
    sealed: bool,
}

#[derive(Clone, Copy)]
enum SnapshotKind {
    OutputText,
    ReasoningSummary,
}

impl SnapshotKind {
    fn item_type(self) -> &'static str {
        match self {
            Self::OutputText => "message",
            Self::ReasoningSummary => "reasoning",
        }
    }

    fn parts_field(self) -> &'static str {
        match self {
            Self::OutputText => "content",
            Self::ReasoningSummary => "summary",
        }
    }

    fn part_type(self) -> &'static str {
        match self {
            Self::OutputText => "output_text",
            Self::ReasoningSummary => "summary_text",
        }
    }

    fn missing_position(self) -> &'static str {
        match self {
            Self::OutputText => "completed output did not identify streamed text",
            Self::ReasoningSummary => {
                "completed output did not identify streamed reasoning summary"
            }
        }
    }

    fn missing_part(self) -> &'static str {
        match self {
            Self::OutputText => "completed output omitted streamed text",
            Self::ReasoningSummary => "completed output omitted streamed reasoning summary",
        }
    }

    fn changed_identity(self) -> &'static str {
        match self {
            Self::OutputText => "completed output changed text item identity",
            Self::ReasoningSummary => "completed output changed reasoning summary item identity",
        }
    }

    fn changed_text(self) -> &'static str {
        match self {
            Self::OutputText => "completed output changed streamed text",
            Self::ReasoningSummary => "completed output changed streamed reasoning summary",
        }
    }
}

impl TextOutput {
    pub(super) fn validate_snapshot(&self, output: &[Value]) -> Result<(), ModelProviderError> {
        self.validate_snapshot_kind(output, SnapshotKind::OutputText)
    }

    pub(super) fn validate_reasoning_summary_snapshot(
        &self,
        output: &[Value],
    ) -> Result<(), ModelProviderError> {
        self.validate_snapshot_kind(output, SnapshotKind::ReasoningSummary)
    }

    fn validate_snapshot_kind(
        &self,
        output: &[Value],
        kind: SnapshotKind,
    ) -> Result<(), ModelProviderError> {
        for part in &self.parts {
            let Some((index, part_index)) = part.position else {
                return Err(protocol_error(kind.missing_position()));
            };
            let item = output
                .get(index)
                .ok_or_else(|| protocol_error(kind.missing_part()))?;
            if item.get("type").and_then(Value::as_str) != Some(kind.item_type())
                || part
                    .item_id
                    .as_deref()
                    .is_some_and(|id| item.get("id").and_then(Value::as_str) != Some(id))
            {
                return Err(protocol_error(kind.changed_identity()));
            }
            let snapshot_part = item
                .get(kind.parts_field())
                .and_then(Value::as_array)
                .and_then(|parts| parts.get(part_index))
                .ok_or_else(|| protocol_error(kind.missing_part()))?;
            if snapshot_part.get("type").and_then(Value::as_str) != Some(kind.part_type())
                || snapshot_part.get("text").and_then(Value::as_str) != Some(part.text.as_str())
            {
                return Err(protocol_error(kind.changed_text()));
            }
        }
        Ok(())
    }

    pub(super) fn barrier(&mut self, output_index: Option<usize>) {
        for part in &mut self.parts {
            part.sealed = true;
        }
        if let Some(index) = output_index {
            self.frontier = Some(self.frontier.unwrap_or_default().max((index, usize::MAX)));
        } else {
            self.unindexed_barrier = true;
        }
    }

    pub(super) fn text(
        &mut self,
        output: Option<usize>,
        content: Option<usize>,
        item_id: Option<&str>,
        text: &str,
        final_text: bool,
    ) -> Result<Vec<ModelEvent>, ModelProviderError> {
        self.text_suffix(output, content, item_id, text, final_text)
            .map(|suffix| {
                suffix
                    .map(|text| vec![ModelEvent::TextDelta { text }])
                    .unwrap_or_default()
            })
    }

    pub(super) fn text_suffix(
        &mut self,
        output: Option<usize>,
        content: Option<usize>,
        item_id: Option<&str>,
        text: &str,
        final_text: bool,
    ) -> Result<Option<String>, ModelProviderError> {
        let position = match (output, content) {
            (Some(o), Some(c)) => Some((o, c)),
            (None, None) => None,
            _ => return Err(protocol_error("text event has incomplete position")),
        };
        if let Some(id) = item_id
            && self.parts.iter().any(|p| {
                p.item_id
                    .as_deref()
                    .zip(p.position.zip(position))
                    .is_some_and(|(prior_id, (a, b))| (prior_id == id) != (a.0 == b.0))
            })
        {
            return Err(protocol_error("text item identity changed output position"));
        }
        let index = self.parts.iter().position(|p| p.position == position);
        let index = if let Some(index) = index {
            index
        } else if self.parts.len() == 1 && self.parts[0].position.is_none() {
            self.parts[0].position = position;
            0
        } else {
            if position.is_none() && !self.parts.is_empty() {
                return Err(protocol_error("ambiguous unindexed text event"));
            }
            if position.zip(self.frontier).is_some_and(|(p, f)| p <= f)
                || (final_text && self.unindexed_barrier)
            {
                return Err(protocol_error("final text would reorder prior output"));
            }
            for part in &mut self.parts {
                part.sealed = true;
            }
            self.parts.push(Part {
                position,
                item_id: None,
                text: String::new(),
                finalized: false,
                sealed: false,
            });
            self.parts.len() - 1
        };
        let part = &mut self.parts[index];
        if let Some(id) = item_id {
            if part.item_id.as_deref().is_some_and(|prior| prior != id) {
                return Err(protocol_error("text item identity changed"));
            }
            part.item_id = Some(id.to_owned());
        }
        let suffix = if final_text {
            if part.finalized && part.text != text {
                return Err(protocol_error("final text changed after finalization"));
            }
            text.strip_prefix(&part.text)
                .ok_or_else(|| protocol_error("final text conflicts with streamed text"))?
        } else {
            if part.finalized {
                return Err(protocol_error("text delta follows finalization"));
            }
            text
        };
        if !suffix.is_empty() && part.sealed {
            return Err(protocol_error("final text would reorder prior output"));
        }
        let suffix = (!suffix.is_empty()).then(|| suffix.to_owned());
        if final_text {
            part.text = text.to_owned();
            part.finalized = true;
        } else {
            part.text.push_str(text);
        }
        if let Some(position) = position {
            self.frontier = Some(self.frontier.unwrap_or_default().max(position));
        }
        Ok(suffix)
    }

    pub(super) fn part(
        &mut self,
        output: Option<usize>,
        content: Option<usize>,
        id: Option<&str>,
        part: &Value,
    ) -> Result<Vec<ModelEvent>, ModelProviderError> {
        if part.get("type").and_then(Value::as_str) != Some("output_text") {
            return Err(protocol_error("unsupported completed message content"));
        }
        let text = part
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| protocol_error("completed text omitted string `text`"))?;
        self.text(output, content, id, text, true)
    }

    pub(super) fn message(
        &mut self,
        output: Option<usize>,
        item: &Value,
    ) -> Result<Vec<ModelEvent>, ModelProviderError> {
        if item.get("status").is_some_and(|s| s != "completed") {
            return Err(protocol_error("completed message has non-completed status"));
        }
        let content = item
            .get("content")
            .and_then(Value::as_array)
            .ok_or_else(|| protocol_error("completed message omitted content array"))?;
        let mut events = Vec::new();
        for (index, part) in content.iter().enumerate() {
            if output.is_none() && content.len() > 1 {
                return Err(protocol_error("ambiguous unindexed message parts"));
            }
            events.extend(self.part(
                output,
                output.map(|_| index),
                item.get("id").and_then(Value::as_str),
                part,
            )?);
        }
        Ok(events)
    }
}
