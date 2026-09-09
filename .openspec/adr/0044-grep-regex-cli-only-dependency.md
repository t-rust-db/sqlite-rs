# 0044 — `grep` subcommand's `regex` dependency is CLI-only, gated exactly like `db-cli` (amends ADR-0040 decision 2)

**Status:** Accepted · **Date:** 2026-09-09 · Amends decision 2 of
[ADR-0040](0040-first-party-git-dependencies.md)

## Context

Issue #45 adds a `grep` subcommand to the `sqlite-rs` binary (`src/bin/sqlite-rs/`):
searches every table/row/column of a SQLite database file for a pattern,
the job `eirtools/sqlgrep` did. Default matching is regex (`-F` switches to
a fixed-string substring match). ADR-0040 decision 2 commits this crate to
zero third-party runtime dependencies — a regex engine is exactly such a
dependency.

ADR-0043 (now superseded/moved) already established the shape for this
exact tension once before: `sqlgrep` as a *separate binary* behind its own
feature, so a `regex` dependency never reached library consumers. That
binary and its feature moved out to `t-rust-db/trigrep` entirely, so
ADR-0040 decision 2 currently stands unamended. This ticket is different in
kind, though: `grep` is a subcommand of the *existing* `sqlite-rs` binary
(same shape as `.dump`/`query` — parser+VDBE-adjacent, wired through
`src/bin/sqlite-rs/main.rs`), not a new standalone tool. It doesn't need a
new binary or a new feature; it needs the existing `cli` feature (already
optional, already the boundary that keeps `db-cli`'s readline/history/REPL
closure out of library-only consumers) to also gate one more dependency.

## Decision

`regex` is declared in `Cargo.toml` as `optional = true`, folded into the
existing `cli` feature exactly the way `db-cli` already is:

```toml
db-cli = { version = "0.4.1", ..., optional = true }
regex = { version = "1.10", optional = true }

[features]
default = ["cli"]
cli = ["dep:db-cli", "dep:regex"]
```

No new feature is introduced — `grep` rides the same `cli` on/off switch
`dump`/`query`/`tables`/`exec`/`repl` already ride, since it is exactly as
CLI-only as they are. `cargo tree -e normal --no-default-features` must
show neither `db-cli` nor `regex`; verified as part of #45's acceptance
criteria.

## Alternatives rejected

- **A dedicated `grep` feature, `regex`-only.** Would let a consumer take
  `cli` (readline/REPL) without `regex`, or vice versa — but no such split
  consumer exists today (nothing else in `cli` needs `regex`, and `grep`
  needs everything `cli` already pulls in to be a binary subcommand at
  all), so the extra feature is speculative granularity ADR-0033's
  "don't build for a second use case that hasn't arrived" instinct argues
  against.
- **A new standalone `sqlgrep`-shaped binary + feature, ADR-0043's shape.**
  Rejected by the issue itself (#45's body: "this is NOT the trigram
  source-tree grep... it needs the parser and VDBE... natural CLI
  subcommand over the library, like `.dump`") — `grep` belongs inside the
  existing binary, so ADR-0043's per-binary-feature answer doesn't apply
  here; that shape stays reserved for a tool that, like the trigram grep,
  needs nothing of the parser/VDBE.
- **Hand-rolled fixed-string/glob-only matching, no `regex` crate.** Would
  keep decision 2 unamended, but the issue's acceptance criteria are
  explicit about `regex`-by-default matching parity with `eirtools/sqlgrep`
  and `grep(1)`; a hand-rolled regex engine is exactly the kind of
  speculative-engineering-for-its-own-sake this crate's simplicity bar (see
  CLAUDE.md) argues against building in-house when a well-audited crate
  exists and can be kept CLI-only.

## Consequences

- `default-features = false` library consumers are unaffected: no `regex`
  in their dependency tree (same guarantee `db-cli`'s gating already
  gives), verified by `cargo tree -e normal --no-default-features`.
- The `cli` feature's dependency surface grows by one crate
  (`regex` 1.x, itself pulling in `regex-automata`/`regex-syntax`/
  `aho-corasick`/`memchr` — all permissively licensed, well-audited, no
  proc-macros) for anyone building the `sqlite-rs` binary. This is judged
  acceptable the same way `db-cli`'s own closure (`libc`, `dirs`) already
  was: it never reaches a library-only consumer.
- Any *future* CLI-only subcommand needing a third-party dependency has a
  established, citable precedent to follow: fold into `cli` if it's truly
  binary-only-inseparable from what `cli` already gates, or open a new ADR
  amending this one if it needs an independent on/off switch.
