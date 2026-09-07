# 0041 — Full oracle parity testing lives in t-rust-db/benchmark, not in this crate

**Status:** Accepted · **Date:** 2026-09-07 · Supersedes the `tests/parity/` clause of ADR-0004

## Context

ADR-0004 made the side-by-side parity suite (`tests/parity/`, one target per value block running the same SQL through sqlite-rs and the pinned sqlite3) the gate for the acceptance/output/schema dimensions of the compatibility contract. Since the org move (#1) t-rust-db/benchmark is the home for every product's cross-engine comparison (column-rs vs DuckDB landed there first), decoupled from any one product's release cycle and test suite. Keeping a second oracle-parity runner inside this crate duplicated that role, dragged the pinned-oracle requirement into `cargo test --test parity` for every contributor, and split "how far are we from SQLite" across two repos.

## Decision

Full oracle parity testing lives **only** in t-rust-db/benchmark, `parity/sqlite-rs/` (t-rust-db/benchmark#1). This crate keeps the regression gates that do not compare engines side by side: the fixture-diff corpus (`tests/corpus`, ADR-0005's pinned oracle still generates and diffs fixtures), the vendored sqllogictest slice, and the tier contracts. Specs cite parity evidence as cross-repo links (`benchmark:parity/sqlite-rs/tests/parity/v02.rs::…`, assurance feature 11), so traceability is unchanged; the `Parity:` dashboard line goes with the suite.

## Alternatives rejected

- Keep `tests/parity/` here and mirror it in benchmark: two copies drift.
- Move the corpus too: the corpus is this crate's byte-level regression net, not parity; every PR needs it locally.

## Consequences

`make test-parity` is gone; run `make -C ../benchmark/parity/sqlite-rs test` against a release build. The benchmark suite pins sqlite-rs by git tag for its two library-level checks and must bump on each release. ADR-0004's three-surface contract is otherwise unchanged.
