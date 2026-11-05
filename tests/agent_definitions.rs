#[path = "support/provider_config.rs"]
mod provider_config;
mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use echidna::domain::domain_profile::DomainProfile;
use echidna::domain::research_control::{BaseContract, ResearchPhase};
use serde_json::Value;

use support::TemporaryDirectory;

fn command(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_echidna"))
        .current_dir(root)
        .args(args)
        .output()
        .expect("Echidna command should run")
}

fn write_config(root: &Path, agent_definition: Option<&str>, personal: Option<&Path>) -> PathBuf {
    let mut config = provider_config::fixture();
    if let Some(agent_definition) = agent_definition {
        config = config.replacen(
            "active_agent_profile = \"default\"",
            &format!(
                "active_agent_profile = \"default\"\nagent_definition = \"{agent_definition}\""
            ),
            1,
        );
    }
    if let Some(personal) = personal {
        config.push_str(&format!(
            "\n[prompts]\npersonal_file = \"{}\"\n",
            personal.display()
        ));
    }
    let path = root.join("provider.toml");
    fs::write(&path, config).unwrap();
    path
}

fn export(root: &Path, name: &str, id: &str) -> PathBuf {
    let path = root.join(name);
    let output = command(
        root,
        &[
            "agent",
            "init",
            path.to_str().unwrap(),
            "--id",
            id,
            "--from",
            "technical-writing",
        ],
    );
    assert!(output.status.success(), "stderr: {:?}", output.stderr);
    path
}

fn checked(root: &Path, config: &Path, definition: &Path) -> Value {
    let output = command(
        root,
        &[
            "--config",
            config.to_str().unwrap(),
            "agent",
            "check",
            definition.to_str().unwrap(),
        ],
    );
    assert!(output.status.success(), "stderr: {:?}", output.stderr);
    serde_json::from_slice(&output.stdout).unwrap()
}

fn inspected(root: &Path, config: &Path, definition: &Path) -> Value {
    let output = command(
        root,
        &[
            "--config",
            config.to_str().unwrap(),
            "agent",
            "inspect",
            definition.to_str().unwrap(),
        ],
    );
    assert!(output.status.success(), "stderr: {:?}", output.stderr);
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn exported_generic_definition_is_directory_loadable_and_source_independent() {
    let root = TemporaryDirectory::new("agent-definition-loader");
    let config = write_config(root.path(), None, None);
    let first = export(root.path(), "first", "writer-fixture");
    let second = export(root.path(), "second", "writer-fixture");

    let first_check = checked(root.path(), &config, &first);
    let second_check = checked(root.path(), &config, &second);
    assert_eq!(first_check["status"], "valid");
    assert_eq!(first_check["agent_id"], "writer-fixture");
    assert_eq!(
        first_check["definition_digest"], second_check["definition_digest"],
        "definition identity must not depend on its source directory"
    );
    assert_eq!(
        first_check["definition_digest"].as_str().map(str::len),
        Some(64)
    );

    fs::write(first.join("persona.md"), "A different technical writer.\n").unwrap();
    let changed = checked(root.path(), &config, &first);
    assert_ne!(
        changed["definition_digest"],
        second_check["definition_digest"]
    );

    let before = fs::read(first.join("agent.toml")).unwrap();
    let duplicate = command(
        root.path(),
        &[
            "agent",
            "init",
            first.to_str().unwrap(),
            "--from",
            "technical-writing",
        ],
    );
    assert!(!duplicate.status.success());
    assert_eq!(fs::read(first.join("agent.toml")).unwrap(), before);
}

#[test]
fn generic_definition_has_no_research_contract_and_binds_selected_personal_bytes() {
    let root = TemporaryDirectory::new("agent-definition-generic");
    let definition = export(root.path(), "writer", "writer-with-personal");
    let personal = root.path().join("personal.md");
    fs::write(&personal, "Prefer direct, factual prose.\n").unwrap();
    let config = write_config(root.path(), None, Some(&personal));
    let output = command(
        root.path(),
        &[
            "--config",
            config.to_str().unwrap(),
            "agent",
            "inspect",
            definition.to_str().unwrap(),
        ],
    );
    assert!(output.status.success(), "stderr: {:?}", output.stderr);
    let inspection: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(inspection["status"], "valid");
    assert!(inspection["binding"]["definition"]["research"].is_null());
    assert_eq!(
        inspection["binding"]["personal"]["content"],
        "Prefer direct, factual prose.\n"
    );
    assert!(
        inspection["instructions"]
            .as_str()
            .unwrap()
            .contains("technical writer")
    );
    assert!(
        inspection["tools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|tool| tool["name"] != "research_get")
    );
}

#[test]
fn bound_prompt_reports_agent_identity_and_logical_memory_scope() {
    let root = TemporaryDirectory::new("agent-definition-runtime-binding-facts");
    let config = write_config(root.path(), None, None);

    let generic = export(root.path(), "generic", "generic-visible");
    let generic_instructions = inspected(root.path(), &config, &generic)["instructions"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(generic_instructions.contains(
        "Runtime binding facts: stable Agent ID `generic-visible`. Configured Memory scope: `agent`."
    ));
    assert!(generic_instructions.contains(
        "Logical Memory path `core.md` is in the Agent Memory namespace for Agent ID `generic-visible`."
    ));
    assert!(generic_instructions.contains("Use memory_read before memory_write"));
    assert!(generic_instructions.contains("Those detail files"));

    let research = root.path().join("research");
    let exported = command(
        root.path(),
        &[
            "agent",
            "init",
            research.to_str().unwrap(),
            "--id",
            "research-visible",
            "--from",
            "builtin-research",
        ],
    );
    assert!(exported.status.success(), "stderr: {:?}", exported.stderr);
    let research_instructions = inspected(root.path(), &config, &research)["instructions"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(research_instructions.contains(
        "Runtime binding facts: stable Agent ID `research-visible`. Configured Memory scope: `project`."
    ));
    assert!(
        research_instructions
            .contains("Logical Memory path `core.md` is in the shared project Memory namespace.")
    );

    let memoryless = export(root.path(), "memoryless", "memoryless-visible");
    let manifest = memoryless.join("agent.toml");
    let content = fs::read_to_string(&manifest)
        .unwrap()
        .replacen("memory = \"agent\"", "memory = \"none\"", 1)
        .replacen("memory_read = \"direct\"", "memory_read = \"hidden\"", 1)
        .replacen("memory_write = \"direct\"", "memory_write = \"hidden\"", 1);
    fs::write(&manifest, content).unwrap();
    let memoryless_instructions = inspected(root.path(), &config, &memoryless)["instructions"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(memoryless_instructions.contains(
        "Runtime binding facts: stable Agent ID `memoryless-visible`. Configured Memory scope: `none`."
    ));
    assert!(
        memoryless_instructions
            .contains("Logical Memory path `core.md` is unavailable because Memory is disabled.")
    );
    assert!(!memoryless_instructions.contains("Use memory_read before memory_write"));

    for instructions in [
        generic_instructions,
        research_instructions,
        memoryless_instructions,
    ] {
        assert!(
            instructions
                .contains("This scope is a logical Memory namespace, not a filesystem sandbox.")
        );
    }
}

#[test]
fn scoped_catalog_accepts_declared_hidden_tools_after_full_catalog_admission() {
    let root = TemporaryDirectory::new("agent-definition-scoped-catalog");
    let definition = export(root.path(), "writer", "scoped-writer");
    let manifest = definition.join("agent.toml");
    let content = fs::read_to_string(&manifest)
        .unwrap()
        .replacen("memory = \"agent\"", "memory = \"none\"", 1)
        .replacen("memory_read = \"direct\"", "memory_read = \"hidden\"", 1)
        .replacen("memory_write = \"direct\"", "memory_write = \"hidden\"", 1)
        + "research_get = \"hidden\"\nresearch_update = \"hidden\"\n";
    fs::write(&manifest, content).unwrap();

    let config = write_config(root.path(), None, None);
    let checked = checked(root.path(), &config, &definition);
    assert_eq!(checked["status"], "valid");
}

#[test]
fn loader_rejects_unknown_fields_and_resource_escape() {
    let root = TemporaryDirectory::new("agent-definition-invalid");
    let definition = export(root.path(), "invalid", "invalid-writer");
    let config = write_config(root.path(), None, None);

    let manifest = definition.join("agent.toml");
    let content = fs::read_to_string(&manifest).unwrap().replacen(
        "name = \"Technical writing\"",
        "name = \"Technical writing\"\nunexpected = true",
        1,
    );
    fs::write(&manifest, content).unwrap();
    let unknown = command(
        root.path(),
        &[
            "--config",
            config.to_str().unwrap(),
            "agent",
            "check",
            definition.to_str().unwrap(),
        ],
    );
    assert!(!unknown.status.success());
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("unknown field"));

    let unknown_tool = export(root.path(), "unknown-tool", "unknown-tool-writer");
    let unknown_tool_manifest = unknown_tool.join("agent.toml");
    fs::write(
        &unknown_tool_manifest,
        format!(
            "{}\nunregistered_tool = \"direct\"\n",
            fs::read_to_string(&unknown_tool_manifest).unwrap()
        ),
    )
    .unwrap();
    let unknown_tool = command(
        root.path(),
        &[
            "--config",
            config.to_str().unwrap(),
            "agent",
            "check",
            unknown_tool_manifest.to_str().unwrap(),
        ],
    );
    assert!(!unknown_tool.status.success());
    assert!(String::from_utf8_lossy(&unknown_tool.stderr).contains("unknown Tool"));

    let clean = export(root.path(), "escaped", "escaped-writer");
    let escaped_manifest = clean.join("agent.toml");
    let content = fs::read_to_string(&escaped_manifest).unwrap().replacen(
        "persona_file = \"persona.md\"",
        "persona_file = \"../personal.md\"",
        1,
    );
    fs::write(&escaped_manifest, content).unwrap();
    let escaped = command(
        root.path(),
        &[
            "--config",
            config.to_str().unwrap(),
            "agent",
            "check",
            escaped_manifest.to_str().unwrap(),
        ],
    );
    assert!(!escaped.status.success());
    assert!(String::from_utf8_lossy(&escaped.stderr).contains("relative file path"));
}

#[test]
fn configured_definition_path_selects_the_exported_agent_for_a_new_session() {
    let root = TemporaryDirectory::new("agent-definition-config-selection");
    let definition = export(root.path(), "configured-agent", "configured-writer");
    let config = write_config(root.path(), Some("configured-agent"), None);
    let initialized = command(root.path(), &["init"]);
    assert!(initialized.status.success());
    let chat = command(root.path(), &["--config", config.to_str().unwrap(), "chat"]);
    assert!(chat.status.success(), "stderr: {:?}", chat.stderr);
    let session = fs::read_dir(root.path().join(".echidna/sessions"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("events.jsonl");
    let events = fs::read_to_string(session)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    let binding = &events[0]["payload"]["binding"]["definition"];
    assert_eq!(binding["id"], "configured-writer");
    assert_eq!(binding["digest"].as_str().map(str::len), Some(64));
    assert!(definition.exists());
}

#[test]
fn resume_rejects_a_tampered_durable_definition_snapshot() {
    let root = TemporaryDirectory::new("agent-definition-binding-integrity");
    let definition = export(root.path(), "writer", "bound-writer");
    let config = write_config(root.path(), Some("writer"), None);
    assert!(command(root.path(), &["init"]).status.success());
    assert!(
        command(root.path(), &["--config", config.to_str().unwrap(), "chat"])
            .status
            .success()
    );
    let events_path = fs::read_dir(root.path().join(".echidna/sessions"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("events.jsonl");
    let mut events = fs::read_to_string(&events_path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    let digest = events[0]["payload"]["binding"]["definition"]["digest"]
        .as_str()
        .unwrap()
        .to_owned();
    let replacement = if &digest[..1] == "0" { "1" } else { "0" };
    events[0]["payload"]["binding"]["definition"]["digest"] =
        Value::String(format!("{replacement}{}", &digest[1..]));
    fs::write(
        &events_path,
        events
            .iter()
            .map(serde_json::to_string)
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .join("\n")
            + "\n",
    )
    .unwrap();

    let resume = command(
        root.path(),
        &[
            "--config",
            config.to_str().unwrap(),
            "chat",
            "--resume",
            "latest",
        ],
    );
    assert!(!resume.status.success());
    assert!(String::from_utf8_lossy(&resume.stderr).contains("digest mismatch"));
    assert!(definition.exists());
}

#[test]
fn exported_builtin_research_definition_matches_current_prompt_resources() {
    let root = TemporaryDirectory::new("agent-definition-research-export");
    let exported_path = root.path().join("research");
    let output = command(
        root.path(),
        &[
            "agent",
            "init",
            exported_path.to_str().unwrap(),
            "--from",
            "builtin-research",
        ],
    );
    assert!(output.status.success(), "stderr: {:?}", output.stderr);

    assert_eq!(
        fs::read_to_string(exported_path.join("persona.md")).unwrap(),
        BaseContract.prompt()
    );
    for domain in DomainProfile::ALL {
        assert_eq!(
            fs::read_to_string(exported_path.join(format!("research/domain/{}.md", domain.id())),)
                .unwrap(),
            domain.prompt()
        );
    }
    for phase in ResearchPhase::ALL {
        assert_eq!(
            fs::read_to_string(exported_path.join(format!("research/phase/{}.md", phase.id())),)
                .unwrap(),
            phase.prompt()
        );
    }
}
