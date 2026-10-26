//! Finite observations for the configured MCP CLI consumer; never tool facts.
use std::sync::Arc;

pub(crate) type McpObserver = Arc<dyn Fn(McpObservation) + Send + Sync>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum McpStartupStage {
    Starting,
    Initializing,
    Listing,
    Ready,
}

impl std::fmt::Display for McpStartupStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Starting => "starting",
            Self::Initializing => "initializing",
            Self::Listing => "listing tools",
            Self::Ready => "ready",
        })
    }
}

#[derive(Clone, Debug)]
pub(crate) enum McpObservation {
    Startup {
        server_id: String,
        stage: McpStartupStage,
    },
    StartupFailed {
        server_id: String,
        stage: McpStartupStage,
    },
    ToolExcluded {
        server_id: String,
        name: String,
        message: String,
    },
    Notification {
        server_id: String,
        method: String,
    },
    UnsupportedRequest {
        server_id: String,
        method: String,
    },
    Cleanup {
        server_id: String,
        group_error: Option<std::io::ErrorKind>,
        kill_error: Option<std::io::ErrorKind>,
        wait_error: Option<std::io::ErrorKind>,
        stderr_bytes: u64,
        stderr_error: Option<std::io::ErrorKind>,
    },
}
