// Copyright 2026 Schuberg Philis
// SPDX-License-Identifier: Apache-2.0
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

//! Coverage follow-up for #351 (`src/vdbe/cursor.rs` was the largest
//! coverage gap in the repo, 82.37% lines / 68.39% functions): hand-
//! assembled `Program`s covering two kinds of gap the existing
//! oracle-diff SQL tests (e.g. `tests/corpus/index_ordered_scan_test.rs`)
//! never reach:
//!
//! - Opcodes no current codegen path (#91) ever emits — `Last` (table
//!   cursor; reserved for a future `ORDER BY rowid DESC` scan fast
//!   path), `NullRow` (reserved for a future outer-join unmatched-row
//!   path), and `IdxLE` (reserved for a future index range-scan upper
//!   bound).
//! - `CursorTypeMismatch`/`MalformedInstruction` error arms a
//!   well-formed, codegen-emitted program never triggers (the planner
//!   only ever pairs an opcode with the cursor kind that opened it) —
//!   these accounted for most of `cursor.rs`'s missed lines, so are
//!   worth locking down even though the "happy path" is unreachable in
//!   practice.
//!
//! Same acceptance shape as `cursor_sorter_test.rs`'s #90 programs.

use std::path::Path;
use std::rc::Rc;

use sqlite_rs::header::DatabaseHeader;
use sqlite_rs::record::Value;
use sqlite_rs::vdbe::{execute_with_db, Instruction, Opcode, Program};
use sqlite_rs::vfs::{PageSource, UnixVfs, Vfs, VfsPageSource};

fn open_db(fixture: &str) -> (Rc<dyn PageSource>, DatabaseHeader) {
    let path = Path::new("tests/corpus/fixtures/btrees").join(fixture);
    let vfs = UnixVfs;
    let file = vfs.open_read(&path).unwrap();
    let mut header_buf = [0u8; 100];
    file.read_at(&mut header_buf, 0).unwrap();
    let header = DatabaseHeader::parse(&header_buf).unwrap();
    let source = VfsPageSource::open(&vfs, &path, header.page_size).unwrap();
    (Rc::new(source), header)
}

/// `Last` positions a table cursor at its highest-rowid row; pairs with
/// `Prev` (unused today, same as `Last`) the way `Rewind` pairs with
/// `Next`. Here it's used standalone to read just the final row.
#[test]
fn last_positions_cursor_at_highest_rowid_row() {
    let (source, header) = open_db("table_multipage.db");
    let program = Program::new(vec![
        /* 0 */ Instruction::new(Opcode::Init, 0, 1, 0),
        /* 1 */ Instruction::new(Opcode::OpenRead, 0, 2, 0),
        /* 2 */ Instruction::new(Opcode::Last, 0, 5, 0), // jump to Halt(5) if empty
        /* 3 */ Instruction::new(Opcode::Rowid, 0, 0, 0),
        /* 4 */ Instruction::new(Opcode::ResultRow, 0, 1, 0),
        /* 5 */ Instruction::new(Opcode::Halt, 0, 0, 0),
    ]);
    let rows = execute_with_db(&program, source, header).unwrap();

    assert_eq!(rows, vec![vec![Value::Integer(3000)]]);
}

/// `Last` on an empty (ephemeral-table) cursor jumps straight to `P2`,
/// never reaching the row-read instructions — the `EphemeralTable`
/// branch of the same opcode the first test exercised on a real table.
#[test]
fn last_jumps_past_body_when_table_empty() {
    let mut open_instr = Instruction::new(Opcode::OpenEphemeral, 0, 0, 0);
    open_instr.p5 = 1; // ephemeral table, not ephemeral index
    let program = Program::new(vec![
        /* 0 */ Instruction::new(Opcode::Init, 0, 1, 0),
        /* 1 */ open_instr,
        /* 2 */ Instruction::new(Opcode::Last, 0, 5, 0),
        /* 3 */ Instruction::new(Opcode::Rowid, 0, 0, 0),
        /* 4 */ Instruction::new(Opcode::ResultRow, 0, 1, 0),
        /* 5 */ Instruction::new(Opcode::Halt, 0, 0, 0),
    ]);
    let rows = sqlite_rs::vdbe::execute(&program).unwrap();

    assert!(rows.is_empty());
}

/// `NullRow` forces a table cursor to read as an all-NULL row (no
/// `Column`/`Rowid` lookup on the underlying b-tree) until the next real
/// positioning opcode re-establishes a current row.
#[test]
fn null_row_forces_null_reads_until_repositioned() {
    let (source, header) = open_db("table_multipage.db");
    let program = Program::new(vec![
        /* 0 */ Instruction::new(Opcode::Init, 0, 1, 0),
        /* 1 */ Instruction::new(Opcode::OpenRead, 0, 2, 0),
        /* 2 */ Instruction::new(Opcode::Rewind, 0, 8, 0),
        /* 3 */ Instruction::new(Opcode::NullRow, 0, 0, 0),
        /* 4 */ Instruction::new(Opcode::Rowid, 0, 0, 0), // r0 = NULL (forced)
        /* 5 */ Instruction::new(Opcode::Column, 0, 1, 1), // r1 = NULL (forced)
        /* 6 */ Instruction::new(Opcode::ResultRow, 0, 2, 0),
        /* 7 */ Instruction::new(Opcode::Rewind, 0, 8, 0), // repositions past NullRow
        /* 8 */ Instruction::new(Opcode::Halt, 0, 0, 0),
    ]);
    let rows = execute_with_db(&program, source, header).unwrap();

    assert_eq!(rows, vec![vec![Value::Null, Value::Null]]);
}

/// `IdxLE` against an `OpenEphemeral` (DISTINCT-style in-memory index)
/// cursor: db-core#527 (`perf/527-ephemeral-index-hash`) moved this
/// cursor kind from a `BTreeMap` to a `HashMap`, and its doc comment on
/// `EphemeralIndexCursor` explains why `Cursor::idx_compare` is now left
/// un-overridden for it — no `IN (SELECT ...)`/`DISTINCT` shape ever
/// emits `IdxLE` against this cursor kind (it always targets a real
/// b-tree/in-memory index cursor instead), so trading the ordered-bound
/// seek `IdxLE` would need for O(1) membership costs nothing real
/// codegen reaches. `IdxLE` here now surfaces the same
/// `MalformedInstruction` "not an index cursor" error as a genuinely
/// malformed program, rather than comparing in the `HashMap`'s
/// arbitrary iteration order.
#[test]
fn idx_le_reports_malformed_instruction_against_ephemeral_cursor() {
    use sqlite_rs::vdbe::P4;

    let program = Program::new(vec![
        /* 0 */ Instruction::new(Opcode::Init, 0, 1, 0),
        /* 1 */ Instruction::new(Opcode::OpenEphemeral, 0, 0, 0),
        /* 2 */ Instruction::with_p4(Opcode::IdxLE, 0, 4, 0, P4::Int(0)),
        /* 3 */ Instruction::new(Opcode::Halt, 0, 0, 0),
    ]);
    let err = sqlite_rs::vdbe::execute(&program).unwrap_err();
    assert!(
        err.to_string().contains("IdxLE"),
        "expected an IdxLE malformed-instruction error, got: {err}"
    );
}

/// A `CursorSlot::Ephemeral` (the DISTINCT-style in-memory index opened
/// by `OpenEphemeral` with `P5` zero) is the wrong cursor kind for every
/// opcode below — each expects a table, ephemeral-table, or index-read
/// cursor instead. These exercise the `CursorTypeMismatch` arms that a
/// well-formed, codegen-emitted program never reaches (the planner only
/// ever pairs an opcode with the cursor kind that opened it), but that a
/// malformed program — or a future codegen bug pairing the wrong opener
/// with an opcode — must still fail on with a clear error rather than
/// panicking or silently misreading memory.
fn program_targeting_ephemeral_index(opcode: Opcode) -> Program {
    Program::new(vec![
        /* 0 */ Instruction::new(Opcode::Init, 0, 1, 0),
        /* 1 */
        Instruction::new(Opcode::OpenEphemeral, 0, 0, 0), // Ephemeral (index-mode)
        /* 2 */ Instruction::new(opcode, 0, 3, 0),
        /* 3 */ Instruction::new(Opcode::Halt, 0, 0, 0),
    ])
}

fn assert_cursor_type_mismatch(opcode: Opcode, opcode_name: &str) {
    let program = program_targeting_ephemeral_index(opcode);
    let err = sqlite_rs::vdbe::execute(&program).unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains(opcode_name),
        "expected a {opcode_name} cursor-type-mismatch error, got: {message}"
    );
}

#[test]
fn seek_index_eq_reports_mismatch_on_non_index_read_cursor() {
    assert_cursor_type_mismatch(Opcode::SeekIndexEq, "SeekIndexEq");
}

#[test]
fn idx_rowid_reports_mismatch_on_non_index_read_cursor() {
    assert_cursor_type_mismatch(Opcode::IdxRowid, "IdxRowid");
}

/// `IdxRowid` on a genuine index-read cursor that was never positioned
/// (no `SeekIndexEq`/`IdxRewind`/etc. ran first) hits its own
/// `MalformedInstruction` arm — distinct from the cursor-type-mismatch
/// path above.
#[test]
fn idx_rowid_reports_malformed_instruction_when_unpositioned() {
    let (source, header) = open_db("index.db");
    let mut open_instr = Instruction::new(Opcode::OpenRead, 0, 2, 0);
    open_instr.p5 = 1; // index-read cursor
    let program = Program::new(vec![
        /* 0 */ Instruction::new(Opcode::Init, 0, 1, 0),
        /* 1 */ open_instr,
        /* 2 */
        Instruction::new(Opcode::IdxRowid, 0, 0, 0), // no positioning opcode ran first
        /* 3 */ Instruction::new(Opcode::Halt, 0, 0, 0),
    ]);
    let err = execute_with_db(&program, source, header).unwrap_err();
    assert!(
        err.to_string().contains("IdxRowid"),
        "expected an IdxRowid malformed-instruction error, got: {err}"
    );
}
