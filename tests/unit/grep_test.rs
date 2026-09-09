// Copyright 2026 Schuberg Philis
// SPDX-License-Identifier: Apache-2.0
//! Integration tests for the `grep` CLI subcommand (issue #45, spec
//! 014-grep). Drives the built `sqlite-rs` binary as a subprocess, the
//! same `Command`+`CARGO_BIN_EXE_sqlite-rs` pattern as
//! `tests/unit/repl_history.rs`.
//!
//! `tests/unit/fixtures/grep_fixture.db` covers text/integer/real/NULL/
//! BLOB columns (built with the `sqlite3` shell — see that file's own
//! header comment). The output line format
//! (`<table>::<row>::<column>::<value>`) is the porting target from
//! `eirtools/sqlgrep` (Apache-2.0) documented in
//! `.openspec/specs/014-grep/spec.md`; this test asserts the *format*
//! is byte-for-byte what that spec documents, not a byte-for-byte
//! replay of eirtools' own fixture (which this port does not vendor —
//! see the spec's descoping note).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Command;

const CLI: &str = env!("CARGO_BIN_EXE_sqlite-rs");

fn fixture(name: &str) -> String {
    format!("{}/tests/unit/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn corpus_fixture(name: &str) -> String {
    format!(
        "{}/tests/corpus/fixtures/journalstates/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

#[test]
fn grep_matches_text_column() {
    let out = Command::new(CLI)
        .args(["grep", "banana", &fixture("grep_fixture.db")])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(stdout, "items::1::name::banana bread\n");
}

#[test]
fn grep_no_match_exits_1() {
    let out = Command::new(CLI)
        .args(["grep", "zzz-no-such-pattern", &fixture("grep_fixture.db")])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
}

#[test]
fn grep_open_error_exits_2() {
    let out = Command::new(CLI)
        .args(["grep", "x", "/no/such/file.db"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}

/// REAL affinity's `3.0` renders with an explicit decimal point (never
/// bare `3`), matching the shell — grepping for the literal decimal
/// point finds it via `-F`.
#[test]
fn grep_fixed_string_matches_real_rendering() {
    let out = Command::new(CLI)
        .args(["grep", "-F", "3.0", &fixture("grep_fixture.db")])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(stdout, "items::0::price::3.0\n");
}

/// `-i` matches case-insensitively.
#[test]
fn grep_ignore_case() {
    let out = Command::new(CLI)
        .args(["grep", "-i", "APPLE", &fixture("grep_fixture.db")])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(stdout, "items::0::name::apple pie\n");
}

/// NULL cells have no `CAST(x AS TEXT)` text form to match against —
/// they never appear as a matching cell, documented as this port's
/// undocumented-in-eirtools NULL decision (spec 014-grep Requirement
/// 2).
#[test]
fn grep_null_cells_never_match() {
    let out = Command::new(CLI)
        .args(["grep", "no name row", &fixture("grep_fixture.db")])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8(out.stdout).unwrap();
    // Only the `note` column's text matches; the row's NULL `name` and
    // NULL `price` columns produce no line.
    assert_eq!(stdout, "items::2::note::no name row\n");
}

/// BLOB cells are skipped by default (`--blob=skip`, the default).
#[test]
fn grep_blob_skipped_by_default() {
    let out = Command::new(CLI)
        .args(["grep", "hello", &fixture("grep_fixture.db")])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
}

/// `--blob=hex` matches the `X'..'`-hex rendering of a BLOB cell.
#[test]
fn grep_blob_hex_mode_matches() {
    let out = Command::new(CLI)
        .args([
            "grep",
            "--blob=hex",
            "68656C6C6F",
            &fixture("grep_fixture.db"),
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(stdout, "items::0::blob_col::X'68656C6C6F'\n");
}

/// Multiple files: `-H`-style filename prefixing kicks in automatically
/// once more than one input file is given.
#[test]
fn grep_multiple_files_prefixes_filename() {
    let out = Command::new(CLI)
        .args([
            "grep",
            "banana",
            &fixture("grep_fixture.db"),
            &fixture("grep_fixture.db"),
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8(out.stdout).unwrap();
    let expected = format!(
        "{p}::items::1::name::banana bread\n{p}::items::1::name::banana bread\n",
        p = fixture("grep_fixture.db")
    );
    assert_eq!(stdout, expected);
}

/// spec 007 Requirement 3: a read-only `grep` must still see rows
/// committed only to uncheckpointed WAL frames, not yet flushed to the
/// main file — reusing the pager's own `wal_pending.db` fixture (three
/// separate commits, none checkpointed).
///
/// Opening a WAL-mode database (even through the read-only-open path
/// `dump::open`/`grep` use) can checkpoint pending WAL frames into the
/// main file as a side effect of opening the pager — this test runs
/// against a throwaway copy rather than the committed fixture directly,
/// so re-running the suite never mutates (and thereby invalidates)
/// `tests/corpus/fixtures/journalstates/wal_pending.db` for
/// `tests/unit/pager_fixtures.rs`'s own tests.
#[test]
fn grep_sees_uncheckpointed_wal_rows() {
    let dir = tempfile_dir();
    let db_copy = dir.join("wal_pending.db");
    let wal_copy = dir.join("wal_pending.db-wal");
    std::fs::copy(corpus_fixture("wal_pending.db"), &db_copy).unwrap();
    std::fs::copy(corpus_fixture("wal_pending.db-wal"), &wal_copy).unwrap();

    let out = Command::new(CLI)
        .args(["grep", "-F", "three", db_copy.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(stdout, "t::2::b::three\n");

    std::fs::remove_dir_all(&dir).ok();
}

/// A fresh, uniquely-named scratch directory under the OS temp dir —
/// this crate has no `tempfile` runtime dependency, so this is a
/// hand-rolled equivalent narrow enough not to need one.
fn tempfile_dir() -> std::path::PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "sqlite-rs-grep-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `--schema` also greps `sqlite_master.sql` (DDL).
#[test]
fn grep_schema_flag_searches_ddl() {
    let out = Command::new(CLI)
        .args([
            "grep",
            "--schema",
            "CREATE TABLE items",
            &fixture("grep_fixture.db"),
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.starts_with("sqlite_master::0::sql::CREATE TABLE items"));
}
