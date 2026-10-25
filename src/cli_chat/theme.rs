//! Presentation only: no theme text enters model input or durable History.
use std::io::{self, IsTerminal};

pub(super) struct Theme {
    terminal: bool,
    color: bool,
}

impl Theme {
    pub(super) fn detect() -> Self {
        let terminal = io::stdin().is_terminal() && io::stdout().is_terminal();
        let color = terminal
            && std::env::var_os("NO_COLOR").is_none()
            && std::env::var_os("TERM").is_none_or(|term| term != "dumb");
        Self { terminal, color }
    }

    pub(super) fn welcome(&self) {
        if self.terminal {
            if self.color {
                console_println!("\x1b[35mEchidna\x1b[0m · Local Agent");
            } else {
                console_println!("Echidna · Local Agent");
            }
            console_println!("Question. Observe. Verify.");
        }
    }

    pub(super) fn answer_prefix(&self) -> &'static str {
        if self.color {
            "\x1b[35mEchidna> \x1b[0m"
        } else {
            "Echidna> "
        }
    }
}
