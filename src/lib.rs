// Copyright 2026 Schuberg Philis
// SPDX-License-Identifier: Apache-2.0
//! A binary-compatible Rust replication of SQLite: read (and, for WAL,
//! write) the same on-disk file format and SQL dialect as the C library,
//! targeting a memory-safe, extensible SQLite rather than a new engine.
//! See the [repository README](https://github.com/iheitlager/sqlite-rs)
//! for the full design rationale.
//!
//! ## Performance
//!
//! 5 of 7 benchmark queries beat or match the sqlite3 oracle (v3.53.4).
//! See [`docs/performance.md`](https://github.com/iheitlager/sqlite-rs/blob/main/docs/performance.md)
//! for the full V4→V7.3 progression.
//!
//! `include_str!` can't pull the README in directly here: `src/` is a
//! qualified Rust subset checked by `make check-mvl-limit` (mvl-rust rust-limit),
//! which doesn't allowlist that macro.
// The VFS used to need a scoped `#![allow(unsafe_code)]` for raw
// `fcntl`/`mmap`/`fork` calls (#50), then went unsafe-free entirely under
// `nix`/`std` (#66). Vendoring `nix`'s `fcntl`/`termios` FFI (#563)
// reintroduced one, deliberately narrow, carve-out (`src/sys/`, see
// `.openspec/adr/0031-vendor-nix-subset.md`) — which has since left this
// crate entirely: `fcntl` with the VFS to db-storage (t-rust-db/sqlite-rs#2),
// `termios` with the line editor to db-cli (#14). No module here allows
// `unsafe` any more; `deny` is kept (rather than `forbid`) only so a future
// carve-out has to be as explicit as those two were.
#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod codegen;
pub mod dump;
pub mod planner;
pub mod vdbe;

/// SQL tokenizer, grammar, AST and printer — re-exported from
/// `db_core::parser::row` (t-rust-db/sqlite-rs#17), the port of this
/// crate's own parser (db-core ADR 0005/0009). Every
/// `crate::parser::*` path keeps resolving; `tokenizer::Span` is
/// re-homed from `db_core::parser::Span` so that path holds too.
pub mod parser {
    pub use db_core::parser::row::*;

    /// The AST, which db-core moved from `parser::row::ast` to the shared
    /// `parser::ast` (db-core#147, ADR 0002 there); `crate::parser::ast::*`
    /// keeps resolving.
    pub mod ast {
        pub use db_core::parser::ast::*;
    }

    /// Tokenizer plus `Span`, which db-core keeps one level up.
    pub mod tokenizer {
        pub use db_core::parser::row::tokenizer::*;
        pub use db_core::parser::Span;
    }
}

/// Virtual filesystem layer — re-exported from `db_storage::row::vfs`
/// (t-rust-db/sqlite-rs#2). The private copy this crate used to carry was
/// the source that module was extracted from (db-core ADR 0006); every
/// `crate::vfs::*` / `sqlite_rs::vfs::*` path keeps resolving unchanged.
pub mod vfs {
    pub use db_storage::row::vfs::*;
}

/// Pager (page cache, rollback journal, WAL, hot-journal recovery) —
/// re-exported from `db_storage::row::pager` (t-rust-db/sqlite-rs#3), same
/// arrangement as [`vfs`].
pub mod pager {
    pub use db_storage::row::pager::*;
}

/// Database header parsing — re-exported from `db_storage::row::header`
/// (t-rust-db/sqlite-rs#5).
pub mod header {
    pub use db_storage::row::header::*;
}

/// Record (varint / serial-type / row) encoding and decoding, `Value` and
/// `Collation` — re-exported from `db_storage::row::record`
/// (t-rust-db/sqlite-rs#4).
pub mod record {
    pub use db_storage::row::record::*;
}

/// Table and index b-tree read/write paths and `sqlite_master` helpers —
/// re-exported from `db_storage::row::btree` (t-rust-db/sqlite-rs#6).
pub mod btree {
    pub use db_storage::row::btree::*;
}

/// DDL reader (`sqlite_master` → `TableSchema`/`IndexSchema`) — re-exported
/// from `db_storage::row::schema` (t-rust-db/sqlite-rs#7).
pub mod schema {
    pub use db_storage::row::schema::*;
}

/// Shell-parity value rendering — re-exported from
/// `db_storage::row::format` (t-rust-db/sqlite-rs#5).
pub mod format {
    pub use db_storage::row::format::*;
}

/// `PRAGMA integrity_check` / `quick_check` — re-exported from
/// `db_storage::row::integrity` (t-rust-db/sqlite-rs#5).
pub mod integrity {
    pub use db_storage::row::integrity::*;
}
