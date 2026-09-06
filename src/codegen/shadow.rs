// Copyright 2026 Schuberg Philis
// SPDX-License-Identifier: Apache-2.0
//! Opt-in shadow compilation through db-core's `codegen::row` (#19,
//! db-core#175). With `SQLITE_RS_CODEGEN=db-core` set, every statement the
//! CLI/REPL/sqllogictest paths compile is first handed to
//! `db_core::codegen::row::dispatch::compile_statement`; a program it
//! produces runs on the shared VM, and a rejection falls back to this
//! crate's own codegen. Each attempt is appended as a tab-separated line
//! (`OK`/`FALLBACK`, reason, SQL) to `SQLITE_RS_CODEGEN_LOG` (default
//! `target/codegen-shadow.log`), so a full oracle run yields the exact gap
//! map the repoint needs instead of a grep estimate. Off by default: with
//! the variable unset this module costs one env lookup per statement.

use std::io::Write;

use crate::schema::TableSchema;
use crate::vdbe::Program;

/// `true` when `SQLITE_RS_CODEGEN=db-core` is set in the environment.
pub fn enabled() -> bool {
    std::env::var("SQLITE_RS_CODEGEN").is_ok_and(|v| v == "db-core")
}

/// Lossy projection of this crate's `TableSchema` onto db-core's: column
/// collations, WITHOUT ROWID/STRICT, and per-index UNIQUE/DESC/COLLATE are
/// dropped because db-core's type has no home for them yet (db-core#175
/// gap 2). A statement that depends on them may therefore compile through
/// db-core and then differ from the oracle — the corpus catches that.
pub fn to_core_schema(schema: &TableSchema) -> db_core::codegen::row::TableSchema {
    db_core::codegen::row::TableSchema {
        name: schema.name.clone(),
        columns: schema.columns.clone(),
        column_types: schema.column_types.clone(),
        rowid_alias: schema.rowid_alias,
        root_page: schema.root_page,
        indexes: schema
            .indexes
            .iter()
            .map(|idx| db_core::codegen::row::IndexSchema {
                name: idx.name.clone(),
                root_page: idx.root_page,
                columns: idx.columns.iter().map(|c| c.name.clone()).collect(),
            })
            .collect(),
    }
}

/// Compiles `sql` through db-core when [`enabled`], logging the outcome.
/// `None` means "not enabled" or "db-core rejected it — use the local
/// codegen".
pub fn try_compile(sql: &str, schemas: &[TableSchema]) -> Option<Program> {
    if !enabled() {
        return None;
    }
    let core: Vec<db_core::codegen::row::TableSchema> =
        schemas.iter().map(to_core_schema).collect();
    match db_core::codegen::row::dispatch::compile_statement(sql, &core) {
        Ok(program) => {
            log_outcome("OK", "", sql);
            Some(program)
        }
        Err(e) => {
            log_outcome("FALLBACK", &e.to_string(), sql);
            None
        }
    }
}

fn log_outcome(status: &str, reason: &str, sql: &str) {
    let path = std::env::var("SQLITE_RS_CODEGEN_LOG")
        .unwrap_or_else(|_| "target/codegen-shadow.log".to_string());
    let one_line = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        // One write per line so concurrent test threads (O_APPEND) never
        // interleave; best effort — a failed log write never fails the
        // statement.
        let line = format!("{status}\t{}\t{}\n", one_line(reason), one_line(sql));
        f.write_all(line.as_bytes()).ok();
    }
}
