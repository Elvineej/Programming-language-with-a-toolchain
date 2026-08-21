//! Slice 5a-1 Task 3 — the bug-surface corpus (spec §5). Each surface is pinned
//! by a snapshot (human-auditable) AND targeted assertions (machine teeth) that
//! fail loudly on lingering vars, wrong generalization, or unresolved rows.

use std::collections::BTreeMap;

use elya::parse::parse_module;
use elya::span::Span;
use elya::types::{infer_with_sites, infer_with_types};
use elya::Session;

fn table_of(src: &str) -> BTreeMap<Span, String> {
    let (m, pd) = parse_module(&Session::new(), src);
    assert!(pd.is_empty(), "parse: {pd:?}");
    let (diags, table) = infer_with_types(&Session::new(), &m);
    assert!(diags.is_empty(), "unexpected type errors: {diags:?}");
    table
}

fn render(src: &str, table: &BTreeMap<Span, String>) -> String {
    let mut out = String::new();
    for (span, ty) in table {
        let slice = &src[span.start as usize..span.end as usize];
        out.push_str(&format!(
            "{}..{} `{}` : {}\n",
            span.start, span.end, slice, ty
        ));
    }
    // Invariant: no internal inference token leaks into a materialized type.
    for bad in ["%r", "%e", "%s", "%t", "%v", "%row"] {
        assert!(!out.contains(bad), "leaked internal token {bad}:\n{out}");
    }
    out
}

/// A rendered type is a bare type variable iff it is `[a-z][0-9]*` (base types
/// are capitalized, `fn(...)`/`Con(...)` are not single-letter).
fn is_var(s: &str) -> bool {
    let mut cs = s.chars();
    matches!(cs.next(), Some(c) if c.is_ascii_lowercase()) && cs.all(|c| c.is_ascii_digit())
}

// --- Surface 1: monomorphic fn — every node concrete, no variable ------------
#[test]
fn surface1_monomorphic_fn() {
    let src = "fn add1(n) { n + 1 }\n";
    let t = table_of(src);
    assert!(
        t.values().all(|v| !is_var(v)),
        "monomorphic program has a var-typed node: {t:?}"
    );
    assert!(
        t.values().any(|v| v == "Int"),
        "expected an Int node: {t:?}"
    );
    insta::assert_snapshot!(render(src, &t));
}

// --- Surface 2: polymorphic fn — the body node is a variable ------------------
#[test]
fn surface2_polymorphic_fn_has_var_node() {
    let src = "fn id(x) { x }\n";
    let t = table_of(src);
    assert!(
        t.values().any(|v| is_var(v)),
        "polymorphic id should have a var-typed node: {t:?}"
    );
    insta::assert_snapshot!(render(src, &t));
}

// --- Surface 3: use-site instantiation — two distinct concrete instances ------
#[test]
fn surface3_use_site_instantiation() {
    let src = "fn id(x) { x }\nfn u() { let a = id(5)  let b = id(True)  0 }\n";
    let t = table_of(src);
    assert!(
        t.values().any(|v| v == "fn(Int) -> Int"),
        "missing Int instance: {t:?}"
    );
    assert!(
        t.values().any(|v| v == "fn(Bool) -> Bool"),
        "missing Bool instance: {t:?}"
    );
    insta::assert_snapshot!(render(src, &t));
}

// --- Surface 4: effectful arrow — row materialized WITH args (hard req) -------
#[test]
fn surface4_row_materialization() {
    // `worker` performs State(Int); referenced as a value, its arrow carries the
    // resolved row {State(Int)} onto a node. Exercises resolve_row's arg path.
    let src = "effect State(s) { fn get() -> s  fn set(v: s) -> Unit }\n\
               fn worker() { set(1) }\n\
               fn use_it() { let g = worker  g() }\n";
    let t = table_of(src);
    assert!(
        t.values().any(|v| v.contains("State(Int)")),
        "row not materialized with its Int arg (resolve_row leak?): {t:?}"
    );
    insta::assert_snapshot!(render(src, &t));
}

// --- Surface 5: let-generalized lambda (value restriction) --------------------
#[test]
fn surface5_value_restriction_lambda() {
    let src = "fn demo() { let f = fn(x) { x }  let a = f(1)  let b = f(True)  0 }\n";
    let t = table_of(src);
    assert!(
        t.values().any(|v| v == "fn(Int) -> Int"),
        "f not instantiated at Int: {t:?}"
    );
    assert!(
        t.values().any(|v| v == "fn(Bool) -> Bool"),
        "f not instantiated at Bool: {t:?}"
    );
    assert!(
        t.values().any(|v| is_var(v)),
        "the lambda body should be a var: {t:?}"
    );
    insta::assert_snapshot!(render(src, &t));
}

// --- Surface 6: match --------------------------------------------------------
#[test]
fn surface6_match() {
    let src = "type Option(a) { None, Some(a) }\n\
               fn m(o) { match o { None -> 0  Some(x) -> x } }\n";
    let t = table_of(src);
    assert!(
        t.values().any(|v| v == "Int"),
        "match result should be Int: {t:?}"
    );
    insta::assert_snapshot!(render(src, &t));
}

// --- Surface 7: linear binding — table agrees with affine_sites ---------------
#[test]
fn surface7_linear_binding_agrees_with_affine_sites() {
    let src = "linear type Tok { Tok }\nfn lin() { let t = Tok\n t }\n";
    let (m, _) = parse_module(&Session::new(), src);
    let (_d, sites) = infer_with_sites(&Session::new(), &m);
    let t = table_of(src);
    assert!(!sites.is_empty(), "expected an affine site");
    for s in &sites {
        assert_eq!(
            t.get(s).map(String::as_str),
            Some("Tok"),
            "affine site {}..{} not typed Tok in the table: {t:?}",
            s.start,
            s.end
        );
    }
    insta::assert_snapshot!(render(src, &t));
}

// --- Invariant A: record-then-zonk ordering ----------------------------------
#[test]
fn record_then_zonk_ordering() {
    // `y` is a fresh var when first recorded, then forced to Int by `y + 1`. If
    // the table were zonked at record-time, early nodes would be vars.
    let src = "fn ord(x) { let y = x  y + 1 }\n";
    let t = table_of(src);
    assert!(
        t.values().all(|v| !is_var(v)),
        "record-then-zonk violated (a node stayed a var): {t:?}"
    );
    assert!(t.values().any(|v| v == "Int"), "expected Int nodes: {t:?}");
}

// --- Invariant B: cross-node variable coherence (shared Names) ----------------
#[test]
fn cross_node_variable_coherence() {
    // Two DISTINCT params → two distinct vars in separate nodes. Under a shared
    // Names they render as two distinct letters (a, b); fresh-per-node Names would
    // wrongly render both as `a`.
    let src = "fn two(x, y) { let p = x  let q = y  0 }\n";
    let t = table_of(src);
    let vars: std::collections::HashSet<&String> = t.values().filter(|v| is_var(v)).collect();
    assert!(
        vars.len() >= 2,
        "shared Names should give >=2 distinct var letters: {t:?}"
    );
}
