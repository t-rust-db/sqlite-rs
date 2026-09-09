// Copyright 2026 Schuberg Philis
// SPDX-License-Identifier: Apache-2.0
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! `make smoke`'s contract, as a test: the binary builds, and `--help` /
//! `--version` exit 0 with something on stdout. Guards the arm in
//! `src/bin/sqlite-rs/main.rs` -- before it existed, `--help` was taken as
//! a database path (`error: --help: file not found`, exit 1).

use std::process::Command;

fn run(arg: &str) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_sqlite-rs"))
        .arg(arg)
        .output()
        .expect("spawn sqlite-rs");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn help_exits_zero_with_usage_on_stdout() {
    for flag in ["--help", "-h"] {
        let (code, stdout, stderr) = run(flag);
        assert_eq!(code, 0, "{flag}: stderr={stderr}");
        assert!(stdout.starts_with("usage: sqlite-rs "), "{flag}: {stdout}");
        assert!(stderr.is_empty(), "{flag}: {stderr}");
    }
}

#[test]
fn version_exits_zero_with_name_and_version() {
    let (code, stdout, _) = run("--version");
    assert_eq!(code, 0);
    assert_eq!(
        stdout.trim(),
        format!("sqlite-rs {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn misuse_still_exits_two_on_stderr() {
    let (code, stdout, stderr) = run("definitely-not-a-subcommand-nor-a-file/x/y");
    assert_ne!(code, 0);
    assert!(stdout.is_empty());
    assert!(!stderr.is_empty());
}
