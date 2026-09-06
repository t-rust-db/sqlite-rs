# 0040: First-party t-rust-db crates are allowed as pinned git dependencies; third-party stays targeted at zero

Date: 2026-09-05

Amends [ADR-0030](0030-zero-proc-macro-dependencies.md) and
[ADR-0031](0031-vendor-nix-subset.md). Both stay accepted for what they
decided (no proc-macro crates; vendor the `nix` subset rather than depend on
`libc`); this ADR narrows one sentence of ADR-0031's consequences — "the
crate now has zero external dependencies" — which the org move makes
untrue by design.

## Context

ADR-0030 and ADR-0031 took this crate from `thiserror` + `rustyline` + `nix`
to an empty `[dependencies]` table, and the supply-chain gates were built
on that: `deny.toml` allows only `crates.io` as a source and only Apache-2.0
as a license, `sqlite-rs.cdx.json` describes an empty production graph, and
`cargo vet` has nothing to audit in the production closure.

t-rust-db/sqlite-rs#1 replaces this crate's private storage, parser, VDBE
and CLI layers with the org's shared crates, which live in sibling
repositories and are consumed the way column-rs already consumes them:

```toml
db-storage = { git = "https://github.com/t-rust-db/db-storage", tag = "v0.4.0", default-features = false, features = ["row"] }
db-core    = { git = "https://github.com/t-rust-db/db-core",    tag = "vX.Y.Z", default-features = false, features = ["parser-row", "vm-row", "codegen-row"] }
db-cli     = { git = "https://github.com/t-rust-db/db-cli",     tag = "vX.Y.Z" }
```

None of these is on crates.io (the Artifactory registry of ADR-0038 is
opt-in and cannot be a committed source), so they are git dependencies.
They are also not dependency-free themselves today:

| crate | third-party runtime deps | status |
|---|---|---|
| db-core | none | zero since db-core#42 (std-only fork-join replaced `rayon`) |
| db-storage | `memmap2`, `ruzstd` (+ `twox-hash`) | unconditional, only `column` uses them → t-rust-db/db-storage#12 makes them optional |
| db-cli | `libc`, `dirs` | termios/XDG; this crate vendored the same ~200 lines instead (ADR-0031) |

`ruzstd` and `twox-hash` are MIT-only, so the Apache-2.0-only license
allow-list cannot stay as it is either.

## Decision

1. **First-party t-rust-db crates may be declared as git dependencies**,
   pinned to a release tag (never a branch), with `default-features = false`
   and an explicit feature list, so this crate compiles exactly the
   modules it uses. `deny.toml`'s `[sources] allow-git` lists each
   `https://github.com/t-rust-db/<crate>` URL as it is adopted; anything
   else stays `unknown-git = "deny"`.
2. **Third-party runtime dependencies remain targeted at zero, one level
   up.** This crate declares none of its own. What arrives transitively
   through a first-party crate is that crate's debt, tracked there
   (t-rust-db/db-storage#12; db-cli's `libc`/`dirs` — decision 2026-09-05:
   keep for now, minimize later, with db-core#42 as the model). The
   `[sources]`/`[licenses]`/`[bans]` gates keep running over the full
   closure, so every transitive crate is still license-checked, advisory-
   checked and vetted here; the change is what is *permitted*, not what is
   *inspected*.
3. **`deny.toml` `[licenses] allow` gains `MIT`** for the MIT-only
   transitive crates. The comment that explained why MIT was dropped is
   replaced by one naming which crates need it, so the allowance can be
   removed again when they go.
4. **SBOMs and `cargo vet` follow the closure.** `sqlite-rs.cdx.json` is
   regenerated whenever a first-party dependency (or its tag) changes, and
   new transitive crates get `cargo vet` exemptions or audits like any
   other — no first-party carve-out from vetting.
5. ADR-0031's carve-out for `src/sys/{fcntl,termios}.rs` stays until the
   module that uses each is repointed (`fcntl` → db-storage `row::vfs`,
   t-rust-db/sqlite-rs#2; `termios` → db-cli, #14). When both are gone,
   this crate has no `unsafe` of its own and ADR-0031's `unsafe` half is
   moot here — it continues to describe the code where it now lives.

## Alternatives rejected

- **Vendor the shared crates into `src/` (path dependencies or copies).**
  Recreates the private-copy situation #1 exists to end.
- **Publish the org crates to crates.io and depend by version.** Not
  available yet (the org crates are `publish = false` on a private
  registry evaluation, ADR-0038), and would not change the transitive
  third-party question.
- **Wait for db-storage/db-cli to reach zero third-party deps before
  depending on them.** Serialises the whole epic behind dependency
  minimisation that has been explicitly deferred; the gates already cover
  the transitive crates.
- **Track a branch instead of a tag.** Loses reproducibility that
  `--locked` and the SBOM assume; a tag bump is a reviewed change, like an
  oracle bump.

## Consequences

- `[dependencies]` is no longer empty; `sqlite-rs.cdx.json`'s production
  component list is non-empty for the first time since #563, and the
  `gen_dev_sbom.py` docstring's "currently empty" remark is stale.
- `cargo vet` gains exemptions for `memmap2`, `ruzstd`, `twox-hash` (and
  later `libc`, `dirs`); the git crates themselves are outside
  `cargo vet`'s registry scope and are trusted as first-party.
- Bumping a first-party tag is the org-internal equivalent of a
  dependency update: it goes through `make check-deny`, `cargo vet`,
  `make sbom` and a full test run like any lockfile change.
- ADR-0030's "zero proc-macro dependencies" holds unchanged: none of the
  first-party crates or their transitive deps are proc-macro crates
  (`cargo tree -e build` stays free of `proc-macro2`/`syn`).
