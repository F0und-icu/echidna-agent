//! Fixed, bounded environment inspection for vulnerability research.

mod probe;
mod schema;

use std::path::Path;

use futures_util::future::join_all;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::tools::{ToolExecutionOutcome, ToolExecutionPlan};
use crate::ports::tool::ToolExecutorError;

use self::probe::{DeviceObservation, NamedProbeObservation, ProbeObservation, ProxyObservation};
use super::{decode_arguments, invalid_plan};

pub(super) const SCHEMA_VERSION: u16 = 1;
const TOOL_NAME: &str = "inspect_environment";
const MAX_PROBE_BYTES: usize = 16 * 1024;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct InspectEnvironmentArgs {}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct OsObservation {
    family: String,
    name: String,
    arch: String,
    kernel: ProbeObservation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct RuntimeObservation {
    name: String,
    version: String,
    arch: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct EnvironmentReport {
    schema_version: u16,
    os: OsObservation,
    runtime: RuntimeObservation,
    git: ProbeObservation,
    proxy: ProxyObservation,
    toolchain: Vec<NamedProbeObservation>,
    connected_devices: Vec<DeviceObservation>,
    fingerprint: String,
}

#[derive(Serialize)]
struct FingerprintInput<'a> {
    schema_version: u16,
    os: &'a OsObservation,
    runtime: &'a RuntimeObservation,
    git: &'a ProbeObservation,
    proxy: &'a ProxyObservation,
    toolchain: &'a [NamedProbeObservation],
    connected_devices: &'a [DeviceObservation],
}

pub(super) fn normalize(
    _project_root: &Path,
    value: Value,
) -> Result<(Value, Vec<crate::domain::tools::ResourceKey>), ToolExecutorError> {
    let arguments: InspectEnvironmentArgs = decode_arguments(TOOL_NAME, value)?;
    Ok((
        serde_json::to_value(arguments).expect("InspectEnvironmentArgs serialization cannot fail"),
        Vec::new(),
    ))
}

pub(super) async fn execute(project_root: &Path, plan: ToolExecutionPlan) -> ToolExecutionOutcome {
    let _: InspectEnvironmentArgs = match decode_arguments(TOOL_NAME, plan.call.arguments.clone()) {
        Ok(arguments) => arguments,
        Err(error) => return invalid_plan(plan, error),
    };
    let probe_bytes = (plan.output_policy.max_bytes / 12).clamp(256, MAX_PROBE_BYTES);
    let toolchain_probes = join_all(probe::TOOLCHAIN_PROBES.iter().map(|fixed| async move {
        let observation = probe::run(*fixed, project_root, probe_bytes).await;
        NamedProbeObservation {
            name: fixed.name.to_owned(),
            available: observation.available,
            status: observation.status,
            version: observation.version,
        }
    }));
    let device_probes = join_all(probe::DEVICE_PROBES.iter().map(
        |(provider, version_probe, list_probe)| {
            probe::device(
                provider,
                *version_probe,
                *list_probe,
                project_root,
                probe_bytes,
            )
        },
    ));
    let (kernel, git, mut toolchain_observations, mut device_observations) = tokio::join!(
        probe::run(probe::KERNEL_PROBE, project_root, probe_bytes),
        probe::run(probe::GIT_PROBE, project_root, probe_bytes),
        toolchain_probes,
        device_probes,
    );
    toolchain_observations.sort_by(|left, right| left.name.cmp(&right.name));
    device_observations.sort_by(|left, right| left.provider.cmp(&right.provider));

    let mut report = EnvironmentReport {
        schema_version: SCHEMA_VERSION,
        os: OsObservation {
            family: std::env::consts::FAMILY.to_owned(),
            name: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
            kernel,
        },
        runtime: RuntimeObservation {
            name: "cyber-agent".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
        },
        git,
        proxy: probe::proxy_observation(),
        toolchain: toolchain_observations,
        connected_devices: device_observations,
        fingerprint: String::new(),
    };
    report.fingerprint = environment_fingerprint(&report);
    ToolExecutionOutcome::succeeded(
        plan,
        serde_json::to_value(report).expect("EnvironmentReport serialization cannot fail"),
    )
}

fn environment_fingerprint(report: &EnvironmentReport) -> String {
    let input = FingerprintInput {
        schema_version: report.schema_version,
        os: &report.os,
        runtime: &report.runtime,
        git: &report.git,
        proxy: &report.proxy,
        toolchain: &report.toolchain,
        connected_devices: &report.connected_devices,
    };
    let bytes = serde_json::to_vec(&input).expect("FingerprintInput serialization cannot fail");
    blake3::hash(&bytes).to_hex().to_string()
}

pub(super) fn input_schema() -> Value {
    schema::input_schema()
}

pub(super) fn output_schema() -> Value {
    schema::output_schema()
}
