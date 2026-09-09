// Copyright 2026 Schuberg Philis
// SPDX-License-Identifier: Apache-2.0
//! `sqlgrep` end-to-end (#34): drives the real binary through
//! `CARGO_BIN_EXE_sqlgrep` against scratch trees, with the cache
//! redirected via `SQLGREP_CACHE_DIR` so nothing touches `~/.cache`.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use sqlite_rs::btree::TableCursor;
use sqlite_rs::dump;
use sqlite_rs::integrity::run_integrity_check;
use sqlite_rs::schema::read_schema;
use sqlite_rs::vfs::UnixVfs;

const SQLGREP: &str = env!("CARGO_BIN_EXE_sqlgrep");

struct Scratch {
    root: PathBuf,
    cache_dir: PathBuf,
}

fn scratch(label: &str) -> Scratch {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "sqlite-rs-sqlgrep-{label}-{}-{n}",
        std::process::id()
    ));
    std::fs::remove_dir_all(&dir).ok();
    let root = dir.join("tree");
    let cache_dir = dir.join("cache");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&cache_dir).unwrap();
    Scratch { root, cache_dir }
}

impl Scratch {
    fn write(&self, rel: &str, content: &str) {
        let p = self.root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(SQLGREP)
            .env("SQLGREP_CACHE_DIR", &self.cache_dir)
            .args(args)
            .arg(&self.root)
            .output()
            .unwrap_or_else(|e| panic!("spawning {SQLGREP}: {e}"))
    }

    fn search(&self, pattern: &str) -> (i32, String) {
        let out = self.run(&[pattern]);
        (
            out.status.code().unwrap(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )
    }

    fn search_args(&self, args: &[&str], pattern: &str) -> (i32, String) {
        let mut a: Vec<&str> = args.to_vec();
        a.push(pattern);
        let out = Command::new(SQLGREP)
            .env("SQLGREP_CACHE_DIR", &self.cache_dir)
            .args(&a)
            .arg(&self.root)
            .output()
            .unwrap();
        (
            out.status.code().unwrap(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )
    }

    fn cache_path(&self) -> PathBuf {
        let out = self.run(&["cache-path"]);
        assert!(out.status.success());
        PathBuf::from(String::from_utf8_lossy(&out.stdout).trim())
    }
}

/// Opens the cache the way any reader would and asserts it is a clean
/// SQLite database with exactly sqlgrep's schema.
fn assert_cache_healthy(path: &Path) {
    let (header, pager) = dump::open(&UnixVfs, path).expect("cache opens");
    let problems = run_integrity_check(&pager, &header, false);
    assert_eq!(problems, ["ok"], "integrity_check: {problems:?}");
    let mut cursor = TableCursor::new(&pager, &header, 1);
    let mut names: Vec<String> = read_schema(&mut cursor, header.text_encoding)
        .unwrap()
        .into_iter()
        .map(|s| s.name)
        .collect();
    names.sort();
    assert_eq!(names, ["files", "meta", "trigrams"]);
}

#[test]
fn index_then_search_prints_file_line_matches_and_grep_exit_codes() {
    let s = scratch("basic");
    s.write("a.txt", "alpha\nneedle_one here\n");
    s.write("sub/b.rs", "fn needle_one() {}\nnope\n");
    s.write("c.txt", "nothing\n");

    let out = s.run(&["index"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("3 added"));

    let (code, stdout) = s.search("needle_one");
    assert_eq!(code, 0);
    let mut lines: Vec<&str> = stdout.lines().collect();
    lines.sort();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].ends_with("a.txt:2:needle_one here"), "{lines:?}");
    assert!(
        lines[1].ends_with("sub/b.rs:1:fn needle_one() {}"),
        "{lines:?}"
    );

    let (code, stdout) = s.search("zzz_absent_zzz");
    assert_eq!(code, 1);
    assert!(stdout.is_empty());

    let out = s.run(&["("]);
    assert_eq!(out.status.code(), Some(2));

    assert_cache_healthy(&s.cache_path());
}

#[test]
fn search_without_prior_index_builds_the_cache_first() {
    let s = scratch("lazy");
    s.write("x.txt", "lazy_needle\n");
    assert!(!s.cache_path().exists());
    let (code, stdout) = s.search("lazy_needle");
    assert_eq!(code, 0);
    assert!(stdout.contains("x.txt:1:lazy_needle"));
    assert!(s.cache_path().exists());
}

#[test]
fn regex_patterns_narrow_by_literals_but_match_by_regex() {
    let s = scratch("regex");
    s.write("a.txt", "foo123bar\nfoo bar\nfooXbar\n");
    let (_, stdout) = s.search("foo[0-9]+bar");
    assert_eq!(stdout.lines().count(), 1);
    assert!(stdout.contains(":1:foo123bar"));
    // No 3-byte literal run: falls back to scanning everything.
    let (_, stdout) = s.search("o.b");
    assert_eq!(stdout.lines().count(), 2, "{stdout}");
    // Case-insensitive also scans everything.
    let (_, stdout) = s.run_search_i("FOOX");
    assert!(stdout.contains(":3:fooXbar"));
}

impl Scratch {
    fn run_search_i(&self, pattern: &str) -> (i32, String) {
        let out = Command::new(SQLGREP)
            .env("SQLGREP_CACHE_DIR", &self.cache_dir)
            .args(["-i", pattern])
            .arg(&self.root)
            .output()
            .unwrap();
        (
            out.status.code().unwrap(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )
    }
}

#[test]
fn incremental_update_touches_only_changed_files() {
    let s = scratch("incremental");
    s.write("keep.txt", "steady_needle\n");
    s.write("edit.txt", "before_needle\n");
    s.write("gone.txt", "gone_needle\n");
    let out = s.run(&["index"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("3 added"));

    // A second pass with nothing changed writes nothing.
    let out = s.run(&["index"]);
    let report = String::from_utf8_lossy(&out.stderr);
    assert!(
        report.contains("0 added, 0 changed, 0 removed, 3 unchanged"),
        "{report}"
    );
    assert!(report.contains("(0 posting lists rewritten)"), "{report}");

    // Distinct content so the mtime/size check cannot be fooled.
    std::thread::sleep(std::time::Duration::from_millis(20));
    s.write("edit.txt", "after_needle!!\n");
    std::fs::remove_file(s.root.join("gone.txt")).unwrap();
    s.write("new.txt", "fresh_needle\n");
    let out = s.run(&["index"]);
    let report = String::from_utf8_lossy(&out.stderr);
    assert!(
        report.contains("1 added, 1 changed, 1 removed, 1 unchanged"),
        "{report}"
    );

    assert_eq!(s.search("after_needle").0, 0);
    assert_eq!(s.search("fresh_needle").0, 0);
    assert_eq!(s.search("steady_needle").0, 0);
    // Stale postings are tombstones: the deleted file never surfaces.
    assert_eq!(s.search("gone_needle").0, 1);
    assert_eq!(s.search("before_needle").0, 1);
    assert_cache_healthy(&s.cache_path());
}

#[test]
fn rebuild_discards_the_old_cache() {
    let s = scratch("rebuild");
    s.write("a.txt", "one_needle\n");
    s.run(&["index"]);
    let before = std::fs::metadata(s.cache_path()).unwrap().len();
    // Grow the cache with a file, then delete it: without --rebuild the
    // tombstoned postings stay; with it the cache is exactly as fresh.
    let varied: String = (0..5000).map(|i| format!("tok{i} ")).collect();
    s.write("big.txt", &varied);
    s.run(&["index"]);
    std::fs::remove_file(s.root.join("big.txt")).unwrap();
    s.run(&["index"]);
    assert!(std::fs::metadata(s.cache_path()).unwrap().len() > before);
    let out = s.run(&["index", "--rebuild"]);
    assert!(out.status.success());
    assert_eq!(std::fs::metadata(s.cache_path()).unwrap().len(), before);
    assert_eq!(s.search("one_needle").0, 0);
    assert_cache_healthy(&s.cache_path());
}

#[test]
fn binary_files_and_symlinks_are_skipped() {
    let s = scratch("binary");
    s.write("text.txt", "bin_needle\n");
    std::fs::write(s.root.join("blob.bin"), b"bin_needle\0\x01\x02").unwrap();
    std::os::unix::fs::symlink(s.root.join("text.txt"), s.root.join("link.txt")).unwrap();
    let (code, stdout) = s.search("bin_needle");
    assert_eq!(code, 0);
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    assert!(stdout.contains("text.txt:1:"));
}

#[test]
fn gitignore_is_honored_inside_a_work_tree() {
    let s = scratch("gitignore");
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(&s.root)
            .args(args)
            .output()
            .expect("git present");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-q"]);
    s.write(".gitignore", "ignored.txt\nbuild/\n");
    s.write("tracked.txt", "gi_needle tracked\n");
    s.write("untracked.txt", "gi_needle untracked-but-not-ignored\n");
    s.write("ignored.txt", "gi_needle ignored\n");
    s.write("build/out.txt", "gi_needle in ignored dir\n");
    git(&["add", "tracked.txt"]);

    let (code, stdout) = s.search("gi_needle");
    assert_eq!(code, 0);
    let mut hits: Vec<&str> = stdout.lines().collect();
    hits.sort();
    assert_eq!(hits.len(), 2, "{stdout}");
    assert!(hits[0].contains("tracked.txt:1:"));
    assert!(hits[1].contains("untracked.txt:1:"));
}

#[test]
fn one_cache_file_per_canonical_root() {
    let s = scratch("roots");
    s.write("a/x.txt", "x\n");
    s.write("b/y.txt", "y\n");
    let path_for = |sub: &str| {
        let out = Command::new(SQLGREP)
            .env("SQLGREP_CACHE_DIR", &s.cache_dir)
            .arg("cache-path")
            .arg(s.root.join(sub))
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    assert_ne!(path_for("a"), path_for("b"));
    // Same root through a non-canonical spelling: same file.
    assert_eq!(path_for("a"), path_for("b/../a/."));
    assert!(path_for("a").starts_with(s.cache_dir.to_str().unwrap()));
}

/// #38: `-n`/`--no-update` must skip the freshness check entirely — a
/// file added after the last `index` stays invisible (its id was never
/// assigned, so it can never be a trigram candidate) until a normal run
/// happens, and no cache write occurs in between. Search always reads
/// the *real* file for the actual match, though: an existing, unchanged
/// file is found by `-n` exactly as it would be without it.
#[test]
fn no_update_flag_searches_the_cache_as_is() {
    let s = scratch("no_update");
    s.write("a.txt", "steady_needle\n");
    s.run(&["index"]);
    let before = std::fs::metadata(s.cache_path())
        .unwrap()
        .modified()
        .unwrap();

    // A brand-new file with the same needle: never indexed, so -n must
    // not see it even though its content matches.
    s.write("b.txt", "steady_needle too\n");

    let (code, stdout) = s.search_args(&["-n"], "steady_needle");
    assert_eq!(code, 0, "{stdout}");
    assert_eq!(
        stdout.lines().count(),
        1,
        "{stdout} (b.txt must be invisible)"
    );
    assert!(stdout.contains("a.txt:1:"), "{stdout}");

    let after = std::fs::metadata(s.cache_path())
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(before, after, "-n must not write to the cache");

    // A plain search (no -n) catches up as normal.
    let (code, stdout) = s.search("steady_needle");
    assert_eq!(code, 0, "{stdout}");
    assert_eq!(stdout.lines().count(), 2, "{stdout}");
}

/// #38: `--no-update` combined with `index` also stays a pure read —
/// verifies the flag has one meaning everywhere it is accepted, not one
/// behavior for `index` and another for a bare search.
#[test]
fn no_update_on_index_makes_it_a_no_op() {
    let s = scratch("no_update_index");
    s.write("a.txt", "one\n");
    let out = Command::new(SQLGREP)
        .env("SQLGREP_CACHE_DIR", &s.cache_dir)
        .args(["index", "--no-update"])
        .arg(&s.root)
        .output()
        .unwrap();
    assert!(out.status.success());
    // The cache file is still created (opening always bootstraps an
    // empty schema) but nothing was scanned or written into it.
    assert!(s.cache_path().exists());
    assert_eq!(
        s.search_args(&["-n"], "one").0,
        1,
        "nothing was ever indexed"
    );
    assert_cache_healthy(&s.cache_path());
}

/// #38: the non-git fallback walk collects metadata while listing files;
/// `index::update` must reuse it (not re-`stat`) and still get correct
/// mtime/size — proven by an edit being detected exactly once, not
/// silently missed because a second stat somehow disagreed with the walk.
#[test]
fn metadata_reused_from_the_walk_still_detects_edits_outside_git() {
    let s = scratch("no_git_walk");
    s.write("a.txt", "before\n");
    let out = s.run(&["index"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("1 added"));

    std::thread::sleep(std::time::Duration::from_millis(20));
    s.write("a.txt", "after_marker\n");
    let out = s.run(&["index"]);
    let report = String::from_utf8_lossy(&out.stderr);
    assert!(
        report.contains("0 added, 1 changed, 0 removed, 0 unchanged"),
        "{report}"
    );
    assert_eq!(s.search("after_marker").0, 0);
    assert_eq!(s.search("before").0, 1);
}

/// Regression pin for a db-storage v0.6.2 bug found while indexing (#34,
/// t-rust-db/db-storage#31): `insert_into_leaf` split a full leaf by cell
/// *count*, so a leaf holding
/// many ~90-byte posting lists next to ~400-byte ones can hand the right
/// half more bytes than a page holds; `write_leaf_page` then wraps instead
/// of erroring and the next descent fails with "unexpected b-tree page
/// type". A vocabulary shared by all 400 files (long posting lists) mixed
/// with per-file tokens (short ones) is exactly that shape. Fixed in
/// db-storage v0.6.3 (t-rust-db/db-storage#31); this stays as the pin.
#[test]
fn mixed_size_posting_lists_split_correctly() {
    let s = scratch("mixed");
    for i in 0..400u32 {
        let mut text = String::new();
        for j in 0..200u32 {
            text.push_str(&format!(
                "w{}x{}y{} ",
                i,
                j,
                i.wrapping_mul(7919).wrapping_add(j)
            ));
        }
        s.write(&format!("f{i}.txt"), &text);
    }
    let out = s.run(&["index"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_cache_healthy(&s.cache_path());
}
