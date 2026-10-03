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
use elya::core::{CoreExpr, CoreKind, CoreModule};
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
    }
    Ok(())
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
        CoreKind::Perform(p) => p.args.iter().try_for_each(|a| go(a, scope)),
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
            if needs_cps(&e.ty) {
                return Err(CodegenError::Unsupported("effectful lambda (Task 8)"));
            }
            let depth = scope.len();
            scope.extend(params.iter().map(|p| p.name.clone()));
            let r = check(body, scope, in_handle, false, fns, cps_fns);
            scope.truncate(depth);
            r
        }
        CoreKind::Handle(h) => {
            if in_handle {
                return Err(CodegenError::Unsupported(
                    "handle nested inside another handle",
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

/// One resumption function per continuation site whose node is a call (a
/// perform site has none until Task 8), and a body and return function per
/// handle.
#[allow(clippy::type_complexity)]
pub(crate) fn declare<'ctx>(
    ctx: &'ctx Context,
    module: &Module<'ctx>,
    nodes: &HashMap<usize, &CoreExpr>,
    sites: &[ContSite],
    handlers: &[HandlerSite],
) -> (
    HashMap<usize, FunctionValue<'ctx>>,
    HashMap<usize, (FunctionValue<'ctx>, FunctionValue<'ctx>)>,
) {
    let i64t = ctx.i64_type();
    let ptrt = ctx.ptr_type(AddressSpace::default());
    let code_ty = i64t.fn_type(&[i64t.into(), ptrt.into()], false);
    let body_ty = i64t.fn_type(&[ptrt.into()], false);
    let mut resume = HashMap::new();
    for (i, s) in sites.iter().enumerate() {
        if !matches!(nodes.get(&s.key).map(|n| &n.kind), Some(CoreKind::App(..))) {
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
        hfns.insert(h.key, (body, ret));
    }
    (resume, hfns)
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

    /// D18: a perform stops the program by name until Task 8.
    fn trap(&self, st: &mut St<'ctx>) -> R<()> {
        self.pop_pending(st)?;
        self.b
            .build_call(self.lc.perform_trap, &[], "trap")
            .map_err(internal)?;
        // `elya_perform_unimplemented` exits; this terminator is never reached
        // -- a placeholder, NOT the trap (the `elya_match_fail` precedent).
        self.b.build_unreachable().map_err(internal)?;
        Ok(())
    }

    // ------------------------------------------------------------- calls --

    /// A continuation site (D10): save what the rest of the region needs in a
    /// frame whose code is the site's resumption function, then `musttail`
    /// the callee with that frame as its continuation.
    fn site_call(
        &self,
        st: &mut St<'ctx>,
        node: &CoreExpr,
        target: FunctionValue<'ctx>,
        args: &[BasicValueEnum<'ctx>],
    ) -> R<()> {
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
        self.pop_pending(st)?;
        let mut vals: Vec<BasicMetadataValueEnum<'ctx>> =
            args.iter().map(|v| (*v).into()).collect();
        vals.push(p.into());
        let call = self.b.build_call(target, &vals, "sc").map_err(internal)?;
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
            return Err(CodegenError::Unsupported("effectful closure call (Task 8)"));
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
                if self.operands(st, &p.args)?.is_none() {
                    return Ok(None);
                }
                self.trap(st)?;
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
            CoreKind::Match(..) => Err(CodegenError::Unsupported(
                "effectful call inside a match (Task 8)",
            )),
            CoreKind::Resume(_) => Err(CodegenError::Unsupported("resume (Task 8)")),
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
                if self.operands(st, &p.args)?.is_none() {
                    return Ok(());
                }
                self.trap(st)
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
            CoreKind::Match(..) if contains_effect(e) => Err(CodegenError::Unsupported(
                "effectful call inside a match (Task 8)",
            )),
            CoreKind::Resume(_) => Err(CodegenError::Unsupported("resume (Task 8)")),
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
    let (body_fn, ret_fn) = *lc
        .handler_fns
        .get(&key)
        .ok_or(CodegenError::Unsupported("handle has no functions"))?;
    let cx = Cx { ctx, func, b, lc };
    let i64t = ctx.i64_type();
    let roots = gc_root_env(b, lc, env)?;
    let p = cx.alloc(3 + h.saved.len())?;
    cx.store_word(p, 0, i64t.const_int(tag as u64, false))?;
    cx.store_word(p, 1, cx.fn_word(ret_fn)?)?;
    cx.store_word(p, 2, i64t.const_int(0, false))?;
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
        cx.store_word(p, 3 + j, w)?;
    }
    gc_unroot(b, lc, roots)?;
    let roots = gc_root_env(b, lc, env)?;
    let call = b.build_call(body_fn, &[p.into()], "hb").map_err(internal)?;
    call.set_call_convention(TAILCC);
    gc_unroot(b, lc, roots)?;
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

/// Load a frame's saved values (words `3..`) into `st`.
fn load_saved<'ctx>(
    cx: &Cx<'_, '_, 'ctx>,
    st: &mut St<'ctx>,
    frame: PointerValue<'ctx>,
    saved: &[(Saved, Ty)],
) -> R<()> {
    for (j, (sv, ty)) in saved.iter().enumerate() {
        let w = cx.load_word(frame, 3 + j)?;
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
        let (body_fn, ret_fn) = lc.handler_fns[&h.key];
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
        load_saved(&cx, &mut st, hf, &h.saved)?;
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
        let mut env: HashMap<String, BasicValueEnum<'ctx>> = HashMap::new();
        let mut binds: Vec<(usize, String, BasicValueEnum<'ctx>)> = Vec::new();
        for (j, (sv, ty)) in h.saved.iter().enumerate() {
            let Saved::Var { name, binding } = sv else {
                return Err(internal("a handler frame saves bindings only"));
            };
            let w = cx.load_word(hf, 3 + j)?;
            binds.push((
                *binding,
                name.clone(),
                word_to_value(b, w, ty, ctx.bool_type(), cx.ptrt())?,
            ));
        }
        binds.sort_by_key(|(i, _, _)| *i);
        for (_, n, val) in binds {
            env.insert(n, val);
        }
        let bound = word_to_value(b, v, &hd.body.ty, ctx.bool_type(), cx.ptrt())?;
        env.insert(r.binder.clone(), bound);
        let out = lower_expr(ctx, ret_fn, b, lc, &r.body, &mut env)?;
        let word = value_to_word(b, out, &node.ty, ctx.i64_type())?;
        b.build_return(Some(&word)).map_err(internal)?;
    }
    Ok(())
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
    load_saved(&cx, &mut st, frame, &site.saved)?;
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
                if resume_operands(&cx, &mut st, &p.args, step.slot, cur)?.is_none() {
                    return Ok(());
                }
                return cx.trap(&mut st);
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
            CoreKind::Match(..) => {
                return Err(CodegenError::Unsupported(
                    "effectful call inside a match (Task 8)",
                ))
            }
            CoreKind::Resume(_) => return Err(CodegenError::Unsupported("resume (Task 8)")),
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
