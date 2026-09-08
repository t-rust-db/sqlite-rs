// Copyright 2026 Schuberg Philis
// SPDX-License-Identifier: Apache-2.0
//! `repl <file>`: a minimal read-eval-print loop (#365) — the CLI
//! surface transaction control (#356/#360) actually needs. `exec`
//! (#358) already runs a `;`-separated multi-statement *script* in one
//! shot; this is the same session machinery (one shared
//! `Rc<RefCell<Pager>>` + autocommit flag, `split_statements`,
//! `execute_transaction_step`) driven interactively from stdin instead,
//! so `BEGIN`/a write/`SELECT`/`COMMIT`/`ROLLBACK` can be typed one at a
//! time and see each other's effects — including an uncommitted write,
//! which `exec`'s one-shot-per-process model has no way to demonstrate
//! (`SELECT` here reads through the *same* shared `Pager`, not a fresh
//! read-only one, precisely so that's true).
//!
//! Deliberately minimal, per the issue's explicit scope-down: no
//! `-csv`/`-explain`/`EXPLAIN QUERY PLAN` (those stay `query`-only). A
//! `;` inside a string/blob literal never ends a statement early —
//! `ends_with_semicolon` goes through the real tokenizer, not a
//! newline-oblivious `str::ends_with(';')`.
//!
//! Line editing and history (#551; hand-rolled per #558, then handed to
//! db-cli in t-rust-db/sqlite-rs#14): input is read through
//! [`db_cli::Readline`] — up/down arrow and Ctrl-P/N history, emacs
//! keys, tab completion via [`crate::completion::SchemaCompleter`] and
//! tokenizer-backed highlighting via [`crate::highlight::SqlHighlighter`]
//! — falling back to plain line reads (no prompt, like `sqlite3`) when
//! stdin isn't a tty (piped scripts, as used by every test in this
//! crate). History persists across sessions at [`history_path`];
//! loading/saving is best-effort — a missing `$HOME`/`$XDG_STATE_HOME`
//! or an unwritable history file never blocks the session.
//!
//! The loop itself is db-cli's (`db_cli::Repl` + `run_repl_with_editor`,
//! t-rust-db/sqlite-rs#15): statement buffering, the built-in
//! dot-commands (`.help`, `.quit`/`.exit`, `.mode`, `.headers`, `.color`)
//! and stdout/stderr routing live there; this file is the
//! [`db_cli::ReplHandler`] that plugs sqlite-rs in — statement
//! completion via the real tokenizer (`ends_with_semicolon` /
//! `split_statements`), execution against the shared `Pager`, rendering
//! via `mode.rs::print_rows` (the byte-oriented `list`/`csv`/`column`/
//! `line` renderers) or db-cli's own `table`/`json`, and the engine
//! dot-commands (#478, #495: `.tables`, `.version`, `.schema`, `.dump`,
//! `.databases`, `.indices`), all `sqlite3`-style prefix-matched.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;

use db_cli::{run_repl_with_editor, OutputMode, Readline, Repl, ReplHandler, ReplOptions};
use sqlite_rs::btree::TableCursor;
use sqlite_rs::codegen::{
    compile_statement, leading_keywords, output_column_names, resolve_from_table_schema,
};
use sqlite_rs::dump;
use sqlite_rs::header::DatabaseHeader;
use sqlite_rs::pager::Pager;
use sqlite_rs::parser::{ends_with_semicolon, parse_select, split_statements, ParseOutcome};
use sqlite_rs::record::Value;
use sqlite_rs::schema::{read_schema, read_views, TableSchema};
use sqlite_rs::vdbe::{execute_transaction_step, execute_with_db};
use sqlite_rs::vfs::{PageSource, UnixVfs};

use crate::completion::SchemaCompleter;
use crate::dot_commands::{
    print_databases, print_dump, print_indices, print_schema, print_version, HELP_ENTRIES,
};
use crate::highlight::SqlHighlighter;
use crate::mode::print_rows;
use crate::pragma_query::{execute_pragma_query, parse_pragma_query};
use crate::query::{compile_select_program, write_list_row, SelectOutcome};
use crate::tables::{list_table_and_view_names, print_table_names};

/// One statement's printable result.
pub enum ReplOutput {
    /// A write or control statement with nothing to show.
    Nothing,
    /// A `SELECT`-shaped result set: column labels (see [`derive_headers`])
    /// and decoded rows, rendered per `.mode`/`.headers`.
    Rows {
        columns: Vec<String>,
        rows: Vec<Vec<Value>>,
    },
    /// CLI-layer pragma rows (ADR-0029), always `list`-rendered as before.
    Text(Vec<Vec<String>>),
}

/// sqlite-rs's [`ReplHandler`]: the session state that used to be
/// `ReplState` (the shared `Pager`, the autocommit flag) plus the schema
/// snapshot the tab completer reads.
struct SqliteHandler {
    pager: Rc<RefCell<Pager>>,
    header: DatabaseHeader,
    db_path: PathBuf,
    autocommit: bool,
    completion_schemas: Rc<RefCell<Vec<TableSchema>>>,
}

impl SqliteHandler {
    /// Best-effort: a schema read error just means "no completion
    /// candidates from the schema", never a fatal error for the REPL.
    fn refresh_completion_schemas(&self) {
        let borrowed = self.pager.borrow();
        let mut cursor = TableCursor::new(&*borrowed, &self.header, 1);
        *self.completion_schemas.borrow_mut() =
            read_schema(&mut cursor, self.header.text_encoding).unwrap_or_default();
    }

    fn schemas_and_views(
        &self,
    ) -> Result<(Vec<TableSchema>, Vec<sqlite_rs::schema::ViewSchema>), String> {
        let borrowed = self.pager.borrow();
        let mut schema_cursor = TableCursor::new(&*borrowed, &self.header, 1);
        let schemas = read_schema(&mut schema_cursor, self.header.text_encoding)
            .map_err(|e| e.to_string())?;
        let mut view_cursor = TableCursor::new(&*borrowed, &self.header, 1);
        let views =
            read_views(&mut view_cursor, self.header.text_encoding).map_err(|e| e.to_string())?;
        Ok((schemas, views))
    }

    /// Runs one already-complete statement against the session's shared
    /// `pager`. Errors come back as the message only — db-cli prefixes
    /// them with `Error: ` (see [`ReplHandler::error_line`]) and sends them
    /// to stderr — and never end the session, matching `sqlite3`.
    fn run_one_statement(&mut self, stmt: &str) -> Result<ReplOutput, String> {
        // #489: checked before anything else, same as `query.rs`'s
        // `run_query` — a `PRAGMA` outside these 9 recognized names (e.g.
        // `journal_mode`) falls through unrecognized and hits the ordinary
        // `compile_statement` write-pragma path below, unchanged.
        if let Some(pragma) = parse_pragma_query(stmt) {
            let (schemas, views) = self.schemas_and_views()?;
            let rows = execute_pragma_query(&pragma, &schemas, &views, &self.header, &self.db_path)
                .map_err(|e| e.to_string())?;
            return Ok(ReplOutput::Text(rows));
        }

        let (schemas, views) = self.schemas_and_views()?;
        let stats_by_table = {
            let borrowed = self.pager.borrow();
            sqlite_rs::planner::load_stats(&*borrowed, &self.header, &schemas)
        };

        let keywords = leading_keywords(stmt);
        let is_select = keywords.first().is_some_and(|kw| kw.as_str() == "SELECT");

        if is_select {
            let select = match parse_select(stmt) {
                ParseOutcome::Accepted(select) => *select,
                ParseOutcome::Unsupported { message, span } => {
                    return Err(format!(
                        "not yet supported (line {}, column {}): {message}",
                        span.line, span.column
                    ));
                }
                ParseOutcome::Invalid { message, span } => {
                    return Err(format!(
                        "syntax error (line {}, column {}): {message}",
                        span.line, span.column
                    ));
                }
            };
            let program =
                match compile_select_program(&select, false, &schemas, &views, &stats_by_table) {
                    Ok(SelectOutcome::Program(p)) => p,
                    // `eqp_mode` is always `false` above, so `Eqp` never comes back.
                    Ok(SelectOutcome::Eqp(_)) => return Err("unexpected EQP output".to_string()),
                    Err(e) => return Err(e.to_string()),
                };
            // Reads through the same shared `Pager` the write path uses
            // (`Rc<RefCell<Pager>>` implements `PageSource`, ADR-0017) —
            // an uncommitted write earlier in this same transaction must
            // be visible here, not just what's on disk.
            let source: Rc<dyn PageSource> = Rc::clone(&self.pager) as Rc<dyn PageSource>;
            let columns = derive_headers(&select, &schemas);
            let rows = execute_with_db(&program, source, self.header).map_err(|e| e.to_string())?;
            return Ok(ReplOutput::Rows { columns, rows });
        }

        let program = compile_statement(stmt, &schemas, &views).map_err(|e| e.to_string())?;
        let (rows, autocommit) = execute_transaction_step(
            &program,
            Rc::clone(&self.pager),
            self.header,
            self.autocommit,
        )
        .map_err(|e| e.to_string())?;
        self.autocommit = autocommit;
        // #645: a non-`SELECT` statement can still emit result rows (e.g.
        // `PRAGMA synchronous`'s bare query form, or `PRAGMA
        // integrity_check`) — render them like a `SELECT`'s. No column
        // names to derive here (no `Select` AST), so `.headers on` renders
        // a blank header line for these.
        if rows.is_empty() {
            Ok(ReplOutput::Nothing)
        } else {
            Ok(ReplOutput::Rows {
                columns: Vec::new(),
                rows,
            })
        }
    }
}

/// Which built-in dot-commands db-cli documents itself, so `help_extra`
/// doesn't list them twice.
const DB_CLI_BUILTINS: &[&str] = &[".help", ".quit", ".exit", ".mode", ".headers", ".color"];

impl ReplHandler for SqliteHandler {
    type Output = ReplOutput;

    fn execute(&mut self, input: &str) -> Result<ReplOutput, String> {
        let result = self.run_one_statement(input);
        // DDL may have changed what tab completion should offer.
        self.refresh_completion_schemas();
        result
    }

    fn format(&self, output: &ReplOutput, mode: OutputMode, headers: bool) -> String {
        let bytes = match output {
            ReplOutput::Nothing => return String::new(),
            ReplOutput::Text(rows) => {
                let mut out = Vec::new();
                for row in rows {
                    let rendered: Vec<Vec<u8>> =
                        row.iter().map(|s| s.clone().into_bytes()).collect();
                    // Writing into a Vec<u8> cannot fail.
                    write_list_row(&mut out, &rendered).ok();
                }
                out
            }
            ReplOutput::Rows { columns, rows } => {
                let local = match mode {
                    OutputMode::List => crate::mode::OutputMode::List,
                    OutputMode::Csv => crate::mode::OutputMode::Csv,
                    OutputMode::Column => crate::mode::OutputMode::Column,
                    OutputMode::Line => crate::mode::OutputMode::Line,
                    // db-cli's own additions have no byte-oriented renderer
                    // here; stringify the cells and let db-cli draw them.
                    OutputMode::Table | OutputMode::Json => {
                        let cells: Vec<Vec<String>> = rows
                            .iter()
                            .map(|r| r.iter().map(cell_string).collect())
                            .collect();
                        return db_cli::render(mode, columns, &cells, headers);
                    }
                };
                let mut out = Vec::new();
                print_rows(&mut out, local, headers, columns, rows).ok();
                out
            }
        };
        // The renderers terminate every line; db-cli's `println!` adds
        // the last newline back. Lossy only for non-UTF-8 blob bytes in
        // `list`/`csv` mode — a db-cli boundary (String, not bytes).
        let text = String::from_utf8_lossy(&bytes);
        text.strip_suffix('\n').unwrap_or(&text).to_string()
    }

    fn command(&mut self, name: &str, arg: &str) -> Option<Vec<String>> {
        if name.is_empty() {
            return None;
        }
        let arg = (!arg.is_empty()).then_some(arg);
        // `sqlite3`-style prefix matching (`.t` .. `.tables`); the engine
        // commands print to stdout themselves, so "handled, nothing more".
        if "tables".starts_with(name) {
            match list_table_and_view_names(
                Rc::clone(&self.pager) as Rc<dyn PageSource>,
                &self.header,
                arg,
            ) {
                Ok(names) => print_table_names(&names),
                Err(e) => eprintln!("Error: {e}"),
            }
        } else if "version".starts_with(name) {
            print_version();
        } else if "schema".starts_with(name) {
            print_schema(&self.pager, &self.header, arg);
        } else if "indices".starts_with(name) {
            print_indices(&self.pager, &self.header, arg);
        } else if "databases".starts_with(name) {
            print_databases(&self.db_path);
        } else if "dump".starts_with(name) {
            print_dump(&self.db_path, arg);
        } else {
            return None;
        }
        Some(Vec::new())
    }

    fn help_extra(&self) -> Vec<String> {
        HELP_ENTRIES
            .iter()
            .filter(|(cmd, _)| !DB_CLI_BUILTINS.iter().any(|b| cmd.starts_with(b)))
            .map(|(cmd, desc)| format!("{cmd:<20}{desc}"))
            .collect()
    }

    /// A `;` inside a string/blob literal never ends a statement early —
    /// this goes through the real tokenizer, not `str::ends_with(';')`.
    fn is_complete(&self, buffer: &str) -> bool {
        ends_with_semicolon(buffer)
    }

    /// `BEGIN; INSERT …;` on one line runs as two statements.
    fn statements(&self, buffer: &str) -> Vec<String> {
        split_statements(buffer)
    }

    fn error_line(&self, message: &str) -> String {
        format!("Error: {message}")
    }
}

/// Renders `v` the way `mode.rs` does for `column`/`line` display.
fn cell_string(v: &Value) -> String {
    let mut scratch = Vec::new();
    sqlite_rs::format::write_query_value(&mut scratch, v);
    String::from_utf8_lossy(&scratch).into_owned()
}

pub fn run_repl(path: &Path) -> ExitCode {
    let (header, pager) = match dump::open(&UnixVfs, path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
    };
    let completion_schemas = Rc::new(RefCell::new(Vec::new()));
    let handler = SqliteHandler {
        pager: Rc::new(RefCell::new(pager)),
        header,
        db_path: path.to_path_buf(),
        autocommit: true,
        completion_schemas: Rc::clone(&completion_schemas),
    };
    handler.refresh_completion_schemas();

    let mut editor = Readline::new();
    editor.set_highlighter(SqlHighlighter);
    editor.set_completer(SchemaCompleter::new(completion_schemas));

    // `sqlite3` starts in `list` mode with headers off.
    let mut repl = Repl::new(handler);
    repl.set_mode(OutputMode::List);

    let history_file = history_path();
    let opts = ReplOptions {
        prompt: "sqlite> ",
        continuation_prompt: "   ...> ",
        history_file: history_file.as_deref(),
    };
    match run_repl_with_editor(repl, editor, opts) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: reading input: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Where the REPL's history lives: `$XDG_STATE_HOME/sqlite-rs/history`
/// when that variable is set and non-empty, else `~/.sqlite-rs_history`;
/// `None` with no `$HOME` at all (history is then session-only). Kept
/// here rather than using [`db_cli::history_path`] so the on-disk
/// location is unchanged from before #14 (`tests/unit/repl_history.rs`).
fn history_path() -> Option<PathBuf> {
    if let Some(xdg_state) = std::env::var_os("XDG_STATE_HOME") {
        if !xdg_state.is_empty() {
            return Some(PathBuf::from(xdg_state).join("sqlite-rs").join("history"));
        }
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".sqlite-rs_history"))
}

/// `.headers on`'s column labels for `select`'s result set: for a
/// single-table, non-compound `SELECT` this is
/// [`output_column_names`]'s "alias, else bare column name, else
/// `columnN`" rule against the resolved `FROM` table; anything the
/// codegen pipeline resolves less directly (no `FROM`, a join, or a
/// compound) falls back to positional `column1..columnN` labels — a
/// scope-cut noted in the issue's write-up rather than plumbing this
/// REPL's header derivation through the full join/compound resolver.
fn derive_headers(
    select: &sqlite_rs::parser::ast::Select,
    schemas: &[sqlite_rs::schema::TableSchema],
) -> Vec<String> {
    let single_table = select.compound.is_empty()
        && select
            .from
            .as_ref()
            .is_some_and(|from| from.joins.is_empty());
    if single_table {
        if let Some(from) = &select.from {
            if let Ok(schema) = resolve_from_table_schema(&from.first, schemas) {
                return output_column_names(select, &schema);
            }
        }
    }
    let count = select.columns.len().max(1);
    (1..=count).map(|i| format!("column{i}")).collect()
}
