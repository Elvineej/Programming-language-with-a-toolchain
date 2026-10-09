//! Slice-2 evaluators: a shared value/env layer, the tree-walker oracle (`tree`),
//! and the CEK machine (`cek`, added in Task 8).

use crate::ast::*;
use crate::diag::Diagnostic;
use crate::span::{Span, Spanned};
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Debug)]
pub enum Value {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Unit,
    /// A top-level function, referenced by name (Slice 2 has no lambdas).
    Fn(String),
    /// A first-class captured continuation (Slice 3c). `resume(v)` re-enters it.
    Resume(Rc<cek::ResumeData>),
    /// A constructed ADT value (Slice 4a): `Cons(1, Nil)` = `Ctor("Cons", [1, Nil])`.
    /// The fields are `Rc`-shared (via `CtorArgs`) so cloning a value is O(1) —
    /// list construction is O(n), not O(n²) — and `CtorArgs`'s iterative `Drop`
    /// keeps a deep chain from overflowing the host stack on destruction.
    Ctor(String, CtorArgs),
    /// A closure (Slice 4b-1): a lambda plus the environment it captured. The
    /// effect row is NOT stored — effects thread dynamically to the call site;
    /// the row lives only in the type. Capturing `env` is an O(1) `Rc` clone.
    Closure {
        params: Rc<[String]>,
        body: Rc<Spanned<Block>>,
        env: Env,
    },
}

/// The `Rc`-shared payload of a `Value::Ctor`. Its `Drop` is iterative so that
/// dropping a deeply-nested value (a million-element `Cons` list) dismantles the
/// chain level by level instead of recursing through nested destructors on the
/// host stack. Only uniquely-owned children are dismantled here; shared children
/// are freed by their last owner.
#[derive(Clone, Debug, PartialEq)]
pub struct CtorArgs(pub Rc<Vec<Value>>);

impl Drop for CtorArgs {
    fn drop(&mut self) {
        let mut stack: Vec<Value> = Vec::new();
        if let Some(children) = Rc::get_mut(&mut self.0) {
            stack.append(children);
        }
        while let Some(mut v) = stack.pop() {
            if let Value::Ctor(_, cargs) = &mut v {
                if let Some(children) = Rc::get_mut(&mut cargs.0) {
                    stack.append(children);
                }
            }
            // `v` drops here with its children already moved out — O(1).
        }
    }
}

// Hand-written so `Value::Resume` compares `false` (continuations are not
// comparable, and the type system never lets a base-typed `==` observe one),
// without forcing `ResumeData`/`Frame` to derive `PartialEq`.
impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        use Value::*;
        match (self, other) {
            (Int(a), Int(b)) => a == b,
            (Float(a), Float(b)) => a == b,
            (Str(a), Str(b)) => a == b,
            (Bool(a), Bool(b)) => a == b,
            (Unit, Unit) => true,
            (Fn(a), Fn(b)) => a == b,
            (Ctor(n1, a1), Ctor(n2, a2)) => n1 == n2 && a1 == a2,
            _ => false,
        }
    }
}

/// Whether `name` denotes a data constructor. Constructors are `Upper`-cased
/// (the lexer guarantees it) and the resolver has already validated existence,
/// so the evaluators recognise a constructor by its leading capital — no table
/// to thread.
pub(crate) fn is_ctor_name(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_uppercase())
}

/// Try to match `value` against `pattern`, returning the variable bindings on
/// success. Recursive: a constructor pattern matches a same-named `Ctor` value
/// and its sub-patterns against the fields.
pub(crate) fn match_pattern(value: &Value, pat: &Pattern) -> Option<Vec<(String, Value)>> {
    match pat {
        Pattern::Wild => Some(Vec::new()),
        Pattern::Var(x) => Some(vec![(x.clone(), value.clone())]),
        Pattern::Lit(l) => {
            let matches = match (l, value) {
                (PatLit::Int(a), Value::Int(b)) => a == b,
                (PatLit::Bool(a), Value::Bool(b)) => a == b,
                (PatLit::Str(a), Value::Str(b)) => a == b,
                (PatLit::Unit, Value::Unit) => true,
                _ => false,
            };
            if matches {
                Some(Vec::new())
            } else {
                None
            }
        }
        Pattern::Ctor { name, args } => match value {
            Value::Ctor(vname, vargs) if vname == name && vargs.0.len() == args.len() => {
                let mut binds = Vec::new();
                for (v, p) in vargs.0.iter().zip(args.iter()) {
                    binds.extend(match_pattern(v, &p.node)?);
                }
                Some(binds)
            }
            _ => None,
        },
    }
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
        if let Decl::Fn(f) = &d.node {
            m.insert(f.name.as_str(), f);
        }
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

pub(crate) fn apply_binop(
    op: BinOp,
    l: Value,
    r: Value,
    span: Span,
) -> Result<Value, RuntimeError> {
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

/// Run `main` under the CEK machine and return its value alongside the
/// interpreter. Same machine, same evaluation order, same errors as
/// [`run_module`]; the only difference is that the result is not discarded.
pub fn run_module_value(module: &Module) -> Result<(Interp, Value), RuntimeError> {
    cek::run_module_value(module)
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
        for st in b.stmts.iter() {
            match &st.node {
                Stmt::Let { name, value, .. } => {
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
                } else if is_ctor_name(name) {
                    Ok(Value::Ctor(name.clone(), CtorArgs(Rc::new(Vec::new()))))
                // nullary constructor
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
            Expr::Lambda { params, body } => {
                let names: Rc<[String]> = params.iter().map(|p| p.node.name.clone()).collect();
                Ok(Value::Closure {
                    params: names,
                    body: body.clone(),
                    env: env.clone(),
                })
            }
            Expr::Call { callee, args } => {
                if let Expr::Qualified { module, name } = &callee.node {
                    if module == "io" && name == "println" {
                        let mut vals = Vec::new();
                        for a in args.iter() {
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
                // Constructor application: `Cons(1, Nil)` builds a `Ctor` value.
                if let Expr::Var(name) = &callee.node {
                    if is_ctor_name(name) {
                        let mut vals = Vec::with_capacity(args.len());
                        for a in args.iter() {
                            vals.push(eval_expr(interp, a, env, fns)?);
                        }
                        return Ok(Value::Ctor(name.clone(), CtorArgs(Rc::new(vals))));
                    }
                }
                let callee_v = eval_expr(interp, callee, env, fns)?;
                match callee_v {
                    Value::Fn(fname) => {
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
                        for (p, a) in fdecl.params.iter().zip(args.iter()) {
                            bindings.push((p.node.name.clone(), eval_expr(interp, a, env, fns)?));
                        }
                        let call_env = Env::new().extend(&bindings);
                        eval_block(interp, &fdecl.body.node, &call_env, fns)
                    }
                    Value::Closure {
                        params,
                        body,
                        env: cenv,
                    } => {
                        if params.len() != args.len() {
                            return Err(rt(
                                span,
                                "closure applied to the wrong number of arguments",
                            ));
                        }
                        let mut bindings = Vec::with_capacity(params.len());
                        for (name, a) in params.iter().zip(args.iter()) {
                            bindings.push((name.clone(), eval_expr(interp, a, env, fns)?));
                        }
                        let call_env = cenv.extend(&bindings);
                        eval_block(interp, &body.node, &call_env, fns)
                    }
                    // A bare constructor value (`Some`, `Cons`) applied: append the
                    // args to build the saturated `Ctor`. Saturation is guaranteed
                    // by the type checker (§2.3, §6).
                    Value::Ctor(name, existing) => {
                        let mut vals: Vec<Value> = (*existing.0).clone();
                        for a in args.iter() {
                            vals.push(eval_expr(interp, a, env, fns)?);
                        }
                        Ok(Value::Ctor(name, CtorArgs(Rc::new(vals))))
                    }
                    _ => Err(rt(callee.span, "value is not callable")),
                }
            }
            Expr::Handle { .. } | Expr::Resume { .. } => {
                Err(rt(span, "effects are not evaluated yet (Slice 3c)"))
            }
            Expr::Match { scrutinee, arms } => {
                let v = eval_expr(interp, scrutinee, env, fns)?;
                for arm in arms.iter() {
                    if let Some(binds) = match_pattern(&v, &arm.node.pat.node) {
                        let arm_env = env.extend(&binds);
                        return eval_expr(interp, &arm.node.body, &arm_env, fns);
                    }
                }
                Err(rt(span, "no match arm matched (non-exhaustive)"))
            }
        }
    }
}

pub mod cek {
    use super::{
        apply_binop, apply_unop, fn_table, is_ctor_name, match_pattern, rt, CtorArgs, Env, Fns,
        Interp, RuntimeError, Value,
    };
    use crate::ast::*;
    use crate::diag::Diagnostic;
    use crate::span::{Span, Spanned};
    use std::collections::HashMap;
    use std::rc::Rc;

    /// Operation name -> its declaring effect. A call to one of these names is a
    /// *perform*. Threaded like `fns`. Since slice 5c-1 it is the shared
    /// `ast::op_effects` index -- the one Core lowering reads -- rather than a
    /// local rebuild: op names are unique per module (E0202), so the evaluator,
    /// lowering and inference cannot disagree about an op's effect.
    type Ops<'a> = HashMap<String, String>;

    fn op_table<'a>(module: &'a Module) -> Ops<'a> {
        crate::ast::op_effects(module)
    }

    /// A first-class captured continuation: the frames above the handler at the
    /// perform point (`captured`, top-first), the handler to re-install beneath
    /// them (deep handler), the environment its clauses run in, and the one-shot
    /// consumed flag (`E0425` on a second `resume`; 3d relaxes this for `multi`).
    #[derive(Debug)]
    pub struct ResumeData {
        captured: Vec<Frame>,
        handler: Rc<Handler>,
        ret_env: Env,
        consumed: std::cell::Cell<bool>,
    }

    #[derive(Clone, Debug)]
    enum CalleeSlot {
        Pending,                                  // still evaluating the callee expression
        Builtin(&'static str),                    // e.g. "io.println"
        Value(Value),                             // an evaluated callee (a Value::Fn)
        Operation { effect: String, op: String }, // a perform of an effect op
        Ctor { name: String },                    // a data-constructor application
    }

    // Frames own their AST via cheap `Rc` clones (the 3a `Box`->`Rc` groundwork),
    // so `Frame`/`Kont`/`State` are lifetime-free — a prerequisite for capturing a
    // continuation into a first-class `Value` (Slice 3c). Sliced positions carry
    // an `Rc<[_]>` plus a `usize` cursor (no per-step reslice), keeping the per-step
    // cost O(1) so the pinned TCE depth is unchanged.
    #[derive(Clone, Debug)]
    enum Frame {
        BinRight {
            op: BinOp,
            rhs: Rc<Spanned<Expr>>,
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
            then_blk: Rc<Spanned<Block>>,
            else_blk: Rc<Spanned<Block>>,
            env: Env,
            span: Span,
        },
        LetCont {
            name: String,
            stmts: Rc<[Spanned<Stmt>]>,
            cursor: usize,
            tail: Option<Rc<Spanned<Expr>>>,
            env: Env,
        },
        SeqDrop {
            stmts: Rc<[Spanned<Stmt>]>,
            cursor: usize,
            tail: Option<Rc<Spanned<Expr>>>,
            env: Env,
        },
        CallArgs {
            callee: CalleeSlot,
            done: Vec<Value>,
            args: Rc<[Spanned<Expr>]>,
            cursor: usize,
            env: Env,
            span: Span,
        },
        // Installed by `handle`; catches the body's normal return (the return
        // clause) and is the boundary an operation searches for.
        HandleK {
            handler: Rc<Handler>,
            env: Env,
        },
        // Evaluating a `resume(arg)`; on return, re-enters the continuation.
        ResumeApply {
            resume: Value,
            span: Span,
        },
        // Evaluating a `match` scrutinee; on return, dispatch to an arm.
        MatchK {
            arms: Rc<[Spanned<MatchArm>]>,
            env: Env,
            span: Span,
        },
    }

    struct KontNode {
        frame: Frame,
        rest: Kont,
    }
    // Alias is non-recursive because the recursion goes through the named
    // `KontNode` struct (recursive type *aliases* are not allowed).
    type Kont = Option<Rc<KontNode>>;

    fn push(f: Frame, k: Kont) -> Kont {
        Some(Rc::new(KontNode { frame: f, rest: k }))
    }

    enum State {
        Eval(Rc<Spanned<Expr>>, Env, Kont),
        Return(Value, Kont),
    }

    /// Run `main` and hand back both the interpreter and the value `main`
    /// evaluated to. `run_module` throws that value away — `elya run` observes
    /// only `io.println` output — but a compiled binary prints it, so the back
    /// end's differential check needs a way to ask what it should have been.
    pub fn run_module_value(module: &Module) -> Result<(Interp, Value), RuntimeError> {
        let fns = fn_table(module);
        let ops = op_table(module);
        let mut interp = Interp::new();
        let Some(main) = fns.get("main").copied() else {
            return Err(rt(Span::EMPTY, "no `main` function found"));
        };
        let start = eval_block_state(&main.body.node, Env::new(), None);
        let v = run_loop(&mut interp, &fns, &ops, start)?;
        Ok((interp, v))
    }

    pub fn run_module(module: &Module) -> Result<Interp, RuntimeError> {
        run_module_value(module).map(|(interp, _)| interp)
    }

    // A block's tail is in tail position: evaluating it does not add a frame.
    fn eval_block_state(b: &Block, env: Env, k: Kont) -> State {
        step_block(b.stmts.clone(), 0, b.tail.clone(), env, k)
    }

    fn step_block(
        stmts: Rc<[Spanned<Stmt>]>,
        cursor: usize,
        tail: Option<Rc<Spanned<Expr>>>,
        env: Env,
        k: Kont,
    ) -> State {
        if cursor >= stmts.len() {
            return match tail {
                Some(t) => State::Eval(t, env, k), // tail position — no frame
                None => State::Return(Value::Unit, k),
            };
        }
        match &stmts[cursor].node {
            Stmt::Let { name, value, .. } => State::Eval(
                Rc::new(value.clone()),
                env.clone(),
                push(
                    Frame::LetCont {
                        name: name.clone(),
                        stmts: stmts.clone(),
                        cursor: cursor + 1,
                        tail,
                        env,
                    },
                    k,
                ),
            ),
            Stmt::Expr(e) => State::Eval(
                Rc::new(e.clone()),
                env.clone(),
                push(
                    Frame::SeqDrop {
                        stmts: stmts.clone(),
                        cursor: cursor + 1,
                        tail,
                        env,
                    },
                    k,
                ),
            ),
        }
    }

    fn run_loop(
        interp: &mut Interp,
        fns: &Fns,
        ops: &Ops,
        mut st: State,
    ) -> Result<Value, RuntimeError> {
        loop {
            interp.note_kont_depth(kont_len(kont_of(&st)));
            // The machine halts exactly when a `Return` meets an empty
            // continuation, and that value is `main`'s result. Catching it here
            // rather than letting `step` fall off the end is what lets the value
            // escape the loop at all; `run_module` still discards it, so nothing
            // about `elya run` changes.
            st = match st {
                State::Return(v, None) => return Ok(v),
                other => match step(interp, fns, ops, other)? {
                    Some(next) => next,
                    // `ret` returns `None` only for an empty continuation, which
                    // the arm above already caught. Defensive, not expected.
                    None => {
                        return Err(rt(
                            Span::EMPTY,
                            "internal: machine halted with frames pending",
                        ))
                    }
                },
            };
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

    fn kont_of(st: &State) -> &Kont {
        match st {
            State::Eval(_, _, k) => k,
            State::Return(_, k) => k,
        }
    }

    fn step(
        interp: &mut Interp,
        fns: &Fns,
        ops: &Ops,
        st: State,
    ) -> Result<Option<State>, RuntimeError> {
        match st {
            State::Eval(e, env, k) => Ok(Some(eval(fns, ops, &e, env, k)?)),
            State::Return(v, k) => ret(interp, fns, v, k),
        }
    }

    fn eval(
        fns: &Fns,
        ops: &Ops,
        e: &Spanned<Expr>,
        env: Env,
        k: Kont,
    ) -> Result<State, RuntimeError> {
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
                } else if is_ctor_name(name) {
                    Value::Ctor(name.clone(), CtorArgs(Rc::new(Vec::new()))) // nullary constructor
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
                State::Eval(expr.clone(), env, push(Frame::UnApply { op: *op, span }, k))
            }
            Expr::Binary { op, lhs, rhs } => State::Eval(
                lhs.clone(),
                env.clone(),
                push(
                    Frame::BinRight {
                        op: *op,
                        rhs: rhs.clone(),
                        env,
                        span,
                    },
                    k,
                ),
            ),
            Expr::If {
                cond,
                then_block,
                else_block,
            } => State::Eval(
                cond.clone(),
                env.clone(),
                push(
                    Frame::IfBranch {
                        then_blk: then_block.clone(),
                        else_blk: else_block.clone(),
                        env,
                        span,
                    },
                    k,
                ),
            ),
            Expr::Block(b) => step_block(b.stmts.clone(), 0, b.tail.clone(), env, k),
            Expr::Lambda { params, body } => {
                let names: Rc<[String]> = params.iter().map(|p| p.node.name.clone()).collect();
                State::Return(
                    Value::Closure {
                        params: names,
                        body: body.clone(),
                        env: env.clone(),
                    },
                    k,
                )
            }
            Expr::Call { callee, args } => {
                let slot = match &callee.node {
                    Expr::Qualified { module, name } if module == "io" && name == "println" => {
                        CalleeSlot::Builtin("io.println")
                    }
                    Expr::Qualified { module, name } => {
                        return Err(rt(span, format!("unknown builtin `{module}.{name}`")))
                    }
                    // A call to a constructor builds a value; to an operation, performs.
                    Expr::Var(name) if is_ctor_name(name) => {
                        CalleeSlot::Ctor { name: name.clone() }
                    }
                    // Locals before ops (slice 5c-2): `env` holds only local
                    // bindings (top-level functions are `fns`), so a bound name
                    // shadows an operation of the same name.
                    Expr::Var(name) if env.get(name).is_some() => CalleeSlot::Pending,
                    Expr::Var(name) => match ops.get(name.as_str()) {
                        Some(effect) => CalleeSlot::Operation {
                            effect: effect.clone(),
                            op: name.clone(),
                        },
                        None => CalleeSlot::Pending,
                    },
                    _ => CalleeSlot::Pending,
                };
                match slot {
                    CalleeSlot::Pending => State::Eval(
                        callee.clone(),
                        env.clone(),
                        push(
                            Frame::CallArgs {
                                callee: CalleeSlot::Pending,
                                done: Vec::new(),
                                args: args.clone(),
                                cursor: 0,
                                env,
                                span,
                            },
                            k,
                        ),
                    ),
                    // A zero-arg operation performs immediately (no args to eval).
                    CalleeSlot::Operation { effect, op } if args.is_empty() => {
                        perform(effect, op, Vec::new(), span, k)?
                    }
                    // Builtin or operation with args: evaluate the args, then apply.
                    other => {
                        if args.is_empty() {
                            return Err(rt(span, "builtin called with no arguments"));
                        }
                        State::Eval(
                            Rc::new(args[0].clone()),
                            env.clone(),
                            push(
                                Frame::CallArgs {
                                    callee: other,
                                    done: Vec::new(),
                                    args: args.clone(),
                                    cursor: 1,
                                    env,
                                    span,
                                },
                                k,
                            ),
                        )
                    }
                }
            }
            // Install the handler and evaluate the body under it.
            Expr::Handle { body, handler } => State::Eval(
                body.clone(),
                env.clone(),
                push(
                    Frame::HandleK {
                        handler: handler.clone(),
                        env,
                    },
                    k,
                ),
            ),
            // Evaluate resume's argument, then re-enter the captured continuation.
            Expr::Resume { arg } => {
                let resume = env
                    .get("$resume")
                    .ok_or_else(|| rt(span, "internal: `resume` outside a handler clause"))?;
                State::Eval(
                    arg.clone(),
                    env,
                    push(Frame::ResumeApply { resume, span }, k),
                )
            }
            // Evaluate the scrutinee, then dispatch to a matching arm.
            Expr::Match { scrutinee, arms } => State::Eval(
                scrutinee.clone(),
                env.clone(),
                push(
                    Frame::MatchK {
                        arms: arms.clone(),
                        env,
                        span,
                    },
                    k,
                ),
            ),
        })
    }

    fn ret(
        interp: &mut Interp,
        fns: &Fns,
        v: Value,
        k: Kont,
    ) -> Result<Option<State>, RuntimeError> {
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
            Frame::IfBranch {
                then_blk,
                else_blk,
                env,
                span,
            } => match v {
                Value::Bool(true) => eval_block_state(&then_blk.node, env, rest),
                Value::Bool(false) => eval_block_state(&else_blk.node, env, rest),
                _ => return Err(rt(span, "if condition must be a Bool")),
            },
            Frame::LetCont {
                name,
                stmts,
                cursor,
                tail,
                env,
            } => {
                let env2 = env.extend(&[(name, v)]);
                step_block(stmts, cursor, tail, env2, rest)
            }
            Frame::SeqDrop {
                stmts,
                cursor,
                tail,
                env,
            } => step_block(stmts, cursor, tail, env, rest),
            Frame::CallArgs {
                callee,
                done,
                args,
                cursor,
                env,
                span,
            } => advance_call(interp, fns, v, callee, done, args, cursor, env, span, rest)?,
            // The body returned normally (no outstanding operation): run the
            // return clause (or identity), discharging the handler.
            Frame::HandleK { handler, env } => match &handler.ret {
                Some(ret_clause) => {
                    let env2 = env.extend(&[(ret_clause.binder.clone(), v)]);
                    State::Eval(ret_clause.body.clone(), env2, rest)
                }
                None => State::Return(v, rest),
            },
            // `resume(v)` re-enters the captured continuation: deep-handler
            // semantics re-install the handler beneath the captured frames.
            Frame::ResumeApply { resume, span } => resume_apply(resume, v, span, rest)?,
            // The scrutinee returned `v`: dispatch to the first matching arm and
            // evaluate its body in the match's continuation slot (tail position).
            Frame::MatchK { arms, env, span } => {
                let mut chosen = None;
                for arm in arms.iter() {
                    if let Some(binds) = match_pattern(&v, &arm.node.pat.node) {
                        chosen = Some((arm.node.body.clone(), env.extend(&binds)));
                        break;
                    }
                }
                match chosen {
                    Some((body, arm_env)) => State::Eval(body, arm_env, rest),
                    None => return Err(rt(span, "no match arm matched (non-exhaustive)")),
                }
            }
        }))
    }

    /// Re-enter a captured continuation with value `u`. A one-shot handler
    /// enforces a single use (`E0425` on a second `resume`); a `with multi`
    /// handler permits re-entry — each resumption is an independent run of the
    /// *same immutable* captured frames. Rebuilds `Kont' = k_cap ++ [HandleK] ++
    /// k_now`, deepest-first.
    fn resume_apply(
        resume: Value,
        u: Value,
        span: Span,
        k_now: Kont,
    ) -> Result<State, RuntimeError> {
        let Value::Resume(rd) = resume else {
            return Err(rt(span, "internal: `resume` target is not a continuation"));
        };
        // One-shot enforcement — skipped for `with multi`. This is the ONLY
        // difference between one-shot and multi-shot: the re-push below is
        // identical, because `rd.captured` is an immutable owned snapshot and
        // each `f.clone()` builds a fresh, independent `Kont` (persistent frames,
        // copy-on-write `Env`) — so re-entering it more than once is sound.
        if !rd.handler.multi {
            if rd.consumed.get() {
                return Err(RuntimeError {
                    diag: Diagnostic::error("E0425", "continuation resumed more than once")
                        .with_label(
                            span,
                            "this handler is one-shot — use `with multi` for multi-shot",
                        ),
                });
            }
            rd.consumed.set(true);
        }
        let mut k = k_now;
        k = push(
            Frame::HandleK {
                handler: rd.handler.clone(),
                env: rd.ret_env.clone(),
            },
            k,
        );
        for f in rd.captured.iter().rev() {
            k = push(f.clone(), k);
        }
        Ok(State::Return(u, k))
    }

    /// Perform operation `op` of effect `effect`: walk the continuation for the
    /// nearest matching handler, split it into the captured prefix `k_cap` and
    /// the suffix `k_rest`, and run the matching clause with `resume` bound.
    fn perform(
        effect: String,
        op: String,
        args: Vec<Value>,
        span: Span,
        k: Kont,
    ) -> Result<State, RuntimeError> {
        let mut cap: Vec<Frame> = Vec::new();
        let mut cur = k;
        loop {
            let Some(node) = cur else {
                // A well-typed program never gets here (E0420 is static); defensive.
                return Err(rt(
                    span,
                    format!("internal: unhandled effect `{op}` reached the machine"),
                ));
            };
            if let Frame::HandleK { handler, env } = &node.frame {
                if handler_handles(handler, &effect, &op) {
                    return run_clause(
                        cap,
                        handler.clone(),
                        env.clone(),
                        &effect,
                        &op,
                        args,
                        span,
                        node.rest.clone(),
                    );
                }
            }
            cap.push(node.frame.clone());
            cur = node.rest.clone();
        }
    }

    /// Does this clause handle a perform of `(effect, op)`? Its op must match,
    /// and its qualifier, if written, must be `effect`. An UNQUALIFIED clause
    /// means its op's effect (slice 5c-1): op names are unique per module (E0202)
    /// and a qualifier must name the op's own effect (E0203), so the op name
    /// alone decides -- no table is threaded here.
    fn clause_matches(c: &OpClause, effect: &str, op: &str) -> bool {
        c.op == op && c.effect.as_deref().map_or(true, |e| e == effect)
    }

    fn handler_handles(handler: &Handler, effect: &str, op: &str) -> bool {
        handler
            .clauses
            .iter()
            .any(|c| clause_matches(&c.node, effect, op))
    }

    #[allow(clippy::too_many_arguments)]
    fn run_clause(
        cap: Vec<Frame>,
        handler: Rc<Handler>,
        ret_env: Env,
        effect: &str,
        op: &str,
        args: Vec<Value>,
        span: Span,
        k_rest: Kont,
    ) -> Result<State, RuntimeError> {
        let clause = handler
            .clauses
            .iter()
            .find(|c| clause_matches(&c.node, effect, op))
            .ok_or_else(|| rt(span, format!("internal: handler has no clause for `{op}`")))?;
        let rd = Rc::new(ResumeData {
            captured: cap,
            handler: handler.clone(),
            ret_env: ret_env.clone(),
            consumed: std::cell::Cell::new(false),
        });
        let mut bindings: Vec<(String, Value)> = clause
            .node
            .params
            .iter()
            .map(|p| p.node.name.clone())
            .zip(args)
            .collect();
        bindings.push(("$resume".to_string(), Value::Resume(rd)));
        let clause_env = ret_env.extend(&bindings);
        Ok(State::Eval(clause.node.body.clone(), clause_env, k_rest))
    }

    #[allow(clippy::too_many_arguments)]
    fn advance_call(
        interp: &mut Interp,
        fns: &Fns,
        v: Value,
        callee: CalleeSlot,
        mut done: Vec<Value>,
        args: Rc<[Spanned<Expr>]>,
        cursor: usize,
        env: Env,
        span: Span,
        rest: Kont,
    ) -> Result<State, RuntimeError> {
        // `v` is the callee value (if the slot was Pending) or the latest argument.
        let callee = match callee {
            CalleeSlot::Pending => CalleeSlot::Value(v),
            other => {
                done.push(v);
                other
            }
        };
        if cursor < args.len() {
            Ok(State::Eval(
                Rc::new(args[cursor].clone()),
                env.clone(),
                push(
                    Frame::CallArgs {
                        callee,
                        done,
                        args: args.clone(),
                        cursor: cursor + 1,
                        env,
                        span,
                    },
                    rest,
                ),
            ))
        } else {
            // All args evaluated — apply. NO frame is pushed here (the TCE lever).
            apply_callee(interp, fns, callee, done, span, rest)
        }
    }

    fn apply_callee(
        interp: &mut Interp,
        fns: &Fns,
        callee: CalleeSlot,
        args: Vec<Value>,
        span: Span,
        k: Kont,
    ) -> Result<State, RuntimeError> {
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
            CalleeSlot::Operation { effect, op } => perform(effect, op, args, span, k),
            CalleeSlot::Ctor { name } => {
                Ok(State::Return(Value::Ctor(name, CtorArgs(Rc::new(args))), k))
            }
            CalleeSlot::Value(Value::Closure {
                params,
                body,
                env: cenv,
            }) => {
                if params.len() != args.len() {
                    return Err(rt(span, "closure applied to the wrong number of arguments"));
                }
                let bindings: Vec<(String, Value)> = params.iter().cloned().zip(args).collect();
                let call_env = cenv.extend(&bindings);
                Ok(eval_block_state(&body.node, call_env, k)) // reuses `k` — TCE-preserving
            }
            // A bare constructor value applied: append the args to the saturated `Ctor`.
            CalleeSlot::Value(Value::Ctor(name, existing)) => {
                let mut vals: Vec<Value> = (*existing.0).clone();
                vals.extend(args);
                Ok(State::Return(Value::Ctor(name, CtorArgs(Rc::new(vals))), k))
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
