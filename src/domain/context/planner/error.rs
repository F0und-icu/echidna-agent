//! Typed failures for Context construction and compilation.

use thiserror::Error;

use super::fragment::{ContextFragmentId, ContextFragmentSource};

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ContextPlannerError {
    #[error(transparent)]
    ModelCapability(#[from] crate::domain::model::ModelCapabilityError),
    #[error(
        "Context output reserve {reserved} is smaller than the requested output limit {requested}"
    )]
    OutputReserveBelowLimit { reserved: u32, requested: u32 },
    #[error("Context Fragment ID must not be empty")]
    EmptyFragmentId,
    #[error("Context content hash must be a lowercase 64-character BLAKE3 digest")]
    InvalidContentHash,
    #[error("Context Subitem ID does not match its Fragment and ordinal")]
    InvalidContextSubitemId,
    #[error("Context Subitem provenance ordinal is outside the Fragment content")]
    ContextSubitemOrdinalOutOfBounds,
    #[error("Context Subitem provenance is assigned more than once")]
    DuplicateContextSubitemProvenance,
    #[error("Context provenance field `{field}` must not be empty")]
    ContextProvenanceFieldRequired { field: &'static str },
    #[error("Context reference list `{field}` repeats `{value}`")]
    DuplicateContextReference { field: &'static str, value: String },
    #[error("fact-reference Context provenance requires at least one typed reference")]
    FactReferenceMissingContextReference,
    #[error("Context semantic class and authority level are inconsistent")]
    InvalidContextAuthorityCombination,
    #[error("explicit Context provenance slot does not match Fragment placement")]
    ContextProvenanceSlotMismatch,
    #[error("Context Fragment/Subitem identity appears more than once in one delivery map")]
    DuplicateContextSubitem,
    #[error("Context byte or ContentItem position exceeds the schema bound")]
    ContextPositionOverflow,
    #[error("ContextDeliveryMap schema or renderer is unsupported")]
    UnsupportedContextDeliveryMap,
    #[error("ContextDeliveryMap does not exactly cover the rendered instructions and items")]
    ContextDeliveryCoverageMismatch,
    #[error("ContextDeliveryMap instruction gaps are not generic-v1 separators")]
    ContextDeliverySeparatorMismatch,
    #[error("ContextDeliveryMap content hash does not match the rendered component")]
    ContextDeliveryContentHashMismatch,
    #[error("ContextDeliveryMap cannot be projected onto the supplied Provider delivery")]
    ContextDeliveryProjectionMismatch,
    #[error("Context transform is inconsistent with its channel or provenance")]
    InvalidContextTransform,
    #[error("Context source `{fragment_source:?}` has the wrong ModelInputIR channel")]
    InvalidFragmentChannel {
        fragment_source: ContextFragmentSource,
    },
    #[error("instruction source `{fragment_source:?}` cannot fall back to a user Artifact item")]
    InstructionArtifactReference {
        fragment_source: ContextFragmentSource,
    },
    #[error(
        "context budget reserves {reserved_output_tokens} output and {reserved_tool_loop_tokens} tool-loop tokens from a {context_window_tokens}-token window"
    )]
    InvalidBudget {
        context_window_tokens: u32,
        reserved_output_tokens: u32,
        reserved_tool_loop_tokens: u32,
    },
    #[error(
        "Context budget window {requested_tokens} exceeds the model capability window {capability_tokens}"
    )]
    BudgetExceedsModelContext {
        requested_tokens: u32,
        capability_tokens: u32,
    },
    #[error(
        "fixed Context input needs {fixed_input_tokens} tokens but the input budget is {input_token_limit}"
    )]
    FixedInputExceedsBudget {
        fixed_input_tokens: u32,
        input_token_limit: u32,
    },
    #[error("Context Fragment ID `{id}` appears more than once")]
    DuplicateFragmentId { id: ContextFragmentId },
    #[error(
        "required Context fragments need {required_tokens} tokens but the input budget is {input_token_limit}"
    )]
    RequiredFragmentsExceedBudget {
        required_tokens: u64,
        input_token_limit: u32,
    },
    #[error("Context input violates History Tool pairing: {message}")]
    InvalidHistoryPairing { message: String },
    #[error("failed to encode a canonical Context component: {message}")]
    CanonicalEncoding { message: String },
    #[error("Context truncation cannot fit into {retained_tokens} tokens")]
    TruncationUnavailable { retained_tokens: u32 },
    #[error(
        "rendered Context input needs {rendered_input_tokens} tokens but the input budget is {input_token_limit}"
    )]
    RenderedInputExceedsBudget {
        rendered_input_tokens: u32,
        input_token_limit: u32,
    },
}
