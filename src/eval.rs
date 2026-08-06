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
    fn note_kont_depth(&mut self, d: usize) {
        self.peak_kont = self.peak_kont.max(d);
    }
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
    cek::run_module(module)
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

pub mod cek {
    use super::{apply_binop, apply_unop, fn_table, rt, Env, Fns, Interp, RuntimeError, Value};
    use crate::ast::*;
    use crate::span::{Span, Spanned};
    use std::rc::Rc;

    #[derive(Clone)]
    enum CalleeSlot {
        Pending,               // still evaluating the callee expression
        Builtin(&'static str), // e.g. "io.println"
        Value(Value),          // an evaluated callee (a Value::Fn)
    }

    #[derive(Clone)]
    enum Frame<'a> {
        BinRight {
            op: BinOp,
            rhs: &'a Spanned<Expr>,
            env: Env,
            span: Span,
        },
        BinApply {
            op: BinOp,
            lval: Value,
            span: Span,
        },
        UnApply {
            op: UnOp,
            span: Span,
        },
        IfBranch {
            then_blk: &'a Block,
            else_blk: &'a Block,
            env: Env,
            span: Span,
        },
        LetCont {
            name: &'a str,
            rest: &'a [Spanned<Stmt>],
            tail: Option<&'a Spanned<Expr>>,
            env: Env,
        },
        SeqDrop {
            rest: &'a [Spanned<Stmt>],
            tail: Option<&'a Spanned<Expr>>,
            env: Env,
        },
        CallArgs {
            callee: CalleeSlot,
            done: Vec<Value>,
            pending: &'a [Spanned<Expr>],
            env: Env,
            span: Span,
        },
    }

    struct KontNode<'a> {
        frame: Frame<'a>,
        rest: Kont<'a>,
    }
    // Alias is non-recursive because the recursion goes through the named
    // `KontNode` struct (recursive type *aliases* are not allowed).
    type Kont<'a> = Option<Rc<KontNode<'a>>>;

    fn push<'a>(f: Frame<'a>, k: Kont<'a>) -> Kont<'a> {
        Some(Rc::new(KontNode { frame: f, rest: k }))
    }

    enum State<'a> {
        Eval(&'a Spanned<Expr>, Env, Kont<'a>),
        Return(Value, Kont<'a>),
    }

    pub fn run_module(module: &Module) -> Result<Interp, RuntimeError> {
        let fns = fn_table(module);
        let mut interp = Interp::new();
        let Some(main) = fns.get("main").copied() else {
            return Err(rt(Span::EMPTY, "no `main` function found"));
        };
        let start = eval_block_state(&main.body.node, Env::new(), None);
        run_loop(&mut interp, &fns, start)?;
        Ok(interp)
    }

    // A block's tail is in tail position: evaluating it does not add a frame.
    fn eval_block_state<'a>(b: &'a Block, env: Env, k: Kont<'a>) -> State<'a> {
        step_block(&b.stmts, b.tail.as_deref(), env, k)
    }

    fn step_block<'a>(
        stmts: &'a [Spanned<Stmt>],
        tail: Option<&'a Spanned<Expr>>,
        env: Env,
        k: Kont<'a>,
    ) -> State<'a> {
        match stmts.split_first() {
            None => match tail {
                Some(t) => State::Eval(t, env, k), // tail position — no frame
                None => State::Return(Value::Unit, k),
            },
            Some((st, rest)) => match &st.node {
                Stmt::Let { name, value } => State::Eval(
                    value,
                    env.clone(),
                    push(Frame::LetCont { name, rest, tail, env }, k),
                ),
                Stmt::Expr(e) => {
                    State::Eval(e, env.clone(), push(Frame::SeqDrop { rest, tail, env }, k))
                }
            },
        }
    }

    fn run_loop<'a>(
        interp: &mut Interp,
        fns: &'a Fns<'a>,
        mut st: State<'a>,
    ) -> Result<(), RuntimeError> {
        loop {
            interp.note_kont_depth(kont_len(kont_of(&st)));
            match step(interp, fns, st)? {
                Some(next) => st = next,
                None => return Ok(()),
            }
        }
    }

    fn kont_len(k: &Kont) -> usize {
        let mut n = 0;
        let mut cur = k;
        while let Some(node) = cur {
            n += 1;
            cur = &node.rest;
        }
        n
    }

    fn kont_of<'a, 'b>(st: &'b State<'a>) -> &'b Kont<'a> {
        match st {
            State::Eval(_, _, k) => k,
            State::Return(_, k) => k,
        }
    }

    fn step<'a>(
        interp: &mut Interp,
        fns: &'a Fns<'a>,
        st: State<'a>,
    ) -> Result<Option<State<'a>>, RuntimeError> {
        match st {
            State::Eval(e, env, k) => Ok(Some(eval(fns, e, env, k)?)),
            State::Return(v, k) => ret(interp, fns, v, k),
        }
    }

    fn eval<'a>(
        fns: &'a Fns<'a>,
        e: &'a Spanned<Expr>,
        env: Env,
        k: Kont<'a>,
    ) -> Result<State<'a>, RuntimeError> {
        let span = e.span;
        Ok(match &e.node {
            Expr::Int(n) => State::Return(Value::Int(*n), k),
            Expr::Float(x) => State::Return(Value::Float(*x), k),
            Expr::Str(s) => State::Return(Value::Str(s.clone()), k),
            Expr::Bool(b) => State::Return(Value::Bool(*b), k),
            Expr::Unit => State::Return(Value::Unit, k),
            Expr::Var(name) => {
                let v = if let Some(v) = env.get(name) {
                    v
                } else if fns.contains_key(name.as_str()) {
                    Value::Fn(name.clone())
                } else {
                    return Err(rt(span, format!("unbound variable `{name}`")));
                };
                State::Return(v, k)
            }
            Expr::Qualified { module, name } => {
                return Err(rt(span, format!("`{module}.{name}` must be called")))
            }
            Expr::Unary { op, expr } => {
                State::Eval(expr, env, push(Frame::UnApply { op: *op, span }, k))
            }
            Expr::Binary { op, lhs, rhs } => State::Eval(
                lhs,
                env.clone(),
                push(Frame::BinRight { op: *op, rhs, env, span }, k),
            ),
            Expr::If { cond, then_block, else_block } => State::Eval(
                cond,
                env.clone(),
                push(
                    Frame::IfBranch {
                        then_blk: &then_block.node,
                        else_blk: &else_block.node,
                        env,
                        span,
                    },
                    k,
                ),
            ),
            Expr::Block(b) => step_block(&b.stmts, b.tail.as_deref(), env, k),
            Expr::Call { callee, args } => {
                let slot = match &callee.node {
                    Expr::Qualified { module, name } if module == "io" && name == "println" => {
                        CalleeSlot::Builtin("io.println")
                    }
                    Expr::Qualified { module, name } => {
                        return Err(rt(span, format!("unknown builtin `{module}.{name}`")))
                    }
                    _ => CalleeSlot::Pending,
                };
                match slot {
                    CalleeSlot::Pending => State::Eval(
                        callee,
                        env.clone(),
                        push(
                            Frame::CallArgs {
                                callee: CalleeSlot::Pending,
                                done: Vec::new(),
                                pending: args,
                                env,
                                span,
                            },
                            k,
                        ),
                    ),
                    builtin => match args.split_first() {
                        Some((first, rest)) => State::Eval(
                            first,
                            env.clone(),
                            push(
                                Frame::CallArgs {
                                    callee: builtin,
                                    done: Vec::new(),
                                    pending: rest,
                                    env,
                                    span,
                                },
                                k,
                            ),
                        ),
                        None => return Err(rt(span, "builtin called with no arguments")),
                    },
                }
            }
        })
    }

    fn ret<'a>(
        interp: &mut Interp,
        fns: &'a Fns<'a>,
        v: Value,
        k: Kont<'a>,
    ) -> Result<Option<State<'a>>, RuntimeError> {
        let Some(node) = k else {
            return Ok(None); // final result; output already captured via io.println
        };
        let (frame, rest) = match Rc::try_unwrap(node) {
            Ok(node) => (node.frame, node.rest),
            Err(shared) => (shared.frame.clone(), shared.rest.clone()),
        };
        Ok(Some(match frame {
            Frame::BinRight { op, rhs, env, span } => {
                State::Eval(rhs, env, push(Frame::BinApply { op, lval: v, span }, rest))
            }
            Frame::BinApply { op, lval, span } => {
                State::Return(apply_binop(op, lval, v, span)?, rest)
            }
            Frame::UnApply { op, span } => State::Return(apply_unop(op, v, span)?, rest),
            Frame::IfBranch { then_blk, else_blk, env, span } => match v {
                Value::Bool(true) => eval_block_state(then_blk, env, rest),
                Value::Bool(false) => eval_block_state(else_blk, env, rest),
                _ => return Err(rt(span, "if condition must be a Bool")),
            },
            Frame::LetCont { name, rest: stmts, tail, env } => {
                let env2 = env.extend(&[(name.to_string(), v)]);
                step_block(stmts, tail, env2, rest)
            }
            Frame::SeqDrop { rest: stmts, tail, env } => step_block(stmts, tail, env, rest),
            Frame::CallArgs { callee, done, pending, env, span } => {
                advance_call(interp, fns, v, callee, done, pending, env, span, rest)?
            }
        }))
    }

    #[allow(clippy::too_many_arguments)]
    fn advance_call<'a>(
        interp: &mut Interp,
        fns: &'a Fns<'a>,
        v: Value,
        callee: CalleeSlot,
        mut done: Vec<Value>,
        pending: &'a [Spanned<Expr>],
        env: Env,
        span: Span,
        rest: Kont<'a>,
    ) -> Result<State<'a>, RuntimeError> {
        // `v` is the callee value (if the slot was Pending) or the latest argument.
        let callee = match callee {
            CalleeSlot::Pending => CalleeSlot::Value(v),
            other => {
                done.push(v);
                other
            }
        };
        match pending.split_first() {
            Some((next, more)) => Ok(State::Eval(
                next,
                env.clone(),
                push(Frame::CallArgs { callee, done, pending: more, env, span }, rest),
            )),
            // All args evaluated — apply. NO frame is pushed here (the TCE lever).
            None => apply_callee(interp, fns, callee, done, span, rest),
        }
    }

    fn apply_callee<'a>(
        interp: &mut Interp,
        fns: &'a Fns<'a>,
        callee: CalleeSlot,
        args: Vec<Value>,
        span: Span,
        k: Kont<'a>,
    ) -> Result<State<'a>, RuntimeError> {
        match callee {
            CalleeSlot::Builtin("io.println") => {
                let [Value::Str(s)] = &args[..] else {
                    return Err(rt(span, "io.println expects a single String"));
                };
                interp.println(s);
                Ok(State::Return(Value::Unit, k))
            }
            CalleeSlot::Builtin(other) => Err(rt(span, format!("unknown builtin `{other}`"))),
            CalleeSlot::Value(Value::Fn(name)) => {
                let fdecl = fns
                    .get(name.as_str())
                    .copied()
                    .ok_or_else(|| rt(span, format!("unknown function `{name}`")))?;
                if fdecl.params.len() != args.len() {
                    return Err(rt(
                        span,
                        format!(
                            "`{}` expects {} argument(s), got {}",
                            name,
                            fdecl.params.len(),
                            args.len()
                        ),
                    ));
                }
                let bindings: Vec<(String, Value)> = fdecl
                    .params
                    .iter()
                    .map(|p| p.node.name.clone())
                    .zip(args)
                    .collect();
                let call_env = Env::new().extend(&bindings);
                Ok(eval_block_state(&fdecl.body.node, call_env, k)) // reuses `k` — no frame
            }
            CalleeSlot::Value(_) => Err(rt(span, "value is not callable")),
            CalleeSlot::Pending => Err(rt(span, "internal: unresolved callee")),
        }
    }
}

#[cfg(test)]
mod cek_tests {
    use super::*;
    use crate::parse::parse_module;
    use crate::Session;

    fn run(src: &str) -> String {
        let (m, d) = parse_module(&Session::new(), src);
        assert!(d.is_empty(), "parse: {d:?}");
        cek::run_module(&m).unwrap().output().to_string()
    }

    #[test]
    fn hello_world_on_cek() {
        assert_eq!(
            run("pub fn main() { io.println(\"Hello, Elya!\") }\n"),
            "Hello, Elya!\n"
        );
    }

    #[test]
    fn arithmetic_functions_if_on_cek() {
        let src = "fn double(x) { x + x }\npub fn main() { \
            let a = double(20)\n let b = a + 2\n \
            if b == 42 { io.println(\"forty-two\") } else { io.println(\"nope\") } }\n";
        assert_eq!(run(src), "forty-two\n");
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
