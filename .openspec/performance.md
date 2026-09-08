# Performance

The performance benchmarks, their committed results snapshot and the V4→V7.3
progression moved to t-rust-db/benchmark
[`perf/sqlite-rs`](https://github.com/t-rust-db/benchmark/tree/main/perf/sqlite-rs)
(#22): tier 1 criterion (sqlite-rs vs libsqlite3 via rusqlite linked to the
pinned oracle), tier 2 hyperfine (CLI vs `sqlite3`), `results/bench-status.json`.

```bash
make -C ../benchmark/perf/sqlite-rs bench        # tier 1
make -C ../benchmark/perf/sqlite-rs bench-cli    # tier 2
make -C ../benchmark/perf/sqlite-rs status       # refresh results/bench-status.json
```

Only the shared fixture generator stays in this crate
(`tools/gen_fixtures.sh --bench`); the benchmark package calls it through
`SQLITE_RS_REPO` (default: the sibling checkout).
