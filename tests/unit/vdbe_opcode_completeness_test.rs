// Copyright 2026 Schuberg Philis
// SPDX-License-Identifier: Apache-2.0
//! Spec 009's opcode inventory vs. the harvested scope (#65, follow-up to
//! #58/#87/#139). `tools/opcodes-v2.json` is the oracle-harvested set that
//! pinned `Opcode`'s variants in the first place; this test is the
//! machine-checked guarantee that the two never drift apart silently.
//!
//! Since #56 the harvest corpus includes GROUP BY, so the inventory records
//! what the pinned oracle actually emits rather than a corpus curated to
//! avoid unimplemented opcodes. The opcodes we have not implemented yet are
//! named in [`KNOWN_UNIMPLEMENTED`] -- an exact, shrinking list, not a
//! tolerance: implementing one, or the oracle emitting a new one, both fail
//! this test until the list is updated.

use std::collections::BTreeSet;

use sqlite_rs::vdbe::Opcode;

/// Opcodes the pinned oracle emits for the harvest corpus that db-core's
/// `vm::row` does not implement yet (sqlite-rs#56). sqlite3 compiles a
/// GROUP BY's group-break as a three-way `Jump` on a `Compare`, where we
/// emit an `Eq`/`IsNull`/`NotNull`/`Goto` chain -- same semantics,
/// different machine. Parity means this list reaches empty.
const KNOWN_UNIMPLEMENTED: &[&str] = &["Compare", "If", "Jump", "Move"];

#[test]
fn opcode_inventory_matches_harvested_set() {
    let json = include_str!("../../tools/opcodes-v2.json");
    let harvested: BTreeSet<&str> = json
        .lines()
        .filter_map(|line| {
            // Top-level opcode entries are exactly 4-space-indented `"Name": {`
            // keys inside the "opcodes" object; nested fields (count,
            // category, ...) sit at 6+ spaces, so indentation disambiguates.
            let rest = line.strip_prefix("    \"")?;
            if line.starts_with("      ") || !line.trim_end().ends_with('{') {
                return None;
            }
            rest.split('"').next()
        })
        .filter(|name| !name.is_empty())
        .collect();

    let enum_names: BTreeSet<String> = Opcode::ALL.iter().map(|o| format!("{o:?}")).collect();
    let enum_names: BTreeSet<&str> = enum_names.iter().map(String::as_str).collect();

    let gap: BTreeSet<&str> = KNOWN_UNIMPLEMENTED.iter().copied().collect();

    // The gap list must name only opcodes the oracle actually harvested --
    // a stale entry here would silently weaken the check below.
    let stale: BTreeSet<&&str> = gap.iter().filter(|o| !harvested.contains(*o)).collect();
    assert!(
        stale.is_empty(),
        "KNOWN_UNIMPLEMENTED names opcodes the harvest does not contain: {stale:?} \
         -- drop them from the list (sqlite-rs#56)"
    );

    let expected: BTreeSet<&str> = harvested.difference(&gap).copied().collect();

    assert_eq!(
        enum_names, expected,
        "Opcode::ALL must list exactly tools/opcodes-v2.json's harvested set minus \
         KNOWN_UNIMPLEMENTED. An opcode newly implemented must be removed from that \
         list; a newly harvested one must be implemented or added to it (sqlite-rs#56)"
    );
}
