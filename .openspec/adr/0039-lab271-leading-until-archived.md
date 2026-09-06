# 0039: Lab271/sqlite-rs is leading until it is archived; t-rust-db/sqlite-rs tracks it by snapshot

Date: 2026-09-05

## Context

`t-rust-db/sqlite-rs` was created by copying the working tree of
[Lab271/sqlite-rs](https://github.com/Lab271/sqlite-rs) at `7701d18`
(t-rust-db/sqlite-rs#1). The copy was deliberate: the org move exists so
this crate can consume `db-storage`, `db-core` and `db-cli` instead of its
own private storage, parser, VDBE and CLI layers, and that repointing
(t-rust-db/sqlite-rs#2–#7, #14–#19) will take several phases.

Meanwhile Lab271/sqlite-rs keeps landing engine work — bug fixes
(Lab271 #696–#698), the embedding-API facade (Lab271 #695), a rows-changed
counter (Lab271 #692). Two repositories are now editing the same 68,000
lines, and every extraction into `db-core`/`db-storage` is a *port from*
sqlite-rs, so the port has to know which copy is authoritative. Left
implicit, the answer drifts: `db-core`'s `parser::row` is already ~390
lines ahead of Lab271's `src/parser` (window functions, db-core#74), with
no rule saying which side should move (db-core#84).

Two questions had to be settled:

1. Which copy is the source of truth while both exist?
2. Should this repository carry Lab271's 1,051-commit history?

## Decision

**Lab271/sqlite-rs is leading until it is archived.** Concretely:

- Engine behaviour — anything under `src/parser`, `src/codegen`,
  `src/vdbe`, and the storage stack until each part is repointed — is
  changed in Lab271/sqlite-rs first. This repository takes snapshots; it
  does not fork engine behaviour. Bug reports about engine behaviour are
  filed in Lab271 while this rule holds.
- Ports into `db-core`/`db-storage`/`db-cli` cite the Lab271 source
  (issue or PR) they were derived from, and when a port *extends* what
  Lab271 has (as `parser::row` did), the extension is back-ported to
  Lab271 in the same window so the two copies converge instead of
  diverge (db-core#84 writes this into db-core's ADR 0005).
- Work that only makes sense here — the repointing itself, the
  dependency-policy ADR (0040), the `.openspec` moves to the upstream
  crates — lands here directly. A path that has been repointed leaves the
  Lab271-leading regime: from then on the `db-*` crate owns it.
- Syncing is a documented, repeatable procedure (`make sync-lab271`,
  README "Syncing from Lab271") that applies Lab271's delta as one
  `chore: sync Lab271/sqlite-rs @<sha>` commit, excluding paths already
  repointed. `Cargo.toml`'s `[package.metadata.lab271] synced` records the
  last Lab271 commit folded in, so "how far behind are we" is a one-line
  answer.

**No history import.** The snapshot stays a snapshot. Lab271/sqlite-rs
remains available as the archive of the 1,051 commits, its issue and PR
discussions, and its ADR trail; nothing is gained by duplicating that here
at the cost of rewriting this repository's `main`.

This ADR retires when Lab271/sqlite-rs is archived
(t-rust-db/sqlite-rs#21): a superseding ADR records that this repository
is the only source of truth, and the sync target and metadata are removed.

## Alternatives rejected

- **t-rust-db leading from day one, Lab271 frozen.** Would have stopped
  in-flight Lab271 work (#692, #695, #696–#698) or forced it to land on a
  tree in the middle of being repointed. The repointing is long enough
  that freezing the engine for its duration was not acceptable.
- **Both leading, merge as needed.** Two authorities for one engine with
  no rule is how the `parser::row` drift happened. Rejected as the status
  quo this ADR exists to end.
- **Import full history (`git fetch` + rebase the snapshot onto it).**
  Rewrites `main`, buys nothing the Lab271 archive does not already hold,
  and the user decided the history is not needed here (2026-09-05).

## Consequences

- Contributors have a one-sentence answer to "where do I fix this":
  engine behaviour → Lab271; integration/repointing → here.
- `make sync-lab271` will produce progressively smaller diffs as paths are
  repointed; an empty diff outside repointed paths is the precondition for
  #21 (archive).
- Ports that extend sqlite-rs carry a back-port obligation, which is
  friction by design: the alternative is permanent divergence.
- `[package.metadata.lab271] synced` is a second pinned-version site
  alongside `[package.metadata.oracle]`; unlike the oracle pin it is not
  gated by `make version-pin`, because being behind is expected, not a
  bug. It is informational.
