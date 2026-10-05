//! Slice 5a-2 Task 3 — the typed-Core snapshot corpus (spec §5), the deliverable.
//! Reuses the 5a-1 programs (tests/typed_inference.rs), lowers each to Core, and
//! pins the result with an insta snapshot PLUS per-surface teeth and two origin
//! proofs: the lookup cross-check (direct nodes) and the derivation check
//! (synthesized Let nodes). The snapshot IS the proof of lowering-preserves-types.

use std::collections::{BTreeMap, HashSet};
use std::rc::Rc;

use elya::core::{lower_module, pretty_typed, CoreExpr, CoreHandle, CoreKind, CoreModule};
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
            CoreKind::Builtin(_, args) => {
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
            CoreKind::Handle(h) => {
                walk(&h.body, out);
                for c in h.clauses.iter() {
                    walk(&c.body, out);
                }
                if let Some(r) = &h.ret {
                    walk(&r.body, out);
                }
            }
            CoreKind::Resume(v) => walk(v, out),
            CoreKind::Perform(p) => {
                for a in p.args.iter() {
                    walk(a, out);
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

#[test]
fn io_println_is_a_distinct_typed_builtin_with_an_io_callee() {
    let src = "pub fn main() { io.println(\"hello\") 42 }\n";
    let (core, table) = lower_src(src);
    let builtin = nodes(&core)
        .into_iter()
        .find(|n| matches!(&n.kind, CoreKind::Builtin(name, _) if name == "io.println"))
        .expect("io.println must lower to a distinct Builtin node");
    let CoreKind::Builtin(name, args) = &builtin.kind else {
        unreachable!("just matched")
    };
    assert_eq!(name, "io.println");
    assert_eq!(args.len(), 1);
    assert!(matches!(builtin.ty, Ty::Base(TyCon::Unit)));
    assert!(
        !nodes(&core)
            .iter()
            .any(|n| matches!(n.kind, CoreKind::App(..))),
        "the builtin call must not travel through App"
    );
    both_origin_checks(&core, &table);

    let io_callees: Vec<&Ty> = table
        .values()
        .filter(|ty| match ty {
            Ty::Fn(params, row, result) => {
                params == &vec![Ty::str()]
                    && row.labels.len() == 1
                    && row.labels.contains_key("IO")
                    && matches!(&row.tail, elya::types::RowTail::Closed)
                    && **result == Ty::unit()
            }
            _ => false,
        })
        .collect();
    assert_eq!(
        io_callees.len(),
        1,
        "exactly the callee span records fn(String) / {{IO}} -> Unit"
    );

    let rendered = pretty_typed(&core, &mut TyPrinter::new());
    assert!(rendered.contains("(builtin io.println"), "{rendered}");
}

#[test]
fn a_user_declared_io_effect_is_refused_at_lowering() {
    // `IO` is the one effect label codegen treats as built in (Task 7a reads the
    // partition off `label != "IO"`). A user-declared `IO` makes that read
    // ambiguous, so lowering — the last pass that still sees `Decl::Effect` —
    // refuses it by name.
    let src = "effect IO { fn write(s: String) -> Unit }\n\
               pub fn main() -> Int { 0 }\n";
    let (m, _pd) = parse_module(&Session::new(), src);
    let (_diags, table) = infer_typed_table(&Session::new(), &m);
    // `CoreModule` does not derive `PartialEq` (only `LowerError` does), so
    // `assert_eq!` on the bare `Result` does not typecheck; compare the
    // unwrapped error instead — this still exercises `LowerError`'s derive.
    assert_eq!(
        lower_module(&m, &table).unwrap_err(),
        elya::core::LowerError::Unsupported("effect IO"),
    );
}

#[test]
fn an_effect_not_named_io_still_lowers() {
    // Negative control: the refusal is keyed on the NAME, not on effect
    // declarations in general. Without this, an implementation that rejected
    // every `Decl::Effect` would pass the test above.
    let src = "effect State { fn get() -> Int  fn set(v: Int) -> Unit }\n\
               pub fn main() -> Int { 0 }\n";
    let (m, _pd) = parse_module(&Session::new(), src);
    let (_diags, table) = infer_typed_table(&Session::new(), &m);
    assert!(lower_module(&m, &table).is_ok());
}

/// `Ty::Error` at any depth: a function's params, result, or effect-label
/// arguments, a tuple's elements, or a constructor's arguments.
fn has_error(t: &Ty) -> bool {
    match t {
        Ty::Error => true,
        Ty::Var(_) | Ty::Base(_) => false,
        Ty::Fn(ps, row, r) => {
            ps.iter().any(has_error)
                || row.labels.values().any(|l| l.args.iter().any(has_error))
                || has_error(r)
        }
        Ty::Tuple(ts) | Ty::Con(_, ts) => ts.iter().any(has_error),
    }
}

#[test]
fn a_block_in_expression_position_lowers_to_nested_lets() {
    // §3.1(b): `handle { ... }` parses its body through `block_expr()`, so Task 5
    // needs blocks lowerable in expression position. A `let` initializer is the
    // smallest program that reaches the same arm today.
    //
    // Two assertions deviate from the 5b-8 plan's text, on purpose: (a) is
    // widened from literal nodes to every node, at any depth; (c) exempts the
    // synthesized `Let` nodes from span membership (they carry their
    // statement's span, which the typed table need not hold — the same
    // exemption `both_origin_checks` makes) and verifies each by derivation
    // instead.
    let src = "pub fn main() -> Int {\n\
               \x20 let x = { let a = 1  a + 2 }\n\
               \x20 x\n\
               }\n";
    let (core, table) = lower_src(src);

    // (a) No node carries `Ty::Error`, anywhere in its type.
    for n in nodes(&core) {
        assert!(
            !has_error(&n.ty),
            "node at {:?} carries Ty::Error: {:?}",
            n.span,
            n.ty
        );
    }

    // (b) The inner block became a `Let` — not a new node kind.
    let lets = nodes(&core)
        .iter()
        .filter(|n| matches!(n.kind, CoreKind::Let(ref name, _, _) if name == "a"))
        .count();
    assert_eq!(
        lets, 1,
        "the block's `let a` should survive as a CoreKind::Let"
    );

    // (c) The frozen table gained nothing. Every direct node's span is a span
    //     the inference table already knows; every synthesized `Let` is
    //     exempt from that and is checked by derivation (its type is its
    //     body's) instead — so the exemption is verified, not a free pass.
    let mut derived = 0;
    for n in nodes(&core) {
        match &n.kind {
            CoreKind::Let(_, _, body) => {
                assert_eq!(
                    n.ty, body.ty,
                    "synthesized Let at {:?}: its type must be its body's (derivation check)",
                    n.span
                );
                derived += 1;
            }
            _ => assert!(
                table.contains_key(&n.span),
                "node at {:?} has a span absent from the typed table",
                n.span
            ),
        }
    }
    // `let x` (main's body) and `let a` (the block): the derivation check must
    // have seen both, or the exemption covered nothing and proved nothing.
    assert_eq!(derived, 2, "expected exactly two synthesized Lets");
}

/// The first `Handle` in `nodes`' pre-order. Panics if lowering produced none.
fn first_handle(core: &CoreModule) -> Rc<CoreHandle> {
    nodes(core)
        .into_iter()
        .find_map(|n| match &n.kind {
            CoreKind::Handle(h) => Some(h.clone()),
            _ => None,
        })
        .expect("the handle should have lowered to a CoreKind::Handle")
}

/// One handled op, qualified: `Log.log(m) -> resume(m)` with a `return` clause.
/// Also the bare-resume program for the A-normalization test below.
const LOG_HANDLE: &str = "effect Log { fn log(msg: String) -> String }\n\
                          pub fn main() {\n\
                          \x20 let r = handle {\n\
                          \x20   log(\"hi\")\n\
                          \x20 } with {\n\
                          \x20   Log.log(m) -> resume(m)\n\
                          \x20   return(x) -> x\n\
                          \x20 }\n\
                          \x20 io.println(r)\n\
                          }\n";

#[test]
fn a_handle_lowers_to_core_handle() {
    let (core, table) = lower_src(LOG_HANDLE);
    let h = first_handle(&core);

    assert_eq!(h.clauses.len(), 1);
    assert_eq!(h.clauses[0].effect, "Log");
    assert_eq!(h.clauses[0].op, "log");
    assert_eq!(h.clauses[0].params.len(), 1);
    assert_eq!(h.clauses[0].params[0].name, "m");
    // The clause parameter's type is READ from the frozen table at the
    // parameter's own span, as a lambda parameter's is — never guessed.
    assert_eq!(h.clauses[0].params[0].ty, Ty::Base(TyCon::Str));

    // `Log` is not `multi`-declared, so the stamp is clear (§5.3 / D4).
    assert!(!h.is_multi_declared, "a one-shot effect is not multi");

    // `return(x) -> x` is present, and its body is a plain Var — not a lambda.
    let r = h.ret.as_ref().expect("return clause");
    assert_eq!(r.binder, "x");
    assert!(matches!(r.body.kind, CoreKind::Var(ref v) if v == "x"));

    // The clause body is `resume(m)` — a Resume node, not an App of a `resume` Var.
    assert!(matches!(h.clauses[0].body.kind, CoreKind::Resume(_)));

    // The handler subtree is held to both origin proofs like every other node,
    // and the printer renders it without leaking an internal token.
    both_origin_checks(&core, &table);
    let rendered = pretty_typed(&core, &mut TyPrinter::new());
    assert!(rendered.contains("(handle "), "{rendered}");
    assert!(rendered.contains("(resume "), "{rendered}");
    for bad in ["%r", "%e", "%s", "%t", "%v", "%row"] {
        assert!(
            !rendered.contains(bad),
            "leaked internal token {bad} in:\n{rendered}"
        );
    }
}

#[test]
fn an_applied_resume_is_a_normalized_but_a_bare_one_is_not() {
    // `resume(v)(s)` becomes `let $k = resume(v) in $k(s)`, so Task 7b sees the
    // resume in a named position. `$k` is SYNTHESIZED, not recorded (D7): the
    // Let carries the resume's span, and the `$k` Var's type is cloned from the
    // resume node rather than looked up.
    //
    // The plan's program had no `return` clause and is a type error (E0400): R
    // is then the body's `Int`, and `resume(s)(s)` applies an Int. A `return`
    // clause makes R a function, and main applies the result.
    let src = "effect St { fn get() -> Int }\n\
               pub fn main() -> Int {\n\
               \x20 let f = handle { get() } with {\n\
               \x20   St.get() -> fn(s) { resume(s)(s) }\n\
               \x20   return(x) -> fn(s) { x }\n\
               \x20 }\n\
               \x20 f(0)\n\
               }\n";
    let (core, table) = lower_src(src);

    let lets: Vec<_> = nodes(&core)
        .into_iter()
        .filter_map(|n| match &n.kind {
            CoreKind::Let(name, v, _) if name == "$k" => Some((n, v.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(lets.len(), 1, "exactly one $k binding");
    assert!(
        matches!(lets[0].1.kind, CoreKind::Resume(_)),
        "$k binds the resume"
    );
    assert_eq!(lets[0].0.span, lets[0].1.span);
    assert!(table.contains_key(&lets[0].0.span));

    // Synthesized is not exempt: the `$k` Let passes the derivation check and
    // the `$k` Var the lookup cross-check, like every other node.
    both_origin_checks(&core, &table);

    // The other half of this test's name: a BARE `resume(m)` is left alone.
    let (bare, _) = lower_src(LOG_HANDLE);
    assert!(
        !nodes(&bare)
            .iter()
            .any(|n| matches!(n.kind, CoreKind::Let(ref name, _, _) if name == "$k")),
        "a bare resume must not be A-normalized"
    );
}

/// Spec §5.2-§5.3 and plan D4: `is_multi_declared` comes from the EFFECT'S
/// DECLARATION, op-keyed through `multi_declared_ops` — not from the handler's
/// own `multi` flag. Lowers `effect multi Flip` handled by `with_kw` and returns
/// the stamp. One `#[test]` per keyword, so each case fails on its own.
fn flip_handled_with_is_stamped_multi(with_kw: &str) -> bool {
    let src = format!(
        "effect multi Flip {{ fn flip() -> Bool }}\n\
         fn g() -> Int {{ if flip() {{ 1 }} else {{ 0 }} }}\n\
         pub fn main() -> Int {{\n\
         \x20 handle {{ g() }} {with_kw} {{\n\
         \x20   Flip.flip() -> resume(True)\n\
         \x20   return(x) -> x\n\
         \x20 }}\n\
         }}\n"
    );
    let (core, _table) = lower_src(&src);
    first_handle(&core).is_multi_declared
}

#[test]
fn a_plain_with_over_a_multi_declared_effect_is_stamped_multi() {
    // §5.2's deliberate over-refusal: the handler is plain, the DECLARATION is
    // `multi`, and the declaration decides. This is the case that tells
    // declaration-keying apart from `Handler::multi`-keying.
    assert!(
        flip_handled_with_is_stamped_multi("with"),
        "plain `with` over a `multi`-declared effect must be stamped multi"
    );
}

#[test]
fn a_with_multi_over_a_multi_declared_effect_is_stamped_multi() {
    // Both keyings agree here (the handler flag and the declaration are both
    // multi), so this case pins the stamp itself, not which source it is read
    // from.
    assert!(
        flip_handled_with_is_stamped_multi("with multi"),
        "`with multi` over a `multi`-declared effect must be stamped multi"
    );
}

#[test]
fn an_unqualified_handler_clause_lowers_with_its_ops_effect() {
    // Slice 5c-1 (replaces 5b-8's `an_unqualified_handler_clause_is_refused_at_
    // lowering`, which pinned the refusal this slice lifts). Op names are unique
    // per module (E0202), so `ask() -> ..` means `Ask.ask() -> ..`, and the
    // evaluator now dispatches to it. Core clauses always carry the effect.
    let unqualified = "effect Ask { fn ask() -> Int }\n\
                       fn one() { ask() }\n\
                       pub fn main() -> Int {\n\
                       \x20 handle { one() } with {\n\
                       \x20   ask() -> resume(2)\n\
                       \x20   return(x) -> x\n\
                       \x20 }\n\
                       }\n";
    let clause_names = |src: &str| -> Vec<(String, String)> {
        let (core, _) = lower_src(src);
        nodes(&core)
            .into_iter()
            .filter_map(|e| match &e.kind {
                CoreKind::Handle(h) => Some(
                    h.clauses
                        .iter()
                        .map(|c| (c.effect.clone(), c.op.clone()))
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            })
            .flatten()
            .collect()
    };
    let want = vec![("Ask".to_string(), "ask".to_string())];
    assert_eq!(clause_names(unqualified), want);
    // Control: the qualified spelling lowers to the same clause.
    assert_eq!(
        clause_names(&unqualified.replace("ask() -> resume", "Ask.ask() -> resume")),
        want
    );
}

#[test]
fn a_perform_lowers_to_a_perform_node() {
    // D11: a call whose callee names an operation is a PERFORM, and Core says
    // so -- effect and op by name, the arguments lowered -- instead of
    // passing it off as an application of a `Var`.
    let (core, table) = lower_src(LOG_HANDLE);
    let (node, p) = nodes(&core)
        .into_iter()
        .find_map(|n| match &n.kind {
            CoreKind::Perform(p) => Some((n, p.clone())),
            _ => None,
        })
        .expect("the perform should have lowered to a CoreKind::Perform");
    assert_eq!(p.effect, "Log");
    assert_eq!(p.op, "log");
    assert_eq!(p.args.len(), 1);
    assert_eq!(
        node.ty,
        Ty::Base(TyCon::Str),
        "a perform's type is the op's result"
    );
    both_origin_checks(&core, &table);
}

#[test]
fn a_local_named_like_an_op_lowers_to_a_call_not_a_perform() {
    // Slice 5c-2 (replaces D11's `an_op_wins_over_a_same_named_fn_...`, whose
    // program -- a top-level `fn ping` beside op `ping` -- is now E0205). A LOCAL
    // shadows the op lexically, as inference and the evaluator resolve it, so
    // `ping()` here is an application of the local, and the module has no
    // Perform at all.
    let src = "effect E { fn ping() -> Int }\n\
               fn user() -> Int {\n\
               \x20 let ping = fn() { 5 }\n\
               \x20 ping()\n\
               }\n\
               pub fn main() -> Int {\n\
               \x20 handle { user() } with {\n\
               \x20   E.ping() -> resume(1)\n\
               \x20   return(x) -> x\n\
               \x20 }\n\
               }\n";
    let performs = |src: &str| {
        let (core, _) = lower_src(src);
        nodes(&core)
            .into_iter()
            .filter(|e| matches!(&e.kind, CoreKind::Perform(_)))
            .count()
    };
    assert_eq!(performs(src), 0, "the shadowed `ping()` must not perform");
    // Control: outside the local's scope the same call performs.
    let unshadowed = src.replace("  let ping = fn() { 5 }\n  ping()", "  ping()");
    assert_ne!(unshadowed, src, "control rewrite must apply");
    assert_eq!(performs(&unshadowed), 1);
}

#[test]
fn a_perform_carries_the_effect_instantiation() {
    // D11 + Shape C: the perform keeps the op's type as inference instantiated
    // it, so the effect's argument (`State(Int)`) is still on the node -- the
    // names `State`/`set` alone would lose it. Step 11a (a polymorphic effect
    // performed at a concrete type) reads it.
    let (core, _t) = lower_src(
        "effect State(s) { fn get() -> s  fn set(v: s) -> Unit }\n\
         fn worker() { set(1) }\n",
    );
    let worker = core
        .fns
        .iter()
        .find(|f| f.name == "worker")
        .expect("worker");
    let CoreKind::Perform(p) = &worker.body.kind else {
        panic!("worker's body should be a Perform: {:?}", worker.body.kind);
    };
    assert_eq!(
        TyPrinter::new().render(&p.op_ty),
        "fn(Int) / {State(Int)} -> Unit",
        "the perform must carry the op's instantiated type"
    );
}
