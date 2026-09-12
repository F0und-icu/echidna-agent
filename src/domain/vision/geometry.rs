//! One pixel-frame contract for newly produced secondary observations.

use serde::{Deserialize, Serialize};

use super::{VisionFactError, VisionRegion};
use crate::domain::artifacts::ArtifactId;

/// Region coordinates are pixels in `delivery_artifact_id`, with a top-left
/// origin. Dimensions describe bytes, not a model's estimate or device screen.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VisionImageGeometry {
    pub delivery_artifact_id: ArtifactId,
    pub source_width: u32,
    pub source_height: u32,
    pub delivery_width: u32,
    pub delivery_height: u32,
}

impl VisionImageGeometry {
    pub(crate) fn validate(
        &self,
        source: &ArtifactId,
        derived: &[ArtifactId],
        regions: &[VisionRegion],
    ) -> Result<(), VisionFactError> {
        if self.source_width == 0
            || self.source_height == 0
            || self.delivery_width == 0
            || self.delivery_height == 0
            || self.delivery_width > self.source_width
            || self.delivery_height > self.source_height
            || self.delivery_artifact_id.as_str().trim().is_empty()
            || (self.delivery_artifact_id != *source
                && !derived.contains(&self.delivery_artifact_id))
            || (self.delivery_artifact_id == *source
                && (self.delivery_width != self.source_width
                    || self.delivery_height != self.source_height))
        {
            return Err(VisionFactError::InvalidImageGeometry);
        }
        for (index, region) in regions.iter().enumerate() {
            region.validate()?;
            if u64::from(region.x) + u64::from(region.width) > u64::from(self.delivery_width)
                || u64::from(region.y) + u64::from(region.height) > u64::from(self.delivery_height)
            {
                return Err(VisionFactError::RegionOutOfBounds { index });
            }
        }
        Ok(())
    }

    /// Exact request metadata, also consumed by read-only Replay. This is not
    /// a second Prompt renderer: ContextPlanner owns placement and rendering.
    pub(crate) fn context_text(&self, source: &ArtifactId) -> String {
        crate::domain::canonical_json::canonical_json(&serde_json::json!({
            "kind": "vision_image_geometry",
            "schema_version": 1,
            "source_artifact_id": source,
            "region_coordinate_space": "delivery_pixels",
            "image_geometry": self,
        }))
        .to_string()
    }
}
