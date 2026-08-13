//! Exhaustiveness & useless-arm checking for `match` (Maranget's usefulness
//! algorithm — "Warnings for pattern matching"). A pattern vector `q` is
//! *useful* w.r.t. a matrix `P` if some value matches `q` but no row of `P`.
//!
//! - Exhaustiveness: a `match` is exhaustive iff a wildcard vector is *not*
//!   useful against its arm matrix. When it is useful, the recursion reconstructs
//!   an uncovered **witness** pattern (with correct constructor names and nested
//!   structure) → `E0430` (error).
//! - Useless arm: arm *i* is redundant iff it is not useful against arms `0..i`
//!   → `E0431` (warning).
//!
//! No inferred types are needed: a column's constructor signature comes from the
//! `type` declarations (for data constructors) or the literal patterns themselves
//! (`Bool` finite, `Int`/`String` infinite, `Unit` single).

use crate::ast::*;
use crate::diag::Diagnostic;
use crate::span::Spanned;
use std::collections::HashMap;

/// Constructor name -> the full `(name, arity)` sibling set of its type.
type Siblings = HashMap<String, Vec<(String, usize)>>;

/// A head constructor. Data constructors carry a name; literals are nullary
/// "constructors" of their base type.
#[derive(Clone, Debug, PartialEq)]
enum Con {
    Data(String),
    Bool(bool),
    Int(i64),
    Str(String),
    Unit,
}

/// An internal pattern: a wildcard, or a constructor applied to sub-patterns.
/// (`ast::Pattern::Var` lowers to `Wild` — a binding is irrelevant to coverage.)
#[derive(Clone, Debug)]
enum Pat {
    Wild,
    Con(Con, Vec<Pat>),
}

fn lower(p: &Pattern) -> Pat {
    match p {
        Pattern::Wild | Pattern::Var(_) => Pat::Wild,
        Pattern::Ctor { name, args } => Pat::Con(
            Con::Data(name.clone()),
            args.iter().map(|a| lower(&a.node)).collect(),
        ),
        Pattern::Lit(l) => {
            let c = match l {
                PatLit::Int(n) => Con::Int(*n),
                PatLit::Bool(b) => Con::Bool(*b),
                PatLit::Str(s) => Con::Str(s.clone()),
                PatLit::Unit => Con::Unit,
            };
            Pat::Con(c, Vec::new())
        }
    }
}

enum Signature {
    /// The complete set of constructors of the column's type, with arities.
    Finite(Vec<(Con, usize)>),
    /// Infinitely many values (`Int`/`String`), or an all-wildcard column.
    Infinite,
}

enum Usefulness {
    Useless,
    Witness(Vec<Pat>),
}

/// The distinct head constructors of a matrix's first column.
fn column_cons(matrix: &[Vec<Pat>]) -> Vec<Con> {
    let mut out: Vec<Con> = Vec::new();
    for row in matrix {
        if let Some(Pat::Con(c, _)) = row.first() {
            if !out.contains(c) {
                out.push(c.clone());
            }
        }
    }
    out
}

/// The constructor signature of a column, from its head constructors (columns are
/// type-homogeneous after type-checking, so the first head determines the type).
fn signature(heads: &[Con], sib: &Siblings) -> Signature {
    match heads.first() {
        None => Signature::Infinite, // all-wildcard column → wildcard witness
        Some(Con::Data(name)) => {
            let siblings = sib.get(name).cloned().unwrap_or_default();
            Signature::Finite(
                siblings
                    .into_iter()
                    .map(|(n, a)| (Con::Data(n), a))
                    .collect(),
            )
        }
        Some(Con::Bool(_)) => Signature::Finite(vec![(Con::Bool(true), 0), (Con::Bool(false), 0)]),
        Some(Con::Unit) => Signature::Finite(vec![(Con::Unit, 0)]),
        Some(Con::Int(_)) | Some(Con::Str(_)) => Signature::Infinite,
    }
}

/// Specialize the matrix by constructor `c` of arity `arity`: keep rows whose
/// head is `c` (expanding its args) or a wildcard (expanding to `arity` wilds).
fn specialize(matrix: &[Vec<Pat>], c: &Con, arity: usize) -> Vec<Vec<Pat>> {
    let mut out = Vec::new();
    for row in matrix {
        match row.first() {
            Some(Pat::Con(rc, rargs)) if rc == c => {
                let mut nr = rargs.clone();
                nr.extend_from_slice(&row[1..]);
                out.push(nr);
            }
            Some(Pat::Wild) => {
                let mut nr = vec![Pat::Wild; arity];
                nr.extend_from_slice(&row[1..]);
                out.push(nr);
            }
            _ => {} // different constructor — row does not match
        }
    }
    out
}

/// The default matrix: rows whose head is a wildcard, with that head dropped.
fn default_matrix(matrix: &[Vec<Pat>]) -> Vec<Vec<Pat>> {
    let mut out = Vec::new();
    for row in matrix {
        if let Some(Pat::Wild) = row.first() {
            out.push(row[1..].to_vec());
        }
    }
    out
}

fn useful(matrix: &[Vec<Pat>], q: &[Pat], sib: &Siblings) -> Usefulness {
    let Some(first) = q.first() else {
        // No columns: useful iff no row remains (nothing matched the whole vector).
        return if matrix.is_empty() {
            Usefulness::Witness(Vec::new())
        } else {
            Usefulness::Useless
        };
    };
    match first {
        Pat::Con(c, cargs) => {
            let arity = cargs.len();
            let spec = specialize(matrix, c, arity);
            let mut newq = cargs.clone();
            newq.extend_from_slice(&q[1..]);
            match useful(&spec, &newq, sib) {
                Usefulness::Useless => Usefulness::Useless,
                Usefulness::Witness(w) => Usefulness::Witness(rebuild(c, arity, w)),
            }
        }
        Pat::Wild => {
            let heads = column_cons(matrix);
            let sig = signature(&heads, sib);
            let complete = match &sig {
                Signature::Finite(all) => all.iter().all(|(c, _)| heads.contains(c)),
                Signature::Infinite => false,
            };
            if complete {
                if let Signature::Finite(all) = &sig {
                    for (c, arity) in all {
                        let spec = specialize(matrix, c, *arity);
                        let mut newq = vec![Pat::Wild; *arity];
                        newq.extend_from_slice(&q[1..]);
                        if let Usefulness::Witness(w) = useful(&spec, &newq, sib) {
                            return Usefulness::Witness(rebuild(c, *arity, w));
                        }
                    }
                }
                Usefulness::Useless
            } else {
                match useful(&default_matrix(matrix), &q[1..], sib) {
                    Usefulness::Useless => Usefulness::Useless,
                    Usefulness::Witness(w) => {
                        // Prepend a head for the first column: a *missing*
                        // constructor (finite/incomplete), else a wildcard.
                        let head = match &sig {
                            Signature::Finite(all) => {
                                let (mc, ma) = all
                                    .iter()
                                    .find(|(c, _)| !heads.contains(c))
                                    .cloned()
                                    .unwrap_or((Con::Unit, 0));
                                Pat::Con(mc, vec![Pat::Wild; ma])
                            }
                            Signature::Infinite => Pat::Wild,
                        };
                        let mut out = vec![head];
                        out.extend(w);
                        Usefulness::Witness(out)
                    }
                }
            }
        }
    }
}

/// Reassemble a witness after specialization: the first `arity` entries are the
/// constructor's args, the rest is the tail.
fn rebuild(c: &Con, arity: usize, w: Vec<Pat>) -> Vec<Pat> {
    let (cw, tail) = w.split_at(arity.min(w.len()));
    let mut out = vec![Pat::Con(c.clone(), cw.to_vec())];
    out.extend_from_slice(tail);
    out
}

fn render_witness(p: &Pat) -> String {
    match p {
        Pat::Wild => "_".to_string(),
        Pat::Con(Con::Data(name), args) => {
            if args.is_empty() {
                name.clone()
            } else {
                let inner: Vec<String> = args.iter().map(render_witness).collect();
                format!("{name}({})", inner.join(", "))
            }
        }
        Pat::Con(Con::Bool(b), _) => if *b { "True" } else { "False" }.to_string(),
        Pat::Con(Con::Int(n), _) => n.to_string(),
        Pat::Con(Con::Str(s), _) => format!("{s:?}"),
        Pat::Con(Con::Unit, _) => "Unit".to_string(),
    }
}

/// Check a single `match`: its non-exhaustiveness (`E0430`) and any redundant
/// arms (`E0431`).
fn check_match(
    arms: &[Spanned<MatchArm>],
    span: crate::span::Span,
    sib: &Siblings,
    out: &mut Vec<Diagnostic>,
) {
    let rows: Vec<Vec<Pat>> = arms.iter().map(|a| vec![lower(&a.node.pat.node)]).collect();

    // Useless arms: arm i is redundant iff not useful against arms 0..i.
    for (i, arm) in arms.iter().enumerate() {
        if let Usefulness::Useless = useful(&rows[..i], &rows[i], sib) {
            out.push(
                Diagnostic::warning("E0431", "unreachable match arm")
                    .with_label(arm.node.pat.span, "already covered by an earlier pattern"),
            );
        }
    }

    // Exhaustiveness: is a wildcard still useful against the whole matrix?
    if let Usefulness::Witness(w) = useful(&rows, &[Pat::Wild], sib) {
        let witness = w
            .first()
            .map(render_witness)
            .unwrap_or_else(|| "_".to_string());
        out.push(
            Diagnostic::error(
                "E0430",
                format!("non-exhaustive match: `{witness}` is not covered"),
            )
            .with_label(
                span,
                "add an arm for the missing pattern (or a `_` catch-all)",
            ),
        );
    }
}

fn walk_expr(e: &Spanned<Expr>, sib: &Siblings, out: &mut Vec<Diagnostic>) {
    match &e.node {
        Expr::Match { scrutinee, arms } => {
            walk_expr(scrutinee, sib, out);
            for arm in arms.iter() {
                walk_expr(&arm.node.body, sib, out);
            }
            check_match(arms, e.span, sib, out);
        }
        Expr::Call { callee, args } => {
            walk_expr(callee, sib, out);
            for a in args.iter() {
                walk_expr(a, sib, out);
            }
        }
        Expr::Unary { expr, .. } => walk_expr(expr, sib, out),
        Expr::Binary { lhs, rhs, .. } => {
            walk_expr(lhs, sib, out);
            walk_expr(rhs, sib, out);
        }
        Expr::If {
            cond,
            then_block,
            else_block,
        } => {
            walk_expr(cond, sib, out);
            walk_block(&then_block.node, sib, out);
            walk_block(&else_block.node, sib, out);
        }
        Expr::Block(b) => walk_block(b, sib, out),
        Expr::Handle { body, handler } => {
            walk_expr(body, sib, out);
            for c in &handler.clauses {
                walk_expr(&c.node.body, sib, out);
            }
            if let Some(r) = &handler.ret {
                walk_expr(&r.body, sib, out);
            }
        }
        Expr::Resume { arg } => walk_expr(arg, sib, out),
        _ => {}
    }
}

fn walk_block(b: &Block, sib: &Siblings, out: &mut Vec<Diagnostic>) {
    for st in b.stmts.iter() {
        match &st.node {
            Stmt::Let { value, .. } => walk_expr(value, sib, out),
            Stmt::Expr(e) => walk_expr(e, sib, out),
        }
    }
    if let Some(t) = &b.tail {
        walk_expr(t, sib, out);
    }
}

/// Check every `match` in the module for exhaustiveness (`E0430`, error) and
/// redundant arms (`E0431`, warning).
pub fn check(module: &Module) -> Vec<Diagnostic> {
    let mut sib: Siblings = HashMap::new();
    for d in &module.decls {
        if let Decl::Type(t) = &d.node {
            let all: Vec<(String, usize)> = t
                .variants
                .iter()
                .map(|v| (v.node.name.clone(), v.node.fields.len()))
                .collect();
            for v in &t.variants {
                sib.insert(v.node.name.clone(), all.clone());
            }
        }
    }
    let mut out = Vec::new();
    for d in &module.decls {
        if let Decl::Fn(f) = &d.node {
            walk_block(&f.body.node, &sib, &mut out);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::Severity;
    use crate::parse::parse_module;
    use crate::Session;

    fn codes(src: &str) -> Vec<(String, Severity)> {
        let (m, d) = parse_module(&Session::new(), src);
        assert!(d.is_empty(), "parse: {d:?}");
        check(&m)
            .into_iter()
            .map(|x| (x.code, x.severity))
            .collect()
    }

    // Finite/infinite boundary, both directions.

    #[test]
    fn bool_exhausted_by_true_false_accepts() {
        // Finite Bool: True + False is exhaustive with NO catch-all.
        let d = codes("fn f(b) { match b { True -> 1  False -> 0 } }\n");
        assert!(d.is_empty(), "expected no diagnostics, got {d:?}");
    }

    #[test]
    fn bool_missing_case_is_e0430() {
        let d = codes("fn f(b) { match b { True -> 1 } }\n");
        assert_eq!(d, vec![("E0430".into(), Severity::Error)]);
    }

    #[test]
    fn int_literal_demands_catchall() {
        // Infinite Int: a literal arm without a catch-all is non-exhaustive.
        let d = codes("fn f(n) { match n { 0 -> 1 } }\n");
        assert_eq!(d, vec![("E0430".into(), Severity::Error)]);
    }

    #[test]
    fn int_with_catchall_accepts() {
        let d = codes("fn f(n) { match n { 0 -> 1  _ -> 2 } }\n");
        assert!(d.is_empty(), "expected no diagnostics, got {d:?}");
    }

    #[test]
    fn adt_full_coverage_accepts() {
        let d = codes(
            "type Option(a) { None, Some(a) }\n\
             fn f(o) { match o { None -> 0  Some(x) -> x } }\n",
        );
        assert!(d.is_empty(), "expected no diagnostics, got {d:?}");
    }

    #[test]
    fn nested_witness_is_constructed() {
        // The correctness-critical case: the missing pattern is nested.
        let (m, _) = parse_module(
            &Session::new(),
            "type List(a) { Nil, Cons(a, List(a)) }\n\
             fn f(xs) { match xs { Nil -> 0  Cons(_, Nil) -> 1 } }\n",
        );
        let d = check(&m);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].code, "E0430");
        assert!(
            d[0].message.contains("Cons(_, Cons(_, _))"),
            "nested witness wrong: {}",
            d[0].message
        );
    }

    #[test]
    fn useless_arm_is_e0431_warning() {
        // The second `Some` is subsumed by the first; the match is still
        // exhaustive (Some + None), so the only diagnostic is the E0431 warning.
        let d = codes(
            "type Option(a) { None, Some(a) }\n\
             fn f(o) { match o { Some(x) -> x  Some(y) -> y  None -> 0 } }\n",
        );
        assert_eq!(d, vec![("E0431".into(), Severity::Warning)]);
    }
}
