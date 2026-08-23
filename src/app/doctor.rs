//! Offline Agent Model Profile capability diagnostics.

use serde::Serialize;

use crate::domain::profile::AgentModelProfile;

/// A credential-free diagnostic for an explicitly selected Agent profile.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AgentProfileDoctorReport {
    pub profile_id: String,
    pub primary_profile_id: String,
    pub vision_profile_id: Option<String>,
    pub vision_mode: crate::domain::profile::VisionMode,
    pub status: AgentProfileDoctorStatus,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentProfileDoctorStatus {
    Ready,
    Invalid { message: String },
}

/// Validates declared profile capabilities only. It never resolves a
/// credential, constructs an Adapter, or performs network I/O.
pub fn inspect_agent_model_profile(profile: &AgentModelProfile) -> AgentProfileDoctorReport {
    let status = match profile.validate() {
        Ok(()) => AgentProfileDoctorStatus::Ready,
        Err(error) => AgentProfileDoctorStatus::Invalid {
            message: error.to_string(),
        },
    };
    AgentProfileDoctorReport {
        profile_id: profile.profile().id.to_string(),
        primary_profile_id: profile.primary().profile().id.to_string(),
        vision_profile_id: profile
            .vision()
            .map(|vision| vision.profile().id.to_string()),
        vision_mode: profile.vision_mode(),
        status,
    }
}
