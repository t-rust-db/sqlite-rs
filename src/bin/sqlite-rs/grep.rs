// Copyright 2026 Schuberg Philis
// SPDX-License-Identifier: Apache-2.0
//! `grep [-F] [-i] [--blob=text|hex|skip] [--schema] [-H|-h] <pattern>
//! <file>...`: searches every row/column of every table in one or more
//! SQLite database files for a pattern, streaming rows through
//! [`sqlite_rs::btree::TableCursor`] table-by-table rather than
//! materializing a whole table (spec 014-grep Requirement 5). This is
//! the job `eirtools/sqlgrep` did before the trigram source-tree grep
//! moved out to `t-rust-db/trigrep` (ADR-0043) — `grep` here is the
//! *in-database-file* cell search, not source-tree trigram search.
//!
//! Output format (one line per matching cell), byte-for-byte parity
//! target with `eirtools/sqlgrep` (Apache-2.0; ported behavior, not
//! verbatim code):
//! ```text
//! [<file>::]<table>::<row index>::<column>::<value>
//! ```
//! Row index is zero-based. Exit codes follow `grep(1)`: 0 (matched),
//! 1 (no match), 2 (usage/open error).
//!
//! Descoped from the `eirtools/sqlgrep` parity target (documented in
//! `.openspec/specs/014-grep/spec.md`): `sqlite://` URL inputs, and
//! explicit query sources (positional `SELECT`, `-` stdin, `@file`) —
//! only the default "every table in `sqlite_master`" scope is
//! implemented in this first cut.

use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::process::ExitCode;
use std::rc::Rc;

use regex::RegexBuilder;
use sqlite_rs::btree::TableCursor;
use sqlite_rs::dump;
use sqlite_rs::record::{decode_record, Value};
use sqlite_rs::schema::{column_defs, column_type, read_schema, TableSchema};
use sqlite_rs::vfs::{PageSource, UnixVfs};

use crate::common::usage_error;

/// `--blob=` handling for BLOB cells (undocumented-in-`eirtools`
/// decision, spec 014-grep Requirement 3): default `Skip` (BLOBs never
/// participate in text matching), `Text` (lossy UTF-8 view), `Hex`
/// (`format_blob`-style hex rendering, reusing the existing
/// `.dump`-quote() renderer already ported for issue #37).
#[derive(Clone, Copy, PartialEq, Eq)]
enum BlobMode {
    Skip,
    Text,
    Hex,
}

struct Options {
    blob_mode: BlobMode,
    schema_too: bool,
    show_filename: Option<bool>,
}

pub fn run_grep(raw_args: Vec<String>) -> ExitCode {
    let mut fixed_strings = false;
    let mut ignore_case = false;
    let mut blob_mode = BlobMode::Skip;
    let mut schema_too = false;
    let mut show_filename: Option<bool> = None;
    let mut positional = Vec::new();

    for arg in raw_args {
        match arg.as_str() {
            "-F" | "--fixed-strings" => fixed_strings = true,
            "-i" | "--ignore-case" => ignore_case = true,
            "--schema" => schema_too = true,
            "-H" => show_filename = Some(true),
            "-h" => show_filename = Some(false),
            "--blob=text" => blob_mode = BlobMode::Text,
            "--blob=hex" => blob_mode = BlobMode::Hex,
            "--blob=skip" => blob_mode = BlobMode::Skip,
            _ => positional.push(arg),
        }
    }

    let mut positional = positional.into_iter();
    let Some(pattern) = positional.next() else {
        return usage_error(
            "grep [-F] [-i] [--blob=text|hex|skip] [--schema] [-H|-h] <pattern> <file>...",
        );
    };
    let files: Vec<String> = positional.collect();
    if files.is_empty() {
        return usage_error(
            "grep [-F] [-i] [--blob=text|hex|skip] [--schema] [-H|-h] <pattern> <file>...",
        );
    }

    let matcher = match Matcher::new(&pattern, fixed_strings, ignore_case) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error: bad pattern: {e}");
            return ExitCode::from(2);
        }
    };

    let opts = Options {
        blob_mode,
        schema_too,
        show_filename,
    };

    let multi = files.len() > 1;
    let mut out = BufWriter::new(io::stdout().lock());
    let mut any_match = false;
    let mut any_error = false;

    for file in &files {
        let path = Path::new(file);
        let prefix = match opts.show_filename {
            Some(true) => Some(file.as_str()),
            Some(false) => None,
            None => multi.then_some(file.as_str()),
        };
        match grep_one_file(
            path,
            prefix,
            &matcher,
            opts.blob_mode,
            opts.schema_too,
            &mut out,
        ) {
            Ok(matched) => any_match |= matched,
            Err(e) => {
                eprintln!("error: {}: {e}", path.display());
                any_error = true;
            }
        }
    }

    if let Err(e) = out.flush() {
        eprintln!("error: {e}");
        any_error = true;
    }

    if any_error {
        ExitCode::from(2)
    } else if any_match {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

enum Matcher {
    Regex(regex::Regex),
    Fixed { needle: String, ignore_case: bool },
}

impl Matcher {
    fn new(pattern: &str, fixed: bool, ignore_case: bool) -> Result<Self, String> {
        if fixed {
            Ok(Matcher::Fixed {
                needle: if ignore_case {
                    pattern.to_ascii_lowercase()
                } else {
                    pattern.to_string()
                },
                ignore_case,
            })
        } else {
            let re = RegexBuilder::new(pattern)
                .case_insensitive(ignore_case)
                .build()
                .map_err(|e| e.to_string())?;
            Ok(Matcher::Regex(re))
        }
    }

    fn is_match(&self, text: &str) -> bool {
        match self {
            Matcher::Regex(re) => re.is_match(text),
            Matcher::Fixed {
                needle,
                ignore_case,
            } => {
                if *ignore_case {
                    text.to_ascii_lowercase().contains(needle.as_str())
                } else {
                    text.contains(needle.as_str())
                }
            }
        }
    }
}

/// Renders a cell to the text form matching is performed against
/// (undocumented-in-`eirtools` decision, spec 014-grep Requirement 2):
/// SQLite's `CAST(x AS TEXT)` rendering for NULL/numeric/text, and
/// `blob_mode`-controlled handling for BLOB. Returns `None` when the
/// cell has no text form to match against (NULL, or a skipped BLOB).
fn cell_text(v: &Value, blob_mode: BlobMode) -> Option<String> {
    match v {
        Value::Null => None,
        Value::Integer(i) => Some(i.to_string()),
        Value::Real(r) => Some(sqlite_rs::format::format_real(*r)),
        Value::Text(s) => Some(s.to_string()),
        Value::Blob(b) => match blob_mode {
            BlobMode::Skip => None,
            BlobMode::Text => Some(String::from_utf8_lossy(b).into_owned()),
            BlobMode::Hex => Some(sqlite_rs::format::format_blob(b)),
        },
    }
}

/// Column indices with REAL type affinity — same rule `dump.rs`'s
/// `real_affinity_columns` uses (issue #37); duplicated here in the
/// small, public-API-only form since that helper is private to
/// `sqlite_rs::dump`.
fn real_affinity_columns(schema: &TableSchema) -> Vec<bool> {
    column_defs(schema)
        .iter()
        .map(|def| {
            let declared_type = column_type(def).to_ascii_uppercase();
            declared_type.contains("REAL")
                || declared_type.contains("FLOA")
                || declared_type.contains("DOUB")
        })
        .collect()
}

fn apply_real_affinity(values: &mut [Value], real_affinity: &[bool]) {
    for (i, is_real) in real_affinity.iter().enumerate() {
        if !is_real {
            continue;
        }
        if let Some(v @ Value::Integer(_)) = values.get_mut(i) {
            if let Value::Integer(n) = *v {
                *v = Value::Real(n as f64);
            }
        }
    }
}

fn grep_one_file(
    path: &Path,
    prefix: Option<&str>,
    matcher: &Matcher,
    blob_mode: BlobMode,
    schema_too: bool,
    out: &mut impl Write,
) -> Result<bool, String> {
    let (header, pager) = dump::open(&UnixVfs, path).map_err(|e| e.to_string())?;
    let source: Rc<dyn PageSource> = Rc::new(pager);
    let mut schema_cursor = TableCursor::new(source, &header, 1);
    let schemas =
        read_schema(&mut schema_cursor, header.text_encoding).map_err(|e| e.to_string())?;

    let mut any_match = false;

    if schema_too {
        for (idx, schema) in schemas.iter().enumerate() {
            if matcher.is_match(&schema.sql) {
                any_match = true;
                print_line(out, prefix, "sqlite_master", idx, "sql", &schema.sql)
                    .map_err(|e| e.to_string())?;
            }
        }
    }

    for schema in &schemas {
        if schema.is_virtual || schema.without_rowid {
            // Virtual tables have no b-tree storage of their own; WITHOUT
            // ROWID tables are out of scope beyond what the VDBE already
            // supports (issue #45 "Not in scope").
            continue;
        }
        let (_, pager) = dump::open(&UnixVfs, path).map_err(|e| e.to_string())?;
        let source: Rc<dyn PageSource> = Rc::new(pager);
        let real_affinity = real_affinity_columns(schema);
        let mut cursor = TableCursor::new(source, &header, schema.root_page);
        let mut row_idx: usize = 0;
        let mut row = cursor.first_row().map_err(|e| format!("{e:?}"))?;
        while let Some(r) = row {
            let mut values =
                decode_record(&r.payload, header.text_encoding).map_err(|e| format!("{e:?}"))?;
            if let Some(alias_idx) = schema.rowid_alias {
                if let Some(v) = values.get_mut(alias_idx) {
                    *v = Value::Integer(r.rowid);
                }
            }
            apply_real_affinity(&mut values, &real_affinity);

            for (col_idx, value) in values.iter().enumerate() {
                let Some(text) = cell_text(value, blob_mode) else {
                    continue;
                };
                if matcher.is_match(&text) {
                    any_match = true;
                    let col_name = schema
                        .columns
                        .get(col_idx)
                        .map(String::as_str)
                        .unwrap_or("?");
                    print_line(out, prefix, &schema.name, row_idx, col_name, &text)
                        .map_err(|e| e.to_string())?;
                }
            }

            row_idx = row_idx.wrapping_add(1);
            row = cursor.next_row().map_err(|e| format!("{e:?}"))?;
        }
    }

    Ok(any_match)
}

fn print_line(
    out: &mut impl Write,
    prefix: Option<&str>,
    table: &str,
    row: usize,
    col: &str,
    value: &str,
) -> io::Result<()> {
    if let Some(p) = prefix {
        writeln!(out, "{p}::{table}::{row}::{col}::{value}")
    } else {
        writeln!(out, "{table}::{row}::{col}::{value}")
    }
}
