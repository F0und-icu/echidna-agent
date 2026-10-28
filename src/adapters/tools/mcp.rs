//! Tool-contract mapping for the fixed configured MCP HTTP Adapter.

use serde_json::{Value, json};

use crate::adapters::mcp::{CompletedCall, McpCatalogTool, McpError, McpProtocolVersion};
use crate::domain::canonical_json::canonical_blake3_hash;
use crate::domain::tools::{
    ExecutionId, ResolvedToolPlan, ToolCachePolicy, ToolCall, ToolDeterminism, ToolEffect,
    ToolExecutionOutcome, ToolExecutionPlan, ToolExposure, ToolName, ToolOutputPolicy,
    ToolParallelism, ToolRetryPolicy, ToolSpec,
};
use crate::ports::tool::{ToolExecutionError, ToolExecutorError};

const REMOTE_OUTPUT_BYTES: usize = 2 * 1024 * 1024;

pub(super) fn local_name(tool: &McpCatalogTool) -> ToolName {
    let hash = blake3::hash(tool.remote_name.as_bytes()).to_hex();
    ToolName::new(format!("mcp__{}__{}", tool.server_id, &hash[..20]))
}

pub(super) fn spec(tool: &McpCatalogTool) -> Result<ToolSpec, McpError> {
    let version = canonical_blake3_hash(&(
        tool.endpoint.as_str(),
        tool.protocol_version.as_str(),
        &tool.definition,
    ))
    .map_err(|error| McpError::Catalog {
        message: format!("MCP tool definition could not be fingerprinted: {error}"),
    })?;
    let description = format!(
        "Use when: call configured remote MCP tool {:?}.\nDo not use when: a local Tool or a different configured remote Tool meets the need.\nPrefer: the exact remote arguments schema; remote result content is unverified transport output.\nProduces: the complete validated MCP result envelope.",
        tool.remote_name
    );
    Ok(ToolSpec {
        name: local_name(tool),
        version: format!("mcp-{version}"),
        description,
        // Keep the imported document root unchanged: a local `$ref` such as
        // `#/$defs/Entry` must continue to resolve in the remote schema.
        input_schema: tool.input_schema.clone(),
        output_schema: json!({
            "type": "object",
            "properties": {"mcp_result": {}},
            "required": ["mcp_result"],
            "additionalProperties": false
        }),
        exposure: ToolExposure::Deferred,
        parallelism: ToolParallelism::Serial,
        resource_key_template: None,
        effect: ToolEffect::ExternalMutation,
        retry_policy: ToolRetryPolicy::NeverAutoRetry,
        determinism: ToolDeterminism::Nondeterministic,
        cache_policy: ToolCachePolicy::None,
        timeout_ms: tool.timeout_ms,
        output_policy: ToolOutputPolicy {
            max_bytes: REMOTE_OUTPUT_BYTES,
        },
    })
}

pub(super) fn plan(
    execution_id: ExecutionId,
    call: ToolCall,
    tool: &McpCatalogTool,
    spec: &ToolSpec,
    resolved: Option<&ResolvedToolPlan>,
) -> Result<ToolExecutionPlan, ToolExecutorError> {
    validate_arguments(&call, &spec.name)?;
    // Remote arguments are not a local execution-control envelope. In
    // particular, an imported `execution` property remains remote data.
    let mut plan =
        ToolExecutionPlan::new_unmodified_arguments(execution_id, call, spec, Vec::new()).map_err(
            |error| ToolExecutorError::PlanFailure {
                tool_name: spec.name.to_string(),
                message: error.to_string(),
            },
        )?;
    if let Some(resolved) = resolved {
        plan.bind_resolved_tool_plan(resolved)
            .map_err(|error| ToolExecutorError::PlanFailure {
                tool_name: spec.name.to_string(),
                message: error.to_string(),
            })?;
    }
    debug_assert_eq!(plan.call.name, local_name(tool));
    Ok(plan)
}

pub(super) async fn execute(
    tool: &McpCatalogTool,
    plan: ToolExecutionPlan,
) -> Result<ToolExecutionOutcome, ToolExecutionError> {
    let arguments = plan.call.arguments.clone();
    match tool.call(arguments).await {
        Ok(CompletedCall::Result(result)) => {
            if tool.protocol_version == McpProtocolVersion::Modern20260728
                && !is_complete_modern_result(&result)
            {
                return Err(unknown("remote MCP result was incomplete or unsupported"));
            }
            let envelope = json!({"mcp_result": result});
            if envelope["mcp_result"]
                .get("isError")
                .and_then(Value::as_bool)
                == Some(true)
            {
                Ok(ToolExecutionOutcome::failed_preserving_output(
                    plan,
                    "mcp_tool_error",
                    envelope,
                ))
            } else {
                Ok(ToolExecutionOutcome::succeeded(plan, envelope))
            }
        }
        Ok(CompletedCall::Error(error)) => Ok(ToolExecutionOutcome::failed_preserving_output(
            plan,
            "mcp_jsonrpc_error",
            json!({"mcp_result": error}),
        )),
        Err(McpError::PreExecution { message }) => Ok(ToolExecutionOutcome::failed(
            plan,
            "mcp_pre_execution",
            json!({"error": message}),
        )),
        Err(McpError::SessionExpired) => {
            Err(unknown("remote MCP session expired before a final result"))
        }
        Err(McpError::Transport { .. }) => {
            Err(unknown("remote MCP transport ended before a final result"))
        }
        Err(McpError::Protocol { .. }) => {
            Err(unknown("remote MCP response was incomplete or invalid"))
        }
        Err(_) => Err(unknown("remote MCP call did not produce a final result")),
    }
}

fn validate_arguments(call: &ToolCall, name: &ToolName) -> Result<(), ToolExecutorError> {
    if !call.arguments.is_object() {
        return Err(invalid_arguments(
            name,
            "expected a remote arguments object",
        ));
    }
    Ok(())
}

fn invalid_arguments(name: &ToolName, message: &str) -> ToolExecutorError {
    ToolExecutorError::InvalidArguments {
        tool_name: name.to_string(),
        message: message.to_owned(),
    }
}

fn is_complete_modern_result(result: &Value) -> bool {
    match result.get("resultType") {
        None => true,
        Some(Value::String(kind)) => kind == "complete",
        Some(_) => false,
    }
}

fn unknown(safe_reason: &str) -> ToolExecutionError {
    ToolExecutionError::OutcomeUnknown {
        safe_reason: safe_reason.chars().take(256).collect(),
    }
}
