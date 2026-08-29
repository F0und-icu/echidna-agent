use serde::{Deserialize, Serialize};

use crate::domain::tools::{ToolExposure, ToolExposurePlan, ToolName};

const BASE_PROMPT: &str = include_str!("prompts/base-v1.md");

/// The only production Agent identity. Presentation preferences compose after
/// it and cannot replace its evidence discipline.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct BaseContract;

impl BaseContract {
    pub const ID: &'static str = "pragmatic-vulnerability-researcher";
    pub const GENERATION: u64 = 1;

    pub const fn prompt(self) -> &'static str {
        BASE_PROMPT
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchPhase {
    Mapping,
    Hypothesis,
    Experiment,
    Verification,
    EvidenceCuration,
}

impl ResearchPhase {
    pub const ALL: [Self; 5] = [
        Self::Mapping,
        Self::Hypothesis,
        Self::Experiment,
        Self::Verification,
        Self::EvidenceCuration,
    ];

    pub const fn generation(self) -> u64 {
        1
    }

    pub const fn id(self) -> &'static str {
        match self {
            Self::Mapping => "mapping",
            Self::Hypothesis => "hypothesis",
            Self::Experiment => "experiment",
            Self::Verification => "verification",
            Self::EvidenceCuration => "evidence_curation",
        }
    }

    pub const fn prompt(self) -> &'static str {
        match self {
            Self::Mapping => include_str!("prompts/mapping-v1.md"),
            Self::Hypothesis => include_str!("prompts/hypothesis-v1.md"),
            Self::Experiment => include_str!("prompts/experiment-v1.md"),
            Self::Verification => include_str!("prompts/verification-v1.md"),
            Self::EvidenceCuration => include_str!("prompts/evidence-curation-v1.md"),
        }
    }

    /// Phase posture changes only model-visible context. FullAccess and Tool
    /// runtime semantics remain unchanged.
    pub fn tool_exposure(self) -> ToolExposurePlan {
        let direct = ToolExposure::Direct;
        let deferred = ToolExposure::Deferred;
        match self {
            Self::Mapping => ToolExposurePlan::new([
                (ToolName::new("write_file"), deferred),
                (ToolName::new("apply_patch"), deferred),
                (ToolName::new("start_job"), deferred),
                (ToolName::new("poll_job"), deferred),
                (ToolName::new("input_job"), deferred),
                (ToolName::new("stop_job"), deferred),
                (ToolName::new("start_pty"), deferred),
                (ToolName::new("poll_pty"), deferred),
                (ToolName::new("input_pty"), deferred),
                (ToolName::new("resize_pty"), deferred),
                (ToolName::new("stop_pty"), deferred),
                (ToolName::new("memory_read"), direct),
                (ToolName::new("memory_write"), deferred),
            ]),
            Self::Hypothesis => ToolExposurePlan::new([
                (ToolName::new("run_command"), deferred),
                (ToolName::new("run_commands"), deferred),
                (ToolName::new("write_file"), deferred),
                (ToolName::new("apply_patch"), deferred),
                (ToolName::new("start_job"), deferred),
                (ToolName::new("poll_job"), deferred),
                (ToolName::new("input_job"), deferred),
                (ToolName::new("stop_job"), deferred),
                (ToolName::new("start_pty"), deferred),
                (ToolName::new("poll_pty"), deferred),
                (ToolName::new("input_pty"), deferred),
                (ToolName::new("resize_pty"), deferred),
                (ToolName::new("stop_pty"), deferred),
                (ToolName::new("memory_read"), direct),
                (ToolName::new("memory_write"), deferred),
            ]),
            Self::Experiment => ToolExposurePlan::new([
                (ToolName::new("write_file"), direct),
                (ToolName::new("apply_patch"), direct),
                (ToolName::new("start_job"), direct),
                (ToolName::new("poll_job"), direct),
                (ToolName::new("input_job"), direct),
                (ToolName::new("stop_job"), direct),
                (ToolName::new("start_pty"), direct),
                (ToolName::new("poll_pty"), direct),
                (ToolName::new("input_pty"), direct),
                (ToolName::new("resize_pty"), direct),
                (ToolName::new("stop_pty"), direct),
                (ToolName::new("artifact_store"), direct),
                (ToolName::new("memory_read"), direct),
                (ToolName::new("memory_write"), deferred),
            ]),
            Self::Verification => ToolExposurePlan::new([
                (ToolName::new("write_file"), deferred),
                (ToolName::new("apply_patch"), deferred),
                (ToolName::new("start_job"), direct),
                (ToolName::new("poll_job"), direct),
                (ToolName::new("input_job"), direct),
                (ToolName::new("stop_job"), direct),
                (ToolName::new("start_pty"), direct),
                (ToolName::new("poll_pty"), direct),
                (ToolName::new("input_pty"), direct),
                (ToolName::new("resize_pty"), direct),
                (ToolName::new("stop_pty"), direct),
                (ToolName::new("artifact_store"), direct),
                (ToolName::new("memory_read"), direct),
                (ToolName::new("memory_write"), deferred),
            ]),
            Self::EvidenceCuration => ToolExposurePlan::new([
                (ToolName::new("run_command"), deferred),
                (ToolName::new("run_commands"), deferred),
                (ToolName::new("write_file"), deferred),
                (ToolName::new("apply_patch"), deferred),
                (ToolName::new("start_job"), deferred),
                (ToolName::new("poll_job"), deferred),
                (ToolName::new("input_job"), deferred),
                (ToolName::new("stop_job"), deferred),
                (ToolName::new("start_pty"), deferred),
                (ToolName::new("poll_pty"), deferred),
                (ToolName::new("input_pty"), deferred),
                (ToolName::new("resize_pty"), deferred),
                (ToolName::new("stop_pty"), deferred),
                (ToolName::new("artifact_store"), direct),
                (ToolName::new("memory_read"), direct),
                (ToolName::new("memory_write"), direct),
            ]),
        }
    }
}
