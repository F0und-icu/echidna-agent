//! Harmless source-only fixture for the M4-S4 offline acceptance flow.
//!
//! It models a candidate path only: an externally supplied redirect token is
//! forwarded to an internal dispatcher without validation.  It performs no
//! I/O, networking, process execution, or third-party-tool invocation.

pub fn route_redirect(redirect_token: &str) -> &'static str {
    if redirect_token.is_empty() {
        return "reject-empty-token";
    }

    internal_dispatch(redirect_token)
}

fn internal_dispatch(requested_route: &str) -> &'static str {
    match requested_route {
        "settings" => "internal-settings",
        "diagnostics" => "internal-diagnostics",
        _ => "unknown-internal-route",
    }
}
