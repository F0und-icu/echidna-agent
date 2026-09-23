//! Terminal steering uses the same cancellation boundary as Ctrl+C.

use std::future::Future;
use std::io;

use tokio::io::{AsyncBufRead, Lines};

use crate::app::cancellation::AgentCancellation;

/// Pipes keep their sequential input contract. Only actual terminal lines
/// interrupt a running Turn; a queued line is processed by the normal CLI
/// boundary after cancellation settles, never inside the sampling plan.
pub(super) async fn interruptible<F, R>(
    operation: F,
    cancellation: &AgentCancellation,
    lines: &mut Lines<R>,
    terminal: bool,
    closed: &mut bool,
    pending: &mut Option<String>,
) -> io::Result<F::Output>
where
    F: Future,
    R: AsyncBufRead + Unpin,
{
    tokio::pin!(operation);
    let mut read_terminal = terminal && !*closed;
    loop {
        tokio::select! {
            // A ready terminal correction takes precedence over a ready next
            // model/tool poll. No already-started effect is claimed undone.
            biased;
            signal = tokio::signal::ctrl_c() => {
                signal?;
                cancellation.cancel();
                return Ok(operation.await);
            }
            line = lines.next_line(), if read_terminal => {
                match line {
                    Ok(Some(line)) if line.trim().is_empty() => continue,
                    Ok(Some(line)) => {
                        if line != "/stop" {
                            *pending = Some(line);
                        }
                        eprintln!("\n[interrupt requested] Waiting for the current operation to settle; no rollback or background-process stop is implied.");
                        cancellation.cancel();
                        return Ok(operation.await);
                    }
                    Ok(None) => {
                        *closed = true;
                        read_terminal = false;
                    }
                    Err(error) => {
                        eprintln!("[input rejected] {error}; cancelling the current Turn. Re-enter your instruction after the stop.");
                        cancellation.cancel();
                        // Preserve an App/durability error from cancellation;
                        // a malformed terminal line must not mask its outcome.
                        return Ok(operation.await);
                    }
                }
            }
            result = &mut operation => return Ok(result),
        }
    }
}

pub(super) fn reject_pending(pending: &mut Option<String>) {
    if pending.take().is_some() {
        eprintln!(
            "[queued input not submitted or saved] Resolve the stop, then re-enter your instruction explicitly. No automatic retry."
        );
    }
}
