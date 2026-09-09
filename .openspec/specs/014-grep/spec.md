---
domain: grep
version: 0.1.0
status: draft
date: 2026-09-09
---

# 014 — grep

`sqlite-rs grep` searches for a pattern *inside* one or more SQLite
database files: every table in `sqlite_master` (or, in a future cut,
explicit `SELECT` query sources), every row, every column, matched
against the cell's text form — the job `eirtools/sqlgrep` did. This is
**not** the trigram source-tree grep (that moved to
[t-rust-db/trigrep](https://github.com/t-rust-db/trigrep) — see
ADR-0043, spec 013 "moved" pointer). `grep` here is a subcommand of the
existing `sqlite-rs` binary (`src/bin/sqlite-rs/grep.rs`), reusing the
same parser/VDBE-adjacent, read-only-open plumbing `dump`/`query`
already use.

Refs: #45

Porting note: the output line format and CLI flag shapes below port the
*behavior* of `eirtools/sqlgrep` (Apache-2.0) — no source code from that
project is vendored, only the observed contract (see `README.md`'s
`### grep` section for the citation).

## Requirements

### Requirement 1: Default table scan [MUST]

`sqlite-rs grep <pattern> <file>...` MUST, with no explicit query
source given, search every non-virtual table listed in each file's
`sqlite_master`, streaming rows through the existing table-cursor
b-tree reader (`sqlite_rs::btree::TableCursor`) rather than
materializing a whole table into memory first.

- WITHOUT ROWID tables and virtual tables are skipped (out of scope,
  matching issue #45's "Not in scope" — a future cut may lift the
  WITHOUT ROWID restriction as VDBE support grows).
- One output line per matching cell:
  `[<file>::]<table>::<row index>::<column>::<value>`, row index
  zero-based, `<file>::` prefix present only per Requirement 4.
- Exit code 0 if at least one cell matched across all inputs, 1 if none
  matched, 2 on a usage or open error.

**Implementation:** `src/bin/sqlite-rs/grep.rs::run_grep`,
`src/bin/sqlite-rs/grep.rs::grep_one_file`

#### Scenario: A text column match is printed and exits 0

- GIVEN a database with table `items(name TEXT, ...)` and a row whose
  `name` is `"banana bread"`
- WHEN `sqlite-rs grep banana <file>` is run
- THEN stdout is exactly `items::<row>::name::banana bread\n` and the
  process exits 0

**Tests:** `tests/unit/grep_test.rs::grep_matches_text_column`

#### Scenario: No match exits 1 with empty stdout

- GIVEN a database with no cell matching a given pattern
- WHEN `sqlite-rs grep <pattern> <file>` is run
- THEN stdout is empty and the process exits 1

**Tests:** `tests/unit/grep_test.rs::grep_no_match_exits_1`

#### Scenario: An unopenable file exits 2

- GIVEN a path that does not exist
- WHEN `sqlite-rs grep <pattern> <path>` is run
- THEN the process exits 2

**Tests:** `tests/unit/grep_test.rs::grep_open_error_exits_2`

### Requirement 2: Matching and cell text rendering [MUST]

Matching is regex by default (the `regex` crate, CLI-only per
ADR-0044), `-F`/`--fixed-strings` switches to a literal substring
match, and `-i`/`--ignore-case` makes either mode case-insensitive.

A cell's text form (what the pattern is matched against) follows
SQLite's `CAST(x AS TEXT)` rendering:

- INTEGER and TEXT render as their ordinary text form.
- REAL renders via the same shell-parity renderer `dump`/`query` already
  use (`sqlite_rs::format::format_real` — 15 significant digits, always
  an explicit decimal point or exponent), so `3.0` never renders as
  bare `3`.
- **NULL has no text form and is never a matching cell** — an
  undocumented-in-`eirtools` decision this port makes explicitly,
  reasoned from the same `CAST(NULL AS TEXT) IS NULL` semantics real
  SQLite uses: there is nothing to CAST a NULL into.
- **BLOB** is controlled by `--blob=skip|text|hex` (default `skip`, also
  undocumented in `eirtools`): `skip` never matches a BLOB cell, `text`
  matches a lossy-UTF-8 view of the bytes, `hex` matches the same
  `X'HEX'` rendering `.dump`'s `quote()`-style renderer already uses
  (`sqlite_rs::format::format_blob`).

**Implementation:** `src/bin/sqlite-rs/grep.rs::Matcher`,
`src/bin/sqlite-rs/grep.rs::cell_text`

#### Scenario: -F matches a REAL cell's rendered decimal point

- GIVEN a REAL column holding a whole-number value stored with REAL
  affinity (renders as `3.0`, not `3`)
- WHEN `sqlite-rs grep -F 3.0 <file>` is run
- THEN the row's REAL cell is reported as matching

**Tests:** `tests/unit/grep_test.rs::grep_fixed_string_matches_real_rendering`

#### Scenario: -i matches case-insensitively

- GIVEN a text cell `"apple pie"`
- WHEN `sqlite-rs grep -i APPLE <file>` is run
- THEN the cell is reported as matching

**Tests:** `tests/unit/grep_test.rs::grep_ignore_case`

#### Scenario: NULL cells never match

- GIVEN a row with a NULL column and a non-NULL column whose text
  matches the pattern
- WHEN `sqlite-rs grep <pattern> <file>` is run
- THEN only the non-NULL column's cell is reported; the NULL column
  produces no output line under any pattern

**Tests:** `tests/unit/grep_test.rs::grep_null_cells_never_match`

### Requirement 3: BLOB handling is opt-in [MUST]

**Implementation:** `src/bin/sqlite-rs/grep.rs::cell_text`,
`src/bin/sqlite-rs/grep.rs::BlobMode`

BLOB cells never match by default; `--blob=hex` and `--blob=text`
explicitly opt in.

#### Scenario: BLOB is skipped by default

- GIVEN a BLOB cell whose bytes, decoded as text, would match the
  pattern
- WHEN `sqlite-rs grep <pattern> <file>` is run with no `--blob` flag
- THEN the process exits 1 (no match)

**Tests:** `tests/unit/grep_test.rs::grep_blob_skipped_by_default`

#### Scenario: --blob=hex matches the hex rendering

- GIVEN a BLOB cell containing bytes `68656C6C6F`
- WHEN `sqlite-rs grep --blob=hex 68656C6C6F <file>` is run
- THEN the cell is reported as matching, rendered as `X'68656C6C6F'`

**Tests:** `tests/unit/grep_test.rs::grep_blob_hex_mode_matches`

### Requirement 4: Multiple files and filename prefixing [MUST]

**Implementation:** `src/bin/sqlite-rs/grep.rs::run_grep`

Given more than one input file, every output line is prefixed with
`<file>::` (grep's `-H` behavior, on by default once there is more than
one file); `-h` forces the prefix off and `-H` forces it on regardless
of file count.

#### Scenario: Two input files both get filename-prefixed output

- GIVEN two copies of the same fixture database
- WHEN `sqlite-rs grep banana a.db b.db` is run
- THEN every output line is prefixed with the originating file's path

**Tests:** `tests/unit/grep_test.rs::grep_multiple_files_prefixes_filename`

### Requirement 5: Streaming, read-only, WAL-aware open [MUST]

**Implementation:** `src/bin/sqlite-rs/grep.rs::grep_one_file`,
`sqlite_rs::dump::open`

Opening a database for `grep` MUST go through the same read-only path
`dump`/`query` already use (`sqlite_rs::dump::open`), which never
writes a journal or WAL file and which merges any committed-but-not-yet-
checkpointed WAL frames into what it reads (spec 007 Requirement 3) —
so `grep` sees exactly what a live `sqlite3` connection would see, even
against a copy-protected (read-only-permissions) file. Per-table
iteration streams rows one at a time through `TableCursor`; no table's
full row set is ever collected into memory before being searched.

#### Scenario: grep sees rows committed only to WAL frames

- GIVEN a database whose main file has three separate committed
  transactions still pending in `-wal` frames, none checkpointed
- WHEN `sqlite-rs grep -F three <file>` is run
- THEN the uncheckpointed row is found and reported

**Tests:** `tests/unit/grep_test.rs::grep_sees_uncheckpointed_wal_rows`

### Requirement 6: --schema also searches DDL [SHOULD]

**Implementation:** `src/bin/sqlite-rs/grep.rs::grep_one_file`

`--schema` additionally greps `sqlite_master.sql` (the verbatim
`CREATE TABLE`/`CREATE INDEX` text), reporting matches as
`sqlite_master::<index>::sql::<ddl>`.

#### Scenario: --schema finds a matching CREATE TABLE statement

- GIVEN a database with a table `items` created as
  `CREATE TABLE items(...)`
- WHEN `sqlite-rs grep --schema "CREATE TABLE items" <file>` is run
- THEN a `sqlite_master::...::sql::...` line matching that DDL is
  printed

**Tests:** `tests/unit/grep_test.rs::grep_schema_flag_searches_ddl`

## Descoped (not implemented in this first cut)

Documented explicitly per issue #45's request to be honest about what
was actually achieved, rather than silently narrowing scope:

- **`sqlite://` URL form for `<file>` arguments.** Only plain filesystem
  paths are accepted. No current caller needs the URL form, and it adds
  a parsing/validation surface with no test coverage to justify it yet.
- **Explicit query sources** (a positional `SELECT`, `-` for stdin,
  `@file` curl-style). Only the default "every table in
  `sqlite_master`" scope is implemented; a query source would additionally
  need the full `parse_select`→codegen→`execute_with_db` pipeline
  `query.rs` already has wired up, which is a larger, separable follow-on
  (tracking issue to be filed) rather than squeezed into this first cut.
- **Byte-identical porting of `eirtools/sqlgrep`'s own test fixtures.**
  This port builds its own fixture (`tests/unit/fixtures/grep_fixture.db`,
  text/integer/real/NULL/BLOB columns) rather than vendoring
  `eirtools/sqlgrep`'s exact binary fixture bytes; what is claimed and
  tested is *output line format* parity
  (`<table>::<row>::<column>::<value>`), not byte-identical fixture
  provenance.
- **WITHOUT ROWID tables** beyond what the VDBE already supports (issue
  #45's own "Not in scope"), and FTS tables.
