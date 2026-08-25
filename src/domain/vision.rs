//! Versioned, provider-neutral Vision facts and fixed image-Tool handoff.
//!
//! A Vision observation is deliberately not a Research `EvidenceRef` or a
//! Finding. Submodules separate persisted facts from the model-facing
//! `view_image` result contract because they evolve for different reasons.

mod facts;
mod view_image;

pub use facts::{
    VISION_FACT_SCHEMA_VERSION, VisionAnalysisFailure, VisionFactError, VisionObservation,
    VisionObservationId, VisionRegion, VisionRouteRecord,
};
pub use view_image::{
    VIEW_IMAGE_SELECTION_SCHEMA_VERSION, ViewImageSelection, ViewImageSelectionError,
};
