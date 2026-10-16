//! Data literals: how a `with any` handler shows, encodes and decodes the operations
//! it handles (2026-10-16, spec `2026-10-16-elya-any-handlers-design.md` D7).
//!
//! The encoding is Elya's own literal syntax: `42`, `-7`, `1.5`, `"a\"b"`, `True`,
//! `False`, `Unit`, `Cons(1, Nil)`. Constructor names are unique per module, so a
//! literal describes itself; `decode` also checks it against the operation's
//! declared answer type, through constructors and their fields.
//!
//! Floats are decimals without an exponent (`1.0`, `0.0000001`), plus the words
//! `inf`, `-inf` and `nan` (lowercase: a constructor may be named `NaN`). Every walk here is iterative: a list of a million cells
//! is shown, parsed and checked without recursing on the host stack.

use crate::ast::{Decl, Module, TypeAnn};
use crate::eval::{CtorArgs, Value};
use std::collections::HashMap;
use std::rc::Rc;

/// What decoding needs from the module: each constructor's type, the type's
/// parameters and the constructor's field types, and each operation's effect, the
/// effect's parameters and the operation's declared answer type.
#[derive(Default, Debug)]
pub struct TypeInfo {
    ctors: HashMap<String, (String, Vec<String>, Vec<TypeAnn>)>,
    ops: HashMap<String, (Vec<String>, TypeAnn)>,
}

impl TypeInfo {
    pub fn of(module: &Module) -> TypeInfo {
        let mut info = TypeInfo::default();
        for d in &module.decls {
            match &d.node {
                Decl::Type(t) => {
                    for v in &t.variants {
                        info.ctors.insert(
                            v.node.name.clone(),
                            (
                                t.name.clone(),
                                t.params.clone(),
                                v.node.fields.iter().map(|f| f.node.clone()).collect(),
                            ),
                        );
                    }
                }
                Decl::Effect(e) => {
                    for op in &e.ops {
                        info.ops.insert(
                            op.node.name.clone(),
                            (e.params.clone(), op.node.ret.node.clone()),
                        );
                    }
                }
                Decl::Fn(_) => {}
            }
        }
        info
    }
}

/// A data value in literal syntax, or what in it is not data.
pub fn show(v: &Value) -> Result<String, &'static str> {
    let mut out = String::new();
    write(v, &mut out)?;
    Ok(out)
}

enum Item<'a> {
    Val(&'a Value),
    Text(&'static str),
}

fn write(v: &Value, out: &mut String) -> Result<(), &'static str> {
    let mut stack = vec![Item::Val(v)];
    while let Some(item) = stack.pop() {
        let v = match item {
            Item::Text(t) => {
                out.push_str(t);
                continue;
            }
            Item::Val(v) => v,
        };
        match v {
            Value::Int(n) => out.push_str(&n.to_string()),
            Value::Float(f) => write_float(*f, out),
            Value::Bool(true) => out.push_str("True"),
            Value::Bool(false) => out.push_str("False"),
            Value::Unit => out.push_str("Unit"),
            Value::Str(s) => {
                out.push('"');
                for c in s.chars() {
                    match c {
                        '"' => out.push_str("\\\""),
                        '\\' => out.push_str("\\\\"),
                        '\n' => out.push_str("\\n"),
                        '\t' => out.push_str("\\t"),
                        '\r' => out.push_str("\\r"),
                        c => out.push(c),
                    }
                }
                out.push('"');
            }
            Value::Ctor(name, args) => {
                out.push_str(name);
                if !args.0.is_empty() {
                    out.push('(');
                    stack.push(Item::Text(")"));
                    for (i, a) in args.0.iter().enumerate().rev() {
                        stack.push(Item::Val(a));
                        if i > 0 {
                            stack.push(Item::Text(", "));
                        }
                    }
                }
            }
            Value::Fn(_) | Value::Closure { .. } => return Err("a function"),
            Value::Resume(_) => return Err("a continuation"),
            Value::Op { .. } => return Err("an operation"),
        }
    }
    Ok(())
}

/// A decimal with a point and no exponent (Rust's shortest round-trip `Display`),
/// or `inf`, `-inf`, `nan`.
fn write_float(f: f64, out: &mut String) {
    if f.is_nan() {
        out.push_str("nan");
    } else if f.is_infinite() {
        out.push_str(if f > 0.0 { "inf" } else { "-inf" });
    } else {
        let s = format!("{f}");
        out.push_str(&s);
        if !s.contains('.') {
            out.push_str(".0");
        }
    }
}

/// An operation as `Effect.op(args)`.
pub fn show_op(effect: &str, op: &str, args: &[Value]) -> Result<String, &'static str> {
    let mut out = format!("{effect}.{op}(");
    for (i, a) in args.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        write(a, &mut out)?;
    }
    out.push(')');
    Ok(out)
}

/// Decode `text` as an answer of operation `op`: parse a literal, then check it
/// against the declared answer type. `Err` names what is wrong.
pub fn decode(info: &TypeInfo, op: &str, text: &str) -> Result<Value, String> {
    let (params, ret) = info
        .ops
        .get(op)
        .cloned()
        .ok_or_else(|| format!("no operation `{op}`"))?;
    // An effect's own type parameter is not known at run time, so an answer of
    // that type cannot be checked: refused by name (the review's S2; letting any
    // data in put a String where an Int was typed).
    if let Some(p) = params.iter().find(|p| mentions(&ret, p)) {
        return Err(format!(
            "the answer type `{}` mentions the effect's parameter `{p}`, which is not known at run time",
            render_ann(&ret)
        ));
    }
    let mut p = Parser {
        s: text.as_bytes(),
        i: 0,
    };
    let v = p
        .value()
        .map_err(|e| format!("`{text}` is not a literal ({e})"))?;
    p.ws();
    if p.i != p.s.len() {
        return Err(format!("`{text}` has input after its value"));
    }
    if !conforms(info, &v, &ret) {
        return Err(format!(
            "`{text}` is not a value of type `{}`",
            render_ann(&ret)
        ));
    }
    Ok(v)
}

fn mentions(ann: &TypeAnn, name: &str) -> bool {
    let mut stack = vec![ann];
    while let Some(a) = stack.pop() {
        if a.name == name {
            return true;
        }
        stack.extend(a.args.iter().map(|x| &x.node));
    }
    false
}

/// Whether `v` is a value of `ann`, through declared types' constructors. The
/// annotation never names an unknown parameter here (`decode` refused effect
/// parameters; a declared type's are bound to its arguments).
fn conforms(info: &TypeInfo, v: &Value, ann: &TypeAnn) -> bool {
    let mut stack: Vec<(&Value, TypeAnn)> = vec![(v, ann.clone())];
    while let Some((v, ann)) = stack.pop() {
        let ok = match ann.name.as_str() {
            "Int" => matches!(v, Value::Int(_)),
            "Float" => matches!(v, Value::Float(_)),
            "Bool" => matches!(v, Value::Bool(_)),
            "String" => matches!(v, Value::Str(_)),
            "Unit" => matches!(v, Value::Unit),
            tname if tname == "fn" || tname.starts_with(|c: char| c.is_ascii_lowercase()) => false,
            tname => {
                let Value::Ctor(c, args) = v else {
                    return false;
                };
                let Some((owner, params, fields)) = info.ctors.get(c) else {
                    return false;
                };
                if owner != tname || fields.len() != args.0.len() || params.len() != ann.args.len()
                {
                    return false;
                }
                // The type's parameters at this use are its arguments.
                let env: HashMap<&str, &TypeAnn> = params
                    .iter()
                    .map(|p| p.as_str())
                    .zip(ann.args.iter().map(|a| &a.node))
                    .collect();
                for (f, a) in fields.iter().zip(args.0.iter()) {
                    stack.push((a, substitute(f, &env)));
                }
                true
            }
        };
        if !ok {
            return false;
        }
    }
    true
}

/// A field annotation with the type's parameters replaced by their arguments.
fn substitute(ann: &TypeAnn, env: &HashMap<&str, &TypeAnn>) -> TypeAnn {
    if ann.args.is_empty() {
        if let Some(bound) = env.get(ann.name.as_str()) {
            return (*bound).clone();
        }
        return ann.clone();
    }
    let mut out = ann.clone();
    for a in out.args.iter_mut() {
        a.node = substitute(&a.node, env);
    }
    out
}

fn render_ann(a: &TypeAnn) -> String {
    let args: Vec<String> = a.args.iter().map(|x| render_ann(&x.node)).collect();
    if a.name == "fn" && !args.is_empty() {
        let (ps, r) = args.split_at(args.len() - 1);
        return format!("fn({}) -> {}", ps.join(", "), r[0]);
    }
    if args.is_empty() {
        return a.name.clone();
    }
    format!("{}({})", a.name, args.join(", "))
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && (self.s[self.i] as char).is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    /// One value, constructors' arguments kept on an explicit stack.
    fn value(&mut self) -> Result<Value, &'static str> {
        let mut open: Vec<(String, Vec<Value>)> = Vec::new();
        loop {
            self.ws();
            let mut v = match self.peek() {
                None => return Err("nothing to read"),
                Some(b'"') => self.string()?,
                Some(c) if c == b'-' || c.is_ascii_digit() => {
                    if self.s[self.i..].starts_with(b"-inf") {
                        self.i += 4;
                        Value::Float(f64::NEG_INFINITY)
                    } else {
                        self.number()?
                    }
                }
                Some(b'i') if self.s[self.i..].starts_with(b"inf") => {
                    self.i += 3;
                    Value::Float(f64::INFINITY)
                }
                Some(b'n') if self.s[self.i..].starts_with(b"nan") => {
                    self.i += 3;
                    Value::Float(f64::NAN)
                }
                Some(c) if c.is_ascii_uppercase() => {
                    let name = self.name()?;
                    let save = self.i;
                    self.ws();
                    if self.peek() == Some(b'(') {
                        self.i += 1;
                        open.push((name, Vec::new()));
                        continue;
                    }
                    self.i = save;
                    match name.as_str() {
                        "True" => Value::Bool(true),
                        "False" => Value::Bool(false),
                        "Unit" => Value::Unit,
                        _ => Value::Ctor(name, CtorArgs(Rc::new(Vec::new()))),
                    }
                }
                Some(_) => return Err("an unexpected character"),
            };
            // Give the value to the constructor it is an argument of, closing
            // every constructor whose `)` follows.
            loop {
                let Some((_, args)) = open.last_mut() else {
                    return Ok(v);
                };
                args.push(v);
                self.ws();
                match self.peek() {
                    Some(b',') => {
                        self.i += 1;
                        break;
                    }
                    Some(b')') => {
                        self.i += 1;
                        let (name, args) = open.pop().expect("an open constructor");
                        v = Value::Ctor(name, CtorArgs(Rc::new(args)));
                    }
                    _ => return Err("a constructor's arguments not closed by `)`"),
                }
            }
        }
    }

    fn name(&mut self) -> Result<String, &'static str> {
        let start = self.i;
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == b'_' {
                self.i += 1;
            } else {
                break;
            }
        }
        std::str::from_utf8(&self.s[start..self.i])
            .map(|n| n.to_string())
            .map_err(|_| "a malformed name")
    }

    fn number(&mut self) -> Result<Value, &'static str> {
        let start = self.i;
        if self.peek() == Some(b'-') {
            self.i += 1;
        }
        let mut float = false;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                self.i += 1;
            } else if c == b'.' {
                float = true;
                self.i += 1;
            } else {
                break;
            }
        }
        let text = std::str::from_utf8(&self.s[start..self.i]).map_err(|_| "a malformed number")?;
        if float {
            text.parse::<f64>()
                .map(Value::Float)
                .map_err(|_| "a malformed number")
        } else {
            text.parse::<i64>()
                .map(Value::Int)
                .map_err(|_| "a malformed or out-of-range integer")
        }
    }

    fn string(&mut self) -> Result<Value, &'static str> {
        self.i += 1; // the opening quote
        let mut bytes: Vec<u8> = Vec::new();
        loop {
            match self.peek() {
                None => return Err("an unterminated string"),
                Some(b'"') => {
                    self.i += 1;
                    break;
                }
                Some(b'\\') => {
                    self.i += 1;
                    let c = self.peek().ok_or("an unterminated string")?;
                    bytes.push(match c {
                        b'"' => b'"',
                        b'\\' => b'\\',
                        b'n' => b'\n',
                        b't' => b'\t',
                        b'r' => b'\r',
                        _ => return Err("an unknown escape"),
                    });
                    self.i += 1;
                }
                Some(c) => {
                    bytes.push(c);
                    self.i += 1;
                }
            }
        }
        String::from_utf8(bytes)
            .map(Value::Str)
            .map_err(|_| "a string that is not UTF-8")
    }
}
