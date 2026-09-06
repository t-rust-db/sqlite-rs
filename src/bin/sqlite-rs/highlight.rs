// Copyright 2026 Schuberg Philis
// SPDX-License-Identifier: Apache-2.0
//! Syntax highlighting for the line being edited (#558): keywords,
//! strings, numbers, comments, dot-commands, colored while typing.
//! Built on the real tokenizer ([`sqlite_rs::parser::tokenizer::Tokenizer`])
//! so keyword/string/number spans match the parser's own idea of a
//! token — never panics on malformed/partial input mid-edit. Plugged
//! into db-cli's editor via its [`db_cli::Highlighter`] hook
//! (t-rust-db/sqlite-rs#14), replacing db-cli's keyword-list default.

use sqlite_rs::parser::tokenizer::{TokenKind, Tokenizer};

const RESET: &str = "\x1b[0m";
const BOLD_BLUE: &str = "\x1b[1;34m";
const GREEN: &str = "\x1b[32m";
const CYAN: &str = "\x1b[36m";
const GRAY: &str = "\x1b[90m";
const YELLOW: &str = "\x1b[33m";

/// Colorizes `line` for display, returning a string with ANSI escapes
/// inserted — zero-width additions only, so db-cli's cursor math stays
/// correct (its `Highlighter` contract).
pub fn highlight(line: &str) -> String {
    if line.trim_start().starts_with('.') {
        return format!("{YELLOW}{line}{RESET}");
    }

    let mut spans: Vec<(usize, usize, &'static str)> = Vec::new();
    for tok in &Tokenizer::tokenize(line) {
        let start = tok.span.offset as usize;
        let len = tok.span.len as usize;
        if len == 0 {
            continue;
        }
        let color = match &tok.kind {
            TokenKind::Keyword(_) | TokenKind::Null | TokenKind::True | TokenKind::False => {
                Some(BOLD_BLUE)
            }
            TokenKind::String(_) | TokenKind::Blob(_) => Some(GREEN),
            TokenKind::Integer(_) | TokenKind::Float(_) => Some(CYAN),
            _ => None,
        };
        if let Some(color) = color {
            spans.push((start, start.saturating_add(len), color));
        }
    }
    for (start, end) in comment_byte_ranges(line) {
        spans.push((start, end, GRAY));
    }
    spans.sort_by_key(|(start, _, _)| *start);

    let mut out = String::with_capacity(line.len().saturating_add(16));
    let mut pos = 0usize;
    for (start, end, color) in spans {
        if start < pos {
            continue; // overlap with an already-emitted span; skip
        }
        let Some(before) = line.get(pos..start) else {
            continue;
        };
        out.push_str(before);
        let Some(span_text) = line.get(start..end) else {
            out.push_str(line.get(start..).unwrap_or(""));
            pos = line.len();
            break;
        };
        out.push_str(color);
        out.push_str(span_text);
        out.push_str(RESET);
        pos = end;
    }
    out.push_str(line.get(pos..).unwrap_or(""));
    out
}

/// Byte ranges covered by `--` line comments or `/* */` block comments
/// — the tokenizer treats these as trivia and never emits tokens for
/// them, so they're found with a small dedicated scan instead. Doesn't
/// try to respect string literals (a `--` inside a string would be
/// mis-highlighted); acceptable for a REPL's live-typing display.
fn comment_byte_ranges(line: &str) -> Vec<(usize, usize)> {
    let bytes: Vec<u8> = line.bytes().collect();
    let len = bytes.len();
    let mut ranges = Vec::new();
    let mut i = 0usize;
    let mut in_string: Option<u8> = None;
    while let Some(&b) = bytes.get(i) {
        match in_string {
            Some(quote) => {
                if b == quote {
                    in_string = None;
                }
                i = i.saturating_add(1);
            }
            None => match b {
                b'\'' | b'"' => {
                    in_string = Some(b);
                    i = i.saturating_add(1);
                }
                b'-' if bytes.get(i.saturating_add(1)) == Some(&b'-') => {
                    ranges.push((i, len));
                    break;
                }
                b'/' if bytes.get(i.saturating_add(1)) == Some(&b'*') => {
                    let search_from = i.saturating_add(2);
                    let end = line
                        .get(search_from..)
                        .and_then(|rest| rest.find("*/"))
                        .map(|p| search_from.saturating_add(p).saturating_add(2))
                        .unwrap_or(len);
                    ranges.push((i, end));
                    i = end;
                }
                _ => i = i.saturating_add(1),
            },
        }
    }
    ranges
}

/// [`db_cli::Highlighter`] wrapper over [`highlight`].
pub struct SqlHighlighter;

impl db_cli::Highlighter for SqlHighlighter {
    fn highlight(&self, line: &str) -> String {
        highlight(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip_ansi(s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                while let Some(&c) = chars.peek() {
                    chars.next();
                    if c == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    #[test]
    fn highlight_preserves_visible_text() {
        let line = "SELECT * FROM t WHERE x = 'a';";
        assert_eq!(strip_ansi(&highlight(line)), line);
    }

    #[test]
    fn dot_command_is_colored_as_one_span() {
        let out = highlight(".tables");
        assert!(out.starts_with(YELLOW));
        assert_eq!(strip_ansi(&out), ".tables");
    }

    #[test]
    fn comment_detected_and_preserves_text() {
        let line = "SELECT 1; -- trailing";
        assert_eq!(strip_ansi(&highlight(line)), line);
        assert!(highlight(line).contains(GRAY));
    }

    #[test]
    fn keyword_is_colored() {
        assert!(highlight("SELECT").contains(BOLD_BLUE));
    }

    #[test]
    fn block_comment_detected_and_preserves_text() {
        let line = "SELECT /* c */ 1;";
        assert_eq!(strip_ansi(&highlight(line)), line);
        assert!(highlight(line).contains(GRAY));
    }
}
