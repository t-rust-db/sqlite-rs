// Copyright 2026 Schuberg Philis
// SPDX-License-Identifier: Apache-2.0
//! The VDBE — `db_core::vm::row` (t-rust-db/sqlite-rs#18), the port of
//! this crate's own `src/vdbe` (db-core ADR 0007/0008), re-exported under
//! the paths codegen and the tests already use, plus the two things that
//! stay here because db-core must not depend on db-storage (ADR 0008):
//!
//! - [`adapter`]: the storage side — db-storage's `TableCursor`/
//!   `IndexCursor` (reached as `crate::btree`) behind db-core's `Cursor` trait, the cursor factory
//!   `OpenRead`/`OpenWrite` resolve root pages through, the pager-backed
//!   `Transaction` hook and the `sqlite_master`/`sqlite_stat1`/
//!   `sqlite_sequence` schema-write hook;
//! - the `execute_*` entry points below, which build a `Vm` with those
//!   hooks installed — the same signatures `src/vdbe/exec.rs` had, so
//!   codegen, the CLI and the test suite are unchanged.
//!
//! `Rc<dyn PageSource>` is still the one `dyn` boundary (ADR-0013), now
//! inside `adapter.rs`; a writable `Vm` still shares its pager through
//! `Rc<RefCell<Pager>>` (ADR-0017).

pub mod adapter;

pub use db_core::vm::row::coerce::{
    cast_to_integer, checked_add, checked_div, checked_mul, checked_rem, checked_sub,
    coerce_text_to_numeric,
};
pub use db_core::vm::row::functions::{call as call_function, like_match};
pub use db_core::vm::row::logic::{and, is, is_not, not, or, sql_eq, sql_lt};
pub use db_core::vm::row::{
    affinity_of, apply_affinity, cast_to, compare, compare_text, comparison_affinity, explain,
    Affinity, AnalyzeIndexTarget, AnalyzeTarget, Collation, ExecError, ExplainRow, FunctionError,
    GroupKeyColumn, Instruction, Opcode, Program, SortKeyColumn, Step, Vm, JOURNAL_MODE_DELETE,
    JOURNAL_MODE_WAL, P4, SYNCHRONOUS_FULL, SYNCHRONOUS_NORMAL, SYNCHRONOUS_OFF, SYNCHRONOUS_QUERY,
    TRANSACTION_MODE_DEFERRED, TRANSACTION_MODE_EXCLUSIVE, TRANSACTION_MODE_IMMEDIATE,
};

/// `crate::vdbe::program::*` paths keep resolving.
pub mod program {
    pub use db_core::vm::row::program::*;
}
/// Aggregate accumulators, for callers that reach for them by module.
pub mod aggregate {
    pub use db_core::vm::row::aggregate::*;
}

use std::cell::RefCell;
use std::rc::Rc;

use crate::header::DatabaseHeader;
use crate::record::Value;
use crate::vfs::PageSource;

use adapter::{BtreeSchemaStorage, PagerTransaction, StorageFactory};

/// Runs `program` on a fresh `Vm` with no database attached (arithmetic,
/// control-flow and sorter programs; `OpenRead` fails with `CursorNotOpen`).
pub fn execute(program: &Program) -> Result<Vec<Vec<Value>>, ExecError> {
    let mut vm = Vm::new();
    db_core::vm::row::execute(&mut vm, program)
}

/// [`execute`] with `?NNN` parameters bound first.
pub fn execute_with_params(
    program: &Program,
    params: Vec<Value>,
) -> Result<Vec<Vec<Value>>, ExecError> {
    let mut vm = Vm::new();
    vm.bind_params(params);
    db_core::vm::row::execute(&mut vm, program)
}

/// Runs `program` read-only over `source` (any `PageSource`: a
/// `VfsPageSource`, a `Pager`, or a shared `Rc<RefCell<Pager>>`).
pub fn execute_with_db(
    program: &Program,
    source: Rc<dyn PageSource>,
    header: DatabaseHeader,
) -> Result<Vec<Vec<Value>>, ExecError> {
    let mut vm = read_only_vm(source, header);
    db_core::vm::row::execute(&mut vm, program)
}

/// [`execute_with_db`] with parameters bound.
pub fn execute_with_db_and_params(
    program: &Program,
    source: Rc<dyn PageSource>,
    header: DatabaseHeader,
    params: Vec<Value>,
) -> Result<Vec<Vec<Value>>, ExecError> {
    let mut vm = read_only_vm(source, header);
    vm.bind_params(params);
    db_core::vm::row::execute(&mut vm, program)
}

/// Runs a write program over `pager`, autocommitting at the end (the
/// `exec`/`query` one-shot paths).
pub fn execute_with_writable_db(
    program: &Program,
    pager: crate::pager::Pager,
    header: DatabaseHeader,
) -> Result<Vec<Vec<Value>>, ExecError> {
    let pager = Rc::new(RefCell::new(pager));
    let mut vm = writable_vm(Rc::clone(&pager), header, true);
    let rows = db_core::vm::row::execute(&mut vm, program)?;
    implicit_commit(&vm, &pager)?;
    Ok(rows)
}

/// One statement of a multi-statement session (the REPL, `exec` scripts):
/// the pager is shared across statements and the autocommit flag is
/// carried in and out, so `BEGIN` in one program and `COMMIT` in a later
/// one see each other (spec 010).
pub fn execute_transaction_step(
    program: &Program,
    pager: Rc<RefCell<crate::pager::Pager>>,
    header: DatabaseHeader,
    autocommit_in: bool,
) -> Result<(Vec<Vec<Value>>, bool), ExecError> {
    let mut vm = writable_vm(Rc::clone(&pager), header, autocommit_in);
    let rows = db_core::vm::row::execute(&mut vm, program)?;
    implicit_commit(&vm, &pager)?;
    Ok((rows, vm.autocommit()))
}

/// The former `exec::run`'s clean-`Halt` rule: outside an explicit
/// transaction every statement is its own transaction, so a program that
/// halted with code 0 while `autocommit` is set flushes the pager's
/// pending pages (spec 010). Inside `BEGIN … COMMIT` the `AutoCommit`
/// opcode flushes instead.
fn implicit_commit(vm: &Vm, pager: &Rc<RefCell<crate::pager::Pager>>) -> Result<(), ExecError> {
    if vm.autocommit() {
        pager.borrow_mut().flush().map_err(|e| {
            ExecError::TransactionFailed(db_core::vm::row::TransactionError(format!(
                "failed to flush pending writes on statement commit: {e}"
            )))
        })?;
    }
    Ok(())
}

fn read_only_vm(source: Rc<dyn PageSource>, header: DatabaseHeader) -> Vm {
    let mut vm = Vm::new();
    vm.set_text_encoding(header.text_encoding);
    vm.set_cursor_factory(Box::new(StorageFactory::read_only(
        Rc::clone(&source),
        header,
    )));
    vm.set_transaction_hook(Box::new(PagerTransaction::read_only(source, header)));
    vm
}

fn writable_vm(
    pager: Rc<RefCell<crate::pager::Pager>>,
    header: DatabaseHeader,
    autocommit: bool,
) -> Vm {
    let source: Rc<dyn PageSource> = Rc::clone(&pager) as Rc<dyn PageSource>;
    let mut vm = Vm::new();
    vm.set_text_encoding(header.text_encoding);
    vm.set_autocommit(autocommit);
    vm.set_cursor_factory(Box::new(StorageFactory::writable(
        Rc::clone(&source),
        Rc::clone(&pager),
        header,
    )));
    vm.set_transaction_hook(Box::new(PagerTransaction::writable(
        source,
        Rc::clone(&pager),
        header,
    )));
    vm.set_schema_storage(Box::new(BtreeSchemaStorage::new(pager, header)));
    vm
}
