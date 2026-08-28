pub fn fixture() -> String {
    r#"
schema_version = 1
active_agent_profile = "default"

[providers.primary]
binding = "http_sse"
wire_api = "responses"
base_url = "http://127.0.0.1:1/v1"
model = "fixture-text-model"
credential_ref = "CYBER_AGENT_PRIMARY_API_KEY"
capability_mapping = "text-v1"
generation = 1

[providers.vision]
binding = "http_sse"
wire_api = "responses"
base_url = "http://127.0.0.1:1/v1"
model = "fixture-vision-model"
credential_ref = "CYBER_AGENT_VISION_API_KEY"
capability_mapping = "vision-v1"
generation = 2

[capability_mappings.text-v1]
version = 1
text_input = true
image_input = false
streaming = true
tool_call = true
structured_output = true
incremental_input = true
usage_reporting = true
context_window = 32768

[capability_mappings.vision-v1]
version = 3
text_input = true
image_input = true
streaming = true
tool_call = false
structured_output = true
incremental_input = false
usage_reporting = true
context_window = 16384

[agent_profiles.default]
primary = "primary"
vision = "vision"
vision_mode = "secondary_tool"
generation = 4
"#
    .trim_start()
    .to_owned()
}
