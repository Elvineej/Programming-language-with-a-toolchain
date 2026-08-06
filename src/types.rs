//! Hindley–Milner type inference (Algorithm J).

use crate::diag::Diagnostic;
use crate::span::Span;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TyCon {
    Int,
    Float,
    Bool,
    Str,
    Unit,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Ty {
    Var(u32),
    Base(TyCon),
    Fn(Vec<Ty>, Box<Ty>),
    Tuple(Vec<Ty>),
    /// Poison value that unifies with anything; suppresses cascade errors.
    Error,
}

impl Ty {
    pub fn int() -> Ty {
        Ty::Base(TyCon::Int)
    }
    pub fn float() -> Ty {
        Ty::Base(TyCon::Float)
    }
    pub fn bool() -> Ty {
        Ty::Base(TyCon::Bool)
    }
    pub fn str() -> Ty {
        Ty::Base(TyCon::Str)
    }
    pub fn unit() -> Ty {
        Ty::Base(TyCon::Unit)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Scheme {
    pub vars: Vec<u32>,
    pub ty: Ty,
}

pub struct Infer {
    subst: Vec<Option<Ty>>,
    pub diags: Vec<Diagnostic>,
}

impl Infer {
    pub fn new() -> Infer {
        Infer {
            subst: Vec::new(),
            diags: Vec::new(),
        }
    }

    pub fn fresh(&mut self) -> Ty {
        let id = self.subst.len() as u32;
        self.subst.push(None);
        Ty::Var(id)
    }

    /// Follow bound variables to a representative, recursively (deep).
    pub fn resolve(&self, t: &Ty) -> Ty {
        match t {
            Ty::Var(v) => match &self.subst[*v as usize] {
                Some(bound) => self.resolve(bound),
                None => Ty::Var(*v),
            },
            Ty::Base(c) => Ty::Base(*c),
            Ty::Fn(ps, r) => Ty::Fn(
                ps.iter().map(|p| self.resolve(p)).collect(),
                Box::new(self.resolve(r)),
            ),
            Ty::Tuple(xs) => Ty::Tuple(xs.iter().map(|x| self.resolve(x)).collect()),
            Ty::Error => Ty::Error,
        }
    }

    fn occurs(&self, v: u32, t: &Ty) -> bool {
        match self.resolve(t) {
            Ty::Var(u) => u == v,
            Ty::Base(_) | Ty::Error => false,
            Ty::Fn(ps, r) => ps.iter().any(|p| self.occurs(v, p)) || self.occurs(v, &r),
            Ty::Tuple(xs) => xs.iter().any(|x| self.occurs(v, x)),
        }
    }

    fn bind(&mut self, v: u32, t: &Ty, span: Span) {
        if let Ty::Var(u) = t {
            if *u == v {
                return;
            }
        }
        if self.occurs(v, t) {
            self.diags.push(
                Diagnostic::error("E0401", "infinite type")
                    .with_label(span, "a type would have to contain itself"),
            );
            self.subst[v as usize] = Some(Ty::Error);
            return;
        }
        self.subst[v as usize] = Some(t.clone());
    }

    pub fn unify(&mut self, a: &Ty, b: &Ty, span: Span) {
        let a = self.resolve(a);
        let b = self.resolve(b);
        match (a, b) {
            (Ty::Error, _) | (_, Ty::Error) => {}
            (Ty::Var(x), Ty::Var(y)) if x == y => {}
            (Ty::Var(x), t) | (t, Ty::Var(x)) => self.bind(x, &t, span),
            (Ty::Base(x), Ty::Base(y)) if x == y => {}
            (Ty::Fn(p1, r1), Ty::Fn(p2, r2)) => {
                if p1.len() != p2.len() {
                    self.diags.push(
                        Diagnostic::error("E0402", "wrong number of arguments").with_label(
                            span,
                            format!("expected {} argument(s), found {}", p1.len(), p2.len()),
                        ),
                    );
                } else {
                    for (x, y) in p1.iter().zip(&p2) {
                        self.unify(x, y, span);
                    }
                    self.unify(&r1, &r2, span);
                }
            }
            (Ty::Tuple(x), Ty::Tuple(y)) if x.len() == y.len() => {
                for (p, q) in x.iter().zip(&y) {
                    self.unify(p, q, span);
                }
            }
            (x, y) => {
                let msg = format!(
                    "expected `{}`, found `{}`",
                    display_ty(self, &x),
                    display_ty(self, &y)
                );
                self.diags
                    .push(Diagnostic::error("E0400", "type mismatch").with_label(span, msg));
            }
        }
    }
}

impl Default for Infer {
    fn default() -> Self {
        Infer::new()
    }
}

// Placeholder until Task 2 adds the zonking printer; unify()'s mismatch arm
// calls display_ty, which is defined in Task 2. For Task 1 we provide a minimal
// version so the crate compiles and mismatch messages are readable.
pub fn display_ty(inf: &Infer, t: &Ty) -> String {
    fn go(t: &Ty, out: &mut String) {
        match t {
            Ty::Var(v) => out.push_str(&format!("t{v}")),
            Ty::Base(TyCon::Int) => out.push_str("Int"),
            Ty::Base(TyCon::Float) => out.push_str("Float"),
            Ty::Base(TyCon::Bool) => out.push_str("Bool"),
            Ty::Base(TyCon::Str) => out.push_str("String"),
            Ty::Base(TyCon::Unit) => out.push_str("Unit"),
            Ty::Fn(ps, r) => {
                out.push_str("fn(");
                for (i, p) in ps.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    go(p, out);
                }
                out.push_str(") -> ");
                go(r, out);
            }
            Ty::Tuple(xs) => {
                out.push('(');
                for (i, x) in xs.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    go(x, out);
                }
                out.push(')');
            }
            Ty::Error => out.push_str("<error>"),
        }
    }
    let resolved = inf.resolve(t);
    let mut out = String::new();
    go(&resolved, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::Span;

    #[test]
    fn unifies_equal_bases() {
        let mut inf = Infer::new();
        inf.unify(&Ty::int(), &Ty::int(), Span::EMPTY);
        assert!(inf.diags.is_empty());
    }

    #[test]
    fn mismatched_bases_are_e0400() {
        let mut inf = Infer::new();
        inf.unify(&Ty::int(), &Ty::str(), Span::EMPTY);
        assert_eq!(inf.diags.len(), 1);
        assert_eq!(inf.diags[0].code, "E0400");
    }

    #[test]
    fn binds_and_resolves_a_var() {
        let mut inf = Infer::new();
        let v = inf.fresh();
        inf.unify(&v, &Ty::bool(), Span::EMPTY);
        assert!(inf.diags.is_empty());
        assert_eq!(inf.resolve(&v), Ty::bool());
    }

    #[test]
    fn occurs_check_is_e0401() {
        let mut inf = Infer::new();
        let v = inf.fresh();
        let f = Ty::Fn(vec![v.clone()], Box::new(Ty::int()));
        inf.unify(&v, &f, Span::EMPTY);
        assert_eq!(inf.diags.len(), 1);
        assert_eq!(inf.diags[0].code, "E0401");
    }

    #[test]
    fn function_arity_mismatch_is_e0402() {
        let mut inf = Infer::new();
        let a = Ty::Fn(vec![Ty::int()], Box::new(Ty::unit()));
        let b = Ty::Fn(vec![Ty::int(), Ty::int()], Box::new(Ty::unit()));
        inf.unify(&a, &b, Span::EMPTY);
        assert_eq!(inf.diags.len(), 1);
        assert_eq!(inf.diags[0].code, "E0402");
    }
}
