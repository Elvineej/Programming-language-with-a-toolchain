// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)
//! `float.parse`, `float.to_bits`, `float.of_bits` in the evaluator (spec
//! `docs/superpowers/specs/2026-11-28-elya-float-in-elya-design.md` D1).

#[allow(dead_code)] // each suite uses some of the programs
#[path = "programs/float_bits.rs"]
mod programs;
use programs::*;

#[test]
fn float_parse_reads_the_nearest_double_and_the_bits_go_both_ways() {
    let o = elya::run_source_outcome("main.elya", &bits_program());
    assert!(o.error.is_none(), "{:?}", o.error);
    assert_eq!(o.output, bits_out());
}

#[test]
fn float_parse_refuses_other_texts_by_name() {
    for t in REFUSED {
        let o = elya::run_source_outcome("main.elya", &refused_program(t));
        let e = o.error.unwrap_or_else(|| panic!("{t:?} was read"));
        assert!(
            e.contains(&format!("float.parse: `{t}` is not a number")),
            "{t:?}: {e}"
        );
    }
}
