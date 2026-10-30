//! The reflection table `with any`'s literals are written and read by (spec
//! `docs/superpowers/specs/2026-10-17-elya-native-any-handlers-design.md` D3).
//!
//! Built for exactly the types `any.show`, `any.check` and `any.decode` are used at, and the
//! types their constructors' fields reach. A flat `i64` array the runtime walks
//! (`elya_any_show`, `elya_any_decode` in `runtime.c`); a node is an offset:
//!
//! - `[KIND_INT]`, `[KIND_BOOL]`, `[KIND_UNIT]`, `[KIND_STRING]`, `[KIND_FN]`,
//!   `[KIND_FLOAT]`;
//! - `[KIND_ADT, n, (tag, name_off, name_len, arity, field_node * arity) * n]`,
//!   the names in a separate byte blob, each the constructor's SOURCE name (an
//!   instance `Cons.mono.3` is written `Cons`);
//! - `[KIND_UNBUILT]`: an instance no constructor or pattern demanded. No value
//!   of it exists, so `show` never reaches it, and `decode` targets demand their
//!   whole closure (D5), so `decode` never does either.
//!
//! LLVM-free, like `any.rs`.

use std::collections::HashMap;

use elya::core::{push_children, CoreExpr, CoreKind, CoreModule};
use elya::types::{Ty, TyCon};

use crate::CodegenError;

pub(crate) const KIND_INT: i64 = 0;
pub(crate) const KIND_BOOL: i64 = 1;
pub(crate) const KIND_UNIT: i64 = 2;
pub(crate) const KIND_STRING: i64 = 3;
pub(crate) const KIND_FN: i64 = 4;
pub(crate) const KIND_ADT: i64 = 5;
pub(crate) const KIND_UNBUILT: i64 = 6;
/// A float: its bits (spec 2026-10-23-elya-float-natively D3, D4).
pub(crate) const KIND_FLOAT: i64 = 7;

#[derive(Default)]
pub(crate) struct Reflect {
    pub(crate) words: Vec<i64>,
    pub(crate) names: Vec<u8>,
    index: HashMap<String, usize>,
}

fn key(t: &Ty) -> String {
    format!("{t:?}")
}

impl Reflect {
    /// The table for every `any.show` and `any.decode` in `core`.
    pub(crate) fn of(
        core: &CoreModule,
        ctors: &HashMap<String, (usize, Vec<Ty>)>,
    ) -> Result<Reflect, CodegenError> {
        let mut targets = Vec::new();
        for f in &core.fns {
            targets_in(&f.body, &mut targets);
            // `main`'s value line (spec 2026-10-30-elya-main-values D1).
            if f.name == "main" && crate::main_shows_value(&f.body.ty) {
                targets.push(f.body.ty.clone());
            }
        }
        let mut r = Reflect::default();
        for t in &targets {
            r.node(t, core, ctors)?;
        }
        Ok(r)
    }

    /// The node of a type the table was built for.
    pub(crate) fn node_of(&self, t: &Ty) -> Option<usize> {
        self.index.get(&key(t)).copied()
    }

    fn node(
        &mut self,
        t: &Ty,
        core: &CoreModule,
        ctors: &HashMap<String, (usize, Vec<Ty>)>,
    ) -> Result<usize, CodegenError> {
        if let Some(n) = self.index.get(&key(t)) {
            return Ok(*n);
        }
        let at = self.words.len();
        self.index.insert(key(t), at);
        match t {
            Ty::Base(TyCon::Int) => self.words.push(KIND_INT),
            Ty::Base(TyCon::Float) => self.words.push(KIND_FLOAT),
            Ty::Base(TyCon::Bool) => self.words.push(KIND_BOOL),
            Ty::Base(TyCon::Unit) => self.words.push(KIND_UNIT),
            Ty::Base(TyCon::Str) => self.words.push(KIND_STRING),
            Ty::Fn(..) => self.words.push(KIND_FN),
            Ty::Con(n, a) if a.is_empty() => {
                let Some(decl) = core.types.iter().find(|d| d.name == *n) else {
                    self.words.push(KIND_UNBUILT);
                    return Ok(at);
                };
                self.words.push(KIND_ADT);
                self.words.push(decl.ctors.len() as i64);
                // Reserve every constructor's entry, then fill in the field
                // nodes: a recursive type reaches this node again.
                let mut slots = Vec::new();
                for c in &decl.ctors {
                    let (tag, _) = ctors
                        .get(&c.name)
                        .ok_or(CodegenError::Unsupported("parametric ADT"))?;
                    let source = c.name.split('.').next().unwrap_or(&c.name);
                    self.words.push(*tag as i64);
                    self.words.push(self.names.len() as i64);
                    self.words.push(source.len() as i64);
                    self.names.extend_from_slice(source.as_bytes());
                    self.words.push(c.fields.len() as i64);
                    for f in &c.fields {
                        slots.push((self.words.len(), f.clone()));
                        self.words.push(-1);
                    }
                }
                for (slot, f) in slots {
                    let n = self.node(&f, core, ctors)?;
                    self.words[slot] = n as i64;
                }
            }
            _ => {
                return Err(CodegenError::Unsupported(
                    "`with any` at a type without a literal form",
                ))
            }
        }
        Ok(at)
    }
}

/// The types `any.show` writes (its operand's) and `any.decode` reads (its own),
/// in pre-order, left to right, with a heap stack (spec
/// 2026-10-30-elya-long-blocks D1).
fn targets_in(e: &CoreExpr, out: &mut Vec<Ty>) {
    let mut stack = vec![e];
    let mut children = Vec::new();
    while let Some(e) = stack.pop() {
        if let CoreKind::Builtin(n, a) = &e.kind {
            match n.as_str() {
                "any.show" | "any.check" => out.push(a[0].ty.clone()),
                "any.decode" => out.push(e.ty.clone()),
                _ => {}
            }
        }
        push_children(e, &mut children);
        stack.extend(children.drain(..).rev());
    }
}
