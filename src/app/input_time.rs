//! Preserve the original receive instant through terminal steering.

use std::io;

use crate::domain::session::TaskInputTime;
use crate::ports::input_clock::InputClock;

pub(crate) struct ReceivedInput {
    pub(crate) text: String,
    pub(crate) time: TaskInputTime,
}

pub(crate) fn capture_input(text: String, clock: &dyn InputClock) -> io::Result<ReceivedInput> {
    Ok(ReceivedInput {
        text,
        time: clock.now().map_err(io::Error::other)?,
    })
}

pub(crate) fn queue_input(pending: &mut Option<ReceivedInput>, input: ReceivedInput) {
    if input.text != "/stop" {
        *pending = Some(input);
    }
}
