//! Name resolution: checks that every reference resolves to a param, a
//! local `let`, a top-level function, or a known builtin.

use crate::ast::*;
use crate::diag::Diagnostic;
use crate::span::{Span, Spanned};
use crate::Session;
use std::collections::HashSet;

pub fn builtins() -> &'static [&'static str] {
    &["io.println"]
}

pub fn check(_session: &Session, module: &Module) -> Vec<Diagnostic> {
    let mut fn_names: HashSet<String> = HashSet::new();
    let mut op_names: HashSet<String> = HashSet::new();
    let mut ctor_names: HashSet<String> = HashSet::new();
    for d in &module.decls {
        match &d.node {
            Decl::Fn(f) => {
                fn_names.insert(f.name.clone());
            }
            Decl::Effect(e) => {
                for op in &e.ops {
                    op_names.insert(op.node.name.clone());
                }
            }
            Decl::Type(t) => {
                for v in &t.variants {
                    ctor_names.insert(v.node.name.clone());
                }
            }
        }
    }
    let mut cx = Cx {
        fns: &fn_names,
        ops: &op_names,
        ctors: &ctor_names,
        in_handler: 0,
        diags: Vec::new(),
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
    cx.diags
}

struct Cx<'a> {
    fns: &'a HashSet<String>,
    ops: &'a HashSet<String>,
    ctors: &'a HashSet<String>,
    /// Nesting depth of handler clauses currently being checked. `resume` is
    /// only legal where this is nonzero (E0210 otherwise).
    in_handler: usize,
    diags: Vec<Diagnostic>,
}

impl Cx<'_> {
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
            Pattern::Wild => {}
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
                for c in &handler.clauses {
                    let clause = &c.node;
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
