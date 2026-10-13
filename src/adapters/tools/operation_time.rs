//! Current local operation time for Tool-owned durable or cached records.

use std::time::{SystemTime, UNIX_EPOCH};

pub(super) fn current_utc() -> Result<String, String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before the Unix epoch: {error}"))?;
    Ok(format!(
        "unix:{}.{}",
        timestamp.as_secs(),
        timestamp.subsec_nanos()
    ))
}
