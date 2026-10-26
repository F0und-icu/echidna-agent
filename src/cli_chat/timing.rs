//! Content-free timing at the existing CLI preparation and request boundaries.

use std::cell::Cell;
use std::time::Instant;

pub(super) struct TurnTiming {
    enabled: bool,
    start: Instant,
    previous: Cell<Instant>,
    first_text: Cell<bool>,
}

impl TurnTiming {
    pub(super) fn new(enabled: bool) -> Self {
        let start = Instant::now();
        Self {
            enabled,
            start,
            previous: Cell::new(start),
            first_text: Cell::new(false),
        }
    }

    pub(super) fn checkpoint(&self, phase: &'static str) {
        if self.enabled {
            let now = Instant::now();
            let previous = self.previous.replace(now);
            console_eprintln!(
                "[debug timing] phase={phase} stage_ms={} turn_ms={}",
                now.duration_since(previous).as_millis(),
                now.duration_since(self.start).as_millis()
            );
        }
    }

    pub(super) fn observe_text(&self, text: &str) {
        if !text.is_empty() && !self.first_text.replace(true) {
            self.checkpoint("first_assistant_text");
        }
    }
}
