use std::process::Command;

fn git(arguments: &[&str]) -> Option<String> {
    let output = Command::new("git").args(arguments).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let root = std::fs::canonicalize(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).ok();
    let git_root =
        git(&["rev-parse", "--show-toplevel"]).and_then(|path| std::fs::canonicalize(path).ok());
    if root.is_none() || root != git_root {
        println!("cargo:rustc-env=ECHIDNA_SOURCE_REVISION=unknown (source archive)");
        return;
    }
    // Track both symbolic HEAD and its loose/packed ref, including Git worktrees.
    for name in ["HEAD", "packed-refs"] {
        if let Some(path) = git(&["rev-parse", "--git-path", name]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    if let Some(reference) = git(&["symbolic-ref", "-q", "HEAD"])
        && let Some(path) = git(&["rev-parse", "--git-path", &reference])
    {
        println!("cargo:rerun-if-changed={path}");
    }
    let revision = git(&["rev-parse", "--verify", "HEAD"])
        .filter(|value| value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .unwrap_or_else(|| "unknown (source archive)".to_owned());
    println!("cargo:rustc-env=ECHIDNA_SOURCE_REVISION={revision}");
}
