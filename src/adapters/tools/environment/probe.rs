//! Fixed process probes and their bounded, provider-neutral normalization.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::super::command::{FixedProbeOutput, run_fixed_probe};

#[derive(Clone, Copy)]
pub(super) struct FixedProbe {
    pub(super) name: &'static str,
    pub(super) argv: &'static [&'static str],
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ProbeStatus {
    Available,
    VersionFailed,
    ProbeFailed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProbeObservation {
    pub(super) available: bool,
    pub(super) status: ProbeStatus,
    pub(super) version: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NamedProbeObservation {
    pub(super) name: String,
    pub(super) available: bool,
    pub(super) status: ProbeStatus,
    pub(super) version: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DeviceObservation {
    pub(super) provider: String,
    pub(super) available: bool,
    pub(super) status: ProbeStatus,
    pub(super) version: Option<String>,
    pub(super) count: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProxyObservation {
    pub(super) http_configured: bool,
    pub(super) https_configured: bool,
    pub(super) all_configured: bool,
    pub(super) no_proxy_configured: bool,
}

pub(super) const KERNEL_PROBE: FixedProbe = FixedProbe {
    name: "kernel",
    argv: &["uname", "-sr"],
};

pub(super) const GIT_PROBE: FixedProbe = FixedProbe {
    name: "git",
    argv: &["git", "--version"],
};

pub(super) const TOOLCHAIN_PROBES: [FixedProbe; 5] = [
    FixedProbe {
        name: "cargo",
        argv: &["cargo", "--version"],
    },
    FixedProbe {
        name: "java",
        argv: &["java", "-version"],
    },
    FixedProbe {
        name: "node",
        argv: &["node", "--version"],
    },
    FixedProbe {
        name: "python3",
        argv: &["python3", "--version"],
    },
    FixedProbe {
        name: "rustc",
        argv: &["rustc", "--version"],
    },
];

pub(super) const DEVICE_PROBES: [(&str, FixedProbe, FixedProbe); 2] = [
    (
        "adb",
        FixedProbe {
            name: "adb",
            argv: &["adb", "version"],
        },
        FixedProbe {
            name: "adb devices",
            argv: &["adb", "devices"],
        },
    ),
    (
        "xcrun",
        FixedProbe {
            name: "xcrun",
            argv: &["xcrun", "--version"],
        },
        FixedProbe {
            name: "xcrun xctrace",
            argv: &["xcrun", "xctrace", "list", "devices"],
        },
    ),
];

pub(super) async fn run(
    fixed: FixedProbe,
    project_root: &Path,
    max_bytes: usize,
) -> ProbeObservation {
    let output = run_fixed_probe(project_root, fixed.argv, max_bytes).await;
    observation(&output)
}

pub(super) async fn device(
    provider: &str,
    version_probe: FixedProbe,
    list_probe: FixedProbe,
    project_root: &Path,
    max_bytes: usize,
) -> DeviceObservation {
    let (version_output, list_output) = tokio::join!(
        run_fixed_probe(project_root, version_probe.argv, max_bytes),
        run_fixed_probe(project_root, list_probe.argv, max_bytes),
    );
    let version = observation(&version_output);
    let list_status = probe_status(&list_output);
    let parsed_count = if list_status == ProbeStatus::Available {
        match provider {
            "adb" => parse_adb_devices(&list_output.stdout.text),
            "xcrun" => parse_xctrace_devices(&list_output.stdout.text),
            _ => None,
        }
    } else {
        None
    };
    let status = if version.status != ProbeStatus::Available {
        version.status
    } else if list_status != ProbeStatus::Available {
        list_status
    } else if parsed_count.is_none() {
        ProbeStatus::ProbeFailed
    } else {
        ProbeStatus::Available
    };
    DeviceObservation {
        provider: provider.to_owned(),
        available: status == ProbeStatus::Available,
        status,
        version: version.version,
        count: parsed_count.unwrap_or(0),
    }
}

fn observation(output: &FixedProbeOutput) -> ProbeObservation {
    let status = probe_status(output);
    if status != ProbeStatus::Available {
        return ProbeObservation {
            available: false,
            status,
            version: None,
        };
    }
    let version = version_token(&output.stdout.text, &output.stderr.text);
    match version {
        Some(version) => ProbeObservation {
            available: true,
            status: ProbeStatus::Available,
            version: Some(version),
        },
        None => ProbeObservation {
            available: false,
            status: ProbeStatus::VersionFailed,
            version: None,
        },
    }
}

fn probe_status(output: &FixedProbeOutput) -> ProbeStatus {
    if output.error.is_some() {
        ProbeStatus::ProbeFailed
    } else if output.exit_code == Some(0) {
        ProbeStatus::Available
    } else {
        ProbeStatus::VersionFailed
    }
}

fn parse_adb_devices(output: &str) -> Option<u64> {
    let mut lines = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    if !lines
        .next()?
        .eq_ignore_ascii_case("List of devices attached")
    {
        return None;
    }
    Some(
        lines
            .filter(|line| {
                let mut fields = line.split_whitespace();
                fields.next().is_some() && fields.next() == Some("device")
            })
            .count() as u64,
    )
}

fn parse_xctrace_devices(output: &str) -> Option<u64> {
    if output.trim().is_empty() {
        return None;
    }
    Some(
        output
            .lines()
            .filter(|line| line.split_whitespace().any(is_device_identifier))
            .count() as u64,
    )
}

fn is_device_identifier(value: &str) -> bool {
    let value = value.trim_matches(|character| matches!(character, '(' | ')' | '[' | ']'));
    let lengths = [8, 4, 4, 4, 12];
    let pieces = value.split('-').collect::<Vec<_>>();
    let simulator_uuid = pieces.len() == lengths.len()
        && pieces.iter().zip(lengths).all(|(piece, length)| {
            piece.len() == length && piece.chars().all(|character| character.is_ascii_hexdigit())
        });
    let physical_udid = value.split_once('-').is_some_and(|(prefix, suffix)| {
        prefix.len() == 8
            && suffix.len() == 16
            && prefix
                .chars()
                .chain(suffix.chars())
                .all(|character| character.is_ascii_hexdigit())
    });
    simulator_uuid || physical_udid
}

fn proxy() -> ProxyObservation {
    let configured = |names: &[&str]| {
        std::env::vars_os().any(|(name, value)| {
            let name = name.to_string_lossy();
            names
                .iter()
                .any(|candidate| name.eq_ignore_ascii_case(candidate))
                && !value.is_empty()
        })
    };
    ProxyObservation {
        http_configured: configured(&["HTTP_PROXY", "http_proxy"]),
        https_configured: configured(&["HTTPS_PROXY", "https_proxy"]),
        all_configured: configured(&["ALL_PROXY", "all_proxy"]),
        no_proxy_configured: configured(&["NO_PROXY", "no_proxy"]),
    }
}

pub(super) fn proxy_observation() -> ProxyObservation {
    proxy()
}

fn version_token(stdout: &str, stderr: &str) -> Option<String> {
    stdout
        .lines()
        .chain(stderr.lines())
        .flat_map(str::split_whitespace)
        .map(|token| {
            let token = token.trim_matches(|character: char| !character.is_ascii_alphanumeric());
            token.strip_prefix('v').unwrap_or(token).to_owned()
        })
        .find(|token| is_version_token(token))
}

fn is_version_token(token: &str) -> bool {
    if token.is_empty() || token.len() > 64 {
        return false;
    }
    let core_end = token.find(['+', '-', '_']).unwrap_or(token.len());
    let (core, suffix) = token.split_at(core_end);
    if core.is_empty()
        || core.split('.').any(|segment| {
            segment.is_empty() || !segment.chars().all(|value| value.is_ascii_digit())
        })
    {
        return false;
    }
    suffix.chars().all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '.' | '+' | '-' | '_')
    })
}
