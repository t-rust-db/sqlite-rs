// Copyright 2026 Schuberg Philis
// SPDX-License-Identifier: Apache-2.0
//! Opt-in shadow compilation through db-core's `codegen::row` (#19,
//! db-core#175). With `SQLITE_RS_CODEGEN=db-core` set, every statement the
//! CLI/REPL/sqllogictest paths compile is first handed to
//! `db_core::codegen::row::dispatch::compile_statement_with_views`; a program it
//! produces runs on the shared VM, and a rejection falls back to this
//! crate's own codegen. Each attempt is appended as a tab-separated line
//! (`OK`/`FALLBACK`, reason, SQL) to `SQLITE_RS_CODEGEN_LOG` (default
//! `target/codegen-shadow.log`), so a full oracle run yields the exact gap
//! map the repoint needs instead of a grep estimate. Off by default: with
//! the variable unset this module costs one env lookup per statement.

use std::io::Write;

use crate::schema::{TableSchema, ViewSchema};
use crate::vdbe::Program;

/// `true` when `SQLITE_RS_CODEGEN=db-core` is set in the environment.
pub fn enabled() -> bool {
    std::env::var("SQLITE_RS_CODEGEN").is_ok_and(|v| v == "db-core")
}

/// Projection of this crate's `TableSchema` onto db-core's. Lossless since
/// db-core v0.68.1 (db-core#205, ADR 0012 there): collations, WITHOUT
/// ROWID/STRICT/virtual, the `CREATE` text and per-index UNIQUE/DESC/COLLATE
/// all have a home. `Collation` is the same type on both sides (db-storage
/// re-exports `db_core::value::Collation`, db-core ADR 0010).
pub fn to_core_schema(schema: &TableSchema) -> db_core::codegen::row::TableSchema {
    db_core::codegen::row::TableSchema {
        name: schema.name.clone(),
        columns: schema.columns.clone(),
        column_types: schema.column_types.clone(),
        column_collations: schema.column_collations.clone(),
        rowid_alias: schema.rowid_alias,
        root_page: schema.root_page,
        without_rowid: schema.without_rowid,
        strict: schema.strict,
        is_virtual: schema.is_virtual,
        sql: schema.sql.clone(),
        indexes: schema
            .indexes
            .iter()
            .map(|idx| db_core::codegen::row::IndexSchema {
                name: idx.name.clone(),
                root_page: idx.root_page,
                unique: idx.unique,
                columns: idx
                    .columns
                    .iter()
                    .map(|c| db_core::codegen::row::IndexedColumn {
                        name: c.name.clone(),
                        desc: c.desc,
                        collation: c.collation,
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn to_core_view(view: &ViewSchema) -> db_core::codegen::row::ViewSchema {
    db_core::codegen::row::ViewSchema {
        name: view.name.clone(),
        sql: view.sql.clone(),
    }
}

/// Compiles `sql` through db-core when [`enabled`], logging the outcome.
/// `None` means "not enabled" or "db-core rejected it — use the local
/// codegen".
pub fn try_compile(sql: &str, schemas: &[TableSchema], views: &[ViewSchema]) -> Option<Program> {
    if !enabled() {
        return None;
    }
    let core: Vec<db_core::codegen::row::TableSchema> =
        schemas.iter().map(to_core_schema).collect();
    let core_views: Vec<db_core::codegen::row::ViewSchema> =
        views.iter().map(to_core_view).collect();
    match db_core::codegen::row::dispatch::compile_statement_with_views(sql, &core, &core_views) {
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
