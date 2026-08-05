//! Slice-1 tree-walking interpreter.

use crate::ast::*;
use crate::diag::Diagnostic;
use crate::span::Span;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Unit,
    Func(Rc<FnDecl>),
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

/// A scope chain of variable bindings.
#[derive(Clone, Debug, Default)]
pub struct Env {
    scopes: Vec<HashMap<String, Value>>,
}

impl Env {
    pub fn new() -> Env {
        Env {
            scopes: vec![HashMap::new()],
        }
    }

    fn child(&self) -> Env {
        let mut e = self.clone();
        e.scopes.push(HashMap::new());
        e
    }

    fn define(&mut self, name: &str, v: Value) {
        self.scopes.last_mut().unwrap().insert(name.to_string(), v);
    }

    fn lookup(&self, name: &str) -> Option<Value> {
        self.scopes.iter().rev().find_map(|s| s.get(name).cloned())
    }
}

pub struct Interp {
    output: String,
    depth: usize,
    max_depth: usize,
    globals_env: Env,
}

impl Interp {
    pub fn new() -> Interp {
        Interp {
            output: String::new(),
            depth: 0,
            max_depth: 0,
            globals_env: Env::new(),
        }
    }

    pub fn output(&self) -> &str {
        &self.output
    }

    pub fn max_depth(&self) -> usize {
        self.max_depth
    }

    pub fn eval_expr(&mut self, e: &Expr, span: Span, env: &Env) -> Result<Value, RuntimeError> {
        self.depth += 1;
        self.max_depth = self.max_depth.max(self.depth);
        let result = self.eval_inner(e, span, env);
        self.depth -= 1;
        result
    }

    fn eval_inner(&mut self, e: &Expr, span: Span, env: &Env) -> Result<Value, RuntimeError> {
        match e {
            Expr::Int(n) => Ok(Value::Int(*n)),
            Expr::Float(x) => Ok(Value::Float(*x)),
            Expr::Str(s) => Ok(Value::Str(s.clone())),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::Unit => Ok(Value::Unit),
            Expr::Var(name) => env
                .lookup(name)
                .ok_or_else(|| rt(span, format!("unbound variable `{name}`"))),
            Expr::Unary { op, expr } => {
                let v = self.eval_expr(&expr.node, expr.span, env)?;
                match (op, v) {
                    (UnOp::Neg, Value::Int(n)) => Ok(Value::Int(-n)),
                    (UnOp::Neg, Value::Float(x)) => Ok(Value::Float(-x)),
                    (UnOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
                    _ => Err(rt(span, "type error in unary operator")),
                }
            }
            Expr::Binary { op, lhs, rhs } => {
                let l = self.eval_expr(&lhs.node, lhs.span, env)?;
                let r = self.eval_expr(&rhs.node, rhs.span, env)?;
                eval_binop(*op, l, r, span)
            }
            Expr::If {
                cond,
                then_block,
                else_block,
            } => match self.eval_expr(&cond.node, cond.span, env)? {
                Value::Bool(true) => self.eval_block(&then_block.node, env),
                Value::Bool(false) => self.eval_block(&else_block.node, env),
                _ => Err(rt(cond.span, "if condition must be a Bool")),
            },
            Expr::Block(b) => self.eval_block(b, env),
            Expr::Qualified { module, name } => {
                Err(rt(span, format!("`{module}.{name}` must be called")))
            }
            Expr::Call { callee, args } => {
                // Builtin call: io.println(...)
                if let Expr::Qualified { module, name } = &callee.node {
                    let full = format!("{module}.{name}");
                    let mut vals = Vec::new();
                    for a in args {
                        vals.push(self.eval_expr(&a.node, a.span, env)?);
                    }
                    return self.call_builtin(&full, vals, span);
                }
                // User function call.
                let callee_val = self.eval_expr(&callee.node, callee.span, env)?;
                let Value::Func(func) = callee_val else {
                    return Err(rt(callee.span, "value is not callable"));
                };
                if func.params.len() != args.len() {
                    return Err(rt(
                        span,
                        format!(
                            "`{}` expects {} argument(s), got {}",
                            func.name,
                            func.params.len(),
                            args.len()
                        ),
                    ));
                }
                let mut call_env = self.globals_env.child();
                for (p, a) in func.params.iter().zip(args) {
                    let v = self.eval_expr(&a.node, a.span, env)?;
                    call_env.define(&p.node.name, v);
                }
                self.eval_block(&func.body.node, &call_env)
            }
        }
    }

    fn eval_block(&mut self, b: &Block, env: &Env) -> Result<Value, RuntimeError> {
        let mut local = env.child();
        for st in &b.stmts {
            match &st.node {
                Stmt::Let { name, value } => {
                    let v = self.eval_expr(&value.node, value.span, &local)?;
                    local.define(name, v);
                }
                Stmt::Expr(e) => {
                    self.eval_expr(&e.node, e.span, &local)?;
                }
            }
        }
        match &b.tail {
            Some(tail) => self.eval_expr(&tail.node, tail.span, &local),
            None => Ok(Value::Unit),
        }
    }

    fn call_builtin(
        &mut self,
        full: &str,
        args: Vec<Value>,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        match full {
            "io.println" => {
                let [Value::Str(s)] = &args[..] else {
                    return Err(rt(span, "io.println expects a single String"));
                };
                self.output.push_str(s);
                self.output.push('\n');
                Ok(Value::Unit)
            }
            _ => Err(rt(span, format!("unknown builtin `{full}`"))),
        }
    }
}

impl Default for Interp {
    fn default() -> Self {
        Interp::new()
    }
}

fn eval_binop(op: BinOp, l: Value, r: Value, span: Span) -> Result<Value, RuntimeError> {
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

pub fn run_module(module: &Module) -> Result<Interp, RuntimeError> {
    let mut interp = Interp::new();
    // Install top-level functions as values in the globals env.
    for d in &module.decls {
        match &d.node {
            Decl::Fn(f) => {
                interp
                    .globals_env
                    .define(&f.name, Value::Func(Rc::new(f.clone())));
            }
        }
    }
    // Find and call `main`.
    let main = module.decls.iter().find_map(|d| match &d.node {
        Decl::Fn(f) if f.name == "main" => Some(f.clone()),
        _ => None,
    });
    let Some(main) = main else {
        return Err(rt(Span::EMPTY, "no `main` function found"));
    };
    let env = interp.globals_env.clone();
    interp.eval_block(&main.body.node, &env)?;
    Ok(interp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{parse_expr_str, parse_module};
    use crate::Session;

    fn eval_str(text: &str) -> Value {
        let (e, diags) = parse_expr_str(&Session::new(), text);
        assert!(diags.is_empty(), "parse diags: {diags:?}");
        let e = e.unwrap();
        let mut interp = Interp::new();
        let env = Env::new();
        interp.eval_expr(&e.node, e.span, &env).unwrap()
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
    fn float_arithmetic_and_concat() {
        assert_eq!(eval_str("1.5 +. 2.0"), Value::Float(3.5));
        assert_eq!(eval_str(r#""a" <> "b""#), Value::Str("ab".into()));
    }

    #[test]
    fn comparison_and_if() {
        assert_eq!(eval_str("if 1 < 2 { 10 } else { 20 }"), Value::Int(10));
        assert_eq!(eval_str("if 2 < 1 { 10 } else { 20 }"), Value::Int(20));
    }

    #[test]
    fn division_by_zero_is_runtime_error_not_panic() {
        let (e, _) = parse_expr_str(&Session::new(), "1 / 0");
        let e = e.unwrap();
        let mut interp = Interp::new();
        let env = Env::new();
        let err = interp.eval_expr(&e.node, e.span, &env).unwrap_err();
        assert_eq!(err.diag.code, "E0300");
    }

    #[test]
    fn hello_world_prints() {
        let src = "pub fn main() / {IO} {\n  io.println(\"Hello, Lyra!\")\n}\n";
        assert_eq!(run(src), "Hello, Lyra!\n");
    }

    #[test]
    fn user_functions_and_calls() {
        // `int.show` is a stdlib fn that doesn't exist until later slices, so we
        // observe control flow by printing a string literal.
        let src = "fn double(x) { x + x }\nfn add(a, b) { a + b }\npub fn main() { let _ = add(double(20), 2)\n io.println(\"ok\") }\n";
        assert_eq!(run(src), "ok\n");
    }
}
