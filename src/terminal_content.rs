//! Local terminal projections only; never changes model input or durable facts.

use serde_json::Value;

/// Preserve readable whitespace without interpreting terminal control sequences.
pub(crate) fn escaped_text(text: &str) -> String {
    let mut rendered = String::new();
    for character in text.chars() {
        escape_character(character, |character| rendered.push(character));
    }
    rendered
}

fn escape_character(character: char, mut emit: impl FnMut(char)) {
    if character.is_control() && !matches!(character, '\n' | '\t') {
        character.escape_default().for_each(emit);
    } else {
        emit(character);
    }
}

pub(crate) struct ContentView {
    text: String,
    remaining_bytes: usize,
    remaining_lines: usize,
    truncated: bool,
}

impl ContentView {
    pub(crate) fn preview(value: &Value) -> String {
        Self::render(value, 4096, 36)
    }

    #[cfg(debug_assertions)]
    pub(crate) fn full(value: &Value) -> String {
        Self::render(value, usize::MAX, usize::MAX)
    }

    fn render(value: &Value, bytes: usize, lines: usize) -> String {
        let mut view = Self {
            text: String::new(),
            remaining_bytes: bytes,
            remaining_lines: lines,
            truncated: false,
        };
        view.value(value, 0);
        if view.truncated {
            view.text
                .push_str("\n  … [仅显示预览；完整结果保留在原有记录中]\n");
        }
        view.text
    }

    fn push(&mut self, text: &str) {
        for character in text.chars() {
            if self.truncated || self.remaining_bytes == 0 || self.remaining_lines == 0 {
                self.truncated = true;
                break;
            }
            // Escape incrementally, without allocating a full unbounded Tool value.
            escape_character(character, |rendered| {
                if rendered.len_utf8() > self.remaining_bytes {
                    self.truncated = true;
                } else if !self.truncated {
                    self.remaining_bytes -= rendered.len_utf8();
                    if rendered == '\n' {
                        self.remaining_lines -= 1;
                    }
                    self.text.push(rendered);
                }
            });
        }
    }

    fn value(&mut self, value: &Value, depth: usize) {
        if self.truncated {
            return;
        }
        match value {
            Value::Object(fields) if !fields.is_empty() => {
                for (key, value) in fields {
                    self.push(&"  ".repeat(depth));
                    self.push(key);
                    self.push(": ");
                    self.child(value, depth);
                    if self.truncated {
                        break;
                    }
                }
            }
            Value::Array(items) if !items.is_empty() => {
                for item in items {
                    self.push(&"  ".repeat(depth));
                    self.push("- ");
                    self.child(item, depth);
                    if self.truncated {
                        break;
                    }
                }
            }
            Value::String(text) => {
                self.push(text);
                self.push("\n");
            }
            scalar => {
                self.push(&scalar.to_string());
                self.push("\n");
            }
        }
    }

    fn child(&mut self, value: &Value, depth: usize) {
        if value.is_object() || value.is_array() {
            self.push("\n");
            self.value(value, depth + 1);
        } else {
            self.value(value, depth);
        }
    }
}
