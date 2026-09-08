// Copyright 2026 Schuberg Philis
// SPDX-License-Identifier: Apache-2.0
//! SQL codegen — re-exported from `db_core::codegen::row`
//! (t-rust-db/sqlite-rs#19, db-core#219 / ADR 0013 there: this crate's
//! `src/codegen/**` moved into db-core verbatim, Lab271 leading per
//! ADR-0039). Every `crate::codegen::*` / `sqlite_rs::codegen::*` path keeps
//! resolving — `compile_statement`, `compile_select*`, `explain_query_plan`,
//! `output_column_names`, `leading_keywords`, the `stmt::{insert,update,
//! delete}` modules, `CodegenError`, `DispatchError` — and the planner
//! consumes the same `TableSchema` this crate's `schema` facade yields
//! (db-core ADR 0014), so nothing is converted at the boundary.
//!
//! The `SQLITE_RS_CODEGEN=db-core` shadow switch that measured the gap
//! before the move is retired with it: there is one codegen now.
pub use db_core::codegen::row::*;
