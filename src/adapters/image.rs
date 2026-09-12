//! Local PNG/JPEG validation and deterministic budgeted derivative encoding.

use std::fs;
use std::path::Path;

use image::ImageFormat as DecoderFormat;
use image::ImageReader;
use thiserror::Error;

use crate::domain::artifacts::ArtifactId;
use crate::domain::image::{
    ImageContractError, ImageDeliveryBudget, ImageFormat, ImageInputLimits, ValidatedImageMetadata,
};
use crate::domain::vision::{ViewImageSelection, VisionImageGeometry};
use crate::ports::artifact::ArtifactStore;

pub(crate) const LOCAL_IMAGE_INPUT_LIMITS: ImageInputLimits =
    ImageInputLimits::new(32 * 1024 * 1024, 12_000, 12_000, 64_000_000);

const DERIVATIVE_FORMAT: ImageFormat = ImageFormat::Png;
const MAX_ENCODE_ATTEMPTS: usize = 32;

/// Exact original and, only where necessary, derived bytes for model delivery.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedLocalImage {
    pub original: PreparedImageBytes,
    pub derivative: Option<PreparedImageBytes>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedImageBytes {
    pub metadata: ValidatedImageMetadata,
    pub bytes: Vec<u8>,
}

impl PreparedLocalImage {
    pub fn delivery(&self) -> &PreparedImageBytes {
        self.derivative.as_ref().unwrap_or(&self.original)
    }

    pub fn transform(&self) -> Option<&'static str> {
        self.derivative.as_ref().map(|_| "image_budget_v1")
    }
}

/// Reads and validates one ordinary local PNG/JPEG without following a
/// symlink. The caller decides how the returned bytes become Artifact facts.
pub fn prepare_local_image(
    path: &Path,
    limits: ImageInputLimits,
    budget: ImageDeliveryBudget,
) -> Result<PreparedLocalImage, LocalImageError> {
    limits.validate()?;
    budget.validate()?;
    let metadata = fs::symlink_metadata(path).map_err(LocalImageError::Metadata)?;
    if metadata.file_type().is_symlink() {
        return Err(LocalImageError::SymlinkRejected);
    }
    if !metadata.file_type().is_file() {
        return Err(LocalImageError::NonRegularFile);
    }
    if metadata.len() > limits.max_input_bytes {
        return Err(ImageContractError::InputTooLarge {
            limit: limits.max_input_bytes,
            actual: metadata.len(),
        }
        .into());
    }
    let bytes = fs::read(path).map_err(LocalImageError::Read)?;
    let original_metadata = inspect_image_bytes(&bytes, limits)?;
    let format = original_metadata.format;
    let (width, height) = (original_metadata.width, original_metadata.height);
    // Read only the format header before allocating the decoded pixel buffer.
    // Local images are untrusted research inputs, so the declared dimensions
    // and pixel count must fail the Domain contract before a full decode.
    let mut reader = ImageReader::with_format(std::io::Cursor::new(&bytes), decoder_format(format));
    let mut decoder_limits = image::Limits::default();
    decoder_limits.max_image_width = Some(limits.max_width);
    decoder_limits.max_image_height = Some(limits.max_height);
    reader.limits(decoder_limits);
    let decoded = reader
        .decode()
        .map_err(|error| LocalImageError::CorruptImage {
            message: error.to_string(),
        })?;
    if decoded.width() != width || decoded.height() != height {
        return Err(LocalImageError::DimensionMismatch {
            header_width: width,
            header_height: height,
            decoded_width: decoded.width(),
            decoded_height: decoded.height(),
        });
    }
    let original = PreparedImageBytes {
        metadata: original_metadata,
        bytes,
    };
    if !original_metadata.requires_derivative(budget)? {
        return Ok(PreparedLocalImage {
            original,
            derivative: None,
        });
    }
    let (mut width, mut height) = budget.target_dimensions(decoded.width(), decoded.height())?;
    for _ in 0..MAX_ENCODE_ATTEMPTS {
        let resized = decoded.resize_exact(width, height, image::imageops::FilterType::Lanczos3);
        let mut derived_bytes = Vec::new();
        resized
            .write_to(
                &mut std::io::Cursor::new(&mut derived_bytes),
                DecoderFormat::Png,
            )
            .map_err(|error| LocalImageError::DerivativeEncode {
                message: error.to_string(),
            })?;
        let derived_metadata = ValidatedImageMetadata::new(
            DERIVATIVE_FORMAT,
            width,
            height,
            u64::try_from(derived_bytes.len()).expect("usize always fits u64"),
            ImageInputLimits::new(
                u64::MAX,
                budget.max_width,
                budget.max_height,
                budget.max_pixels,
            ),
        )?;
        if derived_metadata.size_bytes <= budget.max_output_bytes {
            return Ok(PreparedLocalImage {
                original,
                derivative: Some(PreparedImageBytes {
                    metadata: derived_metadata,
                    bytes: derived_bytes,
                }),
            });
        }
        if width == 1 && height == 1 {
            break;
        }
        width = ((u64::from(width) * 3) / 4).max(1) as u32;
        height = ((u64::from(height) * 3) / 4).max(1) as u32;
    }
    Err(LocalImageError::OutputBudgetUnreachable {
        limit: budget.max_output_bytes,
    })
}

fn inspect_image_bytes(
    bytes: &[u8],
    limits: ImageInputLimits,
) -> Result<ValidatedImageMetadata, LocalImageError> {
    let format = format_from_bytes(bytes)?;
    let (width, height) =
        ImageReader::with_format(std::io::Cursor::new(bytes), decoder_format(format))
            .into_dimensions()
            .map_err(|error| LocalImageError::CorruptImage {
                message: error.to_string(),
            })?;
    Ok(ValidatedImageMetadata::new(
        format,
        width,
        height,
        bytes.len() as u64,
        limits,
    )?)
}

/// Shared immutable Artifact resolution for wire delivery and CLI geometry.
pub(crate) fn resolve_project_image_artifact(
    project: &crate::adapters::project_event_store::Project,
    artifact_id: &ArtifactId,
) -> Result<(String, Vec<u8>), LocalImageError> {
    let error = |message: String| LocalImageError::Artifact { message };
    let state = crate::adapters::research_projection::load_research_state(project)
        .map_err(|err| error(err.to_string()))?;
    let record = state.artifact(artifact_id).ok_or_else(|| {
        error(format!(
            "image Artifact `{artifact_id}` is not present in the selected project"
        ))
    })?;
    if !matches!(record.media_type.as_str(), "image/png" | "image/jpeg") {
        return Err(error(format!(
            "Artifact `{artifact_id}` media type `{}` is not PNG or JPEG",
            record.media_type
        )));
    }
    let bytes = crate::adapters::artifact_store::LocalArtifactStore::for_project(project)
        .read_blob(&record.blob_id)
        .map_err(|err| error(err.to_string()))?;
    if bytes.len() as u64 != record.size_bytes {
        return Err(error(format!(
            "Artifact `{artifact_id}` size does not match its immutable Blob"
        )));
    }
    Ok((record.media_type.clone(), bytes))
}

pub(crate) fn resolve_vision_geometry(
    project: &crate::adapters::project_event_store::Project,
    selection: &ViewImageSelection,
) -> Result<VisionImageGeometry, LocalImageError> {
    let metadata = |id: &ArtifactId| -> Result<ValidatedImageMetadata, LocalImageError> {
        let (media_type, bytes) = resolve_project_image_artifact(project, id)?;
        let metadata = inspect_image_bytes(&bytes, LOCAL_IMAGE_INPUT_LIMITS)?;
        if metadata.format.media_type() != media_type {
            return Err(LocalImageError::Artifact {
                message: "image media type differs from immutable bytes".to_owned(),
            });
        }
        Ok(metadata)
    };
    let source = metadata(&selection.source_artifact_id)?;
    let delivery = if selection.delivery_artifact_id == selection.source_artifact_id {
        source
    } else {
        metadata(&selection.delivery_artifact_id)?
    };
    let geometry = VisionImageGeometry {
        delivery_artifact_id: selection.delivery_artifact_id.clone(),
        source_width: source.width,
        source_height: source.height,
        delivery_width: delivery.width,
        delivery_height: delivery.height,
    };
    geometry
        .validate(
            &selection.source_artifact_id,
            &selection.derived_artifact_ids,
            &[],
        )
        .map_err(|error| LocalImageError::Artifact {
            message: error.to_string(),
        })?;
    Ok(geometry)
}

fn format_from_bytes(bytes: &[u8]) -> Result<ImageFormat, LocalImageError> {
    match image::guess_format(bytes) {
        Ok(DecoderFormat::Png) => Ok(ImageFormat::Png),
        Ok(DecoderFormat::Jpeg) => Ok(ImageFormat::Jpeg),
        Ok(other) => Err(LocalImageError::UnsupportedFormat {
            format: format!("{other:?}"),
        }),
        Err(error) => Err(LocalImageError::CorruptImage {
            message: error.to_string(),
        }),
    }
}

const fn decoder_format(format: ImageFormat) -> DecoderFormat {
    match format {
        ImageFormat::Png => DecoderFormat::Png,
        ImageFormat::Jpeg => DecoderFormat::Jpeg,
    }
}

#[derive(Debug, Error)]
pub enum LocalImageError {
    #[error("image Artifact cannot be resolved: {message}")]
    Artifact { message: String },
    #[error(transparent)]
    Contract(#[from] ImageContractError),
    #[error("image source must not be a symbolic link")]
    SymlinkRejected,
    #[error("image source must be an ordinary file")]
    NonRegularFile,
    #[error("failed to inspect image source: {0}")]
    Metadata(#[source] std::io::Error),
    #[error("failed to read image source: {0}")]
    Read(#[source] std::io::Error),
    #[error("unsupported image format `{format}`; only PNG and JPEG are accepted")]
    UnsupportedFormat { format: String },
    #[error("image bytes are corrupt: {message}")]
    CorruptImage { message: String },
    #[error(
        "image dimensions changed from header {header_width}x{header_height} to decoded {decoded_width}x{decoded_height}"
    )]
    DimensionMismatch {
        header_width: u32,
        header_height: u32,
        decoded_width: u32,
        decoded_height: u32,
    },
    #[error("failed to encode deterministic derived PNG: {message}")]
    DerivativeEncode { message: String },
    #[error("no deterministic PNG can fit the {limit}-byte delivery budget")]
    OutputBudgetUnreachable { limit: u64 },
}
