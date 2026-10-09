//! 5b-8 Task 7b-3 — the EMISSION half of selective CPS (D10, D14, D16-D18).
//!
//! `cps.rs` decides (LLVM-free) which calls are continuation sites and what each
//! frame saves; this module turns that into code:
//!
//! - An effectful top-level function takes its continuation `k` as a trailing
//!   pointer and never returns its own value: it hands the value to `k` with a
//!   `musttail` jump to `k`'s code (D14). Every effectful function, frame code
//!   and handler code answers `i64` -- the handle's answer, as a word -- so all
//!   of them can `musttail` into each other.
//! - A continuation site (a non-tail effectful call) allocates a frame
//!   `[tag][code_ptr][next = k][saved..]` (D10) and `musttail`s the callee with
//!   that frame as its continuation. The frame's code is the site's RESUMPTION
//!   function, which reloads what was saved and runs the rest of the region from
//!   the hole upward.
//! - A `handle` (only where D17 allows one) allocates a handler frame
//!   `[tag][code_ptr = return clause][next = null][saved..]` and calls the body's
//!   function as an ordinary nested call; the body eventually hands its value to
//!   the handler frame, whose code runs the return clause and RETURNS -- through
//!   the chain of `musttail`s -- to the handle site.
//! - A `Perform` is a named trap until Task 8 (D18).
//!
//! GC discipline is the existing one: every allocation and every non-tail call
//! is bracketed by shadow-stack pushes of what is live, and nothing is pushed
//! across a `musttail` (which must be immediately followed by its `ret`).

use std::collections::{BTreeMap, HashMap, HashSet};

use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::types::BasicMetadataTypeEnum;
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, FunctionValue, PointerValue};
use inkwell::values::{IntValue, LLVMTailCallKind};
use inkwell::AddressSpace;

use elya::ast::BinOp;
use elya::core::{CoreExpr, CoreKind, CoreModule, CorePat};
use elya::types::{Ty, TyCon};

use crate::cps::{self, contains_effect, is_tail_slot, needs_cps, ContSite, HandlerSite, Saved};
use crate::{
    bind_local, eq_operand_label, fn_type_of, gc_root_env, gc_unroot, internal, lower_expr, mangle,
    op_label, prim_values, repr_ty, value_to_word, word_to_value, CodegenError, LowerCtx,
    MAX_PARAMS, TAILCC,
};
use crate::{unbind_local, Shadowed};

/// The region's continuation, kept in the name environment under a name no
/// source identifier can spell, so the existing env-rooting discipline roots it
/// at every allocation the direct emitter performs inside a CPS region.
const KONT: &str = "$kont";

type R<T> = Result<T, CodegenError>;

// ---------------------------------------------------------------- analysis --

/// D16: the effectful top-level functions -- those whose own region performs
/// or makes an effectful call.
pub(crate) fn cps_functions(core: &CoreModule) -> HashSet<String> {
    core.fns
        .iter()
        .filter(|f| contains_effect(&f.body))
        .map(|f| f.name.clone())
        .collect()
}

/// Every Core node by address (the identity `LambdaSite::key` and
/// `ContSite::key` already use), so a site's recorded path can be walked.
pub(crate) fn index_nodes(core: &CoreModule) -> HashMap<usize, &CoreExpr> {
    fn go<'a>(e: &'a CoreExpr, out: &mut HashMap<usize, &'a CoreExpr>) {
        out.insert(e as *const CoreExpr as usize, e);
        match &e.kind {
            CoreKind::Lit(_) | CoreKind::Var(_) => {}
            CoreKind::App(f, args) => {
                go(f, out);
                args.iter().for_each(|a| go(a, out));
            }
            CoreKind::Builtin(_, a) | CoreKind::Ctor(_, a) | CoreKind::Prim(_, a) => {
                a.iter().for_each(|x| go(x, out));
            }
            CoreKind::Perform(p) => p.args.iter().for_each(|a| go(a, out)),
            CoreKind::Lambda(_, body) | CoreKind::Resume(body) => go(body, out),
            CoreKind::Let(_, v, b) => {
                go(v, out);
                go(b, out);
            }
            CoreKind::If(c, t, f) => {
                go(c, out);
                go(t, out);
                go(f, out);
            }
            CoreKind::Match(s, arms) => {
                go(s, out);
                arms.iter().for_each(|a| go(&a.body, out));
            }
            CoreKind::Handle(h) => {
                go(&h.body, out);
                h.clauses.iter().for_each(|c| go(&c.body, out));
                if let Some(r) = &h.ret {
                    go(&r.body, out);
                }
            }
        }
    }
    let mut out = HashMap::new();
    for f in &core.fns {
        go(&f.body, &mut out);
    }
    out
}

/// The refusals that keep D16's convention and D17's handle cut sound, run
/// before anything is emitted (like the arity caps in `build_module`).
pub(crate) fn prepass(core: &CoreModule, cps_fns: &HashSet<String>) -> R<()> {
    let fns: HashMap<&str, &elya::core::CoreFn> =
        core.fns.iter().map(|f| (f.name.as_str(), f)).collect();
    for f in &core.fns {
        if cps_fns.contains(&f.name) {
            // D14: the continuation is one more LLVM parameter.
            if f.params.len() + 1 > MAX_PARAMS {
                return Err(CodegenError::Unsupported(
                    "effectful function takes more than four parameters",
                ));
            }
            // The front end's main-discharge check makes this unreachable; a
            // guard, not a feature.
            if f.name == "main" {
                return Err(CodegenError::Unsupported("main performs an effect"));
            }
        }
    }
    for f in &core.fns {
        let mut scope: Vec<String> = f.params.iter().map(|p| p.name.clone()).collect();
        let region_cps = cps_fns.contains(&f.name);
        check(&f.body, &mut scope, false, region_cps, &fns, cps_fns)?;
        let mut locals: Vec<(String, Option<Ty>)> = f
            .params
            .iter()
            .map(|p| (p.name.clone(), Some(p.ty.clone())))
            .collect();
        check_local_conventions(&f.body, &mut locals)?;
    }
    Ok(())
}

/// D16 for LOCAL bindings (5b-9b review). A closure's convention is fixed by
/// the lambda's own type at its definition (`LambdaSite::effectful`); a call
/// site chooses by the callee's type at the USE. They disagree when a binding is
/// generic in its effect row (`let app = fn(g) { g() + 1 }`: open row, direct
/// convention) and the use instantiates it at a user effect -- the call would
/// jump into a direct function with the CPS signature (measured: SIGSEGV where
/// the evaluator printed 8). Refused by the same name as the top-level case.
/// Binders whose type is not tracked (pattern and return binders, `$cont`)
/// carry `None` and are never refused here.
fn check_local_conventions(e: &CoreExpr, locals: &mut Vec<(String, Option<Ty>)>) -> R<()> {
    let go =
        |c: &CoreExpr, locals: &mut Vec<(String, Option<Ty>)>| check_local_conventions(c, locals);
    match &e.kind {
        CoreKind::Lit(_) | CoreKind::Var(_) => Ok(()),
        CoreKind::App(callee, args) => {
            if let CoreKind::Var(name) = &callee.kind {
                if let Some((_, Some(bound))) = locals.iter().rev().find(|(n, _)| n == name) {
                    if needs_cps(&callee.ty) && !needs_cps(bound) {
                        return Err(CodegenError::Unsupported(
                            "effect-polymorphic function used at a user effect",
                        ));
                    }
                }
            }
            go(callee, locals)?;
            args.iter().try_for_each(|a| go(a, locals))
        }
        CoreKind::Builtin(_, a) | CoreKind::Ctor(_, a) | CoreKind::Prim(_, a) => {
            a.iter().try_for_each(|x| go(x, locals))
        }
        CoreKind::Perform(p) => p.args.iter().try_for_each(|a| go(a, locals)),
        CoreKind::Resume(v) => go(v, locals),
        CoreKind::Let(x, v, body) => {
            go(v, locals)?;
            locals.push((x.clone(), Some(v.ty.clone())));
            let r = go(body, locals);
            locals.pop();
            r
        }
        CoreKind::If(c, t, f) => {
            go(c, locals)?;
            go(t, locals)?;
            go(f, locals)
        }
        CoreKind::Match(s, arms) => {
            go(s, locals)?;
            for arm in arms.iter() {
                let mut names = Vec::new();
                crate::closure::pat_binders(&arm.pat, &mut names);
                let depth = locals.len();
                locals.extend(names.into_iter().map(|n| (n, None)));
                let r = go(&arm.body, locals);
                locals.truncate(depth);
                r?;
            }
            Ok(())
        }
        CoreKind::Lambda(params, body) => {
            let depth = locals.len();
            locals.extend(params.iter().map(|p| (p.name.clone(), Some(p.ty.clone()))));
            let r = go(body, locals);
            locals.truncate(depth);
            r
        }
        CoreKind::Handle(h) => {
            go(&h.body, locals)?;
            for c in h.clauses.iter() {
                let depth = locals.len();
                locals.extend(
                    c.params
                        .iter()
                        .map(|p| (p.name.clone(), Some(p.ty.clone()))),
                );
                locals.push((crate::closure::CONT.to_string(), None));
                let r = go(&c.body, locals);
                locals.truncate(depth);
                r?;
            }
            if let Some(r) = &h.ret {
                locals.push((r.binder.clone(), None));
                let out = go(&r.body, locals);
                locals.pop();
                out?;
            }
            Ok(())
        }
    }
}

fn check(
    e: &CoreExpr,
    scope: &mut Vec<String>,
    in_handle: bool,
    region_cps: bool,
    fns: &HashMap<&str, &elya::core::CoreFn>,
    cps_fns: &HashSet<String>,
) -> R<()> {
    let go = |c: &CoreExpr, scope: &mut Vec<String>| {
        check(c, scope, in_handle, region_cps, fns, cps_fns)
    };
    match &e.kind {
        CoreKind::Lit(_) | CoreKind::Var(_) => Ok(()),
        CoreKind::App(callee, args) => {
            if let CoreKind::Var(g) = &callee.kind {
                if !scope.iter().any(|s| s == g) {
                    if let Some(target) = fns.get(g.as_str()) {
                        check_reference(target, &callee.ty, cps_fns)?;
                    }
                }
            }
            go(callee, scope)?;
            args.iter().try_for_each(|a| go(a, scope))
        }
        CoreKind::Builtin(_, a) | CoreKind::Ctor(_, a) | CoreKind::Prim(_, a) => {
            a.iter().try_for_each(|x| go(x, scope))
        }
        CoreKind::Perform(p) => {
            if p.args.len() + 2 > MAX_PARAMS {
                return Err(CodegenError::Unsupported(
                    "effect operation takes more than three parameters",
                ));
            }
            p.args.iter().try_for_each(|a| go(a, scope))
        }
        CoreKind::Resume(v) => go(v, scope),
        CoreKind::Let(x, v, body) => {
            go(v, scope)?;
            scope.push(x.clone());
            let r = go(body, scope);
            scope.pop();
            r
        }
        CoreKind::If(c, t, f) => {
            go(c, scope)?;
            go(t, scope)?;
            go(f, scope)
        }
        CoreKind::Match(s, arms) => {
            go(s, scope)?;
            for arm in arms.iter() {
                let depth = scope.len();
                crate::closure::pat_binders(&arm.pat, scope);
                let r = go(&arm.body, scope);
                scope.truncate(depth);
                r?;
            }
            Ok(())
        }
        CoreKind::Lambda(params, body) => {
            // 5b-9b: an effectful lambda compiles with the CPS convention: the
            // closure, its parameters and the continuation (MAX_PARAMS).
            let effectful = needs_cps(&e.ty);
            if effectful && params.len() + 2 > MAX_PARAMS {
                return Err(CodegenError::Unsupported(
                    "effectful lambda takes more than three parameters",
                ));
            }
            let depth = scope.len();
            scope.extend(params.iter().map(|p| p.name.clone()));
            // Its body is a CPS region, so D17's handle refusals apply in it.
            let r = check(body, scope, in_handle, effectful, fns, cps_fns);
            scope.truncate(depth);
            r
        }
        CoreKind::Handle(h) => {
            // spec 5.4 / A2: refused on the handle node, FIRST -- before any
            // other check and before any clause body is looked at. Keyed on the
            // bit lowering stamped from the effect's declaration (D4).
            if h.is_multi_declared {
                return Err(CodegenError::Unsupported(
                    "multi-shot handler (`with multi`)",
                ));
            }
            if in_handle {
                return Err(CodegenError::Unsupported(
                    "handle nested inside another handle",
                ));
            }
            // A clause takes the op's arguments, the continuation and the
            // handler frame (Task 8): MAX_PARAMS is the measured win64 limit.
            if h.clauses.iter().any(|c| c.params.len() + 2 > MAX_PARAMS) {
                return Err(CodegenError::Unsupported(
                    "effect operation takes more than three parameters",
                ));
            }
            if region_cps {
                return Err(CodegenError::Unsupported(
                    "handle inside an effectful function",
                ));
            }
            check(&h.body, scope, true, true, fns, cps_fns)?;
            for c in h.clauses.iter() {
                let depth = scope.len();
                scope.extend(c.params.iter().map(|p| p.name.clone()));
                scope.push(crate::closure::CONT.to_string());
                let r = check(&c.body, scope, true, false, fns, cps_fns);
                scope.truncate(depth);
                r?;
            }
            if let Some(r) = &h.ret {
                scope.push(r.binder.clone());
                let out = check(&r.body, scope, true, false, fns, cps_fns);
                scope.pop();
                out?;
            }
            Ok(())
        }
    }
}

/// D16's two refusals at a reference to a top-level function.
fn check_reference(target: &elya::core::CoreFn, at: &Ty, cps_fns: &HashSet<String>) -> R<()> {
    let generic_is_polymorphic = target.params.iter().any(|p| cps::ty_has_open_row(&p.ty))
        || cps::ty_has_open_row(&target.body.ty);
    if generic_is_polymorphic && cps::ty_names_user_effect(at) {
        return Err(CodegenError::Unsupported(
            "effect-polymorphic function used at a user effect",
        ));
    }
    if needs_cps(at) != cps_fns.contains(&target.name) {
        return Err(CodegenError::Unsupported(
            "calling convention disagrees with the callee",
        ));
    }
    Ok(())
}

// ------------------------------------------------------------ declarations --

/// Task 8: every `(effect, op)` the module performs or handles, numbered in
/// sorted order -- the index into every handle's clause table.
pub(crate) fn op_ids(core: &CoreModule) -> HashMap<(String, String), usize> {
    let mut all = std::collections::BTreeSet::new();
    for e in index_nodes(core).values() {
        match &e.kind {
            CoreKind::Perform(p) => {
                all.insert((p.effect.clone(), p.op.clone()));
            }
            CoreKind::Handle(h) => {
                for c in h.clauses.iter() {
                    all.insert((c.effect.clone(), c.op.clone()));
                }
            }
            _ => {}
        }
    }
    all.into_iter().enumerate().map(|(i, k)| (k, i)).collect()
}

/// A handle's functions (7b-3 and Task 8).
#[derive(Clone)]
pub(crate) struct HandlerFns<'ctx> {
    /// `(ptr frame) -> i64`: the handled body, a CPS region.
    pub(crate) body: FunctionValue<'ctx>,
    /// `(i64, ptr frame) -> i64`: the return clause, the frame's code.
    pub(crate) ret: FunctionValue<'ctx>,
    /// One per clause, in clause order: `(op args.., ptr cont, ptr frame) -> i64`.
    pub(crate) clauses: Vec<FunctionValue<'ctx>>,
    /// The static clause table, indexed by `op_ids`: a clause's code address,
    /// or 0 where this handle has no clause for that op.
    pub(crate) table: PointerValue<'ctx>,
}

/// One resumption function per continuation site (calls and, since Task 8,
/// performs), and per handle its body, return clause, clause functions and
/// static clause table.
#[allow(clippy::type_complexity)]
pub(crate) fn declare<'ctx>(
    ctx: &'ctx Context,
    module: &Module<'ctx>,
    nodes: &HashMap<usize, &CoreExpr>,
    sites: &[ContSite],
    handlers: &[HandlerSite],
    op_ids: &HashMap<(String, String), usize>,
) -> R<(
    HashMap<usize, FunctionValue<'ctx>>,
    HashMap<usize, HandlerFns<'ctx>>,
)> {
    let i64t = ctx.i64_type();
    let ptrt = ctx.ptr_type(AddressSpace::default());
    let code_ty = i64t.fn_type(&[i64t.into(), ptrt.into()], false);
    let body_ty = i64t.fn_type(&[ptrt.into()], false);
    let mut resume = HashMap::new();
    for (i, s) in sites.iter().enumerate() {
        if !matches!(
            nodes.get(&s.key).map(|n| &n.kind),
            Some(CoreKind::App(..)) | Some(CoreKind::Perform(_))
        ) {
            continue;
        }
        let f = module.add_function(&mangle(&format!("{}.k.{i}", s.owner)), code_ty, None);
        f.set_call_conventions(TAILCC);
        resume.insert(s.key, f);
    }
    let mut hfns = HashMap::new();
    for (i, h) in handlers.iter().enumerate() {
        let body = module.add_function(&mangle(&format!("{}.h.{i}.body", h.owner)), body_ty, None);
        body.set_call_conventions(TAILCC);
        let ret = module.add_function(&mangle(&format!("{}.h.{i}.ret", h.owner)), code_ty, None);
        ret.set_call_conventions(TAILCC);
        let Some(CoreKind::Handle(hd)) = nodes.get(&h.key).map(|n| &n.kind) else {
            return Err(internal("a handler key names a non-handle node"));
        };
        let mut clauses = Vec::with_capacity(hd.clauses.len());
        let mut entries = vec![i64t.const_int(0, false); op_ids.len()];
        let mut filled = std::collections::HashSet::new();
        for (j, c) in hd.clauses.iter().enumerate() {
            let mut sig: Vec<BasicMetadataTypeEnum<'ctx>> = Vec::new();
            for p in c.params.iter() {
                sig.push(repr_ty(ctx, &p.ty)?.into());
            }
            sig.push(ptrt.into());
            sig.push(ptrt.into());
            let f = module.add_function(
                &mangle(&format!("{}.h.{i}.c.{j}", h.owner)),
                i64t.fn_type(&sig, false),
                None,
            );
            f.set_call_conventions(TAILCC);
            let id = *op_ids
                .get(&(c.effect.clone(), c.op.clone()))
                .ok_or_else(|| internal("a clause's op has no id"))?;
            // The FIRST clause for an op wins, as in the evaluator (`find`). Since
            // slice 5c-2 a second clause for one op is E0204, so no source program
            // reaches this; it stays so the two layers agree by construction.
            if filled.insert(id) {
                entries[id] = f.as_global_value().as_pointer_value().const_to_int(i64t);
            }
            clauses.push(f);
        }
        let arr = i64t.const_array(&entries);
        let g = module.add_global(arr.get_type(), Some(AddressSpace::default()), "ctab");
        g.set_initializer(&arr);
        g.set_constant(true);
        g.set_linkage(inkwell::module::Linkage::Private);
        hfns.insert(
            h.key,
            HandlerFns {
                body,
                ret,
                clauses,
                table: g.as_pointer_value(),
            },
        );
    }
    Ok((resume, hfns))
}

// ------------------------------------------------------------------- state --

/// The emitter's view of one CPS region.
struct St<'ctx> {
    /// Name view for the direct emitter: each name's innermost binding below
    /// `depth`, plus `$kont`.
    env: HashMap<String, BasicValueEnum<'ctx>>,
    /// Bindings by BINDING index (the analysis's scope index), so two live
    /// bindings that share a name stay distinct (7b-2's binding identity).
    binds: BTreeMap<usize, (String, BasicValueEnum<'ctx>)>,
    depth: usize,
    /// Values of already-evaluated operands, by node address (`Saved::Temp`).
    temps: HashMap<usize, BasicValueEnum<'ctx>>,
    /// Shadow-stack pushes this function still owes a pop. Every terminator
    /// (`musttail` jump, trap) pops them all first.
    pending: usize,
    kont: PointerValue<'ctx>,
}

impl<'ctx> St<'ctx> {
    fn new(kont: PointerValue<'ctx>) -> Self {
        let mut env = HashMap::new();
        env.insert(KONT.to_string(), kont.into());
        St {
            env,
            binds: BTreeMap::new(),
            depth: 0,
            temps: HashMap::new(),
            pending: 0,
            kont,
        }
    }

    /// The name view at `depth`: each name's innermost binding under its own
    /// name, and every binding it shadows parked under a hidden key
    /// (`bind_local`), so the direct emitter's `gc_root_env` roots shadowed
    /// heap values too (found by the review of the shadowing fix).
    fn rebuild_env(&mut self) {
        self.env.clear();
        for (_, (n, v)) in self.binds.range(..self.depth) {
            let _ = bind_local(&mut self.env, n, *v);
        }
        self.env.insert(KONT.to_string(), self.kont.into());
    }

    /// Bind `x` at the next index; returns what `unbind` needs.
    fn bind(&mut self, x: &str, v: BasicValueEnum<'ctx>) -> (usize, Shadowed) {
        let at = self.depth;
        self.binds.insert(at, (x.to_string(), v));
        self.depth += 1;
        (at, bind_local(&mut self.env, x, v))
    }

    fn unbind(&mut self, x: &str, (at, shadowed): (usize, Shadowed)) {
        self.binds.remove(&at);
        self.depth = at;
        unbind_local(&mut self.env, x, shadowed);
    }
}

/// Everything one emitter call needs; bundled so the recursive folds thread
/// one reference.
struct Cx<'a, 'b, 'ctx> {
    ctx: &'ctx Context,
    func: FunctionValue<'ctx>,
    b: &'b Builder<'ctx>,
    lc: &'b LowerCtx<'a, 'ctx>,
}

impl<'a, 'b, 'ctx> Cx<'a, 'b, 'ctx> {
    fn i64t(&self) -> inkwell::types::IntType<'ctx> {
        self.ctx.i64_type()
    }

    fn ptrt(&self) -> inkwell::types::PointerType<'ctx> {
        self.ctx.ptr_type(AddressSpace::default())
    }

    fn push(&self, v: BasicValueEnum<'ctx>) -> R<bool> {
        if !v.is_pointer_value() {
            return Ok(false);
        }
        self.b
            .build_call(self.lc.gc_push, &[v.into()], "")
            .map_err(internal)?;
        Ok(true)
    }

    /// Root what this function still needs across a call that may collect:
    /// the continuation, every binding below `depth` (shadowed ones included),
    /// and `extra`. Returns the count to pop.
    fn root_live(&self, st: &St<'ctx>, extra: &[BasicValueEnum<'ctx>]) -> R<usize> {
        let mut n = 0;
        if self.push(st.kont.into())? {
            n += 1;
        }
        for (_, (_, v)) in st.binds.range(..st.depth) {
            if self.push(*v)? {
                n += 1;
            }
        }
        for v in extra {
            if self.push(*v)? {
                n += 1;
            }
        }
        Ok(n)
    }

    fn pop_pending(&self, st: &mut St<'ctx>) -> R<()> {
        gc_unroot(self.b, self.lc, st.pending)?;
        st.pending = 0;
        Ok(())
    }

    fn alloc(&self, words: usize) -> R<PointerValue<'ctx>> {
        Ok(self
            .b
            .build_call(
                self.lc.alloc,
                &[self.i64t().const_int(words as u64, false).into()],
                "fr",
            )
            .map_err(internal)?
            .try_as_basic_value()
            .left()
            .ok_or(CodegenError::Unsupported("elya_alloc returned no value"))?
            .into_pointer_value())
    }

    fn store_word(&self, p: PointerValue<'ctx>, at: usize, w: IntValue<'ctx>) -> R<()> {
        let i64t = self.i64t();
        let slot = unsafe {
            self.b
                .build_gep(i64t, p, &[i64t.const_int(at as u64, false)], "w")
        }
        .map_err(internal)?;
        self.b.build_store(slot, w).map_err(internal)?;
        Ok(())
    }

    fn load_word(&self, p: PointerValue<'ctx>, at: usize) -> R<IntValue<'ctx>> {
        let i64t = self.i64t();
        let slot = unsafe {
            self.b
                .build_gep(i64t, p, &[i64t.const_int(at as u64, false)], "w")
        }
        .map_err(internal)?;
        Ok(self
            .b
            .build_load(i64t, slot, "lw")
            .map_err(internal)?
            .into_int_value())
    }

    fn fn_word(&self, f: FunctionValue<'ctx>) -> R<IntValue<'ctx>> {
        self.b
            .build_ptr_to_int(f.as_global_value().as_pointer_value(), self.i64t(), "f2i")
            .map_err(internal)
    }

    /// `musttail` + `ret`: the transfer every CPS terminator is made of (D14).
    fn tail_jump(&self, site: inkwell::values::CallSiteValue<'ctx>) -> R<()> {
        site.set_call_convention(TAILCC);
        site.set_tail_call_kind(LLVMTailCallKind::LLVMTailCallKindMustTail);
        let v = site
            .try_as_basic_value()
            .left()
            .ok_or(CodegenError::Unsupported("call returned no value"))?;
        self.b.build_return(Some(&v)).map_err(internal)?;
        Ok(())
    }

    /// Hand `v` to the region's continuation: jump to `k`'s code with `(v, k)`.
    fn cps_return(&self, st: &mut St<'ctx>, v: BasicValueEnum<'ctx>, ty: &Ty) -> R<()> {
        self.pop_pending(st)?;
        let word = value_to_word(self.b, v, ty, self.i64t())?;
        let code = self.load_word(st.kont, 1)?;
        let fp = self
            .b
            .build_int_to_ptr(code, self.ptrt(), "kc")
            .map_err(internal)?;
        let code_ty = self
            .i64t()
            .fn_type(&[self.i64t().into(), self.ptrt().into()], false);
        let site = self
            .b
            .build_indirect_call(code_ty, fp, &[word.into(), st.kont.into()], "kr")
            .map_err(internal)?;
        self.tail_jump(site)
    }

    // ------------------------------------------------------------- calls --

    /// A continuation site (D10): save what the rest of the region needs in a
    /// frame whose code is the site's resumption function, then `musttail`
    /// the callee with that frame as its continuation.
    fn site_frame(
        &self,
        st: &mut St<'ctx>,
        node: &CoreExpr,
        args: &[BasicValueEnum<'ctx>],
    ) -> R<PointerValue<'ctx>> {
        let key = node as *const CoreExpr as usize;
        let site =
            &self.lc.sites[*self
                .lc
                .site_index
                .get(&key)
                .ok_or(CodegenError::Unsupported(
                    "continuation site missing from the pre-pass",
                ))?];
        let tag = *self
            .lc
            .desc
            .site_tags
            .get(&key)
            .ok_or(CodegenError::Unsupported(
                "continuation site has no descriptor row",
            ))?;
        // A frame that saves nothing is exactly Task 6's `[2, 0b10]` row; any
        // other tag there would make the collector read words that are not
        // there.
        if site.saved.is_empty() && tag != self.lc.desc.frame_tag {
            return Err(internal("an empty frame does not use the frame row"));
        }
        let resume = *self
            .lc
            .resume_fns
            .get(&key)
            .ok_or(CodegenError::Unsupported(
                "continuation site has no resumption function",
            ))?;
        let mut saved = Vec::with_capacity(site.saved.len());
        for (sv, ty) in &site.saved {
            // 7b-2 review obligation (a): a value of unresolved type has no
            // representation and no trace bit. `repr_ty` refuses every such node
            // before emission today; this is the named guard at the frame itself.
            if matches!(ty, Ty::Var(_)) {
                return Err(CodegenError::Unsupported(
                    "saved value of unresolved type (N7)",
                ));
            }
            let v = match sv {
                Saved::Temp(a) => *st.temps.get(a).ok_or(CodegenError::Unsupported(
                    "saved temporary is not available",
                ))?,
                Saved::Var { name, binding } => match st.binds.get(binding) {
                    Some((n, v)) if n == name => *v,
                    _ => return Err(CodegenError::Unsupported("saved binding is not available")),
                },
            };
            saved.push((v, ty));
        }
        let mut extra: Vec<BasicValueEnum<'ctx>> = saved.iter().map(|(v, _)| *v).collect();
        extra.extend_from_slice(args);
        let roots = self.root_live(st, &extra)?;
        let p = self.alloc(3 + saved.len())?;
        self.store_word(p, 0, self.i64t().const_int(tag as u64, false))?;
        self.store_word(p, 1, self.fn_word(resume)?)?;
        let next = self
            .b
            .build_ptr_to_int(st.kont, self.i64t(), "nx")
            .map_err(internal)?;
        self.store_word(p, 2, next)?;
        for (j, (v, ty)) in saved.iter().enumerate() {
            let w = value_to_word(self.b, *v, ty, self.i64t())?;
            self.store_word(p, 3 + j, w)?;
        }
        gc_unroot(self.b, self.lc, roots)?;
        Ok(p)
    }

    /// A non-tail call of an effectful function: build the site's frame, then
    /// `musttail` the callee with it as its continuation.
    fn site_call(
        &self,
        st: &mut St<'ctx>,
        node: &CoreExpr,
        target: FunctionValue<'ctx>,
        args: &[BasicValueEnum<'ctx>],
    ) -> R<()> {
        let p = self.site_frame(st, node, args)?;
        self.pop_pending(st)?;
        let mut vals: Vec<BasicMetadataValueEnum<'ctx>> =
            args.iter().map(|v| (*v).into()).collect();
        vals.push(p.into());
        let call = self.b.build_call(target, &vals, "sc").map_err(internal)?;
        self.tail_jump(call)
    }

    /// Task 8: a perform. Its continuation is the current one (tail position)
    /// or a fresh site frame linked to it. With D17 the chain always ends at
    /// the handler frame, so the handler is already BENEATH the captured frames
    /// (spec 4 point 4) -- deep re-installation with no copying. Wrap the chain
    /// in a one-shot continuation object (D13), find the handler at the chain's
    /// end, and jump to its clause for this `(effect, op)`.
    fn perform(
        &self,
        st: &mut St<'ctx>,
        node: &CoreExpr,
        args: &[BasicValueEnum<'ctx>],
        tail: bool,
    ) -> R<()> {
        let CoreKind::Perform(pf) = &node.kind else {
            return Err(internal("perform on a non-perform node"));
        };
        let k = if tail {
            st.kont
        } else {
            self.site_frame(st, node, args)?
        };
        let i64t = self.i64t();
        let mut extra: Vec<BasicValueEnum<'ctx>> = vec![k.into()];
        extra.extend_from_slice(args);
        let roots = self.root_live(st, &extra)?;
        let cont = self.alloc(4)?;
        gc_unroot(self.b, self.lc, roots)?;
        self.store_word(cont, 0, i64t.const_int(self.lc.desc.cont_tag as u64, false))?;
        let kw = self.b.build_ptr_to_int(k, i64t, "kw").map_err(internal)?;
        self.store_word(cont, 1, kw)?;
        self.store_word(cont, 2, i64t.const_int(0, false))?;
        // O(1): the handler this computation performs to (set by the handle
        // site, re-installed by every resume). Read AFTER the allocation; the
        // global does not move, and nothing here can collect.
        let h = self
            .b
            .build_load(self.ptrt(), self.lc.current_handler, "h")
            .map_err(internal)?
            .into_pointer_value();
        let hw = self.b.build_ptr_to_int(h, i64t, "hw").map_err(internal)?;
        self.store_word(cont, 3, hw)?;
        // No handler at all: unreachable under D17 (main is never CPS and
        // effectful closure calls are refused), but a perform with nothing to
        // perform to must stop by NAME, not dereference null.
        let none = self.b.build_is_null(h, "noh").map_err(internal)?;
        let nobody = self.ctx.append_basic_block(self.func, "no_handler");
        let found = self.ctx.append_basic_block(self.func, "handler");
        self.b
            .build_conditional_branch(none, nobody, found)
            .map_err(internal)?;
        self.b.position_at_end(nobody);
        self.b
            .build_call(self.lc.unhandled, &[], "nh")
            .map_err(internal)?;
        self.b.build_unreachable().map_err(internal)?;
        self.b.position_at_end(found);
        let table = self.load_word(h, 3)?;
        let table = self
            .b
            .build_int_to_ptr(table, self.ptrt(), "tab")
            .map_err(internal)?;
        let id = *self
            .lc
            .op_ids
            .get(&(pf.effect.clone(), pf.op.clone()))
            .ok_or_else(|| internal("a performed op has no id"))?;
        let entry = self.load_word(table, id)?;
        let missing = self
            .b
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                entry,
                i64t.const_int(0, false),
                "nc",
            )
            .map_err(internal)?;
        let bad = self.ctx.append_basic_block(self.func, "unhandled");
        let ok = self.ctx.append_basic_block(self.func, "dispatch");
        self.b
            .build_conditional_branch(missing, bad, ok)
            .map_err(internal)?;
        self.b.position_at_end(bad);
        self.b
            .build_call(self.lc.unhandled, &[], "uh")
            .map_err(internal)?;
        // `elya_unhandled_effect` exits; a placeholder terminator, NOT the trap.
        self.b.build_unreachable().map_err(internal)?;
        self.b.position_at_end(ok);
        self.pop_pending(st)?;
        let mut sig: Vec<BasicMetadataTypeEnum<'ctx>> = Vec::new();
        for a in pf.args.iter() {
            sig.push(repr_ty(self.ctx, &a.ty)?.into());
        }
        sig.push(self.ptrt().into());
        sig.push(self.ptrt().into());
        let clause_ty = i64t.fn_type(&sig, false);
        let fp = self
            .b
            .build_int_to_ptr(entry, self.ptrt(), "cl")
            .map_err(internal)?;
        let mut vals: Vec<BasicMetadataValueEnum<'ctx>> =
            args.iter().map(|v| (*v).into()).collect();
        vals.push(cont.into());
        vals.push(h.into());
        let call = self
            .b
            .build_indirect_call(clause_ty, fp, &vals, "pc")
            .map_err(internal)?;
        self.tail_jump(call)
    }

    /// A tail call of an effectful function: pass the current continuation on.
    fn tail_cps_call(
        &self,
        st: &mut St<'ctx>,
        target: FunctionValue<'ctx>,
        args: &[BasicValueEnum<'ctx>],
    ) -> R<()> {
        self.pop_pending(st)?;
        let mut vals: Vec<BasicMetadataValueEnum<'ctx>> =
            args.iter().map(|v| (*v).into()).collect();
        vals.push(st.kont.into());
        let call = self.b.build_call(target, &vals, "tc").map_err(internal)?;
        self.tail_jump(call)
    }

    fn direct_call(
        &self,
        st: &St<'ctx>,
        target: FunctionValue<'ctx>,
        args: &[BasicValueEnum<'ctx>],
    ) -> R<BasicValueEnum<'ctx>> {
        let roots = self.root_live(st, &[])?;
        let vals: Vec<BasicMetadataValueEnum<'ctx>> = args.iter().map(|v| (*v).into()).collect();
        let call = self.b.build_call(target, &vals, "dc").map_err(internal)?;
        call.set_call_convention(TAILCC);
        gc_unroot(self.b, self.lc, roots)?;
        call.try_as_basic_value()
            .left()
            .ok_or(CodegenError::Unsupported("call returned no value"))
    }

    /// 5b-9b: the indirect CPS call of an effectful closure, `code(clos,
    /// args.., k)`, as a `musttail` jump. `k` is a fresh site frame (non-tail)
    /// or the region's own continuation (tail).
    fn closure_cps_jump(
        &self,
        st: &mut St<'ctx>,
        callee: &CoreExpr,
        clos: PointerValue<'ctx>,
        args: &[BasicValueEnum<'ctx>],
        k: PointerValue<'ctx>,
    ) -> R<()> {
        let Ty::Fn(param_tys, _, _) = &callee.ty else {
            return Err(CodegenError::Unsupported("computed callee"));
        };
        if param_tys.len() != args.len() {
            return Err(CodegenError::Unsupported("closure call arity mismatch"));
        }
        let mut sig: Vec<BasicMetadataTypeEnum<'ctx>> = vec![self.ptrt().into()];
        for t in param_tys {
            sig.push(repr_ty(self.ctx, t)?.into());
        }
        sig.push(self.ptrt().into());
        let fn_ty = self.i64t().fn_type(&sig, false);
        let code = self.load_word(clos, 1)?;
        let fp = self
            .b
            .build_int_to_ptr(code, self.ptrt(), "ci2f")
            .map_err(internal)?;
        self.pop_pending(st)?;
        let mut vals: Vec<BasicMetadataValueEnum<'ctx>> = vec![clos.into()];
        vals.extend(args.iter().map(|v| BasicMetadataValueEnum::from(*v)));
        vals.push(k.into());
        let call = self
            .b
            .build_indirect_call(fn_ty, fp, &vals, "cc")
            .map_err(internal)?;
        self.tail_jump(call)
    }

    /// A non-tail effectful closure call: a continuation site, like
    /// `site_call`. The closure is rooted with the arguments while the frame
    /// is allocated.
    fn closure_site_call(
        &self,
        st: &mut St<'ctx>,
        node: &CoreExpr,
        callee: &CoreExpr,
        clos: BasicValueEnum<'ctx>,
        args: &[BasicValueEnum<'ctx>],
    ) -> R<()> {
        let mut live = vec![clos];
        live.extend_from_slice(args);
        let p = self.site_frame(st, node, &live)?;
        self.closure_cps_jump(st, callee, clos.into_pointer_value(), args, p)
    }

    fn closure_call(
        &self,
        st: &St<'ctx>,
        callee: &CoreExpr,
        clos: BasicValueEnum<'ctx>,
        args: &[BasicValueEnum<'ctx>],
    ) -> R<BasicValueEnum<'ctx>> {
        let Ty::Fn(param_tys, _, ret_ty) = &callee.ty else {
            return Err(CodegenError::Unsupported("computed callee"));
        };
        if needs_cps(&callee.ty) {
            return Err(CodegenError::Unsupported(
                "effectful closure call (not yet compiled natively)",
            ));
        }
        if param_tys.len() != args.len() {
            return Err(CodegenError::Unsupported("closure call arity mismatch"));
        }
        let mut sig: Vec<BasicMetadataTypeEnum<'ctx>> = vec![self.ptrt().into()];
        for t in param_tys {
            sig.push(repr_ty(self.ctx, t)?.into());
        }
        let fn_ty = fn_type_of(repr_ty(self.ctx, ret_ty)?, &sig)?;
        let clos = clos.into_pointer_value();
        let roots = self.root_live(st, &[clos.into()])?;
        let code = self.load_word(clos, 1)?;
        let fp = self
            .b
            .build_int_to_ptr(code, self.ptrt(), "i2f")
            .map_err(internal)?;
        let mut vals: Vec<BasicMetadataValueEnum<'ctx>> = vec![clos.into()];
        vals.extend(args.iter().map(|v| BasicMetadataValueEnum::from(*v)));
        let call = self
            .b
            .build_indirect_call(fn_ty, fp, &vals, "ci")
            .map_err(internal)?;
        call.set_call_convention(TAILCC);
        gc_unroot(self.b, self.lc, roots)?;
        call.try_as_basic_value()
            .left()
            .ok_or(CodegenError::Unsupported("call returned no value"))
    }

    fn ctor(
        &self,
        st: &St<'ctx>,
        name: &str,
        vals: &[BasicValueEnum<'ctx>],
    ) -> R<BasicValueEnum<'ctx>> {
        let (tag, field_tys) = self
            .lc
            .ctors
            .get(name)
            .cloned()
            .ok_or(CodegenError::Unsupported("parametric ADT"))?;
        let roots = self.root_live(st, vals)?;
        let p = self.alloc(1 + field_tys.len())?;
        self.store_word(p, 0, self.i64t().const_int(tag as u64, false))?;
        for (i, v) in vals.iter().enumerate() {
            let w = value_to_word(self.b, *v, &field_tys[i], self.i64t())?;
            self.store_word(p, i + 1, w)?;
        }
        gc_unroot(self.b, self.lc, roots)?;
        Ok(p.into())
    }

    fn prim(
        &self,
        op: BinOp,
        args: &[CoreExpr],
        vals: &[BasicValueEnum<'ctx>],
    ) -> R<BasicValueEnum<'ctx>> {
        // The same refusals, in the same order, as `lower_expr`'s Prim arm.
        if vals.len() != 2 {
            return Err(CodegenError::Unsupported("binary Prim arity"));
        }
        if matches!(op, BinOp::Eq | BinOp::Ne)
            && !matches!(args[0].ty, Ty::Base(TyCon::Int) | Ty::Base(TyCon::Bool))
        {
            return Err(CodegenError::Unsupported(eq_operand_label(op)));
        }
        if matches!(op, BinOp::Concat) {
            return Err(CodegenError::Unsupported(op_label(op)));
        }
        prim_values(
            self.b,
            op,
            vals[0].into_int_value(),
            vals[1].into_int_value(),
        )
    }

    fn println(&self, vals: &[BasicValueEnum<'ctx>]) -> R<BasicValueEnum<'ctx>> {
        if vals.len() != 1 {
            return Err(CodegenError::Unsupported(
                "io.println takes exactly one argument",
            ));
        }
        self.b
            .build_call(self.lc.println, &[vals[0].into()], "pl")
            .map_err(internal)?;
        Ok(self.i64t().const_int(0, false).into())
    }

    // ------------------------------------------------------------ folds --

    /// Evaluate one operand left to right; remember its value if it is a
    /// temporary (a site may save it) and root it while later operands run.
    fn operand(&self, st: &mut St<'ctx>, op: &CoreExpr) -> R<Option<BasicValueEnum<'ctx>>> {
        let Some(v) = self.expr(st, op)? else {
            return Ok(None);
        };
        if !matches!(op.kind, CoreKind::Lit(_) | CoreKind::Var(_)) {
            st.temps.insert(op as *const CoreExpr as usize, v);
        }
        // Root every heap operand except a binding (bindings are rooted from
        // `binds`). A string LITERAL allocates, so it is rooted too: later
        // operands may collect while it waits (found by the 7b-3 review).
        if !matches!(op.kind, CoreKind::Var(_)) && self.push(v)? {
            st.pending += 1;
        }
        Ok(Some(v))
    }

    fn operands(
        &self,
        st: &mut St<'ctx>,
        ops: &[CoreExpr],
    ) -> R<Option<Vec<BasicValueEnum<'ctx>>>> {
        let mut out = Vec::with_capacity(ops.len());
        for op in ops {
            match self.operand(st, op)? {
                Some(v) => out.push(v),
                None => return Ok(None),
            }
        }
        Ok(Some(out))
    }

    /// Pop the operand roots pushed since `before`.
    fn settle(&self, st: &mut St<'ctx>, before: usize) -> R<()> {
        gc_unroot(self.b, self.lc, st.pending - before)?;
        st.pending = before;
        Ok(())
    }

    /// Value position in a CPS region. `None`: control left this function at a
    /// continuation site (or a trap); the rest runs in a resumption function.
    fn expr(&self, st: &mut St<'ctx>, e: &CoreExpr) -> R<Option<BasicValueEnum<'ctx>>> {
        if !contains_effect(e) {
            return Ok(Some(lower_expr(
                self.ctx,
                self.func,
                self.b,
                self.lc,
                e,
                &mut st.env,
            )?));
        }
        let before = st.pending;
        match &e.kind {
            CoreKind::Perform(p) => {
                let Some(vals) = self.operands(st, &p.args)? else {
                    return Ok(None);
                };
                self.perform(st, e, &vals, false)?;
                Ok(None)
            }
            CoreKind::App(callee, args) => {
                let CoreKind::Var(name) = &callee.kind else {
                    return Err(CodegenError::Unsupported("computed callee"));
                };
                let local = st.env.get(name).copied();
                let Some(vals) = self.operands(st, args)? else {
                    return Ok(None);
                };
                if let Some(clos) = local {
                    if needs_cps(&callee.ty) {
                        self.closure_site_call(st, e, callee, clos, &vals)?;
                        return Ok(None);
                    }
                    let v = self.closure_call(st, callee, clos, &vals)?;
                    self.settle(st, before)?;
                    return Ok(Some(v));
                }
                let target = *self.lc.decls.get(name).ok_or(CodegenError::Unsupported(
                    "callee is not a top-level function",
                ))?;
                if self.lc.cps_fns.contains(name) {
                    self.site_call(st, e, target, &vals)?;
                    return Ok(None);
                }
                let v = self.direct_call(st, target, &vals)?;
                self.settle(st, before)?;
                Ok(Some(v))
            }
            CoreKind::Prim(op, args) => {
                let Some(vals) = self.operands(st, args)? else {
                    return Ok(None);
                };
                let v = self.prim(*op, args, &vals)?;
                self.settle(st, before)?;
                Ok(Some(v))
            }
            CoreKind::Builtin(name, args) => {
                if name != "io.println" {
                    return Err(CodegenError::UnsupportedBuiltin(name.clone()));
                }
                let Some(vals) = self.operands(st, args)? else {
                    return Ok(None);
                };
                let v = self.println(&vals)?;
                self.settle(st, before)?;
                Ok(Some(v))
            }
            CoreKind::Ctor(name, fields) => {
                let Some(vals) = self.operands(st, fields)? else {
                    return Ok(None);
                };
                let v = self.ctor(st, name, &vals)?;
                self.settle(st, before)?;
                Ok(Some(v))
            }
            CoreKind::Let(x, v, body) => {
                let Some(val) = self.expr(st, v)? else {
                    return Ok(None);
                };
                let saved = st.bind(x, val);
                let out = self.expr(st, body);
                st.unbind(x, saved);
                out
            }
            CoreKind::If(c, t, f) => {
                let Some(cv) = self.expr(st, c)? else {
                    return Ok(None);
                };
                self.branch_value(st, cv, t, f)
            }
            CoreKind::Match(s, arms) => {
                let Some(sv) = self.expr(st, s)? else {
                    return Ok(None);
                };
                self.match_dispatch(st, sv, s, arms, false)
            }
            CoreKind::Resume(_) => {
                Err(CodegenError::Unsupported("resume outside a handler clause"))
            }
            _ => Err(internal("an effect-free node reached the CPS fold")),
        }
    }

    fn branch_value(
        &self,
        st: &mut St<'ctx>,
        cv: BasicValueEnum<'ctx>,
        t: &CoreExpr,
        f: &CoreExpr,
    ) -> R<Option<BasicValueEnum<'ctx>>> {
        let c = cv.into_int_value();
        let then_bb = self.ctx.append_basic_block(self.func, "cthen");
        let else_bb = self.ctx.append_basic_block(self.func, "celse");
        let join_bb = self.ctx.append_basic_block(self.func, "cjoin");
        self.b
            .build_conditional_branch(c, then_bb, else_bb)
            .map_err(internal)?;
        let snap = st.pending;
        let mut incoming: Vec<(BasicValueEnum<'ctx>, BasicBlock<'ctx>)> = Vec::new();
        for (bb, arm) in [(then_bb, t), (else_bb, f)] {
            self.b.position_at_end(bb);
            st.pending = snap;
            if let Some(v) = self.expr(st, arm)? {
                let exit = self
                    .b
                    .get_insert_block()
                    .ok_or(CodegenError::Unsupported("builder left no block"))?;
                self.b
                    .build_unconditional_branch(join_bb)
                    .map_err(internal)?;
                incoming.push((v, exit));
            }
        }
        st.pending = snap;
        if incoming.is_empty() {
            join_bb
                .remove_from_function()
                .map_err(|_| internal("could not remove an unreachable join"))?;
            return Ok(None);
        }
        self.b.position_at_end(join_bb);
        let ty = incoming[0].0.get_type();
        let phi = match ty {
            inkwell::types::BasicTypeEnum::IntType(t) => self.b.build_phi(t, "cphi"),
            inkwell::types::BasicTypeEnum::PointerType(t) => self.b.build_phi(t, "cphi"),
            _ => return Err(CodegenError::Unsupported("unrepresentable type")),
        }
        .map_err(internal)?;
        for (v, bb) in &incoming {
            phi.add_incoming(&[(v, *bb)]);
        }
        Ok(Some(phi.as_basic_value()))
    }

    /// Slice 5b-9a: a `match` in a CPS region, on an already-evaluated
    /// scrutinee `sv` -- the effect-aware twin of the direct emitter's match
    /// (`lower_expr`'s `CoreKind::Match`): the same tag tests, field loads and
    /// trap on no match. Pattern variables are bound through `St`, in
    /// `pat_binders` order, so a site inside an arm finds them at the binding
    /// indices the analysis recorded, saves them, and roots them. `tail` emits
    /// each arm in tail position (returns `None`); otherwise the arms join in a
    /// phi, like `branch_value`, and an arm that ends in a site joins nothing.
    fn match_dispatch(
        &self,
        st: &mut St<'ctx>,
        sv: BasicValueEnum<'ctx>,
        scrutinee: &CoreExpr,
        arms: &[elya::core::CoreArm],
        tail: bool,
    ) -> R<Option<BasicValueEnum<'ctx>>> {
        if !matches!(scrutinee.ty, Ty::Con(..)) {
            return Err(CodegenError::Unsupported("match scrutinee is not an ADT"));
        }
        let i64t = self.i64t();
        let s = sv.into_pointer_value();
        let tag = self
            .b
            .build_load(i64t, s, "ctag")
            .map_err(internal)?
            .into_int_value();
        let join_bb = self.ctx.append_basic_block(self.func, "cmjoin");
        let mut incoming: Vec<(BasicValueEnum<'ctx>, BasicBlock<'ctx>)> = Vec::new();
        let mut fallthrough = self
            .b
            .get_insert_block()
            .ok_or(CodegenError::Unsupported("builder left no block"))?;
        let mut terminal = false;
        let snap = st.pending;
        for arm in arms.iter() {
            let body_bb = self.ctx.append_basic_block(self.func, "cmarm");
            // The arm's binders, in `pat_binders` order, with their values.
            let mut bindings: Vec<(String, BasicValueEnum<'ctx>)> = Vec::new();
            match &arm.pat {
                CorePat::Ctor(name, pat_args) => {
                    let (tag_idx, field_tys) = self
                        .lc
                        .ctors
                        .get(name)
                        .cloned()
                        .ok_or(CodegenError::Unsupported("parametric ADT"))?;
                    self.b.position_at_end(fallthrough);
                    let cmp = self
                        .b
                        .build_int_compare(
                            inkwell::IntPredicate::EQ,
                            tag,
                            i64t.const_int(tag_idx as u64, false),
                            "cmc",
                        )
                        .map_err(internal)?;
                    let next = self.ctx.append_basic_block(self.func, "cmnext");
                    self.b
                        .build_conditional_branch(cmp, body_bb, next)
                        .map_err(internal)?;
                    fallthrough = next;
                    self.b.position_at_end(body_bb);
                    for (pi, p) in pat_args.iter().enumerate() {
                        match p {
                            CorePat::Var(v) => {
                                let w = self.load_word(s, pi + 1)?;
                                let fv = word_to_value(
                                    self.b,
                                    w,
                                    &field_tys[pi],
                                    self.ctx.bool_type(),
                                    self.ptrt(),
                                )?;
                                bindings.push((v.clone(), fv));
                            }
                            CorePat::Wild => {}
                            CorePat::Ctor(..) => {
                                return Err(CodegenError::Unsupported("nested constructor pattern"))
                            }
                            CorePat::Lit(_) => {
                                return Err(CodegenError::Unsupported("literal pattern"))
                            }
                        }
                    }
                }
                CorePat::Wild | CorePat::Var(_) => {
                    self.b.position_at_end(fallthrough);
                    self.b
                        .build_unconditional_branch(body_bb)
                        .map_err(internal)?;
                    self.b.position_at_end(body_bb);
                    if let CorePat::Var(name) = &arm.pat {
                        bindings.push((name.clone(), sv));
                    }
                    terminal = true;
                }
                CorePat::Lit(_) => return Err(CodegenError::Unsupported("literal pattern")),
            }
            st.pending = snap;
            let mut undo = Vec::with_capacity(bindings.len());
            for (n, v) in &bindings {
                undo.push((n.clone(), st.bind(n, *v)));
            }
            let out = if tail {
                self.tail(st, &arm.body).map(|_| None)
            } else {
                self.expr(st, &arm.body)
            };
            for (n, saved) in undo.into_iter().rev() {
                st.unbind(&n, saved);
            }
            if let Some(v) = out? {
                let exit = self
                    .b
                    .get_insert_block()
                    .ok_or(CodegenError::Unsupported("builder left no block"))?;
                self.b
                    .build_unconditional_branch(join_bb)
                    .map_err(internal)?;
                incoming.push((v, exit));
            }
            if terminal {
                break;
            }
        }
        st.pending = snap;
        if !terminal {
            // No arm matched: the same named trap as the direct emitter.
            self.b.position_at_end(fallthrough);
            self.b
                .build_call(self.lc.fail, &[], "cmfail")
                .map_err(internal)?;
            self.b.build_unreachable().map_err(internal)?;
        }
        if incoming.is_empty() {
            join_bb
                .remove_from_function()
                .map_err(|_| internal("could not remove an unreachable join"))?;
            return Ok(None);
        }
        self.b.position_at_end(join_bb);
        let ty = incoming[0].0.get_type();
        let phi = match ty {
            inkwell::types::BasicTypeEnum::IntType(t) => self.b.build_phi(t, "cmphi"),
            inkwell::types::BasicTypeEnum::PointerType(t) => self.b.build_phi(t, "cmphi"),
            _ => return Err(CodegenError::Unsupported("unrepresentable type")),
        }
        .map_err(internal)?;
        for (v, bb) in &incoming {
            phi.add_incoming(&[(v, *bb)]);
        }
        Ok(Some(phi.as_basic_value()))
    }

    fn branch_tail(
        &self,
        st: &mut St<'ctx>,
        cv: BasicValueEnum<'ctx>,
        t: &CoreExpr,
        f: &CoreExpr,
    ) -> R<()> {
        let then_bb = self.ctx.append_basic_block(self.func, "tthen");
        let else_bb = self.ctx.append_basic_block(self.func, "telse");
        self.b
            .build_conditional_branch(cv.into_int_value(), then_bb, else_bb)
            .map_err(internal)?;
        let snap = st.pending;
        self.b.position_at_end(then_bb);
        self.tail(st, t)?;
        st.pending = snap;
        self.b.position_at_end(else_bb);
        self.tail(st, f)?;
        st.pending = snap;
        Ok(())
    }

    /// Tail position in a CPS region: every path ends in a `musttail` jump or
    /// a trap.
    fn tail(&self, st: &mut St<'ctx>, e: &CoreExpr) -> R<()> {
        match &e.kind {
            CoreKind::App(callee, args) => {
                if let CoreKind::Var(name) = &callee.kind {
                    // 5b-9b: an effectful closure in tail position takes the
                    // region's own continuation.
                    if let Some(clos) = st.env.get(name).copied() {
                        if needs_cps(&callee.ty) {
                            let Some(vals) = self.operands(st, args)? else {
                                return Ok(());
                            };
                            let k = st.kont;
                            return self.closure_cps_jump(
                                st,
                                callee,
                                clos.into_pointer_value(),
                                &vals,
                                k,
                            );
                        }
                    }
                    if !st.env.contains_key(name) && self.lc.cps_fns.contains(name) {
                        let target = *self.lc.decls.get(name).ok_or(CodegenError::Unsupported(
                            "callee is not a top-level function",
                        ))?;
                        let Some(vals) = self.operands(st, args)? else {
                            return Ok(());
                        };
                        return self.tail_cps_call(st, target, &vals);
                    }
                }
                self.tail_value(st, e)
            }
            CoreKind::Perform(p) => {
                let Some(vals) = self.operands(st, &p.args)? else {
                    return Ok(());
                };
                self.perform(st, e, &vals, true)
            }
            CoreKind::Let(x, v, body) => {
                let Some(val) = self.expr(st, v)? else {
                    return Ok(());
                };
                let saved = st.bind(x, val);
                let out = self.tail(st, body);
                st.unbind(x, saved);
                out
            }
            CoreKind::If(c, t, f) => {
                let Some(cv) = self.expr(st, c)? else {
                    return Ok(());
                };
                self.branch_tail(st, cv, t, f)
            }
            CoreKind::Match(s, arms) if contains_effect(e) => {
                let Some(sv) = self.expr(st, s)? else {
                    return Ok(());
                };
                self.match_dispatch(st, sv, s, arms, true).map(|_| ())
            }
            CoreKind::Resume(_) => {
                Err(CodegenError::Unsupported("resume outside a handler clause"))
            }
            _ => self.tail_value(st, e),
        }
    }

    fn tail_value(&self, st: &mut St<'ctx>, e: &CoreExpr) -> R<()> {
        match self.expr(st, e)? {
            Some(v) => self.cps_return(st, v, &e.ty),
            None => Ok(()),
        }
    }
}

// ---------------------------------------------------------------- emission --

/// An effectful top-level function: `(params.., ptr k) -> i64`.
pub(crate) fn emit_cps_fn<'ctx>(
    ctx: &'ctx Context,
    b: &Builder<'ctx>,
    lc: &LowerCtx<'_, 'ctx>,
    f: &elya::core::CoreFn,
) -> R<()> {
    let func = *lc
        .decls
        .get(&f.name)
        .ok_or(CodegenError::Unsupported("undeclared function"))?;
    let entry = ctx.append_basic_block(func, "entry");
    b.position_at_end(entry);
    let kont = func
        .get_nth_param(f.params.len() as u32)
        .ok_or_else(|| internal("effectful function has no continuation parameter"))?
        .into_pointer_value();
    let mut st = St::new(kont);
    for (i, p) in f.params.iter().enumerate() {
        let v = func
            .get_nth_param(i as u32)
            .ok_or_else(|| internal("declared arity disagrees with Core arity"))?;
        st.bind(&p.name, v);
    }
    Cx { ctx, func, b, lc }.tail(&mut st, &f.body)
}

/// Slice 5b-9b: an effectful lambda's lifted body, `(clos, params.., k) ->
/// i64`. The body is a CPS region that CONTINUES its enclosing scope's binding
/// indices (`cps::LambdaRegion`): each capture is loaded from the closure and
/// placed at the index of the innermost binding of its name there -- the index
/// every site inside the body saved it under -- and the parameters are bound
/// from the region's depth on, exactly as the analysis numbered them.
pub(crate) fn emit_cps_lifted<'ctx>(
    ctx: &'ctx Context,
    b: &Builder<'ctx>,
    lc: &LowerCtx<'_, 'ctx>,
    site: &crate::closure::LambdaSite,
) -> R<()> {
    let func = *lc
        .lifted
        .get(&site.symbol)
        .ok_or(CodegenError::Unsupported("lambda body was never declared"))?;
    let region = lc
        .lambda_regions
        .get(&site.key)
        .ok_or_else(|| internal("an effectful lambda has no region"))?;
    let entry = ctx.append_basic_block(func, "entry");
    b.position_at_end(entry);
    let cx = Cx { ctx, func, b, lc };
    let clos = func
        .get_nth_param(0)
        .ok_or_else(|| internal("lifted body has no environment parameter"))?
        .into_pointer_value();
    let kont = func
        .get_nth_param((site.params.len() + 1) as u32)
        .ok_or_else(|| internal("effectful lambda has no continuation parameter"))?
        .into_pointer_value();
    let mut st = St::new(kont);
    for (i, (name, ty)) in site.captures.iter().enumerate() {
        let w = cx.load_word(clos, i + 2)?;
        let v = word_to_value(b, w, ty, ctx.bool_type(), cx.ptrt())?;
        let at = region
            .binding_of(name)
            .ok_or_else(|| internal("a capture is not in the lambda's scope"))?;
        st.binds.insert(at, (name.clone(), v));
    }
    st.depth = region.scope.len();
    st.rebuild_env();
    for (i, p) in site.params.iter().enumerate() {
        let v = func
            .get_nth_param((i + 1) as u32)
            .ok_or_else(|| internal("declared lambda arity disagrees with Core"))?;
        st.bind(&p.name, v);
    }
    cx.tail(&mut st, &site.body)
}

/// A `handle`, from a region that needs no CPS (D17): allocate the handler
/// frame, then call the body's function as an ordinary nested call. Its answer
/// comes back through the return clause as a word.
pub(crate) fn emit_handle_site<'ctx>(
    ctx: &'ctx Context,
    func: FunctionValue<'ctx>,
    b: &Builder<'ctx>,
    lc: &LowerCtx<'_, 'ctx>,
    e: &CoreExpr,
    env: &mut HashMap<String, BasicValueEnum<'ctx>>,
) -> R<BasicValueEnum<'ctx>> {
    let key = e as *const CoreExpr as usize;
    let h = &lc.handlers[*lc.handler_index.get(&key).ok_or(CodegenError::Unsupported(
        "handle missing from the pre-pass",
    ))?];
    let tag = *lc
        .desc
        .handler_tags
        .get(&key)
        .ok_or(CodegenError::Unsupported("handle has no descriptor row"))?;
    let fns = lc
        .handler_fns
        .get(&key)
        .ok_or(CodegenError::Unsupported("handle has no functions"))?;
    let (body_fn, ret_fn) = (fns.body, fns.ret);
    let cx = Cx { ctx, func, b, lc };
    let i64t = ctx.i64_type();
    let roots = gc_root_env(b, lc, env)?;
    let p = cx.alloc(4 + h.saved.len())?;
    cx.store_word(p, 0, i64t.const_int(tag as u64, false))?;
    cx.store_word(p, 1, cx.fn_word(ret_fn)?)?;
    cx.store_word(p, 2, i64t.const_int(0, false))?;
    // Task 8: word 3 is the static clause table (not heap; its mask bit is
    // clear), so a perform can find this handle's clause for its op.
    let tw = b
        .build_ptr_to_int(fns.table, i64t, "tw")
        .map_err(internal)?;
    cx.store_word(p, 3, tw)?;
    for (j, (sv, ty)) in h.saved.iter().enumerate() {
        let Saved::Var { name, .. } = sv else {
            return Err(internal("a handler frame saves bindings only"));
        };
        if matches!(ty, Ty::Var(_)) {
            return Err(CodegenError::Unsupported(
                "saved value of unresolved type (N7)",
            ));
        }
        let v = *env
            .get(name)
            .ok_or(CodegenError::Unsupported("handler binding is not in scope"))?;
        let w = value_to_word(b, v, ty, i64t)?;
        cx.store_word(p, HANDLER_SAVED + j, w)?;
    }
    gc_unroot(b, lc, roots)?;
    // This handle's frame is the current handler for its body; the previous
    // one is restored when the body's answer comes back.
    let ptrt = ctx.ptr_type(AddressSpace::default());
    let outer = b
        .build_load(ptrt, lc.current_handler, "oh")
        .map_err(internal)?;
    b.build_store(lc.current_handler, p).map_err(internal)?;
    let roots = gc_root_env(b, lc, env)?;
    let call = b.build_call(body_fn, &[p.into()], "hb").map_err(internal)?;
    call.set_call_convention(TAILCC);
    gc_unroot(b, lc, roots)?;
    b.build_store(lc.current_handler, outer).map_err(internal)?;
    let word = call
        .try_as_basic_value()
        .left()
        .ok_or(CodegenError::Unsupported("call returned no value"))?
        .into_int_value();
    word_to_value(
        b,
        word,
        &e.ty,
        ctx.bool_type(),
        ctx.ptr_type(AddressSpace::default()),
    )
}

/// Where a site frame's saved values start: after `[tag][code][next]`.
const SITE_SAVED: usize = 3;
/// Where a handler frame's saved values start: after `[tag][code][next][table]`.
const HANDLER_SAVED: usize = 4;

/// Load a frame's saved values (words `first..`) into `st`.
fn load_saved<'ctx>(
    cx: &Cx<'_, '_, 'ctx>,
    st: &mut St<'ctx>,
    frame: PointerValue<'ctx>,
    saved: &[(Saved, Ty)],
    first: usize,
) -> R<()> {
    for (j, (sv, ty)) in saved.iter().enumerate() {
        let w = cx.load_word(frame, first + j)?;
        let v = word_to_value(cx.b, w, ty, cx.ctx.bool_type(), cx.ptrt())?;
        match sv {
            Saved::Temp(a) => {
                st.temps.insert(*a, v);
                if cx.push(v)? {
                    st.pending += 1;
                }
            }
            Saved::Var { name, binding } => {
                st.binds.insert(*binding, (name.clone(), v));
            }
        }
    }
    Ok(())
}

/// The resumption functions of every call site, and each handle's body and
/// return functions.
pub(crate) fn emit_sites_and_handlers<'ctx>(
    ctx: &'ctx Context,
    b: &Builder<'ctx>,
    lc: &LowerCtx<'_, 'ctx>,
) -> R<()> {
    for site in lc.sites {
        if let Some(&func) = lc.resume_fns.get(&site.key) {
            emit_resume_fn(ctx, b, lc, func, site)?;
        }
    }
    for h in lc.handlers {
        let fns = lc.handler_fns[&h.key].clone();
        let (body_fn, ret_fn) = (fns.body, fns.ret);
        let node = *lc.nodes.get(&h.key).ok_or(CodegenError::Unsupported(
            "handle missing from the node index",
        ))?;
        let CoreKind::Handle(hd) = &node.kind else {
            return Err(internal("a handler key names a non-handle node"));
        };
        // Body: `(ptr frame) -> i64`, a CPS region whose continuation is the
        // handler frame itself.
        let entry = ctx.append_basic_block(body_fn, "entry");
        b.position_at_end(entry);
        let hf = body_fn
            .get_nth_param(0)
            .ok_or_else(|| internal("handle body has no frame parameter"))?
            .into_pointer_value();
        let cx = Cx {
            ctx,
            func: body_fn,
            b,
            lc,
        };
        let mut st = St::new(hf);
        load_saved(&cx, &mut st, hf, &h.saved, HANDLER_SAVED)?;
        st.depth = h.depth;
        st.rebuild_env();
        cx.tail(&mut st, &hd.body)?;
        // Return clause: `(i64 value, ptr frame) -> i64`, direct code that
        // RETURNS the handle's answer to the handle site.
        let entry = ctx.append_basic_block(ret_fn, "entry");
        b.position_at_end(entry);
        let v = ret_fn
            .get_nth_param(0)
            .ok_or_else(|| internal("return clause has no value parameter"))?
            .into_int_value();
        let hf = ret_fn
            .get_nth_param(1)
            .ok_or_else(|| internal("return clause has no frame parameter"))?
            .into_pointer_value();
        let cx = Cx {
            ctx,
            func: ret_fn,
            b,
            lc,
        };
        let Some(r) = &hd.ret else {
            // No return clause: the identity, and the body's type is the
            // handle's, so the word passes straight through.
            b.build_return(Some(&v)).map_err(internal)?;
            continue;
        };
        let mut env = handler_env(&cx, hf, h)?;
        let bound = word_to_value(b, v, &hd.body.ty, ctx.bool_type(), cx.ptrt())?;
        // `bind_local`: the binder may shadow a saved name, which must stay rooted.
        let _ = bind_local(&mut env, &r.binder, bound);
        let out = lower_expr(ctx, ret_fn, b, lc, &r.body, &mut env)?;
        let word = value_to_word(b, out, &node.ty, ctx.i64_type())?;
        b.build_return(Some(&word)).map_err(internal)?;
    }
    for h in lc.handlers {
        emit_clauses(ctx, b, lc, h)?;
    }
    Ok(())
}

/// The handler frame's saved bindings as a name environment, in binding
/// order through `bind_local`, so a binding shadowed by a later one of the
/// same name stays rooted.
fn handler_env<'ctx>(
    cx: &Cx<'_, '_, 'ctx>,
    hf: PointerValue<'ctx>,
    h: &HandlerSite,
) -> R<HashMap<String, BasicValueEnum<'ctx>>> {
    let mut binds: Vec<(usize, String, BasicValueEnum<'ctx>)> = Vec::new();
    for (j, (sv, ty)) in h.saved.iter().enumerate() {
        let Saved::Var { name, binding } = sv else {
            return Err(internal("a handler frame saves bindings only"));
        };
        let w = cx.load_word(hf, HANDLER_SAVED + j)?;
        binds.push((
            *binding,
            name.clone(),
            word_to_value(cx.b, w, ty, cx.ctx.bool_type(), cx.ptrt())?,
        ));
    }
    binds.sort_by_key(|(i, _, _)| *i);
    let mut env = HashMap::new();
    for (_, n, v) in binds {
        let _ = bind_local(&mut env, &n, v);
    }
    Ok(env)
}

/// Task 8: one function per clause, `(op args.., ptr cont, ptr frame) -> i64`.
/// A clause body is direct code (D17: nothing outside the handle can be
/// captured, so it performs nothing unhandled); its answer is the handle's
/// answer, returned as a word. `cont` is bound as the clause's continuation
/// (`closure::CONT`), which `resume` -- and any lambda that captured it --
/// reads.
fn emit_clauses<'ctx>(
    ctx: &'ctx Context,
    b: &Builder<'ctx>,
    lc: &LowerCtx<'_, 'ctx>,
    h: &HandlerSite,
) -> R<()> {
    let fns = lc.handler_fns[&h.key].clone();
    let node = *lc.nodes.get(&h.key).ok_or(CodegenError::Unsupported(
        "handle missing from the node index",
    ))?;
    let CoreKind::Handle(hd) = &node.kind else {
        return Err(internal("a handler key names a non-handle node"));
    };
    for (c, &func) in hd.clauses.iter().zip(fns.clauses.iter()) {
        let entry = ctx.append_basic_block(func, "entry");
        b.position_at_end(entry);
        let cx = Cx { ctx, func, b, lc };
        let n = c.params.len() as u32;
        let cont = func
            .get_nth_param(n)
            .ok_or_else(|| internal("clause has no continuation parameter"))?;
        let hf = func
            .get_nth_param(n + 1)
            .ok_or_else(|| internal("clause has no frame parameter"))?
            .into_pointer_value();
        let mut env = handler_env(&cx, hf, h)?;
        for (i, p) in c.params.iter().enumerate() {
            let v = func
                .get_nth_param(i as u32)
                .ok_or_else(|| internal("declared clause arity disagrees with Core"))?;
            let _ = bind_local(&mut env, &p.name, v);
        }
        let _ = bind_local(&mut env, crate::closure::CONT, cont);
        clause_tail(&cx, &c.body, &node.ty, &mut env)?;
    }
    Ok(())
}

/// Tail position in a clause body: a `resume` here is a `musttail` jump into
/// the continuation (so a resume-in-a-loop handler keeps a flat stack, D14);
/// anything else returns its value as the handle's answer.
fn clause_tail<'ctx>(
    cx: &Cx<'_, '_, 'ctx>,
    e: &CoreExpr,
    answer: &Ty,
    env: &mut HashMap<String, BasicValueEnum<'ctx>>,
) -> R<()> {
    match &e.kind {
        CoreKind::Let(x, v, body) => {
            let val = lower_expr(cx.ctx, cx.func, cx.b, cx.lc, v, env)?;
            let sh = bind_local(env, x, val);
            let out = clause_tail(cx, body, answer, env);
            unbind_local(env, x, sh);
            out
        }
        CoreKind::If(c, t, f) => {
            let cv = lower_expr(cx.ctx, cx.func, cx.b, cx.lc, c, env)?.into_int_value();
            let then_bb = cx.ctx.append_basic_block(cx.func, "rthen");
            let else_bb = cx.ctx.append_basic_block(cx.func, "relse");
            cx.b.build_conditional_branch(cv, then_bb, else_bb)
                .map_err(internal)?;
            cx.b.position_at_end(then_bb);
            clause_tail(cx, t, answer, env)?;
            cx.b.position_at_end(else_bb);
            clause_tail(cx, f, answer, env)
        }
        CoreKind::Resume(arg) => {
            let (word, k, code, handler) = resume_prologue(cx, e, arg, env)?;
            // The resumed computation performs to ITS handler.
            cx.b.build_store(cx.lc.current_handler, handler)
                .map_err(internal)?;
            let code_ty = cx
                .i64t()
                .fn_type(&[cx.i64t().into(), cx.ptrt().into()], false);
            let site =
                cx.b.build_indirect_call(code_ty, code, &[word.into(), k.into()], "rj")
                    .map_err(internal)?;
            cx.tail_jump(site)
        }
        _ => {
            let v = lower_expr(cx.ctx, cx.func, cx.b, cx.lc, e, env)?;
            let w = value_to_word(cx.b, v, answer, cx.i64t())?;
            cx.b.build_return(Some(&w)).map_err(internal)?;
            Ok(())
        }
    }
}

/// The shared half of `resume(arg)`: lower the argument, enforce one-shot
/// (D13: a second resume calls the named trap, never re-runs), mark the
/// continuation consumed, and load its chain and code.
fn resume_prologue<'ctx>(
    cx: &Cx<'_, '_, 'ctx>,
    e: &CoreExpr,
    arg: &CoreExpr,
    env: &mut HashMap<String, BasicValueEnum<'ctx>>,
) -> R<(
    IntValue<'ctx>,
    PointerValue<'ctx>,
    PointerValue<'ctx>,
    PointerValue<'ctx>,
)> {
    let _ = e;
    let v = lower_expr(cx.ctx, cx.func, cx.b, cx.lc, arg, env)?;
    let cont = env
        .get(crate::closure::CONT)
        .copied()
        .ok_or(CodegenError::Unsupported("resume outside a handler clause"))?
        .into_pointer_value();
    let used = cx.load_word(cont, 2)?;
    let twice =
        cx.b.build_int_compare(
            inkwell::IntPredicate::NE,
            used,
            cx.i64t().const_int(0, false),
            "used",
        )
        .map_err(internal)?;
    let bad = cx.ctx.append_basic_block(cx.func, "resumed_twice");
    let ok = cx.ctx.append_basic_block(cx.func, "resume");
    cx.b.build_conditional_branch(twice, bad, ok)
        .map_err(internal)?;
    cx.b.position_at_end(bad);
    cx.b.build_call(cx.lc.resume_twice, &[], "rt")
        .map_err(internal)?;
    // `elya_resume_twice` exits; a placeholder terminator, NOT the trap.
    cx.b.build_unreachable().map_err(internal)?;
    cx.b.position_at_end(ok);
    cx.store_word(cont, 2, cx.i64t().const_int(1, false))?;
    let k = cx.load_word(cont, 1)?;
    let k =
        cx.b.build_int_to_ptr(k, cx.ptrt(), "rk")
            .map_err(internal)?;
    let code = cx.load_word(k, 1)?;
    let code =
        cx.b.build_int_to_ptr(code, cx.ptrt(), "rc")
            .map_err(internal)?;
    let hw = cx.load_word(cont, 3)?;
    let handler =
        cx.b.build_int_to_ptr(hw, cx.ptrt(), "rh")
            .map_err(internal)?;
    let word = value_to_word(cx.b, v, &arg.ty, cx.i64t())?;
    Ok((word, k, code, handler))
}

/// Task 8: `resume(arg)` in value position -- a native NESTING call of the
/// continuation (D14). It returns when the resumed computation reaches the
/// handler, with the handle's answer (return clause applied), which is the
/// value of `resume(..)` (deep handlers, spec 4).
pub(crate) fn emit_resume_call<'ctx>(
    ctx: &'ctx Context,
    func: FunctionValue<'ctx>,
    b: &Builder<'ctx>,
    lc: &LowerCtx<'_, 'ctx>,
    e: &CoreExpr,
    arg: &CoreExpr,
    env: &mut HashMap<String, BasicValueEnum<'ctx>>,
) -> R<BasicValueEnum<'ctx>> {
    let cx = Cx { ctx, func, b, lc };
    let (word, k, code, handler) = resume_prologue(&cx, e, arg, env)?;
    let i64t = ctx.i64_type();
    let code_ty = i64t.fn_type(&[i64t.into(), cx.ptrt().into()], false);
    // The resumed computation performs to ITS handler; the caller's comes
    // back when it returns.
    let outer = b
        .build_load(cx.ptrt(), lc.current_handler, "oh")
        .map_err(internal)?;
    b.build_store(lc.current_handler, handler)
        .map_err(internal)?;
    // Everything live in this function survives the resumed computation.
    let roots = gc_root_env(b, lc, env)?;
    let call = b
        .build_indirect_call(code_ty, code, &[word.into(), k.into()], "rs")
        .map_err(internal)?;
    call.set_call_convention(TAILCC);
    gc_unroot(b, lc, roots)?;
    b.build_store(lc.current_handler, outer).map_err(internal)?;
    let w = call
        .try_as_basic_value()
        .left()
        .ok_or(CodegenError::Unsupported("call returned no value"))?
        .into_int_value();
    word_to_value(b, w, &e.ty, ctx.bool_type(), cx.ptrt())
}

/// A site's resumption function: `(i64 value, ptr frame) -> i64`. Reload what
/// the frame saved, fill the hole with `value`, and run the rest of the region
/// from the hole upward along the path the analysis recorded.
fn emit_resume_fn<'ctx>(
    ctx: &'ctx Context,
    b: &Builder<'ctx>,
    lc: &LowerCtx<'_, 'ctx>,
    func: FunctionValue<'ctx>,
    site: &ContSite,
) -> R<()> {
    let node = |k: usize| -> R<&CoreExpr> {
        lc.nodes.get(&k).copied().ok_or(CodegenError::Unsupported(
            "node missing from the node index",
        ))
    };
    let entry = ctx.append_basic_block(func, "entry");
    b.position_at_end(entry);
    let cx = Cx { ctx, func, b, lc };
    let v = func
        .get_nth_param(0)
        .ok_or_else(|| internal("resumption has no value parameter"))?
        .into_int_value();
    let frame = func
        .get_nth_param(1)
        .ok_or_else(|| internal("resumption has no frame parameter"))?
        .into_pointer_value();
    let next = cx.load_word(frame, 2)?;
    let kont = b.build_int_to_ptr(next, cx.ptrt(), "k").map_err(internal)?;
    let mut st = St::new(kont);
    load_saved(&cx, &mut st, frame, &site.saved, SITE_SAVED)?;
    let hole = node(site.key)?;
    let mut cur = word_to_value(b, v, &hole.ty, ctx.bool_type(), cx.ptrt())?;
    let root = node(
        site.path
            .first()
            .ok_or_else(|| internal("a site in tail position has no resumption"))?
            .key,
    )?;
    for i in (0..site.path.len()).rev() {
        let step = &site.path[i];
        let a = node(step.key)?;
        st.depth = step.depth;
        st.rebuild_env();
        let mut tail_i = true;
        for p in &site.path[..i] {
            if !is_tail_slot(node(p.key)?, p.slot) {
                tail_i = false;
            }
        }
        let before = st.pending;
        // The hole's value is an operand of this ancestor. A later site's
        // frame may save it (as `Saved::Temp` of the child node), and later
        // operands may collect while it waits, so record it and root it
        // (found by the 7b-3 review: D-1, D-2).
        let child = match site.path.get(i + 1) {
            Some(p) => p.key,
            None => site.key,
        };
        st.temps.insert(child, cur);
        if cx.push(cur)? {
            st.pending += 1;
        }
        match &a.kind {
            CoreKind::App(callee, args) => {
                let CoreKind::Var(name) = &callee.kind else {
                    return Err(CodegenError::Unsupported("computed callee"));
                };
                let local = st.env.get(name).copied();
                let Some(vals) = resume_operands(&cx, &mut st, args, step.slot - 1, cur)? else {
                    return Ok(());
                };
                if let Some(clos) = local {
                    if needs_cps(&callee.ty) {
                        // 5b-9b: an effectful closure call in a resumption.
                        return if tail_i {
                            let k = st.kont;
                            cx.closure_cps_jump(
                                &mut st,
                                callee,
                                clos.into_pointer_value(),
                                &vals,
                                k,
                            )
                        } else {
                            cx.closure_site_call(&mut st, a, callee, clos, &vals)
                        };
                    }
                    cur = cx.closure_call(&st, callee, clos, &vals)?;
                } else {
                    let target = *lc.decls.get(name).ok_or(CodegenError::Unsupported(
                        "callee is not a top-level function",
                    ))?;
                    if lc.cps_fns.contains(name) {
                        return if tail_i {
                            cx.tail_cps_call(&mut st, target, &vals)
                        } else {
                            cx.site_call(&mut st, a, target, &vals)
                        };
                    }
                    cur = cx.direct_call(&st, target, &vals)?;
                }
            }
            CoreKind::Prim(op, args) => {
                let Some(vals) = resume_operands(&cx, &mut st, args, step.slot, cur)? else {
                    return Ok(());
                };
                cur = cx.prim(*op, args, &vals)?;
            }
            CoreKind::Builtin(name, args) => {
                if name != "io.println" {
                    return Err(CodegenError::UnsupportedBuiltin(name.clone()));
                }
                let Some(vals) = resume_operands(&cx, &mut st, args, step.slot, cur)? else {
                    return Ok(());
                };
                cur = cx.println(&vals)?;
            }
            CoreKind::Ctor(name, fields) => {
                let Some(vals) = resume_operands(&cx, &mut st, fields, step.slot, cur)? else {
                    return Ok(());
                };
                cur = cx.ctor(&st, name, &vals)?;
            }
            CoreKind::Perform(p) => {
                let Some(vals) = resume_operands(&cx, &mut st, &p.args, step.slot, cur)? else {
                    return Ok(());
                };
                return cx.perform(&mut st, a, &vals, tail_i);
            }
            CoreKind::Let(x, _, body) => {
                if step.slot == 0 {
                    st.binds.insert(step.depth, (x.clone(), cur));
                    st.depth = step.depth + 1;
                    st.rebuild_env();
                    if tail_i {
                        return cx.tail(&mut st, body);
                    }
                    match cx.expr(&mut st, body)? {
                        Some(v) => cur = v,
                        None => return Ok(()),
                    }
                }
            }
            CoreKind::If(_, t, f) => {
                if step.slot == 0 {
                    if tail_i {
                        return cx.branch_tail(&mut st, cur, t, f);
                    }
                    match cx.branch_value(&mut st, cur, t, f)? {
                        Some(v) => cur = v,
                        None => return Ok(()),
                    }
                }
            }
            // 5b-9a. Slot 0 is the scrutinee: dispatch on the value the site
            // returned. An arm slot needs nothing -- the arm's value is the
            // match's value -- exactly as an `If` branch.
            CoreKind::Match(s, arms) => {
                if step.slot == 0 {
                    if tail_i {
                        cx.match_dispatch(&mut st, cur, s, arms, true)?;
                        return Ok(());
                    }
                    match cx.match_dispatch(&mut st, cur, s, arms, false)? {
                        Some(v) => cur = v,
                        None => return Ok(()),
                    }
                }
            }
            CoreKind::Resume(_) => {
                return Err(CodegenError::Unsupported("resume outside a handler clause"))
            }
            _ => {
                return Err(internal(
                    "a site's path runs through a leaf or a region boundary",
                ))
            }
        }
        cx.settle(&mut st, before)?;
    }
    cx.cps_return(&mut st, cur, &root.ty)
}

/// Rebuild an ancestor's operand list around the hole at `hole`: operands
/// before it come from the frame (temporaries) or the environment (bindings,
/// re-read literals), the hole is `cur`, and operands after it are evaluated
/// now -- and may themselves be continuation sites.
fn resume_operands<'ctx>(
    cx: &Cx<'_, '_, 'ctx>,
    st: &mut St<'ctx>,
    ops: &[CoreExpr],
    hole: usize,
    cur: BasicValueEnum<'ctx>,
) -> R<Option<Vec<BasicValueEnum<'ctx>>>> {
    let mut out = Vec::with_capacity(ops.len());
    for (j, op) in ops.iter().enumerate() {
        if j < hole {
            let v = match &op.kind {
                CoreKind::Lit(_) => {
                    // Re-lowered, and a string literal allocates: root it while
                    // later operands run (7b-3 review, D-3).
                    let v = lower_expr(cx.ctx, cx.func, cx.b, cx.lc, op, &mut st.env)?;
                    if cx.push(v)? {
                        st.pending += 1;
                    }
                    v
                }
                CoreKind::Var(x) => *st
                    .env
                    .get(x)
                    .ok_or(CodegenError::Unsupported("saved binding is not available"))?,
                _ => *st.temps.get(&(op as *const CoreExpr as usize)).ok_or(
                    CodegenError::Unsupported("saved temporary is not available"),
                )?,
            };
            out.push(v);
        } else if j == hole {
            out.push(cur);
        } else {
            match cx.operand(st, op)? {
                Some(v) => out.push(v),
                None => return Ok(None),
            }
        }
    }
    Ok(Some(out))
}
