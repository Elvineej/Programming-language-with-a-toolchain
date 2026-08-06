//! Slice-2 evaluators: a shared value/env layer, the tree-walker oracle (`tree`),
//! and the CEK machine (`cek`, added in Task 8).

use crate::ast::*;
use crate::diag::Diagnostic;
use crate::span::{Span, Spanned};
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Unit,
    /// A top-level function, referenced by name (Slice 2 has no lambdas).
    Fn(String),
}

#[derive(Debug)]
pub struct RuntimeError {
    pub diag: Diagnostic,
}

fn rt(span: Span, msg: impl Into<String>) -> RuntimeError {
    RuntimeError {
        diag: Diagnostic::error("E0300", msg).with_label(span, "during evaluation"),
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Scope {
    vars: HashMap<String, Value>,
    parent: Option<Rc<Scope>>,
}

/// Persistent parent-pointer environment holding local bindings only.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Env(Option<Rc<Scope>>);

impl Env {
    pub fn new() -> Env {
        Env(None)
    }

    pub fn extend(&self, bindings: &[(String, Value)]) -> Env {
        let mut vars = HashMap::with_capacity(bindings.len());
        for (k, v) in bindings {
            vars.insert(k.clone(), v.clone());
        }
        Env(Some(Rc::new(Scope {
            vars,
            parent: self.0.clone(),
        })))
    }

    pub fn get(&self, name: &str) -> Option<Value> {
        let mut cur = self.0.as_deref();
        while let Some(scope) = cur {
            if let Some(v) = scope.vars.get(name) {
                return Some(v.clone());
            }
            cur = scope.parent.as_deref();
        }
        None
    }
}

pub type Fns<'a> = HashMap<&'a str, &'a FnDecl>;

pub fn fn_table<'a>(module: &'a Module) -> Fns<'a> {
    let mut m = HashMap::new();
    for d in &module.decls {
        let Decl::Fn(f) = &d.node;
        m.insert(f.name.as_str(), f);
    }
    m
}

pub struct Interp {
    output: String,
    peak_kont: usize,
}

impl Interp {
    pub fn new() -> Interp {
        Interp {
            output: String::new(),
            peak_kont: 0,
        }
    }
    pub fn output(&self) -> &str {
        &self.output
    }
    pub fn peak_kont_depth(&self) -> usize {
        self.peak_kont
    }
    fn println(&mut self, s: &str) {
        self.output.push_str(s);
        self.output.push('\n');
    }
    // `note_kont_depth` (writes `peak_kont`) is added with the CEK machine in Task 8.
}

impl Default for Interp {
    fn default() -> Self {
        Interp::new()
    }
}

pub(crate) fn apply_binop(op: BinOp, l: Value, r: Value, span: Span) -> Result<Value, RuntimeError> {
    use BinOp::*;
    use Value::*;
    match (op, l, r) {
        (Add, Int(a), Int(b)) => Ok(Int(a + b)),
        (Sub, Int(a), Int(b)) => Ok(Int(a - b)),
        (Mul, Int(a), Int(b)) => Ok(Int(a * b)),
        (Div, Int(_), Int(0)) => Err(rt(span, "division by zero")),
        (Div, Int(a), Int(b)) => Ok(Int(a / b)),
        (Rem, Int(_), Int(0)) => Err(rt(span, "remainder by zero")),
        (Rem, Int(a), Int(b)) => Ok(Int(a % b)),
        (AddF, Float(a), Float(b)) => Ok(Float(a + b)),
        (SubF, Float(a), Float(b)) => Ok(Float(a - b)),
        (MulF, Float(a), Float(b)) => Ok(Float(a * b)),
        (DivF, Float(a), Float(b)) => Ok(Float(a / b)),
        (Concat, Str(a), Str(b)) => Ok(Str(a + &b)),
        (Eq, a, b) => Ok(Bool(a == b)),
        (Ne, a, b) => Ok(Bool(a != b)),
        (Lt, Int(a), Int(b)) => Ok(Bool(a < b)),
        (Le, Int(a), Int(b)) => Ok(Bool(a <= b)),
        (Gt, Int(a), Int(b)) => Ok(Bool(a > b)),
        (Ge, Int(a), Int(b)) => Ok(Bool(a >= b)),
        (And, Bool(a), Bool(b)) => Ok(Bool(a && b)),
        (Or, Bool(a), Bool(b)) => Ok(Bool(a || b)),
        _ => Err(rt(span, "type error in binary operator")),
    }
}

pub(crate) fn apply_unop(op: UnOp, v: Value, span: Span) -> Result<Value, RuntimeError> {
    match (op, v) {
        (UnOp::Neg, Value::Int(n)) => Ok(Value::Int(-n)),
        (UnOp::Neg, Value::Float(x)) => Ok(Value::Float(-x)),
        (UnOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
        _ => Err(rt(span, "type error in unary operator")),
    }
}

pub fn run_module(module: &Module) -> Result<Interp, RuntimeError> {
    tree::run_module(module) // Task 8 repoints this to cek::run_module
}

pub fn run_module_tree(module: &Module) -> Result<Interp, RuntimeError> {
    tree::run_module(module)
}

pub mod tree {
    use super::*;

    pub fn run_module(module: &Module) -> Result<Interp, RuntimeError> {
        let fns = fn_table(module);
        let mut interp = Interp::new();
        let Some(main) = fns.get("main").copied() else {
            return Err(rt(Span::EMPTY, "no `main` function found"));
        };
        eval_block(&mut interp, &main.body.node, &Env::new(), &fns)?;
        Ok(interp)
    }

    fn eval_block(
        interp: &mut Interp,
        b: &Block,
        env: &Env,
        fns: &Fns,
    ) -> Result<Value, RuntimeError> {
        let mut local = env.clone();
        for st in &b.stmts {
            match &st.node {
                Stmt::Let { name, value } => {
                    let v = eval_expr(interp, value, &local, fns)?;
                    local = local.extend(&[(name.clone(), v)]);
                }
                Stmt::Expr(e) => {
                    eval_expr(interp, e, &local, fns)?;
                }
            }
        }
        match &b.tail {
            Some(t) => eval_expr(interp, t, &local, fns),
            None => Ok(Value::Unit),
        }
    }

    pub(crate) fn eval_expr(
        interp: &mut Interp,
        e: &Spanned<Expr>,
        env: &Env,
        fns: &Fns,
    ) -> Result<Value, RuntimeError> {
        let span = e.span;
        match &e.node {
            Expr::Int(n) => Ok(Value::Int(*n)),
            Expr::Float(x) => Ok(Value::Float(*x)),
            Expr::Str(s) => Ok(Value::Str(s.clone())),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::Unit => Ok(Value::Unit),
            Expr::Var(name) => {
                if let Some(v) = env.get(name) {
                    Ok(v)
                } else if fns.contains_key(name.as_str()) {
                    Ok(Value::Fn(name.clone()))
                } else {
                    Err(rt(span, format!("unbound variable `{name}`")))
                }
            }
            Expr::Qualified { module, name } => {
                Err(rt(span, format!("`{module}.{name}` must be called")))
            }
            Expr::Unary { op, expr } => {
                let v = eval_expr(interp, expr, env, fns)?;
                apply_unop(*op, v, span)
            }
            Expr::Binary { op, lhs, rhs } => {
                let l = eval_expr(interp, lhs, env, fns)?;
                let r = eval_expr(interp, rhs, env, fns)?;
                apply_binop(*op, l, r, span)
            }
            Expr::If {
                cond,
                then_block,
                else_block,
            } => match eval_expr(interp, cond, env, fns)? {
                Value::Bool(true) => eval_block(interp, &then_block.node, env, fns),
                Value::Bool(false) => eval_block(interp, &else_block.node, env, fns),
                _ => Err(rt(cond.span, "if condition must be a Bool")),
            },
            Expr::Block(b) => eval_block(interp, b, env, fns),
            Expr::Call { callee, args } => {
                if let Expr::Qualified { module, name } = &callee.node {
                    if module == "io" && name == "println" {
                        let mut vals = Vec::new();
                        for a in args {
                            vals.push(eval_expr(interp, a, env, fns)?);
                        }
                        let [Value::Str(s)] = &vals[..] else {
                            return Err(rt(span, "io.println expects a single String"));
                        };
                        interp.println(s);
                        return Ok(Value::Unit);
                    }
                    return Err(rt(span, format!("unknown builtin `{module}.{name}`")));
                }
                let callee_v = eval_expr(interp, callee, env, fns)?;
                let Value::Fn(fname) = callee_v else {
                    return Err(rt(callee.span, "value is not callable"));
                };
                let fdecl = fns
                    .get(fname.as_str())
                    .copied()
                    .ok_or_else(|| rt(span, format!("unknown function `{fname}`")))?;
                if fdecl.params.len() != args.len() {
                    return Err(rt(
                        span,
                        format!(
                            "`{}` expects {} argument(s), got {}",
                            fname,
                            fdecl.params.len(),
                            args.len()
                        ),
                    ));
                }
                let mut bindings = Vec::with_capacity(fdecl.params.len());
                for (p, a) in fdecl.params.iter().zip(args) {
                    bindings.push((p.node.name.clone(), eval_expr(interp, a, env, fns)?));
                }
                let call_env = Env::new().extend(&bindings);
                eval_block(interp, &fdecl.body.node, &call_env, fns)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{parse_expr_str, parse_module};
    use crate::Session;

    fn eval_str(text: &str) -> Value {
        let (e, d) = parse_expr_str(&Session::new(), text);
        assert!(d.is_empty(), "parse diags: {d:?}");
        let e = e.unwrap();
        let mut interp = Interp::new();
        let fns = Fns::new();
        tree::eval_expr(&mut interp, &e, &Env::new(), &fns).unwrap()
    }

    fn run(src: &str) -> String {
        let (m, d) = parse_module(&Session::new(), src);
        assert!(d.is_empty(), "parse diags: {d:?}");
        run_module(&m).unwrap().output().to_string()
    }

    #[test]
    fn arithmetic() {
        assert_eq!(eval_str("1 + 2 * 3"), Value::Int(7));
        assert_eq!(eval_str("(1 + 2) * 3"), Value::Int(9));
        assert_eq!(eval_str("10 - 3 - 2"), Value::Int(5));
    }

    #[test]
    fn float_and_concat() {
        assert_eq!(eval_str("1.5 +. 2.0"), Value::Float(3.5));
        assert_eq!(eval_str(r#""a" <> "b""#), Value::Str("ab".into()));
    }

    #[test]
    fn comparison_and_if() {
        assert_eq!(eval_str("if 1 < 2 { 10 } else { 20 }"), Value::Int(10));
        assert_eq!(eval_str("if 2 < 1 { 10 } else { 20 }"), Value::Int(20));
    }

    #[test]
    fn division_by_zero_is_runtime_error() {
        let (e, _) = parse_expr_str(&Session::new(), "1 / 0");
        let e = e.unwrap();
        let mut interp = Interp::new();
        let err = tree::eval_expr(&mut interp, &e, &Env::new(), &Fns::new()).unwrap_err();
        assert_eq!(err.diag.code, "E0300");
    }

    #[test]
    fn hello_world_prints() {
        assert_eq!(
            run("pub fn main() { io.println(\"Hello, Elya!\") }\n"),
            "Hello, Elya!\n"
        );
    }

    #[test]
    fn user_functions_and_calls() {
        let src = "fn double(x) { x + x }\nfn add(a, b) { a + b }\n\
                   pub fn main() { let _ = add(double(20), 2)\n io.println(\"ok\") }\n";
        assert_eq!(run(src), "ok\n");
    }
}
