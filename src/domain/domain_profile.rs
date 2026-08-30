//! Fixed vulnerability-research target domains.
//!
//! The four compile-time values are one closed axis inside the single Agent.
//! They are not personalities, workflow stages or a dynamic registry.

use serde::{Deserialize, Serialize};

use crate::domain::research::TargetKind;
use crate::domain::tools::{ToolExposure, ToolExposurePlan, ToolName};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DomainProfile {
    Source,
    Web,
    Mobile,
    Binary,
}

impl DomainProfile {
    pub const ALL: [Self; 4] = [Self::Source, Self::Web, Self::Mobile, Self::Binary];

    pub const fn generation(self) -> u64 {
        1
    }

    pub const fn id(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Web => "web",
            Self::Mobile => "mobile",
            Self::Binary => "binary",
        }
    }

    pub const fn target_kind(self) -> TargetKind {
        match self {
            Self::Source => TargetKind::Source,
            Self::Web => TargetKind::Web,
            Self::Mobile => TargetKind::Mobile,
            Self::Binary => TargetKind::Binary,
        }
    }

    /// Firmware remains unsupported until a separately justified fixed
    /// DomainProfile exists.
    pub fn supports_target_kind(self, target_kind: TargetKind) -> bool {
        self.target_kind() == target_kind
    }

    pub const fn prompt(self) -> &'static str {
        match self {
            Self::Source => include_str!("domain_profile/prompts/source-v1.md"),
            Self::Web => include_str!("domain_profile/prompts/web-v1.md"),
            Self::Mobile => include_str!("domain_profile/prompts/mobile-v1.md"),
            Self::Binary => include_str!("domain_profile/prompts/binary-v1.md"),
        }
    }

    /// Domain overrides stay bounded to domain-specific availability. Phase
    /// posture is composed separately by the Tool exposure owner.
    pub fn tool_exposure(self) -> ToolExposurePlan {
        match self {
            Self::Source => {
                ToolExposurePlan::new([(ToolName::new("view_image"), ToolExposure::Deferred)])
            }
            Self::Web | Self::Mobile => {
                ToolExposurePlan::new([(ToolName::new("http_request"), ToolExposure::Direct)])
            }
            Self::Binary => ToolExposurePlan::default(),
        }
    }
}
