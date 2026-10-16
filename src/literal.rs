//! Data literals: how a `with any` handler shows, encodes and decodes the operations
//! it handles (2026-10-16, spec `2026-10-16-elya-any-handlers-design.md` D7).
//!
//! The encoding is Elya's own literal syntax: `42`, `-7`, `1.5`, `"a\"b"`, `True`,
//! `False`, `()`, `Cons(1, Nil)`. Constructor names are unique per module, so a
//! literal describes itself; `decode` also checks it against the operation's
//! declared answer type, through constructors and their fields.

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

fn write(v: &Value, out: &mut String) -> Result<(), &'static str> {
    match v {
        Value::Int(n) => out.push_str(&n.to_string()),
        Value::Float(f) => out.push_str(&format!("{f:?}")),
        Value::Bool(true) => out.push_str("True"),
        Value::Bool(false) => out.push_str("False"),
        Value::Unit => out.push_str("()"),
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
                for (i, a) in args.0.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    write(a, out)?;
                }
                out.push(')');
            }
        }
        Value::Fn(_) | Value::Closure { .. } => return Err("a function"),
        Value::Resume(_) => return Err("a continuation"),
        Value::Op { .. } => return Err("an operation"),
    }
    Ok(())
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
    // The effect's own parameters are not known at run time: any data (D7).
    let env: HashMap<String, Option<TypeAnn>> = params.into_iter().map(|p| (p, None)).collect();
    if !conforms(info, &v, &ret, &env) {
        return Err(format!(
            "`{text}` is not a value of type `{}`",
            render_ann(&ret)
        ));
    }
    Ok(v)
}

fn conforms(
    info: &TypeInfo,
    v: &Value,
    ann: &TypeAnn,
    env: &HashMap<String, Option<TypeAnn>>,
) -> bool {
    match ann.name.as_str() {
        "Int" => matches!(v, Value::Int(_)),
        "Float" => matches!(v, Value::Float(_)),
        "Bool" => matches!(v, Value::Bool(_)),
        "String" => matches!(v, Value::Str(_)),
        "Unit" => matches!(v, Value::Unit),
        "fn" => false,
        n if n.starts_with(|c: char| c.is_ascii_lowercase()) => match env.get(n) {
            Some(Some(bound)) => conforms(info, v, bound, &HashMap::new()),
            // An unknown parameter: any data whose constructors exist.
            _ => known(info, v),
        },
        tname => {
            let Value::Ctor(c, args) = v else {
                return false;
            };
            let Some((owner, params, fields)) = info.ctors.get(c) else {
                return false;
            };
            if owner != tname || fields.len() != args.0.len() {
                return false;
            }
            // The type's parameters at this use: its arguments, resolved through
            // the enclosing environment.
            let inner: HashMap<String, Option<TypeAnn>> = params
                .iter()
                .cloned()
                .zip(ann.args.iter().map(|a| Some(resolve_ann(&a.node, env))))
                .collect();
            fields
                .iter()
                .zip(args.0.iter())
                .all(|(f, a)| conforms(info, a, f, &inner))
        }
    }
}

/// Whether every constructor in `v` is declared, at its arity.
fn known(info: &TypeInfo, v: &Value) -> bool {
    match v {
        Value::Ctor(c, args) => {
            info.ctors.get(c).map(|(_, _, f)| f.len()) == Some(args.0.len())
                && args.0.iter().all(|a| known(info, a))
        }
        _ => true,
    }
}

/// An annotation with the environment's bound parameters substituted. An unbound
/// one stays a parameter, which `conforms` then accepts as any data.
fn resolve_ann(ann: &TypeAnn, env: &HashMap<String, Option<TypeAnn>>) -> TypeAnn {
    if ann.args.is_empty() && ann.name.starts_with(|c: char| c.is_ascii_lowercase()) {
        if let Some(Some(bound)) = env.get(&ann.name) {
            return bound.clone();
        }
        return ann.clone();
    }
    let mut out = ann.clone();
    for a in out.args.iter_mut() {
        a.node = resolve_ann(&a.node, env);
    }
    out
}

fn render_ann(a: &TypeAnn) -> String {
    if a.args.is_empty() {
        return a.name.clone();
    }
    let args: Vec<String> = a.args.iter().map(|x| render_ann(&x.node)).collect();
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

    fn value(&mut self) -> Result<Value, &'static str> {
        self.ws();
        match self.peek() {
            None => Err("nothing to read"),
            Some(b'"') => self.string(),
            Some(b'(') => {
                self.i += 1;
                self.ws();
                if self.peek() == Some(b')') {
                    self.i += 1;
                    Ok(Value::Unit)
                } else {
                    Err("`(` that is not `()`")
                }
            }
            Some(c) if c == b'-' || c.is_ascii_digit() => self.number(),
            Some(c) if c.is_ascii_uppercase() => self.ctor(),
            Some(_) => Err("an unexpected character"),
        }
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
            } else if matches!(c, b'.' | b'e' | b'E')
                || (matches!(c, b'+' | b'-') && matches!(self.s[self.i - 1], b'e' | b'E'))
            {
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

    fn ctor(&mut self) -> Result<Value, &'static str> {
        let start = self.i;
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == b'_' {
                self.i += 1;
            } else {
                break;
            }
        }
        let name = std::str::from_utf8(&self.s[start..self.i])
            .map_err(|_| "a malformed name")?
            .to_string();
        match name.as_str() {
            "True" => return Ok(Value::Bool(true)),
            "False" => return Ok(Value::Bool(false)),
            _ => {}
        }
        let save = self.i;
        self.ws();
        if self.peek() != Some(b'(') {
            self.i = save;
            return Ok(Value::Ctor(name, CtorArgs(Rc::new(Vec::new()))));
        }
        self.i += 1;
        let mut args = Vec::new();
        loop {
            args.push(self.value()?);
            self.ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b')') => {
                    self.i += 1;
                    break;
                }
                _ => return Err("a constructor's arguments not closed by `)`"),
            }
        }
        Ok(Value::Ctor(name, CtorArgs(Rc::new(args))))
    }
}
