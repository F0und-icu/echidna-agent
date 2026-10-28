//! Neutral 23-tool / 272k equivalent of the reported capacity failure.
use super::*;
use echidna::ports::tool::ToolExecutor;

#[test]
fn clerk_equivalent_keeps_23_tools_large_mcp_mirrors_and_original_image_across_compaction() {
    let root = TemporaryDirectory::new("context2-clerk-equivalent");
    initialize_project(root.path());
    fs::write(root.path().join("neutral.png"), png_bytes(16, 12)).unwrap();
    let remote_names = (0..8)
        .map(|n| format!("fixture.metric_{n}"))
        .collect::<Vec<_>>();
    let aliases = remote_names
        .iter()
        .map(|name| {
            format!(
                "mcp__fixture__{}",
                &blake3::hash(name.as_bytes()).to_hex()[..20]
            )
        })
        .collect::<Vec<_>>();
    let tools=remote_names.iter().map(|name|json!({
        "name":name,"description":"Neutral offline metric",
        "inputSchema":{"type":"object","properties":{
            "round":{"type":"integer","minimum":1},
            "scope":{"type":"string","description":"neutral schema documentation ".repeat(100)}
        },"required":["round"],"additionalProperties":false}
    })).collect::<Vec<_>>();
    let catalog = || {
        format!(
            "data: {}\n\n",
            json!({"jsonrpc":"2.0","id":"1","result":{"tools":tools}})
        )
    };
    let structured = json!({"payload":"N".repeat(68_000),"status":"complete"});
    let mirror = json!({"content":[{"type":"text","text":structured.to_string()}],"structuredContent":structured,"isError":false});
    assert!((135_000..140_000).contains(&mirror.to_string().len()));
    let mut mcp_responses = vec![(200, catalog())];
    for n in 2..=6 {
        mcp_responses.push((
            200,
            format!(
                "data: {}\n\n",
                json!({"jsonrpc":"2.0","id":n.to_string(),"result":mirror})
            ),
        ));
    }
    mcp_responses.push((200, catalog())); // a fresh client discovers on explicit restart
    let mcp = spawn_responses_server(mcp_responses);
    let definition = root.path().join("clerk");
    fs::create_dir(&definition).unwrap();
    fs::write(
        definition.join("persona.md"),
        "A precise neutral order-clerk fixture.",
    )
    .unwrap();
    fs::write(
        definition.join("work.md"),
        "Retain the exact operator constraints and finish the requested offline work.",
    )
    .unwrap();
    let keep = [
        "read_file",
        "read_files",
        "list_files",
        "search_files",
        "write_file",
        "apply_patch",
        "run_command",
        "run_commands",
        "http_request",
        "view_image",
        "read_output",
        "inspect_project",
        "memory_read",
        "memory_write",
        "tool_search",
    ];
    let registry = echidna::adapters::tools::ToolRegistry::for_project(root.path());
    assert!(keep.iter().all(|name| {
        registry
            .catalog()
            .iter()
            .any(|spec| spec.name.as_str() == *name)
    }));
    let mut manifest = String::from(
        "schema_version = 1\nid = \"context2-clerk\"\nname = \"Context2 clerk\"\n[instructions]\npersona_file = \"persona.md\"\nwork_file = \"work.md\"\nmethod_files = []\n[context]\npersonal = \"none\"\nproject = \"none\"\nmemory = \"agent\"\n[tools]\n",
    );
    for name in keep
        .iter()
        .copied()
        .chain(aliases.iter().map(String::as_str))
    {
        manifest.push_str(&format!("{name} = \"direct\"\n"));
    }
    fs::write(definition.join("agent.toml"), manifest).unwrap();
    let mut model_responses = (1..=5)
        .map(|n| {
            (
                200,
                m4_long_flow_tool_response(
                    &format!("metric-{n}"),
                    &aliases[0],
                    json!({"round":n}),
                    &format!("metric-response-{n}"),
                ),
            )
        })
        .collect::<Vec<_>>();
    model_responses.push((200,compaction_summary_response("Completed earlier neutral metric polls; retain original limit and image. Latest batch remains exact.","clerk-handoff")));
    model_responses.push((200, text_response("NEUTRAL-WORK-COMPLETE", "clerk-final")));
    let model = spawn_responses_server(model_responses);
    let config = write_context_window_config(root.path(), &model.base_url, 272_000);
    let encoded = fs::read_to_string(&config)
        .unwrap()
        .replace("image_input = false", "image_input = true")
        .replace(
            "vision_mode = \"secondary_tool\"",
            "vision_mode = \"primary_direct\"",
        );
    fs::write(&config,format!("{encoded}\n[runtime]\nauto_compact = true\n[mcp_servers.fixture]\nurl = {}\nprotocol_version = \"2026-07-28\"\n",serde_json::to_string(&mcp.base_url).unwrap())).unwrap();
    let task =
        "Complete five neutral metric polls; preserve ORIGIN-LIMIT-ALPHA and the original image.";
    let output = run_chat(
        root.path(),
        &config,
        &format!("/image neutral.png\n{task}\n/exit\n"),
        &["--agent", definition.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let wires = (0..7)
        .map(|_| request_body(&model.requests.recv_timeout(F2A_WAIT).unwrap()))
        .collect::<Vec<_>>();
    model.join();
    assert_eq!(wires[0]["tools"].as_array().unwrap().len(), 23);
    for wire in &wires[..5] {
        assert_eq!(wire["tools"], wires[0]["tools"]);
    }
    assert!(wires[5].get("tools").is_none());
    assert_eq!(wires[6]["tools"], wires[0]["tools"]);
    for wire in [&wires[0], &wires[6]] {
        assert!(json_contains_text(&wire["input"], task));
        assert_eq!(wire["input"].to_string().matches(task).count(), 1);
        assert_eq!(
            wire["input"]
                .to_string()
                .matches("data:image/png;base64,")
                .count(),
            1
        );
    }
    let project = Project::discover(root.path()).unwrap();
    let state = reduced_session(&project, &session_id(root.path()));
    let results = state
        .history()
        .items()
        .iter()
        .filter_map(|item| match item {
            echidna::domain::model::ContentItem::ToolResult { result }
                if result.tool_name.as_str() == aliases[0] =>
            {
                Some(result)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(results.len(), 5);
    for result in results {
        assert_eq!(result.output["mcp_result"], mirror);
    }
    assert_eq!(
        state
            .latest_compaction()
            .unwrap()
            .summary()
            .schema_version(),
        6
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("NEUTRAL-WORK-COMPLETE"));
    let restarted = run_chat(
        root.path(),
        &config,
        "/context\n/exit\n",
        &["--resume", "latest"],
    );
    assert!(
        restarted.status.success(),
        "{}",
        String::from_utf8_lossy(&restarted.stderr)
    );
    let requests = (0..7)
        .map(|_| request_body(&mcp.requests.recv_timeout(F2A_WAIT).unwrap()))
        .collect::<Vec<_>>();
    mcp.join();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request["method"] == "tools/call")
            .count(),
        5
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request["method"] == "tools/list")
            .count(),
        2
    );
    assert_eq!(state.execution_receipt_count(), 6); // one image intake + five actual fixture calls

    // The graph and Summary remain genuine; forging only the retained suffix
    // must not erase the last settled batch of the still-unfinished Turn.
    let id = session_id(root.path());
    let mut events = session_events(root.path(), &id);
    let compaction = events
        .iter_mut()
        .find(|event| event["event_type"] == "session.compaction_created")
        .unwrap();
    let payload = &mut compaction["payload"];
    let cutoff = payload["retained_from_normalized_history_index"]
        .as_u64()
        .unwrap();
    let retained = payload["retained_history"].as_array().unwrap().len();
    assert!(
        retained >= 2,
        "the genuine handoff retains the latest batch"
    );
    payload["retained_from_normalized_history_index"] = json!(cutoff + retained as u64);
    payload["retained_history"] = json!([]);
    let forged = events
        .into_iter()
        .map(|event| serde_json::from_value::<EventEnvelope<Value>>(event).unwrap());
    let error = SessionReducer.reduce(forged).unwrap_err();
    assert!(error.to_string().contains("retained History"), "{error}");
}
