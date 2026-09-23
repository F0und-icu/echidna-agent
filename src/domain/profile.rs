//! Pure named Provider composition and capability-driven Vision resolution.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::domain::context::{ModelIdentity, ModelProfile};
use crate::domain::model::ModelCapabilities;

/// The only supported image-routing modes for one Agent profile.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VisionMode {
    Auto,
    PrimaryDirect,
    SecondaryTool,
    Disabled,
}

/// The resolved destination for image input. `SecondaryTool` remains a
/// configuration decision until the separately scoped Vision service exists.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VisionRoute {
    NoImage,
    PrimaryDirect,
    SecondaryTool,
    Rejected,
}

/// Stable explanations for a pure Vision route decision.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum VisionRouteReason {
    PrimaryImageCapable,
    PrimaryImageUnsupportedSecondarySelected,
    ForcedPrimaryDirect,
    ForcedSecondaryTool,
    VisionDisabled,
    NoImageCapableProvider,
}

impl VisionRouteReason {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::PrimaryImageCapable => "PRIMARY_IMAGE_CAPABLE",
            Self::PrimaryImageUnsupportedSecondarySelected => {
                "PRIMARY_IMAGE_UNSUPPORTED_SECONDARY_SELECTED"
            }
            Self::ForcedPrimaryDirect => "FORCED_PRIMARY_DIRECT",
            Self::ForcedSecondaryTool => "FORCED_SECONDARY_TOOL",
            Self::VisionDisabled => "VISION_DISABLED",
            Self::NoImageCapableProvider => "NO_IMAGE_CAPABLE_PROVIDER",
        }
    }
}

/// The deterministic result of capability-only Vision routing.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct VisionRouteDecision {
    pub route: VisionRoute,
    pub reason: Option<VisionRouteReason>,
}

/// A named Provider identity, declared capabilities, and generation. It has
/// no credential material and no Provider-specific protocol fields.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProviderProfile {
    model: ModelIdentity,
    profile: ModelProfile,
    capabilities: ModelCapabilities,
}

impl ProviderProfile {
    pub fn new(
        model: ModelIdentity,
        profile: ModelProfile,
        capabilities: ModelCapabilities,
    ) -> Self {
        Self {
            model,
            profile,
            capabilities,
        }
    }

    pub fn model(&self) -> &ModelIdentity {
        &self.model
    }

    pub fn profile(&self) -> &ModelProfile {
        &self.profile
    }

    pub fn capabilities(&self) -> &ModelCapabilities {
        &self.capabilities
    }

    fn supports_secondary_vision(&self) -> bool {
        supports_secondary_vision(&self.capabilities)
    }
}

/// The one primary Provider and optional one Vision Provider selected for an
/// AgentLoop. This is composition data, never a model-selection policy.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AgentModelProfile {
    profile: ModelProfile,
    primary: ProviderProfile,
    vision: Option<ProviderProfile>,
    vision_mode: VisionMode,
}

impl AgentModelProfile {
    pub fn new(
        profile: ModelProfile,
        primary: ProviderProfile,
        vision: Option<ProviderProfile>,
        vision_mode: VisionMode,
    ) -> Result<Self, AgentModelProfileError> {
        let composition = Self {
            profile,
            primary,
            vision,
            vision_mode,
        };
        composition.validate()?;
        Ok(composition)
    }

    pub fn validate(&self) -> Result<(), AgentModelProfileError> {
        for provider in std::iter::once(&self.primary).chain(self.vision.iter()) {
            provider
                .capabilities
                .resolve_output_token_limit(None)
                .map_err(|_| AgentModelProfileError::InvalidOutputTokenLimit)?;
        }
        if !self.primary.capabilities().text_input {
            return Err(AgentModelProfileError::PrimaryTextInputRequired);
        }
        if !self.primary.capabilities().streaming {
            return Err(AgentModelProfileError::PrimaryStreamingRequired);
        }
        if !self.primary.capabilities().tool_call {
            return Err(AgentModelProfileError::PrimaryToolCallRequired);
        }
        if self
            .vision
            .as_ref()
            .is_some_and(|vision| vision.profile() == self.primary.profile())
        {
            return Err(AgentModelProfileError::PrimaryVisionProfileMustDiffer);
        }
        if self
            .vision
            .as_ref()
            .is_some_and(|vision| !vision.supports_secondary_vision())
        {
            return Err(AgentModelProfileError::VisionCapabilitiesIncomplete);
        }
        match self.vision_mode {
            VisionMode::Auto | VisionMode::Disabled => Ok(()),
            VisionMode::PrimaryDirect if self.primary.capabilities().image_input => Ok(()),
            VisionMode::PrimaryDirect => Err(AgentModelProfileError::PrimaryImageInputRequired),
            VisionMode::SecondaryTool => match self.vision.as_ref() {
                Some(vision) if vision.supports_secondary_vision() => Ok(()),
                Some(_) => Err(AgentModelProfileError::VisionCapabilitiesIncomplete),
                None => Err(AgentModelProfileError::VisionProfileRequired),
            },
        }
    }

    pub fn profile(&self) -> &ModelProfile {
        &self.profile
    }

    pub fn primary(&self) -> &ProviderProfile {
        &self.primary
    }

    pub fn vision(&self) -> Option<&ProviderProfile> {
        self.vision.as_ref()
    }

    pub const fn vision_mode(&self) -> VisionMode {
        self.vision_mode
    }

    pub fn resolve_vision_route(&self, has_image: bool) -> VisionRouteDecision {
        resolve_vision_route(
            self.vision_mode,
            self.primary.capabilities(),
            self.vision().map(ProviderProfile::capabilities),
            has_image,
        )
    }
}

/// Resolves image handling from declared capabilities only. Model identifiers,
/// price, latency, transient failures, and output quality are deliberately not
/// inputs to this function.
pub fn resolve_vision_route(
    mode: VisionMode,
    primary: &ModelCapabilities,
    vision: Option<&ModelCapabilities>,
    has_image: bool,
) -> VisionRouteDecision {
    if !has_image {
        return VisionRouteDecision {
            route: VisionRoute::NoImage,
            reason: None,
        };
    }
    match mode {
        VisionMode::Auto if primary.image_input => VisionRouteDecision {
            route: VisionRoute::PrimaryDirect,
            reason: Some(VisionRouteReason::PrimaryImageCapable),
        },
        VisionMode::Auto if vision.is_some_and(supports_secondary_vision) => VisionRouteDecision {
            route: VisionRoute::SecondaryTool,
            reason: Some(VisionRouteReason::PrimaryImageUnsupportedSecondarySelected),
        },
        VisionMode::Auto => VisionRouteDecision {
            route: VisionRoute::Rejected,
            reason: Some(VisionRouteReason::NoImageCapableProvider),
        },
        VisionMode::PrimaryDirect if primary.image_input => VisionRouteDecision {
            route: VisionRoute::PrimaryDirect,
            reason: Some(VisionRouteReason::ForcedPrimaryDirect),
        },
        VisionMode::PrimaryDirect => VisionRouteDecision {
            route: VisionRoute::Rejected,
            reason: Some(VisionRouteReason::NoImageCapableProvider),
        },
        VisionMode::SecondaryTool if vision.is_some_and(supports_secondary_vision) => {
            VisionRouteDecision {
                route: VisionRoute::SecondaryTool,
                reason: Some(VisionRouteReason::ForcedSecondaryTool),
            }
        }
        VisionMode::SecondaryTool => VisionRouteDecision {
            route: VisionRoute::Rejected,
            reason: Some(VisionRouteReason::NoImageCapableProvider),
        },
        VisionMode::Disabled => VisionRouteDecision {
            route: VisionRoute::Rejected,
            reason: Some(VisionRouteReason::VisionDisabled),
        },
    }
}

fn supports_secondary_vision(capabilities: &ModelCapabilities) -> bool {
    capabilities.text_input
        && capabilities.image_input
        && capabilities.structured_output
        && capabilities.streaming
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum AgentModelProfileError {
    #[error("a Provider profile declares an invalid output token limit")]
    InvalidOutputTokenLimit,
    #[error("the primary Provider profile must declare text input")]
    PrimaryTextInputRequired,
    #[error("the primary Provider profile must declare streaming")]
    PrimaryStreamingRequired,
    #[error("the primary Provider profile must declare Tool Call support")]
    PrimaryToolCallRequired,
    #[error("the primary and optional Vision Provider profiles must differ")]
    PrimaryVisionProfileMustDiffer,
    #[error("primary_direct Vision mode requires primary image input")]
    PrimaryImageInputRequired,
    #[error("secondary_tool Vision mode requires a Vision Provider profile")]
    VisionProfileRequired,
    #[error(
        "the optional Vision Provider must declare text, image, structured-output, and streaming support"
    )]
    VisionCapabilitiesIncomplete,
}
