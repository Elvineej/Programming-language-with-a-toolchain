//! Slice 5a-2 Task 3 — the typed-Core snapshot corpus (spec §5), the deliverable.
//! Reuses the 5a-1 programs (tests/typed_inference.rs), lowers each to Core, and
//! pins the result with an insta snapshot PLUS per-surface teeth and two origin
//! proofs: the lookup cross-check (direct nodes) and the derivation check
//! (synthesized Let nodes). The snapshot IS the proof of lowering-preserves-types.

use std::collections::{BTreeMap, HashSet};

use elya::core::{lower_module, pretty_typed, CoreExpr, CoreKind, CoreModule};
use elya::parse::parse_module;
use elya::span::Span;
use elya::types::{infer_typed_table, Ty, TyCon, TyPrinter};
use elya::Session;

/// A rendered type is a bare type variable iff it is `[a-z][0-9]*`.
fn is_var(s: &str) -> bool {
    let mut cs = s.chars();
    matches!(cs.next(), Some(c) if c.is_ascii_lowercase()) && cs.all(|c| c.is_ascii_digit())
}

/// Parse → infer (raw table) → lower. Panics on any parse/type/lowering error.
fn lower_src(src: &str) -> (CoreModule, BTreeMap<Span, Ty>) {
    let (m, pd) = parse_module(&Session::new(), src);
    assert!(pd.is_empty(), "parse: {pd:?}");
    let (diags, table) = infer_typed_table(&Session::new(), &m);
    assert!(diags.is_empty(), "type errors: {diags:?}");
    let core = lower_module(&m, &table).expect("lowering the corpus subset should succeed");
    (core, table)
}

/// Pre-order collection of every `CoreExpr` node in a module.
fn nodes(core: &CoreModule) -> Vec<&CoreExpr> {
    fn walk<'a>(e: &'a CoreExpr, out: &mut Vec<&'a CoreExpr>) {
        out.push(e);
        match &e.kind {
            CoreKind::Lit(_) | CoreKind::Var(_) => {}
            CoreKind::App(f, args) => {
                walk(f, out);
                for a in args.iter() {
                    walk(a, out);
                }
            }
            CoreKind::Prim(_, args) => {
                for a in args.iter() {
                    walk(a, out);
                }
            }
            CoreKind::Lambda(_, body) => walk(body, out),
            CoreKind::Let(_, value, body) => {
                walk(value, out);
                walk(body, out);
            }
            CoreKind::If(cond, then_e, else_e) => {
                walk(cond, out);
                walk(then_e, out);
                walk(else_e, out);
            }
            CoreKind::Ctor(_, fields) => {
                for f in fields.iter() {
                    walk(f, out);
                }
            }
            CoreKind::Match(scrut, arms) => {
                walk(scrut, out);
                for arm in arms.iter() {
                    walk(&arm.body, out);
                }
            }
        }
    }
    let mut out = Vec::new();
    for f in &core.fns {
        walk(&f.body, &mut out);
    }
    out
}

/// Render one type in isolation (ground types need no shared context).
fn render1(t: &Ty) -> String {
    TyPrinter::new().render(t)
}

/// The two origin proofs (spec §5): every direct-origin node's inline type equals
/// `node_types[span]` (lookup cross-check); every synthesized `Let` node's type
/// equals its body's (derivation check). Synthesized nodes are verified, not exempted.
fn both_origin_checks(core: &CoreModule, table: &BTreeMap<Span, Ty>) {
    for n in nodes(core) {
        match &n.kind {
            CoreKind::Let(_, _, body) => {
                assert_eq!(
                    n.ty, body.ty,
                    "synthesized Let node type must equal its body's (derivation check)"
                );
            }
            _ => {
                assert_eq!(
                    Some(&n.ty),
                    table.get(&n.span),
                    "direct Core node type must equal node_types[span] (lookup cross-check)"
                );
            }
        }
    }
}

const CORPUS: &[&str] = &[
    "fn add1(n) { n + 1 }\n",
    "fn id(x) { x }\n",
    "fn id(x) { x }\nfn u() { let a = id(5)  let b = id(True)  0 }\n",
    "effect State(s) { fn get() -> s  fn set(v: s) -> Unit }\n\
     fn worker() { set(1) }\n\
     fn use_it() { let g = worker  g() }\n",
    "fn demo() { let f = fn(x) { x }  let a = f(1)  let b = f(True)  0 }\n",
    "type Option(a) { None, Some(a) }\n\
     fn m(o) { match o { None -> 0  Some(x) -> x } }\n",
    "linear type Tok { Tok }\nfn lin() { let t = Tok\n t }\n",
    "fn two(x, y) { let p = x  let q = y  0 }\n",
];

#[test]
fn both_origin_proofs_and_no_token_leak_across_corpus() {
    for src in CORPUS {
        let (core, table) = lower_src(src);
        both_origin_checks(&core, &table);
        let rendered = pretty_typed(&core, &mut TyPrinter::new());
        for bad in ["%r", "%e", "%s", "%t", "%v", "%row"] {
            assert!(
                !rendered.contains(bad),
                "leaked internal token {bad} in:\n{rendered}"
            );
        }
    }
}

#[test]
fn ty_printer_shares_letters_across_calls() {
    let mut p = TyPrinter::new();
    let a1 = p.render(&Ty::Var(7));
    let a2 = p.render(&Ty::Var(7));
    assert_eq!(a1, a2, "shared var must render identically across calls");
    let b = p.render(&Ty::Var(8));
    assert_ne!(a1, b, "distinct vars must render as distinct letters");
}

// --- Surface 1: monomorphic fn — no Core node carries a variable ---------------
#[test]
fn surface1_add1_fully_monomorphic() {
    let (core, _t) = lower_src("fn add1(n) { n + 1 }\n");
    assert!(
        nodes(&core).iter().all(|n| !matches!(n.ty, Ty::Var(_))),
        "monomorphic program has a var-typed Core node"
    );
    insta::assert_snapshot!(pretty_typed(&core, &mut TyPrinter::new()));
}

// --- Surface 2: polymorphic fn — the body node stays a variable ----------------
#[test]
fn surface2_id_body_stays_polymorphic() {
    let (core, _t) = lower_src("fn id(x) { x }\n");
    let body = &core.fns[0].body;
    assert!(
        matches!(body.kind, CoreKind::Var(ref x) if x == "x"),
        "id body should be Var(\"x\"), got {:?}",
        body.kind
    );
    assert!(
        matches!(body.ty, Ty::Var(_)),
        "lowering monomorphized or errored a polymorphic node: {:?}",
        body.ty
    );
    insta::assert_snapshot!(pretty_typed(&core, &mut TyPrinter::new()));
}

// --- Surface 3: use-site instantiation — two distinct concrete instances -------
#[test]
fn surface3_use_site_two_instances() {
    let (core, _t) = lower_src("fn id(x) { x }\nfn u() { let a = id(5)  let b = id(True)  0 }\n");
    let rendered: Vec<String> = nodes(&core).iter().map(|n| render1(&n.ty)).collect();
    assert!(
        rendered.iter().any(|r| r == "fn(Int) -> Int"),
        "missing Int instance callee: {rendered:?}"
    );
    assert!(
        rendered.iter().any(|r| r == "fn(Bool) -> Bool"),
        "missing Bool instance callee: {rendered:?}"
    );
    insta::assert_snapshot!(pretty_typed(&core, &mut TyPrinter::new()));
}

// --- Surface 4: effectful arrow — row rides inline through Ty::Fn's EffectRow ---
#[test]
fn surface4_effect_row_rides_inline() {
    let (core, _t) = lower_src(
        "effect State(s) { fn get() -> s  fn set(v: s) -> Unit }\n\
         fn worker() { set(1) }\n\
         fn use_it() { let g = worker  g() }\n",
    );
    let rendered: Vec<String> = nodes(&core).iter().map(|n| render1(&n.ty)).collect();
    assert!(
        rendered.iter().any(|r| r.contains("State(Int)")),
        "row not materialized inline on any Core node: {rendered:?}"
    );
    insta::assert_snapshot!(pretty_typed(&core, &mut TyPrinter::new()));
}

// --- Surface 5: let-generalized lambda (value restriction) ---------------------
#[test]
fn surface5_lambda_body_var_and_instances() {
    let (core, _t) =
        lower_src("fn demo() { let f = fn(x) { x }  let a = f(1)  let b = f(True)  0 }\n");
    let lambda_body_is_var = nodes(&core)
        .iter()
        .any(|n| matches!(&n.kind, CoreKind::Lambda(_, body) if matches!(body.ty, Ty::Var(_))));
    assert!(lambda_body_is_var, "lambda body should be a Ty::Var node");
    let rendered: Vec<String> = nodes(&core).iter().map(|n| render1(&n.ty)).collect();
    assert!(
        rendered.iter().any(|r| r == "fn(Int) -> Int"),
        "no Int instance: {rendered:?}"
    );
    assert!(
        rendered.iter().any(|r| r == "fn(Bool) -> Bool"),
        "no Bool instance: {rendered:?}"
    );
    insta::assert_snapshot!(pretty_typed(&core, &mut TyPrinter::new()));
}

// --- Surface 6: match — the Match node is Int ----------------------------------
#[test]
fn surface6_match_node_is_int() {
    let (core, _t) = lower_src(
        "type Option(a) { None, Some(a) }\n\
         fn m(o) { match o { None -> 0  Some(x) -> x } }\n",
    );
    let match_ty = nodes(&core)
        .iter()
        .find(|n| matches!(n.kind, CoreKind::Match(..)))
        .map(|n| render1(&n.ty));
    assert_eq!(
        match_ty.as_deref(),
        Some("Int"),
        "match result should be Int"
    );
    insta::assert_snapshot!(pretty_typed(&core, &mut TyPrinter::new()));
}

// --- Surface 7: nullary ctor — the Ctor("Tok", []) node carries Con("Tok", []) ----
#[test]
fn surface7_nullary_ctor_carries_con() {
    let (core, _t) = lower_src("linear type Tok { Tok }\nfn lin() { let t = Tok\n t }\n");
    let tok_ty = nodes(&core)
        .iter()
        .find(|n| matches!(&n.kind, CoreKind::Ctor(x, args) if x == "Tok" && args.is_empty()))
        .map(|n| n.ty.clone());
    assert!(
        matches!(tok_ty, Some(Ty::Con(ref n, ref a)) if n == "Tok" && a.is_empty()),
        "Tok node should carry Con(\"Tok\", []), got {tok_ty:?}"
    );
    insta::assert_snapshot!(pretty_typed(&core, &mut TyPrinter::new()));
}

// --- Invariant: two distinct variables render as two distinct letters ----------
#[test]
fn coherence_two_distinct_vars_render_distinct_letters() {
    let (core, _t) = lower_src("fn two(x, y) { let p = x  let q = y  0 }\n");
    // Render the whole tree through ONE shared printer, then count distinct
    // single-letter var annotations — two distinct params must not collapse to one.
    let mut printer = TyPrinter::new();
    let mut letters = HashSet::new();
    for n in nodes(&core) {
        let r = printer.render(&n.ty);
        if is_var(&r) {
            letters.insert(r);
        }
    }
    assert!(
        letters.len() >= 2,
        "shared TyPrinter should give >=2 distinct var letters: {letters:?}"
    );
}

// --- Surface 8 (5b-2): `if` lowers to a two-branch Core node ------------------
#[test]
fn if_lowers_to_a_two_branch_core_node() {
    // `else_block` is `Rc<Spanned<Block>>`, not an Option (src/ast.rs:180-184),
    // so every surface `if` is already two-branch: Core needs no synthesized
    // Unit else, and the back end's phi always has exactly two incoming values.
    let (core, table) = lower_src("pub fn main() { if 1 < 2 { 10 } else { 20 } }\n");
    let root = &core.fns[0].body;
    let CoreKind::If(cond, then_e, else_e) = &root.kind else {
        panic!(
            "main's body should lower to CoreKind::If, got {:?}",
            root.kind
        );
    };
    assert_eq!(
        render1(&root.ty),
        "Int",
        "the if node takes its branches' type"
    );
    assert_eq!(render1(&cond.ty), "Bool");
    assert_eq!(render1(&then_e.ty), "Int");
    assert_eq!(render1(&else_e.ty), "Int");

    // Every `If` node is direct-origin: its span is the surface `if`'s span, so
    // the frozen table must already hold its type (spec §2.2 — `infer_expr` is
    // the single record point and `Expr::If` runs through it).
    both_origin_checks(&core, &table);
    assert!(
        nodes(&core)
            .iter()
            .any(|n| matches!(n.kind, CoreKind::If(..))),
        "the walk helper must reach into If children"
    );
}

// --- Slice 5b-3 §3.3: CoreFn carries its signature --------------------------

#[test]
fn core_parameters_carry_their_inferred_types() {
    // The back end declares an LLVM function type from these. A parameter that
    // reached Core as a bare name would leave the back end guessing.
    let src = "fn add3(x) { x + 3 }
pub fn main() { add3(4) }
";
    let (core, _table) = lower_src(src);
    let add3 = core
        .fns
        .iter()
        .find(|f| f.name == "add3")
        .expect("add3 lowered");
    assert_eq!(add3.params.len(), 1);
    assert_eq!(add3.params[0].name, "x");
    assert!(
        matches!(add3.params[0].ty, Ty::Base(TyCon::Int)),
        "{:?}",
        add3.params[0].ty
    );
    // The body's root type IS the return type (§3.3) — there is no separate
    // field that could disagree with it.
    assert!(
        matches!(add3.body.ty, Ty::Base(TyCon::Int)),
        "{:?}",
        add3.body.ty
    );
}

/// Slice 5b-6 Task 1 (obligation T3, CR-1 route (a)). A lambda parameter's type
/// is RECORDED at its span by the checker and carried into Core, rather than
/// reconstructed in the back end from call-site context. The lambda here is
/// applied to an `Int` at a monomorphic use, so the recorded type zonks to a
/// concrete `Int` — which is exactly what the back end needs to build the lifted
/// function's signature.
#[test]
fn lambda_parameters_carry_recorded_types() {
    let (core, _table) = lower_src("pub fn main() {\n  let f = fn(x) { x + 1 }\n  f(41)\n}\n");
    let lam = nodes(&core)
        .into_iter()
        .find(|n| matches!(&n.kind, CoreKind::Lambda(..)))
        .expect("the program contains a lambda");
    let CoreKind::Lambda(params, _) = &lam.kind else {
        unreachable!("just matched")
    };
    assert_eq!(params.len(), 1, "one parameter");
    assert_eq!(params[0].name, "x", "the name survives lowering");
    assert_eq!(
        params[0].ty,
        Ty::Base(TyCon::Int),
        "the TYPE survives lowering — this is what T3 was about"
    );
}
