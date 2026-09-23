//! CLI-only reasoning presentation and compile-time-gated wire diagnostics.

use std::io::{self, Write};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(debug_assertions)]
use serde::Serialize;
#[cfg(debug_assertions)]
use serde_json::{Value, json};

use crate::domain::model::ModelEvent;
#[cfg(debug_assertions)]
use crate::terminal_content::ContentView;
use crate::terminal_content::escaped_text;

static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);

pub(super) struct RequestOutput {
    role: &'static str,
    request_id: u64,
    credential: String,
    saw_reasoning: bool,
    pending_reasoning_deltas: String,
    reasoning_line_open: bool,
    completed: bool,
    rejected: bool,
    #[cfg(debug_assertions)]
    vision_text_open: bool,
    #[cfg(debug_assertions)]
    full: bool,
}

impl RequestOutput {
    pub(super) fn new(
        role: &'static str,
        credential: String,
        #[cfg(debug_assertions)] full: bool,
    ) -> Self {
        Self {
            role,
            request_id: NEXT_REQUEST.fetch_add(1, Ordering::Relaxed),
            credential,
            saw_reasoning: false,
            pending_reasoning_deltas: String::new(),
            reasoning_line_open: false,
            completed: false,
            rejected: false,
            #[cfg(debug_assertions)]
            vision_text_open: false,
            #[cfg(debug_assertions)]
            full,
        }
    }

    #[cfg(debug_assertions)]
    pub(super) fn request(&self, request: &impl Serialize) {
        if self.full {
            match serde_json::to_value(request) {
                Ok(mut value) => {
                    sanitize_wire(&mut value);
                    redact_credential(&mut value, &self.credential);
                    let mode = if value.get("previous_response_id").is_some() {
                        "增量（仅本次发送内容）"
                    } else {
                        "Full"
                    };
                    let mut stderr = io::stderr().lock();
                    let _ = writeln!(
                        stderr,
                        "\n[debug 发送 {} #{} · {mode}]",
                        self.role, self.request_id
                    );
                    let _ = writeln!(
                        stderr,
                        "[内容视图：省略工具定义及传输字段；已脱敏，非逐字 wire 抓包]"
                    );
                    for (key, label) in [
                        ("instructions", "提示词"),
                        ("input", "输入消息"),
                        ("text", "响应格式"),
                    ] {
                        if let Some(content) = value.get(key) {
                            let _ =
                                writeln!(stderr, "── {label} ──\n{}", ContentView::full(content));
                        }
                    }
                    let _ = writeln!(
                        stderr,
                        "[debug 发送结束；模型正文{}]\n",
                        if self.role == "primary" {
                            "见 assistant 输出"
                        } else {
                            "见下方 Vision 输出"
                        }
                    );
                }
                Err(_) => self.emit("diagnostic_error", json!("request view unavailable")),
            }
        }
    }

    #[cfg(debug_assertions)]
    pub(super) fn status(&self, status: u16) {
        if self.full && status >= 400 {
            self.emit("HTTP 错误", json!(status));
        }
    }

    #[cfg(debug_assertions)]
    pub(super) fn response(&self, data: &str) {
        if self.full {
            match serde_json::from_str::<Value>(data) {
                Ok(value) => {
                    // Routine SSE wrappers are not content. Semantic output is
                    // observed through the existing decoder below, not reparsed here.
                    if let Some(error) = value.get("error").or_else(|| {
                        value
                            .get("response")
                            .and_then(|response| response.get("error"))
                    }) && !error.is_null()
                    {
                        self.emit("模型错误", error.clone());
                    }
                }
                Err(_) => {
                    self.response_omitted("non-JSON response cannot be structurally redacted")
                }
            }
        }
    }

    #[cfg(debug_assertions)]
    pub(super) fn http_error_body(&self, data: &str) {
        if data.len() > super::MAX_DEBUG_ERROR_BODY_BYTES {
            self.response_omitted("HTTP error body omitted: exceeds 65536 bytes");
        } else if self.full {
            match serde_json::from_str(data) {
                Ok(value) => self.emit("HTTP 响应", value),
                Err(_) => {
                    self.response_omitted("non-JSON response cannot be structurally redacted")
                }
            }
        }
    }

    #[cfg(debug_assertions)]
    pub(super) fn response_omitted(&self, reason: &str) {
        if self.full {
            self.emit("response_omitted", json!(reason));
        }
    }

    pub(super) fn model_event(&mut self, event: &ModelEvent) {
        match event {
            ModelEvent::ReasoningDelta { text } => {
                self.pending_reasoning_deltas.push_str(text);
                self.reasoning_text(text);
            }
            ModelEvent::ReasoningItem { text } => {
                if self.pending_reasoning_deltas != *text {
                    self.end_reasoning_line();
                    self.reasoning_text(text);
                }
                self.pending_reasoning_deltas.clear();
                self.end_reasoning_line();
            }
            ModelEvent::TextDelta { text } => {
                self.end_reasoning_line();
                #[cfg(debug_assertions)]
                if self.full && self.role == "vision" {
                    let mut stderr = io::stderr().lock();
                    if !self.vision_text_open {
                        let _ = writeln!(stderr, "[debug Vision 输出 · 未完成]");
                        self.vision_text_open = true;
                    }
                    let _ = write!(
                        stderr,
                        "{}",
                        escaped_text(&redact_text(text, &self.credential))
                    );
                    let _ = stderr.flush();
                }
                #[cfg(not(debug_assertions))]
                let _ = text;
            }
            ModelEvent::ToolCall { call } => {
                self.end_reasoning_line();
                #[cfg(debug_assertions)]
                if self.full {
                    self.emit(
                        "模型工具调用（尚未执行）",
                        json!({"name":call.name, "arguments":call.arguments}),
                    );
                }
                #[cfg(not(debug_assertions))]
                let _ = call;
            }
            _ => {}
        }
    }

    pub(super) fn completed(&mut self) {
        self.completed = true;
        self.end_reasoning_line();
        if self.full_debug() {
            #[cfg(debug_assertions)]
            self.emit("响应已完成", Value::Null);
        } else if self.saw_reasoning {
            self.end_reasoning_line();
            let _ = writeln!(io::stderr().lock(), "[思考响应已完成]");
        }
    }

    pub(super) fn rejected(&mut self, error: &crate::ports::model::ModelProviderError) {
        self.rejected = true;
        self.end_reasoning_line();
        let protocol_error = matches!(
            error,
            crate::ports::model::ModelProviderError::ProtocolFailure { .. }
        );
        #[cfg(debug_assertions)]
        if self.full {
            self.emit(
                if protocol_error {
                    "协议解码失败：响应未提交（不代表网络中断）"
                } else {
                    "Provider 返回停止状态：响应未提交"
                },
                json!("失败内容未展示；无法保证结构化脱敏。不自动修复或重试。"),
            );
            return;
        }
        if self.saw_reasoning {
            let _ = writeln!(
                io::stderr().lock(),
                "[思考响应未提交：{}]",
                if protocol_error {
                    "协议解码失败"
                } else {
                    "Provider 返回停止状态"
                }
            );
        }
    }

    fn reasoning_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let mut stderr = io::stderr().lock();
        if !self.saw_reasoning {
            let _ = writeln!(stderr, "[思考 {} #{} · 未完成]", self.role, self.request_id);
            self.saw_reasoning = true;
        }
        let text = redact_text(text, &self.credential);
        // Preserve readable whitespace, but never execute model-supplied terminal controls.
        let rendered = escaped_text(&text);
        let _ = stderr.write_all(rendered.as_bytes());
        self.reasoning_line_open = !text.ends_with('\n');
        let _ = stderr.flush();
    }

    fn end_reasoning_line(&mut self) {
        if self.reasoning_line_open {
            let _ = writeln!(io::stderr().lock());
            self.reasoning_line_open = false;
        }
    }

    fn full_debug(&self) -> bool {
        #[cfg(debug_assertions)]
        {
            self.full
        }
        #[cfg(not(debug_assertions))]
        {
            false
        }
    }

    #[cfg(debug_assertions)]
    fn emit(&self, kind: &str, mut payload: Value) {
        if self.full {
            sanitize_wire(&mut payload);
        }
        redact_credential(&mut payload, &self.credential);
        let _ = writeln!(
            io::stderr().lock(),
            "\n[debug {} #{} · {kind}]\n{}",
            self.role,
            self.request_id,
            if payload.is_null() {
                String::new()
            } else {
                ContentView::full(&payload)
            }
        );
    }
}

impl Drop for RequestOutput {
    fn drop(&mut self) {
        if self.completed || self.rejected {
            return;
        }
        if self.full_debug() {
            self.end_reasoning_line();
            #[cfg(debug_assertions)]
            self.emit("响应中断：未观察到完成", Value::Null);
        } else if self.saw_reasoning {
            self.end_reasoning_line();
            let _ = writeln!(io::stderr().lock(), "[思考中断：响应未完成]");
        }
    }
}

#[cfg(debug_assertions)]
fn sanitize_wire(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if matches!(
                    key.to_ascii_lowercase().as_str(),
                    "authorization" | "api_key" | "access_token" | "encrypted_content"
                ) {
                    *value = json!("[redacted]");
                } else {
                    sanitize_wire(value);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(sanitize_wire),
        Value::String(text) if text.starts_with("data:image/") => {
            *text = format!("[image data omitted; {} encoded bytes]", text.len());
        }
        _ => {}
    }
}

#[cfg(debug_assertions)]
fn redact_credential(value: &mut Value, credential: &str) {
    match value {
        Value::String(text) => *text = redact_text(text, credential),
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| redact_credential(item, credential)),
        Value::Object(object) => object
            .values_mut()
            .for_each(|item| redact_credential(item, credential)),
        _ => {}
    }
}

fn redact_text(text: &str, credential: &str) -> String {
    if credential.is_empty() {
        text.to_owned()
    } else {
        text.replace(credential, "[redacted]")
    }
}
