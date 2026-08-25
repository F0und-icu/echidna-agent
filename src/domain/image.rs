//! Pure local-image input limits and deterministic resize planning.
//!
//! Filesystem access, decoder errors and output bytes belong to the image
//! Adapter. This module intentionally only owns the versioned semantic limits
//! and the target-dimension calculation that is recorded with a derivative.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The only image formats accepted by the first local-image path.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageFormat {
    Png,
    Jpeg,
}

impl ImageFormat {
    pub const fn media_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
        }
    }
}

/// Hard limits for accepting an original local image.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ImageInputLimits {
    pub max_input_bytes: u64,
    pub max_width: u32,
    pub max_height: u32,
    pub max_pixels: u64,
}

impl ImageInputLimits {
    pub const fn new(
        max_input_bytes: u64,
        max_width: u32,
        max_height: u32,
        max_pixels: u64,
    ) -> Self {
        Self {
            max_input_bytes,
            max_width,
            max_height,
            max_pixels,
        }
    }

    pub fn validate(self) -> Result<(), ImageContractError> {
        if self.max_input_bytes == 0
            || self.max_width == 0
            || self.max_height == 0
            || self.max_pixels == 0
        {
            return Err(ImageContractError::InvalidLimit);
        }
        Ok(())
    }
}

/// A delivery budget for a derived image. The original image is never
/// transformed in place.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ImageDeliveryBudget {
    pub max_output_bytes: u64,
    pub max_width: u32,
    pub max_height: u32,
    pub max_pixels: u64,
}

impl ImageDeliveryBudget {
    pub const fn new(
        max_output_bytes: u64,
        max_width: u32,
        max_height: u32,
        max_pixels: u64,
    ) -> Self {
        Self {
            max_output_bytes,
            max_width,
            max_height,
            max_pixels,
        }
    }

    pub fn validate(self) -> Result<(), ImageContractError> {
        if self.max_output_bytes == 0
            || self.max_width == 0
            || self.max_height == 0
            || self.max_pixels == 0
        {
            return Err(ImageContractError::InvalidBudget);
        }
        Ok(())
    }

    /// Calculates one deterministic aspect-ratio preserving target. Rounding
    /// always floors after limiting, and dimensions never become zero.
    pub fn target_dimensions(
        self,
        source_width: u32,
        source_height: u32,
    ) -> Result<(u32, u32), ImageContractError> {
        self.validate()?;
        if source_width == 0 || source_height == 0 {
            return Err(ImageContractError::InvalidDimensions);
        }
        let source_pixels = u64::from(source_width) * u64::from(source_height);
        let width_scale = self.max_width as f64 / source_width as f64;
        let height_scale = self.max_height as f64 / source_height as f64;
        let pixel_scale = (self.max_pixels as f64 / source_pixels as f64).sqrt();
        let scale = width_scale.min(height_scale).min(pixel_scale).min(1.0);
        let width = ((source_width as f64 * scale).floor() as u32).max(1);
        let height = ((source_height as f64 * scale).floor() as u32).max(1);
        Ok((width, height))
    }
}

/// Decoder-derived metadata after the Adapter has verified the original bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ValidatedImageMetadata {
    pub format: ImageFormat,
    pub width: u32,
    pub height: u32,
    pub size_bytes: u64,
}

impl ValidatedImageMetadata {
    pub fn new(
        format: ImageFormat,
        width: u32,
        height: u32,
        size_bytes: u64,
        limits: ImageInputLimits,
    ) -> Result<Self, ImageContractError> {
        limits.validate()?;
        if size_bytes > limits.max_input_bytes {
            return Err(ImageContractError::InputTooLarge {
                limit: limits.max_input_bytes,
                actual: size_bytes,
            });
        }
        if width == 0 || height == 0 {
            return Err(ImageContractError::InvalidDimensions);
        }
        if width > limits.max_width || height > limits.max_height {
            return Err(ImageContractError::DimensionsTooLarge {
                max_width: limits.max_width,
                max_height: limits.max_height,
                width,
                height,
            });
        }
        let pixels = u64::from(width) * u64::from(height);
        if pixels > limits.max_pixels {
            return Err(ImageContractError::PixelCountTooLarge {
                limit: limits.max_pixels,
                actual: pixels,
            });
        }
        Ok(Self {
            format,
            width,
            height,
            size_bytes,
        })
    }

    pub fn pixels(self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    pub fn requires_derivative(
        self,
        budget: ImageDeliveryBudget,
    ) -> Result<bool, ImageContractError> {
        budget.validate()?;
        Ok(self.size_bytes > budget.max_output_bytes
            || self.width > budget.max_width
            || self.height > budget.max_height
            || self.pixels() > budget.max_pixels)
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ImageContractError {
    #[error("image input limits must be non-zero")]
    InvalidLimit,
    #[error("image delivery budget must be non-zero")]
    InvalidBudget,
    #[error("image dimensions must be non-zero")]
    InvalidDimensions,
    #[error("image input is {actual} bytes, exceeding the {limit}-byte limit")]
    InputTooLarge { limit: u64, actual: u64 },
    #[error("image dimensions {width}x{height} exceed the {max_width}x{max_height} limit")]
    DimensionsTooLarge {
        max_width: u32,
        max_height: u32,
        width: u32,
        height: u32,
    },
    #[error("image has {actual} pixels, exceeding the {limit}-pixel limit")]
    PixelCountTooLarge { limit: u64, actual: u64 },
}
