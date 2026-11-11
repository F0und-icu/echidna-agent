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

#[path = "private_cases/command_interruption.rs"]
mod command_interruption;

#[path = "private_cases/compaction_continuity.rs"]
mod compaction_continuity;
#[path = "private_cases/compaction_replay.rs"]
mod compaction_replay;
#[path = "private_cases/context2_budgets.rs"]
mod context2_budgets;
#[path = "private_cases/context2_domain.rs"]
mod context2_domain;
#[path = "private_cases/delivery_failure.rs"]
mod delivery_failure;
#[path = "private_cases/legacy_v5_fixture.rs"]
mod legacy_v5_fixture;
#[path = "private_cases/model_delivery.rs"]
mod model_delivery;
#[path = "private_cases/request_audit_full.rs"]
mod request_audit_full;

#[path = "private_cases/improvement_policy.rs"]
mod improvement_policy;

#[path = "private_cases/argument_correction_checkpoint.rs"]
mod argument_correction_checkpoint;

#[path = "private_cases/runtime_integrity.rs"]
mod runtime_integrity;
#[path = "private_cases/session_read_reuse.rs"]
mod session_read_reuse;
#[path = "private_cases/task_cli_subscription_delivery.rs"]
mod task_cli_subscription_delivery;
#[path = "private_cases/task_context_version_replay.rs"]
mod task_context_version_replay;
#[path = "private_cases/task_delivery_suffix.rs"]
mod task_delivery_suffix;
#[path = "private_cases/task_input_clock.rs"]
mod task_input_clock;
#[path = "private_cases/task_progress_prefix.rs"]
mod task_progress_prefix;
#[path = "private_cases/task_subscription_epoch.rs"]
mod task_subscription_epoch;
#[path = "private_cases/task_vision_allocation.rs"]
mod task_vision_allocation;

#[path = "private_cases/terminal_input.rs"]
mod terminal_input;
