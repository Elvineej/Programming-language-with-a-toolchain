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
    /// Shared, so reading a variable or passing an argument is O(1) whatever
    /// the length (the 2026-10-30 review's F2: `byte_at` in a loop copied the
    /// whole string at every read, so `replay.parse` was quadratic).
    Str(Rc<str>),
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
    /// An operation caught by a `with any` handler (2026-10-16): its effect, its
    /// name and its arguments. Only `op.*` builtins look inside.
    Op {
        effect: String,
        op: String,
        args: Rc<Vec<Value>>,
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
            // Two operations are equal when they are the same question.
            (
                Op {
                    effect: e1,
                    op: o1,
                    args: a1,
                },
                Op {
                    effect: e2,
                    op: o2,
                    args: a2,
                },
            ) => e1 == e2 && o1 == o2 && a1 == a2,
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
                (PatLit::Str(a), Value::Str(b)) => a.as_str() == &**b,
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

/// What answers the host's builtins (spec 2026-10-16-elya-cli-record-replay):
/// the live host, the live host while every answer is written down, or a
/// trace whose answers are read back in order.
#[derive(Clone, Debug, Default)]
pub enum World {
    #[default]
    Live,
    /// `(question, answer)`, both in literal syntax, in the order asked.
    Record(Vec<(String, String)>),
    Replay {
        entries: Vec<(String, String)>,
        next: usize,
    },
}

pub struct Interp {
    /// The module's constructors and operation signatures, for `op.decode`.
    types: Rc<crate::literal::TypeInfo>,
    /// Who answers `time.*` (record and replay).
    pub world: World,
    output: String,
    peak_kont: usize,
    steps: u64,
    kont_work: u64,
}

impl Interp {
    pub fn new() -> Interp {
        Interp {
            types: Rc::new(crate::literal::TypeInfo::default()),
            world: World::Live,
            output: String::new(),
            peak_kont: 0,
            steps: 0,
            kont_work: 0,
        }
    }
    pub fn output(&self) -> &str {
        &self.output
    }
    pub fn peak_kont_depth(&self) -> usize {
        self.peak_kont
    }
    /// The machine's work, timing-free: steps taken plus every continuation node
    /// a whole-continuation operation visited or copied (the depth probe, a
    /// perform's search and capture, a resume's re-installation). A program that
    /// does linear work costs linearly; `tests/eval_complexity.rs` pins it.
    pub fn cost(&self) -> u64 {
        self.steps + self.kont_work
    }
    fn note_kont_work(&mut self, n: usize) {
        self.kont_work += n as u64;
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

fn overflow(span: Span) -> RuntimeError {
    rt(span, "integer overflow")
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
        // `Int` arithmetic is exact or fails by name (spec
        // 2026-10-10-elya-checked-integer-arithmetic, D1): never a wrapped value,
        // never a host panic. Native code traps on the same inputs.
        (Add, Int(a), Int(b)) => a.checked_add(b).map(Int).ok_or_else(|| overflow(span)),
        (Sub, Int(a), Int(b)) => a.checked_sub(b).map(Int).ok_or_else(|| overflow(span)),
        (Mul, Int(a), Int(b)) => a.checked_mul(b).map(Int).ok_or_else(|| overflow(span)),
        (Div, Int(_), Int(0)) => Err(rt(span, "division by zero")),
        (Div, Int(a), Int(b)) => a.checked_div(b).map(Int).ok_or_else(|| overflow(span)),
        (Rem, Int(_), Int(0)) => Err(rt(span, "remainder by zero")),
        // `MIN % -1` is exactly 0; `checked_rem` would call it an overflow.
        (Rem, Int(_), Int(-1)) => Ok(Int(0)),
        (Rem, Int(a), Int(b)) => Ok(Int(a % b)),
        (AddF, Float(a), Float(b)) => Ok(Float(a + b)),
        (SubF, Float(a), Float(b)) => Ok(Float(a - b)),
        (MulF, Float(a), Float(b)) => Ok(Float(a * b)),
        (DivF, Float(a), Float(b)) => Ok(Float(a / b)),
        (Concat, Str(a), Str(b)) => {
            let mut s = String::with_capacity(a.len() + b.len());
            s.push_str(&a);
            s.push_str(&b);
            Ok(Str(s.into()))
        }
        (Eq, a, b) => structural_eq(&a, &b, span).map(Bool),
        (Ne, a, b) => structural_eq(&a, &b, span).map(|e| Bool(!e)),
        (Lt, Int(a), Int(b)) => Ok(Bool(a < b)),
        (Le, Int(a), Int(b)) => Ok(Bool(a <= b)),
        (Gt, Int(a), Int(b)) => Ok(Bool(a > b)),
        (Ge, Int(a), Int(b)) => Ok(Bool(a >= b)),
        // IEEE ordered: false with a NaN (spec 2026-10-24-elya-float-part-2 D1).
        (LtF, Float(a), Float(b)) => Ok(Bool(a < b)),
        (LeF, Float(a), Float(b)) => Ok(Bool(a <= b)),
        (GtF, Float(a), Float(b)) => Ok(Bool(a > b)),
        (GeF, Float(a), Float(b)) => Ok(Bool(a >= b)),
        (And, Bool(a), Bool(b)) => Ok(Bool(a && b)),
        (Or, Bool(a), Bool(b)) => Ok(Bool(a || b)),
        _ => Err(rt(span, "type error in binary operator")),
    }
}

/// `==` on data (spec 2026-10-16-elya-structural-equality, D2): the same
/// constructor and equal fields, strings by their bytes. An explicit worklist,
/// never recursion: comparing two million-element lists recursed once per cell
/// and overflowed the host stack. E0436 keeps functions and continuations
/// out; one arriving anyway is an internal error by name.
fn structural_eq(a: &Value, b: &Value, span: Span) -> Result<bool, RuntimeError> {
    use Value::*;
    let mut work: Vec<(&Value, &Value)> = vec![(a, b)];
    while let Some((x, y)) = work.pop() {
        match (x, y) {
            (Int(p), Int(q)) => {
                if p != q {
                    return Ok(false);
                }
            }
            (Float(p), Float(q)) => {
                if p != q {
                    return Ok(false);
                }
            }
            (Str(p), Str(q)) => {
                if p != q {
                    return Ok(false);
                }
            }
            (Bool(p), Bool(q)) => {
                if p != q {
                    return Ok(false);
                }
            }
            (Unit, Unit) => {}
            (Ctor(n1, a1), Ctor(n2, a2)) => {
                if n1 != n2 || a1.0.len() != a2.0.len() {
                    return Ok(false);
                }
                // No shortcut for a shared value: a `Float` NaN in it is not
                // equal to itself, so sharing must not decide (the review).
                work.extend(a1.0.iter().zip(a2.0.iter()));
            }
            // N3: two operations caught by `with any` compare by what they are
            // (E0434 makes their arguments data).
            (
                Op {
                    effect: e1,
                    op: o1,
                    args: a1,
                },
                Op {
                    effect: e2,
                    op: o2,
                    args: a2,
                },
            ) => {
                if e1 != e2 || o1 != o2 || a1.len() != a2.len() {
                    return Ok(false);
                }
                work.extend(a1.iter().zip(a2.iter()));
            }
            _ => {
                return Err(rt(
                    span,
                    "`==` reached a value that is not data (E0436 should have rejected it)",
                ))
            }
        }
    }
    Ok(true)
}

pub use crate::resolve::INT_BINARY_BUILTINS;

/// `int.<name>(a, b)`. A shift amount outside `0..=63` fails by name.
pub(crate) fn int_binary(name: &str, args: &[Value], span: Span) -> Result<Value, RuntimeError> {
    let [Value::Int(a), Value::Int(b)] = args else {
        return Err(rt(span, format!("int.{name} expects two Ints")));
    };
    let (a, b) = (*a, *b);
    let shift = |b: i64| -> Result<u32, RuntimeError> {
        if (0..=63).contains(&b) {
            Ok(b as u32)
        } else {
            Err(rt(
                span,
                format!("int.{name}: a shift by {b} is outside 0..=63"),
            ))
        }
    };
    let v = match name {
        "wrapping_add" => a.wrapping_add(b),
        "wrapping_sub" => a.wrapping_sub(b),
        "wrapping_mul" => a.wrapping_mul(b),
        "and" => a & b,
        "or" => a | b,
        "xor" => a ^ b,
        "shl" => a << shift(b)?,
        "shr" => a >> shift(b)?,
        "ushr" => ((a as u64) >> shift(b)?) as i64,
        _ => return Err(rt(span, format!("unknown builtin `int.{name}`"))),
    };
    Ok(Value::Int(v))
}

/// `float.<name>(x)` (spec 2026-10-24-elya-float-part-2 D3, D4): `of_int`
/// rounds to nearest (ties to even); `truncate` and `round` (halves away from
/// zero) fail by name on NaN and outside `Int`'s range.
pub(crate) fn float_conversion(
    name: &str,
    args: &[Value],
    span: Span,
) -> Result<Value, RuntimeError> {
    match (name, args) {
        ("of_int", [Value::Int(n)]) => Ok(Value::Float(*n as f64)),
        ("truncate" | "round", [Value::Float(x)]) => {
            if x.is_nan() {
                return Err(rt(span, format!("float.{name}: NaN has no Int value")));
            }
            let t = if name == "round" {
                x.round()
            } else {
                x.trunc()
            };
            // [-2^63, 2^63): both bounds are exact floats.
            if (-9223372036854775808.0..9223372036854775808.0).contains(&t) {
                Ok(Value::Int(t as i64))
            } else {
                Err(rt(
                    span,
                    format!(
                        "float.{name}: {} is outside Int's range",
                        crate::literal::float_text(*x)
                    ),
                ))
            }
        }
        _ => Err(rt(span, format!("float.{name}: a wrong argument"))),
    }
}

/// `string.length`, `string.byte_at`, `string.slice` (spec
/// 2026-10-24-elya-trace-text D1): a string by byte. Each failure is named; a
/// slice may not split a UTF-8 character, so a `String` stays UTF-8.
pub(crate) fn string_builtin(
    name: &str,
    args: &[Value],
    span: Span,
) -> Result<Value, RuntimeError> {
    match (name, args) {
        ("length", [Value::Str(s)]) => Ok(Value::Int(s.len() as i64)),
        ("byte_at", [Value::Str(s), Value::Int(i)]) => {
            match usize::try_from(*i).ok().and_then(|k| s.as_bytes().get(k)) {
                Some(b) => Ok(Value::Int(i64::from(*b))),
                None => Err(rt(
                    span,
                    format!(
                        "string.byte_at: index {i} is outside a string of {} bytes",
                        s.len()
                    ),
                )),
            }
        }
        ("slice", [Value::Str(s), Value::Int(from), Value::Int(to)]) => {
            let len = s.len() as i64;
            if !(0 <= *from && from <= to && *to <= len) {
                return Err(rt(
                    span,
                    format!("string.slice: {from}..{to} is not a range in a string of {len} bytes"),
                ));
            }
            let (a, b) = (*from as usize, *to as usize);
            if !s.is_char_boundary(a) || !s.is_char_boundary(b) {
                return Err(rt(
                    span,
                    format!("string.slice: {from}..{to} splits a UTF-8 character"),
                ));
            }
            Ok(Value::Str(Rc::from(&s[a..b])))
        }
        _ => Err(rt(span, format!("string.{name}: a wrong argument"))),
    }
}

pub(crate) fn apply_unop(op: UnOp, v: Value, span: Span) -> Result<Value, RuntimeError> {
    match (op, v) {
        (UnOp::Neg, Value::Int(n)) => n
            .checked_neg()
            .map(Value::Int)
            .ok_or_else(|| overflow(span)),
        (UnOp::Neg, Value::Float(x)) => Ok(Value::Float(-x)),
        (UnOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
        (UnOp::NegF, Value::Float(x)) => Ok(Value::Float(-x)),
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

/// [`run_module_value`] with the host's answers given by `world`.
pub fn run_module_in(module: &Module, world: World) -> Result<(Interp, Value), RuntimeError> {
    cek::run_module_in(module, world)
}

/// [`run_module_in`] keeping the interpreter (its output and world) when the
/// run fails.
pub fn run_module_keeping(module: &Module, world: World) -> (Interp, Result<Value, RuntimeError>) {
    cek::run_module_keeping(module, world)
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
            Expr::Str(s) => Ok(Value::Str(Rc::from(s.as_str()))),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::Unit => Ok(Value::Unit),
            // Only in a module with a parse error, which is never run.
            Expr::Error => Err(rt(span, "a statement that did not parse")),
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
                    if module == "time" && (name == "now_ms" || name == "sleep_ms") {
                        let mut vals = Vec::new();
                        for a in args.iter() {
                            vals.push(eval_expr(interp, a, env, fns)?);
                        }
                        return super::time_builtin(name, &vals, span);
                    }
                    if module == "int" && name == "to_string" {
                        let mut vals = Vec::new();
                        for a in args.iter() {
                            vals.push(eval_expr(interp, a, env, fns)?);
                        }
                        let [Value::Int(n)] = &vals[..] else {
                            return Err(rt(span, "int.to_string expects a single Int"));
                        };
                        return Ok(Value::Str(n.to_string().into()));
                    }
                    if module == "float" && name == "to_string" {
                        let mut vals = Vec::new();
                        for a in args.iter() {
                            vals.push(eval_expr(interp, a, env, fns)?);
                        }
                        let [Value::Float(x)] = &vals[..] else {
                            return Err(rt(span, "float.to_string expects a single Float"));
                        };
                        return Ok(Value::Str(crate::literal::float_text(*x).into()));
                    }
                    if module == "float"
                        && crate::resolve::FLOAT_CONVERSIONS.contains(&name.as_str())
                    {
                        let mut vals = Vec::new();
                        for a in args.iter() {
                            vals.push(eval_expr(interp, a, env, fns)?);
                        }
                        return super::float_conversion(name, &vals, span);
                    }
                    if module == "string"
                        && crate::resolve::STRING_BUILTINS.contains(&name.as_str())
                    {
                        let mut vals = Vec::new();
                        for a in args.iter() {
                            vals.push(eval_expr(interp, a, env, fns)?);
                        }
                        return super::string_builtin(name, &vals, span);
                    }
                    if module == "int" && super::INT_BINARY_BUILTINS.contains(&name.as_str()) {
                        let mut vals = Vec::new();
                        for a in args.iter() {
                            vals.push(eval_expr(interp, a, env, fns)?);
                        }
                        return super::int_binary(name, &vals, span);
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

    /// A first-class captured continuation: the segment of frames above the
    /// innermost handler at the perform point (`top`, shared, never copied), the
    /// handler boundaries the perform skipped on its way out (`skipped`, innermost
    /// first: each one's handler, environment and the segment beneath it), the
    /// answering handler to re-install beneath them all (deep handler), the
    /// environment its clauses run in, and the one-shot consumed flag (`E0425` on
    /// a second `resume`; `with multi` lifts it).
    pub struct ResumeData {
        top: Seg,
        skipped: Vec<(Rc<Handler>, Env, Seg)>,
        handler: Rc<Handler>,
        ret_env: Env,
        consumed: std::cell::Cell<bool>,
    }

    // By hand: a derived `Debug` would print every captured frame.
    impl std::fmt::Debug for ResumeData {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("ResumeData")
                .field("top_len", &seg_len(&self.top))
                .field("skipped", &self.skipped.len())
                .field("consumed", &self.consumed.get())
                .finish()
        }
    }

    #[derive(Clone, Debug)]
    enum CalleeSlot {
        Pending,                                  // still evaluating the callee expression
        Builtin(&'static str),                    // e.g. "io.println"
        IntBinary(&'static str),                  // `int.<name>` of two Ints
        FloatConversion(&'static str),            // `float.<name>` conversions
        StringBuiltin(&'static str),              // `string.<name>` by byte
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

    // The continuation is segmented at handlers (spec
    // 2026-10-10-elya-eval-linear-continuations, D1): `top` is the frames above
    // the innermost handler, `meta` the handler boundaries beneath it, each with
    // the segment of frames below it. A perform walks boundaries, never frames,
    // and shares `top` instead of copying it; a resume rebuilds only the
    // boundaries the perform skipped. Every node caches its depth (D2), so the
    // peak-depth probe is O(1) per step.

    /// One frame of a segment, and the rest of that segment.
    struct KontNode {
        frame: Frame,
        rest: Seg,
        /// Frames in the segment from this node down: `1 + seg_len(rest)`.
        len: usize,
    }
    // Alias is non-recursive because the recursion goes through the named
    // `KontNode` struct (recursive type *aliases* are not allowed).
    type Seg = Option<Rc<KontNode>>;

    /// A handler boundary: an installed `handle`. It catches the segment above
    /// it returning normally (the return clause), and it is what a perform
    /// searches for.
    struct MetaNode {
        handler: Rc<Handler>,
        env: Env,
        /// The frames between this boundary and the next one down.
        below: Seg,
        rest: Meta,
        /// The whole depth from this boundary down: the boundary counts as one
        /// frame (what `HandleK` counted), plus `below`, plus `rest`.
        depth: usize,
    }
    type Meta = Option<Rc<MetaNode>>;

    #[derive(Clone, Default)]
    struct Kont {
        top: Seg,
        meta: Meta,
    }

    impl Kont {
        fn is_empty(&self) -> bool {
            self.top.is_none() && self.meta.is_none()
        }
        fn depth(&self) -> usize {
            seg_len(&self.top) + meta_depth(&self.meta)
        }
    }

    fn seg_len(s: &Seg) -> usize {
        s.as_ref().map_or(0, |n| n.len)
    }

    fn meta_depth(m: &Meta) -> usize {
        m.as_ref().map_or(0, |n| n.depth)
    }

    // Iterative drops (D3): an abandoned 100 000-frame continuation must not
    // recurse through nested destructors on the host stack. Only uniquely-owned
    // successors are dismantled; a shared one is freed by its last owner.
    impl Drop for KontNode {
        fn drop(&mut self) {
            let mut cur = self.rest.take();
            while let Some(rc) = cur {
                cur = match Rc::try_unwrap(rc) {
                    Ok(mut node) => node.rest.take(),
                    Err(_) => None,
                };
            }
        }
    }

    impl Drop for MetaNode {
        fn drop(&mut self) {
            let mut cur = self.rest.take();
            while let Some(rc) = cur {
                cur = match Rc::try_unwrap(rc) {
                    Ok(mut node) => node.rest.take(),
                    Err(_) => None,
                };
            }
        }
    }

    fn push(f: Frame, k: Kont) -> Kont {
        let len = seg_len(&k.top) + 1;
        Kont {
            top: Some(Rc::new(KontNode {
                frame: f,
                rest: k.top,
                len,
            })),
            meta: k.meta,
        }
    }

    /// Install a handler boundary over `k`: the new `top` is empty.
    fn install(handler: Rc<Handler>, env: Env, k: Kont) -> Kont {
        let depth = 1 + k.depth();
        Kont {
            top: None,
            meta: Some(Rc::new(MetaNode {
                handler,
                env,
                below: k.top,
                rest: k.meta,
                depth,
            })),
        }
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
        run_module_in(module, super::World::Live)
    }

    /// `run_module_value` with the host answered by `world`; a replay whose
    /// trace still has entries when `main` returns is an error.
    pub fn run_module_in(
        module: &Module,
        world: super::World,
    ) -> Result<(Interp, Value), RuntimeError> {
        let (interp, v) = run_module_keeping(module, world);
        v.map(|v| (interp, v))
    }

    /// [`run_module_in`] that keeps the interpreter whether `main` returns or
    /// fails: a recording of a failing run is the one most worth replaying
    /// (the record-replay review's finding).
    pub fn run_module_keeping(
        module: &Module,
        world: super::World,
    ) -> (Interp, Result<Value, RuntimeError>) {
        let fns = fn_table(module);
        let ops = op_table(module);
        let mut interp = Interp::new();
        interp.world = world;
        interp.types = Rc::new(crate::literal::TypeInfo::of(module));
        let Some(main) = fns.get("main").copied() else {
            return (interp, Err(rt(Span::EMPTY, "no `main` function found")));
        };
        let start = eval_block_state(&main.body.node, Env::new(), Kont::default());
        let v = match run_loop(&mut interp, &fns, &ops, start) {
            Ok(v) => v,
            Err(e) => return (interp, Err(e)),
        };
        if let super::World::Replay { entries, next } = &interp.world {
            let left = entries.len() - *next;
            if let Some((q, a)) = entries.get(*next) {
                let n = *next + 1;
                let noun = if left == 1 { "entry" } else { "entries" };
                let msg = format!(
                    "replay: `main` returned with {left} unread trace {noun}, from entry {n} (`{q} = {a}`): the run asked less than the recording"
                );
                return (interp, Err(rt(Span::EMPTY, msg)));
            }
        }
        (interp, Ok(v))
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
            interp.steps += 1;
            interp.note_kont_depth(kont_of(&st).depth());
            // The machine halts exactly when a `Return` meets an empty
            // continuation, and that value is `main`'s result. Catching it here
            // rather than letting `step` fall off the end is what lets the value
            // escape the loop at all; `run_module` still discards it, so nothing
            // about `elya run` changes.
            st = match st {
                State::Return(v, k) if k.is_empty() => return Ok(v),
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
            State::Eval(e, env, k) => Ok(Some(eval(interp, fns, ops, &e, env, k)?)),
            State::Return(v, k) => ret(interp, fns, v, k),
        }
    }

    fn eval(
        interp: &mut Interp,
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
            Expr::Str(s) => State::Return(Value::Str(Rc::from(s.as_str())), k),
            Expr::Bool(b) => State::Return(Value::Bool(*b), k),
            Expr::Unit => State::Return(Value::Unit, k),
            Expr::Error => return Err(rt(span, "a statement that did not parse")),
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
                    Expr::Qualified { module, name } if module == "int" && name == "to_string" => {
                        CalleeSlot::Builtin("int.to_string")
                    }
                    Expr::Qualified { module, name }
                        if module == "float" && name == "to_string" =>
                    {
                        CalleeSlot::Builtin("float.to_string")
                    }
                    Expr::Qualified { module, name } if module == "time" && name == "now_ms" => {
                        CalleeSlot::Builtin("time.now_ms")
                    }
                    Expr::Qualified { module, name }
                        if module == "float"
                            && crate::resolve::FLOAT_CONVERSIONS.contains(&name.as_str()) =>
                    {
                        match crate::resolve::FLOAT_CONVERSIONS
                            .iter()
                            .position(|b| b == name)
                        {
                            Some(i) => {
                                CalleeSlot::FloatConversion(crate::resolve::FLOAT_CONVERSIONS[i])
                            }
                            None => {
                                return Err(rt(span, format!("unknown builtin `float.{name}`")))
                            }
                        }
                    }
                    Expr::Qualified { module, name } if module == "string" => {
                        match crate::resolve::STRING_BUILTINS
                            .iter()
                            .position(|b| b == name)
                        {
                            Some(i) => {
                                CalleeSlot::StringBuiltin(crate::resolve::STRING_BUILTINS[i])
                            }
                            None => {
                                return Err(rt(span, format!("unknown builtin `string.{name}`")))
                            }
                        }
                    }
                    Expr::Qualified { module, name } if module == "int" => {
                        match super::INT_BINARY_BUILTINS.iter().position(|b| b == name) {
                            Some(i) => CalleeSlot::IntBinary(super::INT_BINARY_BUILTINS[i]),
                            None => return Err(rt(span, format!("unknown builtin `int.{name}`"))),
                        }
                    }
                    Expr::Qualified { module, name } if module == "time" && name == "sleep_ms" => {
                        CalleeSlot::Builtin("time.sleep_ms")
                    }
                    Expr::Qualified { module, name } if module == "op" => match name.as_str() {
                        "name" => CalleeSlot::Builtin("op.name"),
                        "show" => CalleeSlot::Builtin("op.show"),
                        "perform" => CalleeSlot::Builtin("op.perform"),
                        "encode" => CalleeSlot::Builtin("op.encode"),
                        "decode" => CalleeSlot::Builtin("op.decode"),
                        _ => return Err(rt(span, format!("unknown builtin `op.{name}`"))),
                    },
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
                        perform(interp, effect, op, Vec::new(), span, k)?
                    }
                    // A zero-arg builtin (`time.now_ms()`) applies immediately.
                    CalleeSlot::Builtin(b) if args.is_empty() => {
                        apply_callee(interp, fns, CalleeSlot::Builtin(b), Vec::new(), span, k)?
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
            Expr::Handle { body, handler } => {
                State::Eval(body.clone(), env.clone(), install(handler.clone(), env, k))
            }
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
        let Kont { top, meta } = k;
        let Some(mut node) = top else {
            let Some(boundary) = meta else {
                return Ok(None); // final result; output already captured via io.println
            };
            // The segment above a handler returned normally (no outstanding
            // operation): run the return clause (or identity), discharging it.
            let rest = Kont {
                top: boundary.below.clone(),
                meta: boundary.rest.clone(),
            };
            return Ok(Some(match &boundary.handler.ret {
                Some(ret_clause) => {
                    let env2 = boundary.env.extend(&[(ret_clause.binder.clone(), v)]);
                    State::Eval(ret_clause.body.clone(), env2, rest)
                }
                None => State::Return(v, rest),
            }));
        };
        // Pop the top frame: moved out when this continuation owns it, cloned
        // when it is shared (a multi-shot continuation resumed again later).
        // `KontNode` has a `Drop`, so its fields cannot be destructured out;
        // the owned frame is swapped for an inert placeholder that is dropped
        // with the emptied node on the next line and never read.
        let (frame, seg_rest) = match Rc::get_mut(&mut node) {
            Some(owned) => (
                std::mem::replace(
                    &mut owned.frame,
                    Frame::UnApply {
                        op: UnOp::Neg,
                        span: Span::EMPTY,
                    },
                ),
                owned.rest.take(),
            ),
            None => (node.frame.clone(), node.rest.clone()),
        };
        drop(node);
        let rest = Kont {
            top: seg_rest,
            meta,
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
            // `resume(v)` re-enters the captured continuation: deep-handler
            // semantics re-install the handler beneath the captured frames.
            Frame::ResumeApply { resume, span } => resume_apply(interp, resume, v, span, rest)?,
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
    /// *same immutable* captured segment. Builds `top ++ skipped ++ [handler] ++
    /// k_now`: the answering handler becomes a boundary over `k_now`, the skipped
    /// boundaries are rebuilt over it, and `top` is reused as is -- O(skipped + 1),
    /// whatever the depth.
    fn resume_apply(
        interp: &mut Interp,
        resume: Value,
        u: Value,
        span: Span,
        k_now: Kont,
    ) -> Result<State, RuntimeError> {
        let Value::Resume(rd) = resume else {
            return Err(rt(span, "internal: `resume` target is not a continuation"));
        };
        // One-shot enforcement — skipped for `with multi`. This is the ONLY
        // difference between one-shot and multi-shot: the re-installation below
        // is identical, because `rd.top` is an immutable persistent segment
        // (popping a shared node clones its frame; copy-on-write `Env`) — so
        // re-entering it more than once is sound.
        if !rd.handler.multi {
            if rd.consumed.get() {
                return Err(RuntimeError {
                    // The native trap's words (spec 2026-10-22 D1). The label
                    // names both halves: `with multi` alone is E0427 over an
                    // effect not declared `multi`.
                    diag: Diagnostic::error("E0425", "a one-shot continuation was resumed twice")
                        .with_label(
                            span,
                            "this handler is one-shot: only a `with multi` handler of an `effect multi` resumes more than once",
                        ),
                });
            }
            rd.consumed.set(true);
        }
        let mut k = install(rd.handler.clone(), rd.ret_env.clone(), k_now);
        interp.note_kont_work(1);
        for (handler, env, below) in rd.skipped.iter().rev() {
            interp.note_kont_work(1);
            let depth = 1 + seg_len(below) + meta_depth(&k.meta);
            k.meta = Some(Rc::new(MetaNode {
                handler: handler.clone(),
                env: env.clone(),
                below: below.clone(),
                rest: k.meta.take(),
                depth,
            }));
        }
        k.top = rd.top.clone();
        Ok(State::Return(u, k))
    }

    /// Perform operation `op` of effect `effect`: walk the handler boundaries for
    /// the nearest matching handler, split the continuation into the captured
    /// part (`top` and the boundaries skipped) and the suffix `k_rest` beneath the
    /// answering handler, and run the matching clause with `resume` bound.
    fn perform(
        interp: &mut Interp,
        effect: String,
        op: String,
        args: Vec<Value>,
        span: Span,
        k: Kont,
    ) -> Result<State, RuntimeError> {
        let Kont { top, meta } = k;
        let mut skipped: Vec<(Rc<Handler>, Env, Seg)> = Vec::new();
        let mut cur = meta;
        loop {
            let Some(boundary) = cur else {
                // A well-typed program never gets here (E0420 is static); defensive.
                return Err(rt(
                    span,
                    format!("internal: unhandled effect `{op}` reached the machine"),
                ));
            };
            interp.note_kont_work(1);
            if handler_handles(&boundary.handler, &effect, &op) {
                let k_rest = Kont {
                    top: boundary.below.clone(),
                    meta: boundary.rest.clone(),
                };
                let captured = Captured { top, skipped };
                return run_clause(
                    captured,
                    boundary.handler.clone(),
                    boundary.env.clone(),
                    &effect,
                    &op,
                    args,
                    span,
                    k_rest,
                );
            }
            skipped.push((
                boundary.handler.clone(),
                boundary.env.clone(),
                boundary.below.clone(),
            ));
            cur = boundary.rest.clone();
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
        // `with any` handles every operation that reaches it (D2: `IO` never
        // does -- it is builtins, not performs).
        if handler.any {
            return true;
        }
        handler
            .clauses
            .iter()
            .any(|c| clause_matches(&c.node, effect, op))
    }

    /// What a perform captures: the segment above the innermost handler and the
    /// boundaries between it and the answering handler.
    struct Captured {
        top: Seg,
        skipped: Vec<(Rc<Handler>, Env, Seg)>,
    }

    #[allow(clippy::too_many_arguments)]
    fn run_clause(
        cap: Captured,
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
            .find(|c| handler.any || clause_matches(&c.node, effect, op))
            .ok_or_else(|| rt(span, format!("internal: handler has no clause for `{op}`")))?;
        // A `with any` clause binds the operation itself.
        let args = if handler.any {
            vec![Value::Op {
                effect: effect.to_string(),
                op: op.to_string(),
                args: Rc::new(args),
            }]
        } else {
            args
        };
        let rd = Rc::new(ResumeData {
            top: cap.top,
            skipped: cap.skipped,
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
            CalleeSlot::Builtin(b @ ("time.now_ms" | "time.sleep_ms")) => {
                let v = super::host_builtin(interp, b, &args, span)?;
                Ok(State::Return(v, k))
            }
            CalleeSlot::IntBinary(name) => {
                Ok(State::Return(super::int_binary(name, &args, span)?, k))
            }
            CalleeSlot::FloatConversion(name) => Ok(State::Return(
                super::float_conversion(name, &args, span)?,
                k,
            )),
            CalleeSlot::StringBuiltin(name) => {
                Ok(State::Return(super::string_builtin(name, &args, span)?, k))
            }
            CalleeSlot::Builtin("int.to_string") => {
                let [Value::Int(n)] = &args[..] else {
                    return Err(rt(span, "int.to_string expects a single Int"));
                };
                Ok(State::Return(Value::Str(n.to_string().into()), k))
            }
            CalleeSlot::Builtin("float.to_string") => {
                let [Value::Float(x)] = &args[..] else {
                    return Err(rt(span, "float.to_string expects a single Float"));
                };
                Ok(State::Return(
                    Value::Str(crate::literal::float_text(*x).into()),
                    k,
                ))
            }
            CalleeSlot::Builtin(b) if b.starts_with("op.") => {
                let Some(Value::Op {
                    effect,
                    op,
                    args: op_args,
                }) = args.first()
                else {
                    return Err(rt(span, format!("{b} expects an operation")));
                };
                let (effect, op, op_args) = (effect.clone(), op.clone(), op_args.clone());
                let named = format!("{effect}.{op}");
                match (b, &args[1..]) {
                    ("op.name", []) => Ok(State::Return(Value::Str(named.into()), k)),
                    ("op.show", []) => match crate::literal::show_op(&effect, &op, &op_args) {
                        Ok(s) => Ok(State::Return(Value::Str(s.into()), k)),
                        Err(what) => Err(rt(
                            span,
                            format!(
                                "op.show: an argument of `{named}` is {what}, which is not data"
                            ),
                        )),
                    },
                    // The same operation again, from the clause: outside the
                    // handler, so it reaches the next one out.
                    ("op.perform", []) => perform(interp, effect, op, (*op_args).clone(), span, k),
                    ("op.encode", [v]) => match crate::literal::show(v) {
                        Ok(s) => Ok(State::Return(Value::Str(s.into()), k)),
                        Err(what) => Err(rt(
                            span,
                            format!(
                                "op.encode: the answer of `{named}` is {what}, which is not data"
                            ),
                        )),
                    },
                    ("op.decode", [Value::Str(text)]) => {
                        match crate::literal::decode(&interp.types, &op, text) {
                            Ok(v) => Ok(State::Return(v, k)),
                            Err(why) => Err(rt(
                                span,
                                format!("op.decode: {why}, for an answer of `{named}`"),
                            )),
                        }
                    }
                    _ => Err(rt(span, format!("{b}: wrong arguments"))),
                }
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
            CalleeSlot::Operation { effect, op } => perform(interp, effect, op, args, span, k),
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

/// A host builtin's answer under the interpreter's `World`: live, live and
/// written down, or read back from a trace (spec
/// 2026-10-16-elya-cli-record-replay §2).
fn host_builtin(
    interp: &mut Interp,
    name: &str,
    args: &[Value],
    span: Span,
) -> Result<Value, RuntimeError> {
    let mut q = format!("{name}(");
    for (i, a) in args.iter().enumerate() {
        if i > 0 {
            q.push_str(", ");
        }
        q.push_str(&crate::literal::show(a).map_err(|e| rt(span, e))?);
    }
    q.push(')');
    match &mut interp.world {
        World::Live => time_builtin(&name["time.".len()..], args, span),
        World::Record(log) => {
            let v = time_builtin(&name["time.".len()..], args, span)?;
            let a = crate::literal::show(&v).map_err(|e| rt(span, e))?;
            log.push((q, a));
            Ok(v)
        }
        World::Replay { entries, next } => {
            let n = *next + 1;
            let Some((want, answer)) = entries.get(*next).cloned() else {
                return Err(rt(
                    span,
                    format!("replay diverged at entry {n}: the program asked `{q}`, the trace has ended"),
                ));
            };
            if want != q {
                return Err(rt(
                    span,
                    format!("replay diverged at entry {n}: the program asked `{q}`, the trace holds `{want}`"),
                ));
            }
            *next += 1;
            // Decoded at the builtin's own result type: a trace cannot inject
            // a value of another type.
            match name {
                "time.now_ms" => answer.trim().parse::<i64>().map(Value::Int).map_err(|_| {
                    rt(
                        span,
                        format!("replay entry {n}: `{answer}` is not an answer of type Int"),
                    )
                }),
                _ if answer.trim() == "Unit" => Ok(Value::Unit),
                _ => Err(rt(
                    span,
                    format!("replay entry {n}: `{answer}` is not an answer of type Unit"),
                )),
            }
        }
    }
}

/// `time.now_ms()` and `time.sleep_ms(ms)` (spec
/// 2026-10-15-elya-async-step2-timers D3): monotonic milliseconds since the
/// first reading in this process, and a wait of at least `ms` (none if `ms <= 0`).
fn time_builtin(name: &str, args: &[Value], span: Span) -> Result<Value, RuntimeError> {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let start = *START.get_or_init(std::time::Instant::now);
    match (name, args) {
        ("now_ms", []) => Ok(Value::Int(
            i64::try_from(start.elapsed().as_millis()).unwrap_or(i64::MAX),
        )),
        ("sleep_ms", [Value::Int(ms)]) => {
            if *ms > 0 {
                std::thread::sleep(std::time::Duration::from_millis(*ms as u64));
            }
            Ok(Value::Unit)
        }
        _ => Err(rt(span, format!("time.{name}: wrong arguments"))),
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
