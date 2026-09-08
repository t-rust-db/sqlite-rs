# 0042 — No private crate registry: first-party crates are git dependencies; the Artifactory evaluation is withdrawn (supersedes ADR-0038)

**Status:** Accepted · **Date:** 2026-09-08 · Supersedes ADR-0038

## Context

ADR-0038 made JFrog Artifactory access opt-in local config so colleagues could
depend on this crate through a private Cargo registry. That need is now met
differently: the org's crates depend on each other as pinned git
dependencies (ADR-0040), the repo moved from Lab271 to t-rust-db, and the
Artifactory evaluation this was part of is over. What remained of it here was
a `JFrog` workflow gated on a secret nobody sets, a `.cargo/config.toml.example`
template pointing at an index that no longer serves this project, a docs page,
and a Dependabot bump for the JFrog CLI action (#10).

## Decision

Remove every JFrog/Artifactory artifact: `.github/workflows/jfrog.yml`,
`.cargo/config.toml.example`, `docs/src/jfrog-registry.md`, the gitignore
rationale. `package.publish` stays `false`; consumers depend on this crate by
git tag (ADR-0040), and the open Dependabot bump for the JFrog action is
closed rather than merged. Local `.cargo/config.toml` and
`.cargo/credentials.toml` stay gitignored on general principle.

## Alternatives rejected

- Keep the workflow dormant behind the secret gate: dead CI jobs still show up
  as skipped checks on every PR and attract Dependabot bumps.
- Publish to crates.io: not on the table while the crates are `publish =
  false` and still moving weekly; revisit if the git-tag pin chain (db-core →
  db-storage → sqlite-rs/column-rs) becomes the bottleneck.

## Consequences

ADR-0038 and ADR-0040's references to it are historical (ADRs are immutable).
Cargo resolves from crates.io plus the pinned t-rust-db git sources only,
which is already what `deny.toml`'s `sources` check allows.
