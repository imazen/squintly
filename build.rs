//! Captures the git commit at build time so exports can carry a `build_commit`
//! field — the discipline the CLAUDE.md ML-data section demands for every
//! auto-generated artifact (so "is this data still valid?" stays a single
//! grep, not a forensic audit).
//!
//! Three layered fallbacks, in order: (1) `SQUINTLY_BUILD_COMMIT` env var if
//! provided (CI plumbs this); (2) `git rev-parse HEAD` if the build host has
//! a git checkout; (3) the literal string `unknown` so the binary always
//! compiles even off-tree (cargo install, vendored builds).

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=SQUINTLY_BUILD_COMMIT");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=build.rs");

    let commit = std::env::var("SQUINTLY_BUILD_COMMIT")
        .ok()
        .or_else(|| {
            Command::new("git")
                .args(["rev-parse", "HEAD"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=SQUINTLY_BUILD_COMMIT={commit}");
}
