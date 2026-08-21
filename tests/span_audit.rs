//! Slice 5a-1 Task 1 — the hard prerequisite gate (spec §2). The typed table is
//! Shape A: keyed on `Span`. That is only sound if no two distinct expression
//! nodes share a span and none is `Span::EMPTY`. This audit proves it over the
//! examples + synthetic-span-prone constructs, and is retained as a regression
//! guard. If it fails, fall back to Shape B (NodeId) — do NOT work around it.

use std::collections::HashMap;
use std::fs;

use elya::ast::*;
use elya::parse::parse_module;
use elya::span::{Span, Spanned};
use elya::Session;

fn collect_block(b: &Block, out: &mut Vec<Span>) {
    for st in b.stmts.iter() {
        match &st.node {
            Stmt::Let { value, .. } => collect_expr(value, out),
            Stmt::Expr(e) => collect_expr(e, out),
        }
    }
    if let Some(t) = &b.tail {
        collect_expr(t, out);
    }
}

fn collect_expr(e: &Spanned<Expr>, out: &mut Vec<Span>) {
    out.push(e.span);
    // EXHAUSTIVE — no `_` arm. Mirrors the exact node class `infer_expr` visits.
    match &e.node {
        Expr::Int(_)
        | Expr::Float(_)
        | Expr::Str(_)
        | Expr::Bool(_)
        | Expr::Unit
        | Expr::Var(_)
        | Expr::Qualified { .. } => {}
        Expr::Call { callee, args } => {
            collect_expr(callee, out);
            for a in args.iter() {
                collect_expr(a, out);
            }
        }
        Expr::Unary { expr, .. } => collect_expr(expr, out),
        Expr::Binary { lhs, rhs, .. } => {
            collect_expr(lhs, out);
            collect_expr(rhs, out);
        }
        Expr::If {
            cond,
            then_block,
            else_block,
        } => {
            collect_expr(cond, out);
            collect_block(&then_block.node, out);
            collect_block(&else_block.node, out);
        }
        Expr::Block(b) => collect_block(b, out),
        Expr::Handle { body, handler } => {
            collect_expr(body, out);
            for c in &handler.clauses {
                collect_expr(&c.node.body, out);
            }
            if let Some(r) = &handler.ret {
                collect_expr(&r.body, out);
            }
        }
        Expr::Resume { arg } => collect_expr(arg, out),
        Expr::Match { scrutinee, arms } => {
            collect_expr(scrutinee, out);
            for arm in arms.iter() {
                collect_expr(&arm.node.body, out);
            }
        }
        Expr::Lambda { body, .. } => collect_block(&body.node, out),
    }
}

fn audit(name: &str, src: &str) -> Vec<String> {
    let (m, diags) = parse_module(&Session::new(), src);
    assert!(
        diags.is_empty(),
        "{name}: parse errors (fix the program, not the audit): {diags:?}"
    );
    let mut spans = Vec::new();
    for d in &m.decls {
        if let Decl::Fn(f) = &d.node {
            collect_block(&f.body.node, &mut spans);
        }
    }
    let mut problems = Vec::new();
    if spans.contains(&Span::EMPTY) {
        problems.push(format!("{name}: an expression node has Span::EMPTY"));
    }
    let mut seen: HashMap<Span, usize> = HashMap::new();
    for s in &spans {
        *seen.entry(*s).or_insert(0) += 1;
    }
    for (s, c) in seen {
        if c > 1 {
            problems.push(format!(
                "{name}: span {}..{} shared by {c} distinct nodes",
                s.start, s.end
            ));
        }
    }
    problems
}

#[test]
fn expression_spans_are_unique_and_nonempty() {
    let mut problems = Vec::new();
    for path in ["examples/01_hello.elya", "examples/02_arith.elya"] {
        let src = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
        problems.extend(audit(path, &src));
    }
    // Constructs most likely to emit synthetic or duplicated spans.
    let corpus: &[(&str, &str)] = &[
        (
            "bare_ctor_value",
            "type Option(a) { None, Some(a) }\nfn f() { let g = Some\n g(1) }\n",
        ),
        ("lambda", "fn f() { let g = fn(x) { x }\n g(1) }\n"),
        (
            "handler",
            "effect Ask { fn ask() -> Bool }\nfn f() { handle { ask() } with { Ask.ask() -> resume(True) } }\n",
        ),
        (
            "match_nested",
            "type List(a) { Nil, Cons(a, List(a)) }\nfn f(xs) { match xs { Nil -> 0  Cons(_, ys) -> 1 } }\n",
        ),
        ("linear", "linear type Tok { Tok }\nfn f() { let t = Tok\n t }\n"),
        ("nested_calls", "fn f(a) { a }\nfn g() { f(f(f(1))) }\n"),
    ];
    for (name, src) in corpus {
        problems.extend(audit(name, src));
    }
    assert!(
        problems.is_empty(),
        "SPAN AUDIT FAILED — fall back to Shape B (NodeId); do NOT work around:\n{}",
        problems.join("\n")
    );
}
