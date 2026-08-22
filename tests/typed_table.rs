//! Slice 5a-2 Task 1 — the raw `Span → Ty` accessor. Proves it feeds the same
//! zonked types `infer_with_types` renders, but structured (not stringified), and
//! that a polymorphic node's `Ty::Var` rides through unmonomorphized.

use std::collections::BTreeSet;

use elya::parse::parse_module;
use elya::span::Span;
use elya::types::{infer_typed_table, infer_with_types, Ty, TyCon};
use elya::Session;

/// A rendered type is a bare type variable iff it is `[a-z][0-9]*`.
fn is_var(s: &str) -> bool {
    let mut cs = s.chars();
    matches!(cs.next(), Some(c) if c.is_ascii_lowercase()) && cs.all(|c| c.is_ascii_digit())
}

#[test]
fn raw_table_shares_spans_with_rendered_and_carries_structured_types() {
    let src = "fn add1(n) { n + 1 }\n";
    let (m, pd) = parse_module(&Session::new(), src);
    assert!(pd.is_empty(), "parse: {pd:?}");

    let (d_raw, raw) = infer_typed_table(&Session::new(), &m);
    let (d_str, rendered) = infer_with_types(&Session::new(), &m);
    assert!(d_raw.is_empty(), "raw diags: {d_raw:?}");
    assert!(d_str.is_empty(), "str diags: {d_str:?}");

    // Both accessors type exactly the same set of spans.
    let keys_raw: BTreeSet<Span> = raw.keys().copied().collect();
    let keys_str: BTreeSet<Span> = rendered.keys().copied().collect();
    assert_eq!(keys_raw, keys_str, "raw/rendered span sets differ");

    // The raw table carries structured `Ty` (not strings): a monomorphic program
    // has at least one Int node, and its span renders "Int" in the string table.
    let mut saw_int = false;
    for (span, t) in &raw {
        if matches!(t, Ty::Base(TyCon::Int)) {
            saw_int = true;
            assert_eq!(rendered.get(span).map(String::as_str), Some("Int"));
        }
    }
    assert!(saw_int, "expected an Int-typed node: {raw:?}");
}

#[test]
fn raw_table_preserves_the_polymorphic_var() {
    // The load-bearing case: a polymorphic body zonks to a bare `Ty::Var`; the raw
    // table must carry it as a variable (not a ground type), matching the rendered
    // table's bare-letter entry (stay-polymorphic, 5a-1 §4).
    let src = "fn id(x) { x }\n";
    let (m, pd) = parse_module(&Session::new(), src);
    assert!(pd.is_empty(), "parse: {pd:?}");

    let (_d, raw) = infer_typed_table(&Session::new(), &m);
    let (_d2, rendered) = infer_with_types(&Session::new(), &m);

    assert!(
        raw.values().any(|t| matches!(t, Ty::Var(_))),
        "polymorphic id should carry a Ty::Var node: {raw:?}"
    );
    for (span, t) in &raw {
        if matches!(t, Ty::Var(_)) {
            let r = rendered.get(span).expect("var span rendered");
            assert!(is_var(r), "raw Var did not render as a bare var: {r}");
        }
    }
}
