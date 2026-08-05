//! Name resolution: checks that every reference resolves to a param, a
//! local `let`, a top-level function, or a known builtin.

use crate::ast::*;
use crate::diag::Diagnostic;
use crate::span::Span;
use crate::Session;
use std::collections::HashSet;

pub fn builtins() -> &'static [&'static str] {
    &["io.println"]
}

pub fn check(_session: &Session, module: &Module) -> Vec<Diagnostic> {
    let mut fn_names: HashSet<String> = HashSet::new();
    for d in &module.decls {
        match &d.node {
            Decl::Fn(f) => {
                fn_names.insert(f.name.clone());
            }
        }
    }
    let mut cx = Cx {
        fns: &fn_names,
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
        }
    }
    cx.diags
}

struct Cx<'a> {
    fns: &'a HashSet<String>,
    diags: Vec<Diagnostic>,
}

impl Cx<'_> {
    fn resolves_var(&self, name: &str, scope: &[HashSet<String>]) -> bool {
        scope.iter().rev().any(|s| s.contains(name)) || self.fns.contains(name)
    }

    fn check_block(&mut self, b: &Block, scope: &mut Vec<HashSet<String>>) {
        scope.push(HashSet::new());
        for st in &b.stmts {
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
                for a in args {
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
}
