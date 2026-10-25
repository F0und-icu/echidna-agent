//! One structural redaction owner for local request diagnostics.

use serde_json::{Map, Value};

pub(super) struct WireRedaction {
    patterns: Vec<String>,
}

impl WireRedaction {
    pub(super) fn new(secrets: &[&str]) -> Self {
        let mut patterns = Vec::new();
        for secret in secrets.iter().filter(|secret| !secret.is_empty()) {
            patterns.push((*secret).to_owned());
            let encoded = serde_json::to_string(secret).expect("a string is JSON serializable");
            patterns.push(encoded[1..encoded.len() - 1].to_owned());
        }
        patterns.sort_by_key(|pattern| std::cmp::Reverse(pattern.len()));
        patterns.dedup();
        Self { patterns }
    }

    pub(super) fn text(&self, text: &str) -> String {
        self.patterns.iter().fold(text.to_owned(), |text, pattern| {
            text.replace(pattern, "[redacted]")
        })
    }

    pub(super) fn sanitize(&self, value: &mut Value) {
        match value {
            Value::Object(object) => {
                let mut sanitized = Map::new();
                for (key, mut value) in std::mem::take(object) {
                    if sensitive_field(&key) {
                        value = Value::String("[redacted]".to_owned());
                    } else {
                        self.sanitize(&mut value);
                    }
                    let key = self.text(&key);
                    let mut unique_key = key.clone();
                    let mut collision = 0;
                    // Different secret-bearing keys can redact to the same key.
                    // Preserve every member instead of silently overwriting one.
                    while sanitized.contains_key(&unique_key) {
                        collision += 1;
                        unique_key = format!("{key} [redacted-key collision {collision}]");
                    }
                    sanitized.insert(unique_key, value);
                }
                *object = sanitized;
            }
            Value::Array(items) => items.iter_mut().for_each(|item| self.sanitize(item)),
            Value::String(text) => {
                // Tool arguments/results may themselves be serialized JSON.
                if let Ok(mut nested @ (Value::Object(_) | Value::Array(_))) =
                    serde_json::from_str(text)
                {
                    let original = nested.clone();
                    self.sanitize(&mut nested);
                    if nested != original {
                        *text = nested.to_string();
                    }
                }
                *text = self.text(&omit_image_data(text));
            }
            _ => {}
        }
    }
}

fn sensitive_field(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "authorization"
            | "proxy-authorization"
            | "cookie"
            | "cookies"
            | "set-cookie"
            | "api_key"
            | "api-key"
            | "x-api-key"
            | "access_token"
            | "refresh_token"
            | "chatgpt-account-id"
            | "account_id"
            | "encrypted_content"
    )
}

fn omit_image_data(text: &str) -> String {
    let mut remaining = text;
    let mut sanitized = String::new();
    while let Some(start) = remaining.find("data:image/") {
        sanitized.push_str(&remaining[..start]);
        let image = &remaining[start..];
        let end = image
            .find(|character: char| {
                character.is_whitespace() || matches!(character, '"' | '\'' | '<' | '>' | '\\')
            })
            .unwrap_or(image.len());
        sanitized.push_str(&format!("[image data omitted; {end} encoded bytes]"));
        remaining = &image[end..];
    }
    sanitized.push_str(remaining);
    sanitized
}
