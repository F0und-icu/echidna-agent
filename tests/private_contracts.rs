//! Private production contracts share one library compilation without public test APIs.
include!("../src/lib.rs");

#[path = "support/provider_config.rs"]
mod provider_config;
mod support;

#[path = "private_cases/api_model_catalog.rs"]
mod api_model_catalog;
#[path = "private_cases/core_memory_opaque_continuation.rs"]
mod core_memory_opaque_continuation;
#[path = "private_cases/failed_request_capture.rs"]
mod failed_request_capture;
#[path = "private_cases/mcp_http.rs"]
mod mcp_http;
#[path = "private_cases/mcp_stdio.rs"]
mod mcp_stdio;
#[path = "private_cases/model_request_budget.rs"]
mod model_request_budget;
#[path = "private_cases/model_selection_picker.rs"]
mod model_selection_picker;
#[path = "private_cases/model_transport_retry.rs"]
mod model_transport_retry;
#[path = "private_cases/responses_subscription_debug.rs"]
mod responses_subscription_debug;
#[path = "private_cases/rsi_subscription_continuation.rs"]
mod rsi_subscription_continuation;
#[path = "private_cases/session_incremental_durability.rs"]
mod session_incremental_durability;
#[path = "private_cases/session_recovered_completion.rs"]
mod session_recovered_completion;

#[path = "private_cases/research_turn.rs"]
mod research_turn;

#[path = "private_cases/tool_catalog.rs"]
mod tool_catalog;

#[path = "private_cases/compaction_replay.rs"]
mod compaction_replay;

#[path = "private_cases/improvement_policy.rs"]
mod improvement_policy;
