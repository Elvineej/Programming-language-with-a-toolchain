//! A stack of named bindings that finds a name's innermost binding without
//! searching the stack (spec 2026-10-31-elya-scalability D12).
//!
//! The native passes kept their scope as a `Vec` and searched it from the
//! innermost binding out at every use, so a name bound above a long block (a
//! parameter, a top-level function) was searched past every `let` below it
//! each time it was used: quadratic in the block's length. Each name here
//! keeps the positions of its bindings, innermost last.

use std::cell::Cell;
use std::collections::HashMap;

/// What a binding is called.
pub(crate) trait Named {
    fn name(&self) -> &str;
}

impl Named for String {
    fn name(&self) -> &str {
        self
    }
}

impl<T> Named for (String, T) {
    fn name(&self) -> &str {
        &self.0
    }
}

thread_local! {
    static PROBES: Cell<u64> = const { Cell::new(0) };
}

/// Bindings examined on this thread so far: one per [`Scope`] lookup, one per
/// pointer an [`Env`] roots (`tests/native_scalability.rs` pins its growth).
/// A measure for tests, not an interface.
#[doc(hidden)]
pub fn probes() -> u64 {
    PROBES.with(|c| c.get())
}

fn probe(n: u64) {
    PROBES.with(|c| c.set(c.get() + n));
}

/// Bindings in order, outermost first, with each name's positions.
pub(crate) struct Scope<T: Named> {
    items: Vec<T>,
    at: HashMap<String, Vec<usize>>,
}

impl<T: Named> Default for Scope<T> {
    fn default() -> Self {
        Scope {
            items: Vec::new(),
            at: HashMap::new(),
        }
    }
}

impl<T: Named> Scope<T> {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn len(&self) -> usize {
        self.items.len()
    }

    pub(crate) fn push(&mut self, t: T) {
        let i = self.items.len();
        match self.at.get_mut(t.name()) {
            Some(v) => v.push(i),
            None => {
                self.at.insert(t.name().to_string(), vec![i]);
            }
        }
        self.items.push(t);
    }

    pub(crate) fn pop(&mut self) -> Option<T> {
        let t = self.items.pop()?;
        if let Some(v) = self.at.get_mut(t.name()) {
            v.pop();
            if v.is_empty() {
                self.at.remove(t.name());
            }
        }
        Some(t)
    }

    pub(crate) fn truncate(&mut self, len: usize) {
        while self.items.len() > len {
            self.pop();
        }
    }

    /// The position of `name`'s innermost binding.
    pub(crate) fn rposition(&self, name: &str) -> Option<usize> {
        probe(1);
        self.at.get(name).and_then(|v| v.last().copied())
    }

    /// The position of `name`'s innermost binding below `depth`: the one a
    /// node at that depth sees.
    pub(crate) fn rposition_below(&self, name: &str, depth: usize) -> Option<usize> {
        probe(1);
        let v = self.at.get(name)?;
        let i = v.partition_point(|&p| p < depth);
        i.checked_sub(1).map(|i| v[i])
    }

    pub(crate) fn contains(&self, name: &str) -> bool {
        self.rposition(name).is_some()
    }

    /// `name`'s innermost binding.
    pub(crate) fn find(&self, name: &str) -> Option<&T> {
        self.rposition(name).map(|i| &self.items[i])
    }

    pub(crate) fn get_mut(&mut self, i: usize) -> &mut T {
        &mut self.items[i]
    }

    pub(crate) fn get(&self, i: usize) -> &T {
        &self.items[i]
    }
}

impl<T: Named> Extend<T> for Scope<T> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, it: I) {
        for t in it {
            self.push(t);
        }
    }
}

impl<T: Named> FromIterator<T> for Scope<T> {
    fn from_iter<I: IntoIterator<Item = T>>(it: I) -> Self {
        let mut s = Scope::new();
        s.extend(it);
        s
    }
}

/// The direct emitter's names in scope and their values (spec
/// 2026-10-31-elya-scalability D12). It also keeps the names that hold a
/// pointer, in name order, and how many of its keys are hidden ones
/// (`bind_local`'s): rooting the environment at an allocation sorted every
/// name in scope, and parking a shadowed value counted every key, each a walk
/// of the whole block above.
#[derive(Clone, Default, Debug)]
pub(crate) struct Env<'ctx> {
    map: HashMap<String, inkwell::values::BasicValueEnum<'ctx>>,
    ptrs: std::collections::BTreeSet<String>,
    hidden: usize,
}

impl<'ctx> Env<'ctx> {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn insert(
        &mut self,
        k: String,
        v: inkwell::values::BasicValueEnum<'ctx>,
    ) -> Option<inkwell::values::BasicValueEnum<'ctx>> {
        if v.is_pointer_value() {
            self.ptrs.insert(k.clone());
        } else {
            self.ptrs.remove(&k);
        }
        let hidden = k.starts_with(crate::SHADOW);
        let prev = self.map.insert(k, v);
        if hidden && prev.is_none() {
            self.hidden += 1;
        }
        prev
    }

    pub(crate) fn remove(&mut self, k: &str) -> Option<inkwell::values::BasicValueEnum<'ctx>> {
        let prev = self.map.remove(k);
        if prev.is_some() {
            self.ptrs.remove(k);
            if k.starts_with(crate::SHADOW) {
                self.hidden -= 1;
            }
        }
        prev
    }

    pub(crate) fn get(&self, k: &str) -> Option<&inkwell::values::BasicValueEnum<'ctx>> {
        self.map.get(k)
    }

    pub(crate) fn contains_key(&self, k: &str) -> bool {
        self.map.contains_key(k)
    }

    pub(crate) fn clear(&mut self) {
        self.map.clear();
        self.ptrs.clear();
        self.hidden = 0;
    }

    /// The values that are pointers, in their names' order.
    pub(crate) fn pointers(
        &self,
    ) -> impl Iterator<Item = inkwell::values::BasicValueEnum<'ctx>> + '_ {
        self.ptrs.iter().map(|k| {
            probe(1);
            self.map[k]
        })
    }

    /// How many hidden keys are present.
    pub(crate) fn hidden(&self) -> usize {
        self.hidden
    }

    /// Stop rooting `k`'s value (spec 2026-10-31-elya-scalability D13): the
    /// code still to come never reads that binding, so a collection may free
    /// what it holds. The name still finds it; a later `insert` of the key
    /// roots it again.
    pub(crate) fn retire(&mut self, k: &str) {
        self.ptrs.remove(k);
    }

    /// Whether `k` holds a pointer that rooting pushes: bound, a pointer,
    /// not retired.
    pub(crate) fn rooted(&self, k: &str) -> bool {
        self.ptrs.contains(k)
    }
}

impl<'ctx> std::ops::Index<&str> for Env<'ctx> {
    type Output = inkwell::values::BasicValueEnum<'ctx>;
    fn index(&self, k: &str) -> &Self::Output {
        &self.map[k]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_innermost_binding_is_found_and_truncation_uncovers_the_outer() {
        let mut s: Scope<String> = ["x", "y", "x"].iter().map(|n| n.to_string()).collect();
        assert_eq!(s.rposition("x"), Some(2));
        assert_eq!(s.rposition_below("x", 2), Some(0));
        assert_eq!(s.rposition_below("x", 0), None);
        assert_eq!(s.rposition("z"), None);
        s.truncate(2);
        assert_eq!(s.rposition("x"), Some(0));
        s.truncate(0);
        assert!(!s.contains("x"));
        assert_eq!(s.len(), 0);
    }

    /// `Env` roots exactly its pointer-valued names, in name order, through
    /// rebinding (a pointer over an `Int`, an `Int` over a pointer), removal
    /// and clearing, and counts a hidden key once however often it is bound
    /// (the review of part 3a: a name rebound from an `Int` to a heap value,
    /// left out of the roots, was pinned by no test).
    #[test]
    fn env_roots_exactly_its_pointer_valued_names() {
        use inkwell::values::BasicValueEnum;
        let ctx = inkwell::context::Context::create();
        let p: BasicValueEnum = ctx
            .ptr_type(inkwell::AddressSpace::default())
            .const_null()
            .into();
        let i: BasicValueEnum = ctx.i64_type().const_int(7, false).into();
        let mut env = Env::new();
        env.insert("b".to_string(), i);
        env.insert("b".to_string(), p);
        env.insert("c".to_string(), p);
        env.insert("c".to_string(), i);
        env.insert("a".to_string(), p);
        assert_eq!(env.ptrs.iter().collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(env.pointers().count(), 2);
        let hidden = format!("{}000000:a", crate::SHADOW);
        env.insert(hidden.clone(), p);
        env.insert(hidden.clone(), p);
        assert_eq!(env.hidden(), 1);
        assert_eq!(env.pointers().count(), 3);
        env.remove(&hidden);
        assert_eq!(env.hidden(), 0);
        env.remove("b");
        env.remove("absent");
        assert_eq!(env.ptrs.iter().collect::<Vec<_>>(), ["a"]);
        assert!(env.contains_key("c"));
        env.clear();
        assert_eq!(env.pointers().count(), 0);
        assert!(!env.contains_key("a"));
        assert_eq!(env.hidden(), 0);
    }
}
