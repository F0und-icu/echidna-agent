//! CLI-only reasoning presentation and compile-time-gated wire diagnostics.

use crate::terminal_console as console;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(debug_assertions)]
use serde::Serialize;
#[cfg(debug_assertions)]
use serde_json::{Value, json};

#[cfg(debug_assertions)]
use super::redaction::WireRedaction;
use crate::domain::model::ModelEvent;
#[cfg(debug_assertions)]
use crate::terminal_content::ContentView;
use crate::terminal_content::escaped_text;

static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);

pub(super) fn discarded_partial_response(role: &str, retry: usize) {
    let _ = writeln!(
        console::stdout(),
        "\n[response interrupted; partial {role} output discarded; retry {retry}/2]"
    );
}

pub(super) struct RequestOutput {
    role: &'static str,
    request_id: u64,
    credential: String,
    #[cfg(debug_assertions)]
    account: Option<String>,
    saw_reasoning: bool,
    pending_reasoning_deltas: String,
    reasoning_line_open: bool,
    saw_reasoning_summary: bool,
    reasoning_summary_line_open: bool,
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
        #[cfg(debug_assertions)] account: Option<String>,
        #[cfg(debug_assertions)] full: bool,
    ) -> Self {
        Self {
            role,
            request_id: NEXT_REQUEST.fetch_add(1, Ordering::Relaxed),
            credential,
            #[cfg(debug_assertions)]
            account,
            saw_reasoning: false,
            pending_reasoning_deltas: String::new(),
            reasoning_line_open: false,
            saw_reasoning_summary: false,
            reasoning_summary_line_open: false,
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
                    self.sanitize_wire(&mut value);
                    let mode = if value.get("previous_response_id").is_some() {
                        "增量（仅本次发送内容）"
                    } else {
                        "Full"
                    };
                    let mut stderr = console::stderr();
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
                    if value.get("type").and_then(Value::as_str) == Some("error") {
                        // The decoder alone projects this event's selected, bounded
                        // and credential-redacted fields in every build mode.
                        return;
                    }
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
        if data.len() > super::MAX_HTTP_ERROR_BODY_BYTES {
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
        self.end_reasoning_summary_line();
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
                    let mut stderr = console::stderr();
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

    pub(super) fn reasoning_summary(&mut self, text: &str, starts_part: bool) {
        if text.is_empty() {
            return;
        }
        self.end_reasoning_line();
        if starts_part && self.saw_reasoning_summary {
            self.end_reasoning_summary_line();
        }
        let mut stderr = console::stderr();
        if !self.saw_reasoning_summary {
            let _ = writeln!(
                stderr,
                "[推理摘要 {} #{} · 未完成]",
                self.role, self.request_id
            );
            self.saw_reasoning_summary = true;
        }
        let text = redact_text(text, &self.credential);
        let rendered = escaped_text(&text);
        let _ = stderr.write_all(rendered.as_bytes());
        self.reasoning_summary_line_open = !text.ends_with('\n');
        let _ = stderr.flush();
    }

    pub(super) fn completed(&mut self) {
        self.completed = true;
        self.end_reasoning_line();
        self.end_reasoning_summary_line();
        if self.saw_reasoning_summary {
            let _ = writeln!(console::stderr(), "[推理摘要已完成]");
        }
        if self.full_debug() {
            #[cfg(debug_assertions)]
            self.emit("响应已完成", Value::Null);
        } else if self.saw_reasoning {
            self.end_reasoning_line();
            let _ = writeln!(console::stderr(), "[思考响应已完成]");
        }
    }

    pub(super) fn rejected(&mut self, error: &crate::ports::model::ModelProviderError) {
        self.rejected = true;
        self.end_reasoning_line();
        self.end_reasoning_summary_line();
        let protocol_error = matches!(
            error,
            crate::ports::model::ModelProviderError::ProtocolFailure { .. }
                | crate::ports::model::ModelProviderError::InvalidToolArguments { .. }
        );
        if self.saw_reasoning_summary {
            let _ = writeln!(
                console::stderr(),
                "[推理摘要未提交：{}]",
                if protocol_error {
                    "协议解码失败"
                } else {
                    "Provider 返回停止状态"
                }
            );
        }
        #[cfg(debug_assertions)]
        if self.full {
            self.emit(
                if protocol_error {
                    "协议解码失败：响应未提交（不代表网络中断）"
                } else {
                    "Provider 返回停止状态：响应未提交"
                },
                json!("失败内容未展示；无法保证结构化脱敏。Adapter 不修复或重试；是否重新生成由 AgentLoop 决定。"),
            );
            return;
        }
        if self.saw_reasoning {
            let _ = writeln!(
                console::stderr(),
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
        let mut stderr = console::stderr();
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
            let _ = writeln!(console::stderr());
            self.reasoning_line_open = false;
        }
    }

    fn end_reasoning_summary_line(&mut self) {
        if self.reasoning_summary_line_open {
            let _ = writeln!(console::stderr());
            self.reasoning_summary_line_open = false;
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
        self.sanitize_wire(&mut payload);
        let _ = writeln!(
            console::stderr(),
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

    #[cfg(debug_assertions)]
    fn sanitize_wire(&self, value: &mut Value) {
        WireRedaction::new(&[&self.credential, self.account.as_deref().unwrap_or("")])
            .sanitize(value);
    }
}

impl Drop for RequestOutput {
    fn drop(&mut self) {
        if self.completed || self.rejected {
            return;
        }
        self.end_reasoning_summary_line();
        if self.saw_reasoning_summary {
            let _ = writeln!(console::stderr(), "[推理摘要中断：响应未完成]");
        }
        if self.full_debug() {
            self.end_reasoning_line();
            #[cfg(debug_assertions)]
            self.emit("响应中断：未观察到完成", Value::Null);
        } else if self.saw_reasoning {
            self.end_reasoning_line();
            let _ = writeln!(console::stderr(), "[思考中断：响应未完成]");
        }
    }
}

fn redact_text(text: &str, credential: &str) -> String {
    if credential.is_empty() {
        text.to_owned()
    } else {
        text.replace(credential, "[redacted]")
    }
}
