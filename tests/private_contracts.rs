//! Private contracts reuse the production library without widening its API.
include!("../src/lib.rs");

#[path = "support/provider_config.rs"]
mod provider_config;
mod support;

#[path = "private_cases/api_model_catalog.rs"]
mod api_model_catalog;
#[path = "private_cases/argument_correction_checkpoint.rs"]
mod argument_correction_checkpoint;
#[path = "private_cases/compaction_graph.rs"]
mod compaction_graph;
#[path = "private_cases/failed_request_capture.rs"]
mod failed_request_capture;
#[path = "private_cases/main_configuration.rs"]
mod main_configuration;
#[path = "private_cases/main_context_budgets.rs"]
mod main_context_budgets;
#[path = "private_cases/model_request_budget.rs"]
mod model_request_budget;
#[path = "private_cases/request_audit_logical.rs"]
mod request_audit_logical;
