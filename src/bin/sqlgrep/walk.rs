// Copyright 2026 Schuberg Philis
// SPDX-License-Identifier: Apache-2.0
//! File enumeration for one indexed root (#34).
//!
//! Inside a git work tree the list comes from `git ls-files --cached
//! --others --exclude-standard`, which is git's own `.gitignore`/
//! `.git/info/exclude`/global-excludes semantics — byte-exact with what
//! ripgrep and tgrep aim to reproduce, with no ignore-rule parser of our
//! own to get subtly wrong. Outside a work tree it is a plain recursive
//! walk that skips `.git` directories and never follows symlinks.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Paths relative to `root`, sorted, one per regular file to consider.
pub fn list_files(root: &Path) -> std::io::Result<Vec<String>> {
    let mut files = match git_ls_files(root) {
        Some(list) => list,
        None => {
            let mut out = Vec::new();
            walk_dir(root, root, &mut out)?;
            out
        }
    };
    files.sort_unstable();
    files.dedup();
    Ok(files)
}

fn git_ls_files(root: &Path) -> Option<Vec<String>> {
    let inside = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .ok()?;
    if !inside.status.success() || String::from_utf8_lossy(&inside.stdout).trim() != "true" {
        return None;
    }
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(
        out.stdout
            .split(|&b| b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect(),
    )
}

fn walk_dir(root: &Path, dir: &Path, out: &mut Vec<String>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path: PathBuf = entry.path();
        // `symlink_metadata` so a symlinked directory is neither followed
        // nor listed — loops and out-of-root escapes are both avoided.
        let meta = std::fs::symlink_metadata(&path)?;
        if meta.is_dir() {
            if entry.file_name() == ".git" {
                continue;
            }
            walk_dir(root, &path, out)?;
        } else if meta.is_file() {
            if let Ok(rel) = path.strip_prefix(root) {
                out.push(rel.to_string_lossy().into_owned());
            }
        }
    }
    Ok(())
}
