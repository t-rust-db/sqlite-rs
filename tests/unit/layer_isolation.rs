// Copyright 2026 Schuberg Philis
// SPDX-License-Identifier: Apache-2.0
//! Enforces spec 001-architecture Requirement 1 (Layer Isolation), both
//! scenarios, after the Tier 0 core moved to `db_storage::row`
//! (t-rust-db/sqlite-rs#2–#7):
//! - "B-tree does not know SQL": Tier 0 (vfs, pager, header, record,
//!   btree, schema) now lives in a separate crate that cannot depend on
//!   this one, so the crate graph enforces the original claim. What this
//!   crate still has to guarantee is a single entry point: storage is
//!   reached only through the `src/lib.rs` re-export facade
//!   (`crate::btree`, `crate::pager`, …), never by naming `db_storage::`
//!   directly elsewhere in `src/` — otherwise the facade stops being the
//!   one place a future storage swap has to touch.
//! - "VDBE does not know file format": vdbe/codegen must reach storage
//!   only through the B-tree API (and the `PageSource` boundary of
//!   ADR-0013), never the pager directly — whether spelled `crate::pager`
//!   or `db_storage::row::pager`.
//!
//! Without this, both boundaries hold only by convention — a stray
//! `use db_storage::row::btree` in `vdbe/cursor.rs`, or a stray
//! `use crate::pager` in `vdbe/exec.rs`, would compile silently.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::arithmetic_side_effects
)]

use std::path::{Path, PathBuf};

/// The one file allowed to name `db_storage::` — the re-export facade.
const STORAGE_FACADE: &str = "src/lib.rs";
const FACADE_BYPASS: &[&str] = &["db_storage::"];

const TIER1_2_ROOTS: &[&str] = &["src/vdbe.rs", "src/codegen.rs"];

/// `crate::vfs::PageSource` is the sanctioned exception (ADR-0013's
/// `Rc<dyn PageSource>` boundary), so only the pager is forbidden here.
const STORAGE_BYPASS: &[&str] = &["use crate::pager", "db_storage::row::pager"];

fn collect_rs_files(root: &Path, out: &mut Vec<PathBuf>) {
    if root.is_file() {
        out.push(root.to_path_buf());
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Collects every `.rs` file under each root, plus each root's sibling
/// submodule directory of the same name (e.g. `db-storage/src/row/schema/mod.rs` +
/// `db-storage/src/row/schema/`), and returns the list.
fn collect_module_trees(roots: &[&str]) -> Vec<PathBuf> {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let mut files = Vec::new();
    for root in roots {
        let full_root = Path::new(manifest_dir).join(root);
        collect_rs_files(&full_root, &mut files);
        if let Some(stem) = full_root.file_stem() {
            let sibling_dir = full_root.with_file_name(stem);
            if sibling_dir.is_dir() {
                collect_rs_files(&sibling_dir, &mut files);
            }
        }
    }
    files
}

fn find_violations(files: &[PathBuf], forbidden: &[&str]) -> Vec<String> {
    let mut violations = Vec::new();
    for file in files {
        let src = std::fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("reading {}: {e}", file.display()));
        for pat in forbidden {
            if src.contains(pat) {
                violations.push(format!("{}: contains `{pat}`", file.display()));
            }
        }
    }
    violations
}

#[test]
fn storage_is_reached_only_through_the_lib_rs_facade() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let mut files = Vec::new();
    collect_rs_files(&Path::new(manifest_dir).join("src"), &mut files);
    let facade = Path::new(manifest_dir).join(STORAGE_FACADE);
    files.retain(|f| *f != facade);
    assert!(
        !files.is_empty(),
        "no source files found under src/ besides the facade"
    );

    let violations = find_violations(&files, FACADE_BYPASS);
    assert!(
        violations.is_empty(),
        "Tier 0 layer isolation violated (spec 001-architecture Requirement 1, \
         \"B-tree does not know SQL\") — storage types must come through the \
         `src/lib.rs` re-export facade (`crate::btree`, `crate::pager`, …), \
         never `db_storage::` directly:
{}",
        violations.join(
            "
"
        )
    );
}

#[test]
fn vdbe_and_codegen_do_not_bypass_btree_for_storage_access() {
    let files = collect_module_trees(TIER1_2_ROOTS);
    assert!(
        !files.is_empty(),
        "no vdbe/codegen source files found — check TIER1_2_ROOTS paths"
    );

    let violations = find_violations(&files, STORAGE_BYPASS);
    assert!(
        violations.is_empty(),
        "VDBE/codegen layer isolation violated (spec 001-architecture Requirement 1, \
         \"VDBE does not know file format\") — storage access must go through \
         the B-tree API, not the pager directly:\n{}",
        violations.join("\n")
    );
}
