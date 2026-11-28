// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)
//! The functions that never allocate (2026-11-15): a caller need not root
//! what it holds across a call to one, since nothing can collect during it.
//!
//! The greatest fixpoint over the module: start from every function, and
//! drop one whose body may allocate by itself (a constructor, a lambda, a function named as a value, `<>`, an allocating
//! builtin, an effect: a perform, a handler, a resume) or calls anything but
//! a function still in the set, by name. Recursion keeps a function in (a
//! loop that allocates nothing never allocates); a call through a local
//! closure, or of a name a local shadows, drops it. Sound by induction on
//! the run: a member executes only non-allocating operations and calls of
//! members.

use crate::builtin_allocates;
use elya::ast::BinOp;
use elya::core::{CoreExpr, CoreKind, CoreModule, CorePat};
use elya::types::Ty;
use std::collections::HashSet;

/// The names of the module's functions that never allocate.
pub(crate) fn never_allocating(core: &CoreModule) -> HashSet<String> {
    let mut set: HashSet<String> = core.fns.iter().map(|f| f.name.clone()).collect();
    loop {
        let mut dropped = false;
        for f in &core.fns {
            if !set.contains(&f.name) {
                continue;
            }
            let mut locals: HashSet<String> = f.params.iter().map(|p| p.name.clone()).collect();
            binders(&f.body, &mut locals);
            if allocates(&f.body, &set, &locals) {
                set.remove(&f.name);
                dropped = true;
            }
        }
        if !dropped {
            return set;
        }
    }
}

/// Every name `e` binds (lets, patterns, lambda parameters): a call of one
/// of these is never taken for a top-level function's. A walk on a heap
/// stack, not the host's: a block is a `Let` spine as long as the block
/// (`native_long_blocks.rs` compiles 60,000 statements on 512 KiB).
fn binders(e: &CoreExpr, out: &mut HashSet<String>) {
    fn pat(p: &CorePat, out: &mut HashSet<String>) {
        match p {
            CorePat::Var(x) => {
                out.insert(x.clone());
            }
            CorePat::Ctor(_, ps) => ps.iter().for_each(|q| pat(q, out)),
            CorePat::Wild | CorePat::Lit(_) => {}
        }
    }
    let mut work = vec![e];
    while let Some(e) = work.pop() {
        match &e.kind {
            CoreKind::Let(name, v, body) => {
                out.insert(name.clone());
                work.push(v);
                work.push(body);
            }
            CoreKind::Lambda(ps, body) => {
                out.extend(ps.iter().map(|p| p.name.clone()));
                work.push(body);
            }
            CoreKind::Match(s, arms) => {
                work.push(s);
                for a in arms.iter() {
                    pat(&a.pat, out);
                    work.push(&a.body);
                }
            }
            CoreKind::App(c, args) => {
                work.push(c);
                work.extend(args.iter());
            }
            CoreKind::Builtin(_, args) | CoreKind::Ctor(_, args) | CoreKind::Prim(_, args) => {
                work.extend(args.iter())
            }
            CoreKind::If(c, t, f) => {
                work.push(c);
                work.push(t);
                work.push(f);
            }
            // Effects allocate: `allocates` stops at them.
            CoreKind::Handle(_)
            | CoreKind::Resume(_)
            | CoreKind::Perform(_)
            | CoreKind::Lit(_)
            | CoreKind::Var(_) => {}
        }
    }
}

/// Whether evaluating `e` may allocate, given the functions still in `set`:
/// whether any node in it may by itself (a heap stack, as `binders`).
fn allocates(e: &CoreExpr, set: &HashSet<String>, locals: &HashSet<String>) -> bool {
    let mut work = vec![e];
    while let Some(e) = work.pop() {
        let here = match &e.kind {
            // A string literal is a constant object (no allocation).
            CoreKind::Lit(_) => false,
            // A local is read; a top-level function named as a value may be
            // built into a closure.
            CoreKind::Var(name) => !locals.contains(name) && matches!(e.ty, Ty::Fn(..)),
            CoreKind::App(callee, args) => match &callee.kind {
                CoreKind::Var(name) if !locals.contains(name) && set.contains(name) => {
                    work.extend(args.iter());
                    false
                }
                _ => true,
            },
            CoreKind::Builtin(name, args) => {
                work.extend(args.iter());
                builtin_allocates(name)
            }
            CoreKind::Prim(op, args) => {
                work.extend(args.iter());
                matches!(op, BinOp::Concat)
            }
            CoreKind::Let(_, v, body) => {
                work.push(v);
                work.push(body);
                false
            }
            CoreKind::If(c, t, f) => {
                work.push(c);
                work.push(t);
                work.push(f);
                false
            }
            CoreKind::Match(s, arms) => {
                work.push(s);
                work.extend(arms.iter().map(|a| &a.body));
                false
            }
            CoreKind::Ctor(..)
            | CoreKind::Lambda(..)
            | CoreKind::Handle(_)
            | CoreKind::Resume(_)
            | CoreKind::Perform(_) => true,
        };
        if here {
            return true;
        }
    }
    false
}
