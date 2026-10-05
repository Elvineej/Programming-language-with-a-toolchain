//! Name resolution: checks that every reference resolves to a param, a
//! local `let`, a top-level function, or a known builtin.

use crate::ast::*;
use crate::diag::Diagnostic;
use crate::span::{Span, Spanned};
use crate::Session;
use std::collections::{HashMap, HashSet};

pub fn builtins() -> &'static [&'static str] {
    &["io.println"]
}

pub fn check(_session: &Session, module: &Module) -> Vec<Diagnostic> {
    walk(module).0
}

/// Slice 5c-2: the spans of call-site CALLEES that name an operation but are
/// bound by a LOCAL (let, parameter, lambda or clause parameter, match binder)
/// in scope -- calls of the local, not performs. Computed by the same scope
/// walk that decides E0200, so Core lowering cannot disagree with `check`.
/// (Inference and the evaluator consult their own environments, which under
/// E0205 can only hold such a name as a local.)
pub fn locally_shadowed_op_calls(module: &Module) -> HashSet<Span> {
    walk(module).1
}

fn walk(module: &Module) -> (Vec<Diagnostic>, HashSet<Span>) {
    let mut fn_names: HashSet<String> = HashSet::new();
    let mut op_names: HashSet<String> = HashSet::new();
    let mut ctor_names: HashSet<String> = HashSet::new();
    // Slice 5c-1: op name -> its ONE declaring effect. A perform cannot be
    // qualified, and every op index downstream (`Infer.ops`, the evaluator's
    // `op_table`, `ast::op_effects`) is keyed by op name, so a second
    // declaration is E0202 rather than a silent overwrite.
    let mut op_effect: HashMap<String, String> = HashMap::new();
    let mut op_arity: HashMap<String, usize> = HashMap::new();
    let mut dup_ops: HashSet<String> = HashSet::new();
    let mut effect_names: HashSet<String> = HashSet::new();
    let mut dup_diags: Vec<Diagnostic> = Vec::new();
    for d in &module.decls {
        match &d.node {
            Decl::Fn(f) => {
                fn_names.insert(f.name.clone());
            }
            Decl::Effect(e) => {
                effect_names.insert(e.name.clone());
                for op in &e.ops {
                    let name = &op.node.name;
                    if let Some(first) = op_effect.get(name) {
                        dup_ops.insert(name.clone());
                        dup_diags.push(
                            Diagnostic::error(
                                "E0202",
                                format!("operation `{name}` is declared more than once"),
                            )
                            .with_label(
                                op.span,
                                format!("`{name}` is already declared by effect `{first}`"),
                            )
                            .with_help(
                                "operation names are unique across a module: a perform names only the operation, so it must say which effect it performs",
                            ),
                        );
                    } else {
                        op_effect.insert(name.clone(), e.name.clone());
                        op_arity.insert(name.clone(), op.node.param_tys.len());
                    }
                    op_names.insert(name.clone());
                }
            }
            Decl::Type(t) => {
                for v in &t.variants {
                    ctor_names.insert(v.node.name.clone());
                }
            }
        }
    }
    // E0205 (slice 5c-2): a top-level function named like an operation could
    // never be called -- a call of that name performs -- so it is an error.
    // Checked after every declaration is seen; effects may follow functions.
    for d in &module.decls {
        if let Decl::Fn(f) = &d.node {
            if let Some(effect) = op_effect.get(&f.name) {
                dup_diags.push(
                    Diagnostic::error(
                        "E0205",
                        format!(
                            "function `{}` has the name of an operation of `{effect}`",
                            f.name
                        ),
                    )
                    .with_label(d.span, "a call of this name performs the operation instead")
                    .with_help("rename the function; a local binding may shadow an operation, a top-level function may not"),
                );
            }
        }
    }
    let mut cx = Cx {
        fns: &fn_names,
        ops: &op_names,
        ctors: &ctor_names,
        op_effect: &op_effect,
        op_arity: &op_arity,
        dup_ops: &dup_ops,
        effects: &effect_names,
        in_handler: 0,
        diags: dup_diags,
        shadowed_calls: HashSet::new(),
    };
    for d in &module.decls {
        match &d.node {
            Decl::Fn(f) => {
                let mut scope: Vec<HashSet<String>> = vec![HashSet::new()];
                for p in &f.params {
                    scope.last_mut().unwrap().insert(p.node.name.clone());
                }
                cx.check_block(&f.body.node, &mut scope);
            }
            Decl::Effect(_) => {} // effect operation bodies are just signatures
            Decl::Type(_) => {}   // type declarations are signatures only
        }
    }
    (cx.diags, cx.shadowed_calls)
}

struct Cx<'a> {
    fns: &'a HashSet<String>,
    ops: &'a HashSet<String>,
    ctors: &'a HashSet<String>,
    /// Op name -> its declaring effect (unique by E0202), and the declared
    /// effect names: what a handler clause must resolve against (E0203).
    op_effect: &'a HashMap<String, String>,
    /// Op name -> its parameter count; a clause must bind exactly that many.
    op_arity: &'a HashMap<String, usize>,
    /// Op names already reported E0202: a clause on one is not checked again
    /// (its owner is ambiguous, so any E0203 there would be noise or false).
    dup_ops: &'a HashSet<String>,
    effects: &'a HashSet<String>,
    /// Nesting depth of handler clauses currently being checked. `resume` is
    /// only legal where this is nonzero (E0210 otherwise).
    in_handler: usize,
    diags: Vec<Diagnostic>,
    /// See `locally_shadowed_op_calls`.
    shadowed_calls: HashSet<Span>,
}

impl Cx<'_> {
    /// E0203: a clause must name a declared operation, and its qualifier, if
    /// written, must be that operation's effect (slice 3 spec §7.2, enforced
    /// from slice 5c-1). An unqualified clause then means the op's effect.
    fn check_clause_names_an_op(&mut self, clause: &OpClause, span: Span) {
        let op = &clause.op;
        if self.dup_ops.contains(op) {
            return;
        }
        let msg = match (self.op_effect.get(op), &clause.effect) {
            (None, Some(q)) if !self.effects.contains(q) => {
                format!("`{q}` is not a declared effect")
            }
            (None, _) => format!("no effect declares an operation `{op}`"),
            (Some(_), Some(q)) if !self.effects.contains(q) => {
                format!("`{q}` is not a declared effect")
            }
            (Some(owner), Some(q)) if owner != q => {
                format!("`{op}` is an operation of `{owner}`, not `{q}`")
            }
            // Pre-existing hole closed here (review of 5c-1): a clause binding
            // the wrong number of arguments checked clean, then failed at run
            // time ("unbound variable") or crashed natively.
            _ => match self.op_arity.get(op) {
                Some(&n) if n != clause.params.len() => {
                    let plural = if n == 1 { "" } else { "s" };
                    format!(
                        "`{op}` takes {n} argument{plural}, but this clause binds {}",
                        clause.params.len()
                    )
                }
                _ => return,
            },
        };
        let mut d = Diagnostic::error("E0203", "this clause does not match a declared operation")
            .with_label(span, msg);
        // The qualifier fix-it only where the qualifier is what is wrong.
        let qualifier_wrong = clause
            .effect
            .as_ref()
            .is_some_and(|q| self.op_effect.get(op) != Some(q));
        if let (Some(owner), true) = (self.op_effect.get(op), qualifier_wrong) {
            d = d.with_help(format!("write `{owner}.{op}(…)`, or just `{op}(…)`"));
        }
        self.diags.push(d);
    }

    fn resolves_var(&self, name: &str, scope: &[HashSet<String>]) -> bool {
        scope.iter().rev().any(|s| s.contains(name))
            || self.fns.contains(name)
            || self.ops.contains(name)
            || self.ctors.contains(name)
    }

    /// Walk a pattern: bind its variables into `scope`, and validate that each
    /// constructor name is known (`E0432` otherwise).
    fn bind_pattern(&mut self, pat: &Spanned<Pattern>, scope: &mut HashSet<String>) {
        match &pat.node {
            Pattern::Wild | Pattern::Lit(_) => {}
            Pattern::Var(x) => {
                scope.insert(x.clone());
            }
            Pattern::Ctor { name, args } => {
                if !self.ctors.contains(name) {
                    self.diags.push(
                        Diagnostic::error("E0432", format!("unknown constructor `{name}`"))
                            .with_label(pat.span, "no such constructor"),
                    );
                }
                for a in args {
                    self.bind_pattern(a, scope);
                }
            }
        }
    }

    fn check_block(&mut self, b: &Block, scope: &mut Vec<HashSet<String>>) {
        scope.push(HashSet::new());
        for st in b.stmts.iter() {
            match &st.node {
                Stmt::Let { name, value } => {
                    self.check_expr(&value.node, value.span, scope);
                    scope.last_mut().unwrap().insert(name.clone());
                }
                Stmt::Expr(e) => self.check_expr(&e.node, e.span, scope),
            }
        }
        if let Some(tail) = &b.tail {
            self.check_expr(&tail.node, tail.span, scope);
        }
        scope.pop();
    }

    fn check_expr(&mut self, e: &Expr, span: Span, scope: &mut Vec<HashSet<String>>) {
        match e {
            Expr::Int(_) | Expr::Float(_) | Expr::Str(_) | Expr::Bool(_) | Expr::Unit => {}
            Expr::Var(name) => {
                if !self.resolves_var(name, scope) {
                    self.diags.push(
                        Diagnostic::error("E0200", format!("unresolved name `{name}`"))
                            .with_label(span, "not found in this scope"),
                    );
                }
            }
            Expr::Qualified { module, name } => {
                let full = format!("{module}.{name}");
                if !builtins().contains(&full.as_str()) {
                    self.diags.push(
                        Diagnostic::error("E0201", format!("unknown builtin `{full}`"))
                            .with_label(span, "no such function"),
                    );
                }
            }
            Expr::Call { callee, args } => {
                // Locals before ops (5c-2): an op name bound by a local in scope
                // is a call of the local.
                if let Expr::Var(name) = &callee.node {
                    if self.ops.contains(name) && scope.iter().any(|s| s.contains(name)) {
                        self.shadowed_calls.insert(callee.span);
                    }
                }
                self.check_expr(&callee.node, callee.span, scope);
                for a in args.iter() {
                    self.check_expr(&a.node, a.span, scope);
                }
            }
            Expr::Unary { expr, .. } => self.check_expr(&expr.node, expr.span, scope),
            Expr::Binary { lhs, rhs, .. } => {
                self.check_expr(&lhs.node, lhs.span, scope);
                self.check_expr(&rhs.node, rhs.span, scope);
            }
            Expr::If {
                cond,
                then_block,
                else_block,
            } => {
                self.check_expr(&cond.node, cond.span, scope);
                self.check_block(&then_block.node, scope);
                self.check_block(&else_block.node, scope);
            }
            Expr::Block(b) => self.check_block(b, scope),
            Expr::Handle { body, handler } => {
                // The handled computation is not itself inside a clause, so a
                // bare `resume` here is still an error (in_handler unchanged).
                self.check_expr(&body.node, body.span, scope);
                self.in_handler += 1;
                let mut handled: HashSet<&str> = HashSet::new();
                for c in &handler.clauses {
                    let clause = &c.node;
                    self.check_clause_names_an_op(clause, c.span);
                    // E0204 (5c-2): one clause per op. Op names are unique, so the
                    // op name decides whatever the spelling. Ops already reported
                    // (E0202) or unknown (E0203) are not judged again.
                    let known = self.op_effect.contains_key(&clause.op)
                        && !self.dup_ops.contains(&clause.op);
                    if known && !handled.insert(clause.op.as_str()) {
                        self.diags.push(
                            Diagnostic::error(
                                "E0204",
                                "a handler has more than one clause for an operation",
                            )
                            .with_label(
                                c.span,
                                format!(
                                    "`{}` already has a clause in this handler; this one can never run",
                                    clause.op
                                ),
                            ),
                        );
                    }
                    scope.push(HashSet::new());
                    for p in &clause.params {
                        scope.last_mut().unwrap().insert(p.node.name.clone());
                    }
                    self.check_expr(&clause.body.node, clause.body.span, scope);
                    scope.pop();
                }
                self.in_handler -= 1;
                if let Some(ret) = &handler.ret {
                    // The return clause runs after the computation completes;
                    // `resume` is not in scope there.
                    scope.push(HashSet::new());
                    scope.last_mut().unwrap().insert(ret.binder.clone());
                    self.check_expr(&ret.body.node, ret.body.span, scope);
                    scope.pop();
                }
            }
            Expr::Resume { arg } => {
                if self.in_handler == 0 {
                    self.diags.push(
                        Diagnostic::error("E0210", "`resume` used outside a handler")
                            .with_label(span, "`resume` is only valid inside a handler clause"),
                    );
                }
                self.check_expr(&arg.node, arg.span, scope);
            }
            Expr::Match { scrutinee, arms } => {
                self.check_expr(&scrutinee.node, scrutinee.span, scope);
                for arm in arms.iter() {
                    let mut bound = HashSet::new();
                    self.bind_pattern(&arm.node.pat, &mut bound);
                    scope.push(bound);
                    self.check_expr(&arm.node.body.node, arm.node.body.span, scope);
                    scope.pop();
                }
            }
            Expr::Lambda { params, body } => {
                scope.push(HashSet::new());
                for p in params {
                    scope.last_mut().unwrap().insert(p.node.name.clone());
                }
                self.check_block(&body.node, scope);
                scope.pop();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_module;
    use crate::Session;

    fn diags(src: &str) -> Vec<crate::diag::Diagnostic> {
        let (m, pdiags) = parse_module(&Session::new(), src);
        assert!(pdiags.is_empty(), "parse diags: {pdiags:?}");
        check(&Session::new(), &m)
    }

    #[test]
    fn undefined_variable_is_e0200() {
        let d = diags("fn f() { x }\n");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].code, "E0200");
    }

    #[test]
    fn params_lets_and_fns_resolve() {
        let d = diags("fn g() { 1 }\nfn f(a) { let b = a\n g() }\n");
        assert!(d.is_empty(), "unexpected: {d:?}");
    }

    #[test]
    fn unknown_builtin_is_e0201() {
        let d = diags(r#"fn f() { io.nope("x") }"#);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].code, "E0201");
    }

    #[test]
    fn known_builtin_resolves() {
        let d = diags(r#"fn f() { io.println("x") }"#);
        assert!(d.is_empty(), "unexpected: {d:?}");
    }

    #[test]
    fn operation_call_resolves() {
        let d =
            diags("effect Log {\n  fn log(msg: String) -> Unit\n}\nfn f() {\n  log(\"hi\")\n}\n");
        assert!(d.is_empty(), "unexpected: {d:?}");
    }

    #[test]
    fn resume_inside_handler_resolves() {
        let src = "effect Log {\n  fn log(msg: String) -> Unit\n}\n\
                   fn g() { 1 }\n\
                   fn f() {\n  handle g() with {\n    Log.log(m) -> resume(m)\n    return(r) -> r\n  }\n}\n";
        let d = diags(src);
        assert!(d.is_empty(), "unexpected: {d:?}");
    }

    #[test]
    fn resume_outside_handler_is_e0210() {
        let d = diags("fn f() {\n  resume(1)\n}\n");
        assert_eq!(d.len(), 1, "expected one diag, got: {d:?}");
        assert_eq!(d[0].code, "E0210");
    }
}
