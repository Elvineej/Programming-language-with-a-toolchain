//! Native codegen: Core → LLVM via inkwell. Slice 5b-1 covered the arithmetic
//! subset (Int literals, Var, Prim(Add/Sub/Mul), Let); Slice 5b-2 adds control
//! flow — `If` as a three-block diamond joined by `phi`, `Bool` as i1, the six
//! comparisons as signed `icmp`, and `&&`/`||` as bit-wise `and`/`or` on i1.
//! Slice 5b-3 adds functions: every top-level fn is emitted, mangled `elya_*`
//! and carrying `tailcc`, in two passes so mutual recursion resolves without
//! ordering the module. Two value widths (i64, i1), no closures, no effects.
//!
//! Proof is EXECUTION (tests/native_codegen.rs), never IR inspection (spec §0):
//! `emit_ir` is a debugging aid and nothing in the suite asserts on its output.
//! Semantic-fidelity rule (§3.4, §4.1): native codegen must be neither more- nor
//! less-undefined than the tree evaluator. Hence no Div/Rem (UB on a zero
//! divisor, and it drags in the runtime-error path), Add/Sub/Mul emitted WITHOUT
//! nsw/nuw so overflow is defined two's-complement wrapping, and `&&`/`||`
//! STRICT rather than short-circuiting — because both of Elya's evaluators are
//! strict, so a short-circuit diamond would make native binaries *less*
//! undefined than `elya run`, observable the moment Div lands. Short-circuiting
//! is a front-end question, not a back-end one.

use std::collections::HashMap;
use std::path::Path;

use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use inkwell::types::BasicMetadataTypeEnum;
use inkwell::types::BasicTypeEnum;
use inkwell::values::BasicMetadataValueEnum;
use inkwell::values::BasicValueEnum;
use inkwell::values::CallSiteValue;
use inkwell::values::FunctionValue;
use inkwell::values::LLVMTailCallKind;
use inkwell::AddressSpace;
use inkwell::IntPredicate;
use inkwell::OptimizationLevel;

use elya::ast::BinOp;
use elya::core::{CoreExpr, CoreFn, CoreKind, CoreLit, CoreModule, CorePat};
use elya::types::{Ty, TyCon};

#[derive(Debug)]
pub enum CodegenError {
    /// Out-of-subset construct. A typed boundary, not a panic (mirrors
    /// `LowerError::Unsupported`); the payload names the construct.
    Unsupported(&'static str),
    /// `module.verify()` failed — a bug in our own emission, surfaced loudly.
    Verify(String),
    Io(std::io::Error),
    /// `clang` exited non-zero; its stderr is surfaced (§3.6).
    Link {
        code: Option<i32>,
        stderr: String,
    },
}

impl std::fmt::Display for CodegenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CodegenError::Unsupported(w) => write!(f, "codegen: unsupported construct ({w})"),
            CodegenError::Verify(e) => write!(f, "codegen: verification failed: {e}"),
            CodegenError::Io(e) => write!(f, "codegen: {e}"),
            CodegenError::Link { code, stderr } => {
                write!(f, "codegen: clang exited {code:?}: {stderr}")
            }
        }
    }
}

/// Internal/environmental failures (builder on well-formed IR, target-machine
/// setup) ride the Io variant — loud, but distinct from user-facing boundaries.
fn internal(e: impl std::fmt::Display) -> CodegenError {
    CodegenError::Io(std::io::Error::other(e.to_string()))
}

/// Static label per BinOp, for specific Unsupported messages (§8: the *specific*
/// message, never a panic). Kept total over the AST operator set — the match is
/// exhaustive by construction, so a new `BinOp` variant fails the build here
/// rather than degrading into a vague message at runtime.
fn op_label(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "Add",
        BinOp::Sub => "Sub",
        BinOp::Mul => "Mul",
        BinOp::Div => "Div",
        BinOp::Rem => "Rem",
        BinOp::AddF => "AddF",
        BinOp::SubF => "SubF",
        BinOp::MulF => "MulF",
        BinOp::DivF => "DivF",
        BinOp::Eq => "Eq",
        BinOp::Ne => "Ne",
        BinOp::Lt => "Lt",
        BinOp::Le => "Le",
        BinOp::Gt => "Gt",
        BinOp::Ge => "Ge",
        BinOp::Concat => "Concat",
        BinOp::And => "And",
        BinOp::Or => "Or",
    }
}

/// `CodegenError::Unsupported` carries a `&'static str`, so the Eq/Ne operand
/// refusal is a fixed pair of strings rather than a formatted type name. Losing
/// the type name is the price of refusing by name at all — and the type is one
/// line up in any backtrace the user would be reading.
fn eq_operand_label(op: BinOp) -> &'static str {
    match op {
        BinOp::Ne => "Ne on an unrepresentable operand type",
        _ => "Eq on an unrepresentable operand type",
    }
}

/// Every Elya function gets this prefix (§4.1). Prefixing is injective, so no
/// two Elya names collide, and no Elya name can collide with the three symbols
/// this back end generates or imports: `@main` (the print shim), `@printf`
/// (external), `@.fmt` (the format string). `main` mangles to `elya_main` —
/// byte-identical to the name 5b-1 already hardcoded, so the shim is unchanged.
/// An Elya function literally named `elya_main` mangles to `elya_elya_main`.
fn mangle(name: &str) -> String {
    format!("elya_{name}")
}

/// LLVM's `tailcc`. Value from llvm/IR/CallingConv.h: `Tail = 18`. EVERY
/// declared Elya function and EVERY Elya call site uses it. That uniformity is
/// what makes `musttail`'s convention-match requirement (Task 4) true by
/// construction rather than by case analysis.
const TAILCC: u32 = 18;

/// The flat arity cap (§5.1, from §2's probe matrix). Beyond it, a win64
/// guaranteed tail call hits `LLVM ERROR: Can't handle guaranteed tail call
/// under win64 yet` — a `report_fatal_error` with no source span that kills the
/// process. Refusing at 6 is what keeps that unreachable.
const MAX_PARAMS: usize = 5;

/// §3.1 module shape, as N2 leaves it: SOME function is named `main` and takes
/// no parameters. The "exactly one function" half is gone — that is the whole
/// point of this slice.
fn find_main(core: &CoreModule) -> Result<&CoreFn, CodegenError> {
    let f = core
        .fns
        .iter()
        .find(|f| f.name == "main")
        .ok_or(CodegenError::Unsupported("no `main`"))?;
    if !f.params.is_empty() {
        return Err(CodegenError::Unsupported("main takes parameters"));
    }
    Ok(f)
}

/// §3.1 type mapping: `Int` -> i64, `Bool` -> i1, and — since N4 — `Ty::Con`
/// -> a pointer. Reads the INLINE `ty` field on each Core node (Shape C — the
/// reason this fold needs no side-table lookups). Everything else is refused by
/// name, `Ty::Var(_)` included; when that fires, that is N7 knocking.
fn repr_ty<'ctx>(ctx: &'ctx Context, ty: &Ty) -> Result<BasicTypeEnum<'ctx>, CodegenError> {
    match ty {
        Ty::Base(TyCon::Int) => Ok(ctx.i64_type().into()),
        Ty::Base(TyCon::Bool) => Ok(ctx.bool_type().into()),
        // N4 (spec §1): an ADT value is a pointer to its heap object.
        Ty::Con(..) => Ok(ctx.ptr_type(AddressSpace::default()).into()),
        _ => Err(CodegenError::Unsupported("unrepresentable type")),
    }
}

/// `main` returns i64: `@elya_main`'s signature says so and the print shim's
/// format string is `%lld`. A `Bool`-bodied main is a representable value in an
/// unrepresentable *place*, so it is refused at the module boundary rather than
/// inside the fold — which is why this message stayed "non-Int value" when the
/// fold widened.
fn require_int(ty: &Ty) -> Result<(), CodegenError> {
    if matches!(ty, Ty::Base(TyCon::Int)) {
        Ok(())
    } else {
        Err(CodegenError::Unsupported("non-Int value"))
    }
}

/// Pass 1 of §4.2: declare every function before any body is emitted, so a body
/// can call a function whose body does not exist yet. That is what makes mutual
/// recursion resolve without ordering the module.
///
/// The map is keyed by the UNMANGLED Elya name — that is what a `CoreKind::Var`
/// callee carries. Mangling happens only at `add_function`.
///
/// This is also where §5.2 is enforced: `repr_ty` runs over every parameter
/// type and every body type in the module, so a polymorphic or otherwise
/// unrepresentable function refuses the whole module here — even one `main`
/// never calls. That whole-module strictness is deliberate and tracked as
/// obligation T2.
fn declare_all<'ctx>(
    ctx: &'ctx Context,
    module: &Module<'ctx>,
    core: &CoreModule,
) -> Result<HashMap<String, FunctionValue<'ctx>>, CodegenError> {
    let mut decls: HashMap<String, FunctionValue<'ctx>> = HashMap::new();
    for f in &core.fns {
        // Checked BEFORE `add_function`: LLVM silently uniquifies a duplicate
        // symbol (`elya_f.1`) rather than complaining, which would give us two
        // functions where Core has one. Belt and braces — the front end already
        // rejects duplicate definitions.
        if decls.contains_key(&f.name) {
            return Err(CodegenError::Unsupported("duplicate top-level function"));
        }
        let mut params: Vec<BasicMetadataTypeEnum<'ctx>> = Vec::with_capacity(f.params.len());
        for p in f.params.iter() {
            params.push(repr_ty(ctx, &p.ty)?.into());
        }
        let fn_ty = match repr_ty(ctx, &f.body.ty)? {
            BasicTypeEnum::IntType(t) => t.fn_type(&params, false),
            BasicTypeEnum::PointerType(t) => t.fn_type(&params, false),
            _ => return Err(CodegenError::Unsupported("unrepresentable type")),
        };
        let func = module.add_function(&mangle(&f.name), fn_ty, None);
        // NOTE the plural: `set_call_conventions` is the FunctionValue method.
        // The call-site method is `set_call_convention`, singular. Both are
        // needed and they must agree, or the module is wrong.
        func.set_call_conventions(TAILCC);
        decls.insert(f.name.clone(), func);
    }
    Ok(decls)
}

/// Pass 2 of §4.2. The value environment starts empty and is seeded from the
/// LLVM parameters, so each function gets its own — nothing leaks across a
/// call, which is what `distinct_envs` pins.
fn emit_body<'ctx>(
    ctx: &'ctx Context,
    b: &Builder<'ctx>,
    lc: &LowerCtx<'ctx>,
    f: &CoreFn,
) -> Result<(), CodegenError> {
    let func = *lc
        .decls
        .get(&f.name)
        .ok_or(CodegenError::Unsupported("undeclared function"))?;
    let entry = ctx.append_basic_block(func, "entry");
    b.position_at_end(entry);
    let mut env: HashMap<String, BasicValueEnum<'ctx>> = HashMap::new();
    for (i, p) in f.params.iter().enumerate() {
        let v = func
            .get_nth_param(i as u32)
            .ok_or_else(|| internal("declared arity disagrees with Core arity"))?;
        env.insert(p.name.clone(), v);
    }
    // Every function body is in tail position by definition; `lower_tail` emits
    // the terminator, so there is no `build_return` here any more.
    lower_tail(ctx, func, b, lc, &f.body, &mut env)
}

/// One place where an Elya call becomes an LLVM call, used from both `lower_expr`
/// (ordinary position) and, in Task 4, `lower_tail` (tail position). The only
/// difference between the two is the tail-call kind the caller sets afterwards.
///
/// The callee kind is inspected FIRST, so §5.3's "computed callee" refusal fires
/// before any argument is lowered and before the existing `Lambda` arm is ever
/// reached.
fn build_elya_call<'ctx>(
    ctx: &'ctx Context,
    func: FunctionValue<'ctx>,
    b: &Builder<'ctx>,
    lc: &LowerCtx<'ctx>,
    callee: &CoreExpr,
    args: &[CoreExpr],
    env: &mut HashMap<String, BasicValueEnum<'ctx>>,
) -> Result<CallSiteValue<'ctx>, CodegenError> {
    let CoreKind::Var(name) = &callee.kind else {
        return Err(CodegenError::Unsupported("computed callee"));
    };
    let target = *lc.decls.get(name).ok_or(CodegenError::Unsupported(
        "callee is not a top-level function",
    ))?;
    let mut vals: Vec<BasicMetadataValueEnum<'ctx>> = Vec::with_capacity(args.len());
    for a in args.iter() {
        // Left to right, matching the evaluator's argument order.
        vals.push(lower_expr(ctx, func, b, lc, a, env)?.into());
    }
    let site = b.build_call(target, &vals, "c").map_err(internal)?;
    // Singular here (CallSiteValue), plural on the declaration (FunctionValue).
    site.set_call_convention(TAILCC);
    Ok(site)
}

/// §4.3: the tail half of the emission split. `lower_expr` produces a VALUE;
/// `lower_tail` emits a TERMINATOR. On `Ok` the current block is terminated —
/// every arm here ends in a `ret` or hands off to a recursive call that does.
///
/// The reason for the split is narrow and load-bearing: `musttail` requires the
/// call to be immediately followed by a `ret` of its result. Threading tail
/// position through emission is what makes that adjacency structural rather
/// than something to hope for.
fn lower_tail<'ctx>(
    ctx: &'ctx Context,
    func: FunctionValue<'ctx>,
    b: &Builder<'ctx>,
    lc: &LowerCtx<'ctx>,
    e: &CoreExpr,
    env: &mut HashMap<String, BasicValueEnum<'ctx>>,
) -> Result<(), CodegenError> {
    match &e.kind {
        CoreKind::If(cond, then_e, else_e) => {
            // NO join block and NO `phi`. Each arm terminates itself, so a tail
            // call inside an arm is immediately followed by its own `ret` —
            // which a join block would break by inserting a branch between them.
            // The `phi` diamond in `lower_expr` stays exercised by the corpus
            // programs whose `if`s sit in `let`-value position.
            let c = lower_expr(ctx, func, b, lc, cond, env)?.into_int_value();
            if c.get_type().get_bit_width() != 1 {
                return Err(CodegenError::Unsupported("non-Bool if condition"));
            }
            let then_bb = ctx.append_basic_block(func, "then");
            let else_bb = ctx.append_basic_block(func, "else");
            b.build_conditional_branch(c, then_bb, else_bb)
                .map_err(internal)?;
            b.position_at_end(then_bb);
            lower_tail(ctx, func, b, lc, then_e, env)?;
            // Position explicitly rather than assuming where the recursive call
            // left the builder: a nested tail `if` leaves it in ITS else block.
            b.position_at_end(else_bb);
            lower_tail(ctx, func, b, lc, else_e, env)?;
            Ok(())
        }
        CoreKind::Let(x, rhs, body) => {
            // The bound value is NOT in tail position; only the body is.
            let v = lower_expr(ctx, func, b, lc, rhs, env)?;
            let prev = env.insert(x.clone(), v);
            let out = lower_tail(ctx, func, b, lc, body, env);
            // Restore any shadowed binding, exactly as `lower_expr` does.
            match prev {
                Some(p) => {
                    env.insert(x.clone(), p);
                }
                None => {
                    env.remove(x);
                }
            }
            out
        }
        CoreKind::App(callee, args) => {
            let site = build_elya_call(ctx, func, b, lc, callee, args, env)?;
            // The guarantee, in one line. `musttail` is VERIFIER-ENFORCED: if
            // the convention, the return type, or the adjacency of the `ret`
            // were wrong, `module.verify()` rejects the module rather than
            // emitting a call that grows the stack. That is the whole reason
            // §2 chose `musttail` over the unchecked `tail` hint, which built a
            // binary that overflowed at runtime.
            site.set_tail_call_kind(LLVMTailCallKind::LLVMTailCallKindMustTail);
            let v = site
                .try_as_basic_value()
                .left()
                .ok_or(CodegenError::Unsupported("call returned no value"))?;
            b.build_return(Some(&v)).map_err(internal)?;
            Ok(())
        }
        _ => {
            let v = lower_expr(ctx, func, b, lc, e, env)?;
            b.build_return(Some(&v)).map_err(internal)?;
            Ok(())
        }
    }
}

/// §3.3 expression lowering: a recursive fold returning a `BasicValueEnum`,
/// threading a binding environment. NO alloca, NO mem2reg — bindings are
/// immutable and values map directly to SSA registers; the save/restore around
/// `Let` is what makes shadowing correct. Each node's width comes from its own
/// inline type, so an i1, an i64, and (with N4) a pointer coexist in one enum.
fn lower_expr<'ctx>(
    ctx: &'ctx Context,
    func: FunctionValue<'ctx>,
    b: &Builder<'ctx>,
    lc: &LowerCtx<'ctx>,
    e: &CoreExpr,
    env: &mut HashMap<String, BasicValueEnum<'ctx>>,
) -> Result<BasicValueEnum<'ctx>, CodegenError> {
    // Check for "function used as a value" BEFORE repr_ty, because a function
    // name in value position has a function type that repr_ty will reject as
    // "unrepresentable type" — but the correct refusal is the more specific
    // "function used as a value" (§5.4's companion).
    if let CoreKind::Var(x) = &e.kind {
        if !env.contains_key(x) && lc.decls.contains_key(x) {
            return Err(CodegenError::Unsupported("function used as a value"));
        }
    }

    let node_ty = repr_ty(ctx, &e.ty)?;
    match &e.kind {
        CoreKind::Lit(CoreLit::Int(n)) => {
            Ok(node_ty.into_int_type().const_int(*n as u64, true).into())
        }
        CoreKind::Lit(CoreLit::Bool(v)) => Ok(node_ty
            .into_int_type()
            .const_int(u64::from(*v), false)
            .into()),
        CoreKind::Lit(_) => Err(CodegenError::Unsupported("non-Int literal")),
        CoreKind::Var(x) => match env.get(x) {
            Some(v) => Ok(*v),
            None => Err(CodegenError::Unsupported("unbound var")),
        },
        CoreKind::Let(x, rhs, body) => {
            let v = lower_expr(ctx, func, b, lc, rhs, env)?;
            let prev = env.insert(x.clone(), v);
            let out = lower_expr(ctx, func, b, lc, body, env);
            // Restore any shadowed binding — `let x = 1; let x = x + 1` stays correct.
            match prev {
                Some(p) => {
                    env.insert(x.clone(), p);
                }
                None => {
                    env.remove(x);
                }
            }
            out
        }
        CoreKind::Prim(op, args) => {
            if args.len() != 2 {
                return Err(CodegenError::Unsupported("binary Prim arity"));
            }
            // Eq/Ne are fully polymorphic (src/types.rs:920-922 unifies the two
            // operands and pins neither), so the refusal is dispatched on the
            // OPERAND type — and BEFORE the operands are lowered, so the message
            // names the operator rather than reporting the operand's type as
            // unrepresentable. Every other operator in the subset is monomorphic
            // by the time it reaches here.
            if matches!(op, BinOp::Eq | BinOp::Ne)
                && !matches!(args[0].ty, Ty::Base(TyCon::Int) | Ty::Base(TyCon::Bool))
            {
                return Err(CodegenError::Unsupported(eq_operand_label(*op)));
            }
            let l = lower_expr(ctx, func, b, lc, &args[0], env)?.into_int_value();
            let r = lower_expr(ctx, func, b, lc, &args[1], env)?.into_int_value();
            // Deliberately NO nsw/nuw flags: defined two's-complement wrapping
            // (§3.4). Overflow reconciliation with the evaluator is tracked in
            // spec §11 — not silently decided here.
            //
            // Comparisons are SIGNED: Elya's Int is i64 two's-complement, so
            // `(0 - 1) < 1` must be true. `and`/`or` are strict and bit-wise on
            // i1 because BOTH evaluators are strict (spec §4.1) — a
            // short-circuit diamond here would make native less-undefined than
            // `elya run`, which is the mirror image of the Div trade.
            let built = match op {
                BinOp::Add => b.build_int_add(l, r, "add"),
                BinOp::Sub => b.build_int_sub(l, r, "sub"),
                BinOp::Mul => b.build_int_mul(l, r, "mul"),
                BinOp::Lt => b.build_int_compare(IntPredicate::SLT, l, r, "lt"),
                BinOp::Le => b.build_int_compare(IntPredicate::SLE, l, r, "le"),
                BinOp::Gt => b.build_int_compare(IntPredicate::SGT, l, r, "gt"),
                BinOp::Ge => b.build_int_compare(IntPredicate::SGE, l, r, "ge"),
                BinOp::Eq => b.build_int_compare(IntPredicate::EQ, l, r, "eq"),
                BinOp::Ne => b.build_int_compare(IntPredicate::NE, l, r, "ne"),
                BinOp::And => b.build_and(l, r, "and"),
                BinOp::Or => b.build_or(l, r, "or"),
                other => return Err(CodegenError::Unsupported(op_label(*other))),
            };
            built.map(|v| v.into()).map_err(internal)
        }
        CoreKind::App(callee, args) => {
            // Ordinary (non-tail) position: `tailcc` convention, NO tail-call
            // kind. Task 4 adds the tail-position path.
            let site = build_elya_call(ctx, func, b, lc, callee, args, env)?;
            site.try_as_basic_value()
                .left()
                .ok_or(CodegenError::Unsupported("call returned no value"))
        }
        CoreKind::Lambda(..) => Err(CodegenError::Unsupported("Lambda")),
        CoreKind::If(cond, then_e, else_e) => {
            let c = lower_expr(ctx, func, b, lc, cond, env)?.into_int_value();
            // The checker unifies the condition with Bool, so §3.1's mapping
            // makes it i1. Anything else is a bug in our own lowering, surfaced
            // rather than handed to `build_conditional_branch`.
            if c.get_type().get_bit_width() != 1 {
                return Err(CodegenError::Unsupported("non-Bool if condition"));
            }
            let then_bb = ctx.append_basic_block(func, "then");
            let else_bb = ctx.append_basic_block(func, "else");
            let join_bb = ctx.append_basic_block(func, "ifcont");
            b.build_conditional_branch(c, then_bb, else_bb)
                .map_err(internal)?;

            b.position_at_end(then_bb);
            let tv = lower_expr(ctx, func, b, lc, then_e, env)?;
            // THE TRAP (§3.2): a nested `if` inside this branch left the builder
            // in ITS join block, not in `then_bb`. `phi` names the block control
            // actually flows FROM, so read the exit block back from the builder
            // instead of assuming it is the block we positioned at.
            let then_exit = b
                .get_insert_block()
                .ok_or(CodegenError::Unsupported("builder left no block"))?;
            b.build_unconditional_branch(join_bb).map_err(internal)?;

            b.position_at_end(else_bb);
            let ev = lower_expr(ctx, func, b, lc, else_e, env)?;
            let else_exit = b
                .get_insert_block()
                .ok_or(CodegenError::Unsupported("builder left no block"))?;
            b.build_unconditional_branch(join_bb).map_err(internal)?;

            b.position_at_end(join_bb);
            // Exactly two incoming values, always: the AST's `else_block` is not
            // an Option, so there is no one-armed `if` to synthesize a Unit
            // branch for. Both branches carry the same type — the checker
            // unified them — so one phi type is correct.
            let phi = match tv.get_type() {
                BasicTypeEnum::IntType(t) => b.build_phi(t, "iftmp"),
                BasicTypeEnum::PointerType(t) => b.build_phi(t, "iftmp"),
                _ => return Err(CodegenError::Unsupported("unrepresentable type")),
            }
            .map_err(internal)?;
            phi.add_incoming(&[(&tv, then_exit), (&ev, else_exit)]);
            Ok(phi.as_basic_value())
        }
        CoreKind::Ctor(name, fields) => {
            let i64t = ctx.i64_type();
            // A miss means the constructor's type was parametric and deferred in
            // Task 2 — refused by name, never unwrapped.
            let (tag, field_tys) = lc
                .ctors
                .get(name)
                .cloned()
                .ok_or(CodegenError::Unsupported("parametric ADT"))?;
            let vals: Vec<BasicValueEnum<'ctx>> = fields
                .iter()
                .map(|f| lower_expr(ctx, func, b, lc, f, env))
                .collect::<Result<_, _>>()?;
            let p = b
                .build_call(
                    lc.alloc,
                    &[i64t.const_int((1 + field_tys.len()) as u64, false).into()],
                    "a",
                )
                .map_err(internal)?
                .try_as_basic_value()
                .left()
                .ok_or(CodegenError::Unsupported("elya_alloc returned no value"))?
                .into_pointer_value();
            b.build_store(p, i64t.const_int(tag as u64, false))
                .map_err(internal)?;
            for (i, fv) in vals.into_iter().enumerate() {
                let fp =
                    unsafe { b.build_gep(i64t, p, &[i64t.const_int((i + 1) as u64, false)], "fp") }
                        .map_err(internal)?;
                let word = match &field_tys[i] {
                    Ty::Base(TyCon::Int) => fv.into_int_value(),
                    Ty::Base(TyCon::Bool) => b
                        .build_int_z_extend(fv.into_int_value(), i64t, "zw")
                        .map_err(internal)?,
                    _ => b
                        .build_ptr_to_int(fv.into_pointer_value(), i64t, "p2i")
                        .map_err(internal)?,
                };
                b.build_store(fp, word).map_err(internal)?;
            }
            Ok(p.into())
        }
        CoreKind::Match(scrutinee, arms) => {
            let i64t = ctx.i64_type();
            let ptrt = ctx.ptr_type(AddressSpace::default());
            let s = lower_expr(ctx, func, b, lc, scrutinee, env)?.into_pointer_value();
            let tag = b
                .build_load(i64t, s, "tag")
                .map_err(internal)?
                .into_int_value();
            let join_bb = ctx.append_basic_block(func, "mjoin");
            let mut incoming: Vec<(BasicValueEnum<'ctx>, inkwell::basic_block::BasicBlock<'ctx>)> =
                Vec::new();
            let mut fallthrough = b
                .get_insert_block()
                .ok_or_else(|| internal("builder left no block"))?;
            let mut terminal = false;
            for arm in arms.iter() {
                let body_bb = ctx.append_basic_block(func, "marm");
                match &arm.pat {
                    CorePat::Ctor(name, pat_args) => {
                        let (tag_idx, field_tys) = lc
                            .ctors
                            .get(name)
                            .cloned()
                            .ok_or(CodegenError::Unsupported("parametric ADT"))?;
                        b.position_at_end(fallthrough);
                        let cmp = b
                            .build_int_compare(
                                IntPredicate::EQ,
                                tag,
                                i64t.const_int(tag_idx as u64, false),
                                "mc",
                            )
                            .map_err(internal)?;
                        let next = ctx.append_basic_block(func, "mnext");
                        b.build_conditional_branch(cmp, body_bb, next)
                            .map_err(internal)?;
                        fallthrough = next;
                        b.position_at_end(body_bb);
                        let mut bindings: Vec<(String, BasicValueEnum<'ctx>)> = Vec::new();
                        for (pi, p) in pat_args.iter().enumerate() {
                            let fp = unsafe {
                                b.build_gep(
                                    i64t,
                                    s,
                                    &[i64t.const_int((pi + 1) as u64, false)],
                                    "fp",
                                )
                            }
                            .map_err(internal)?;
                            let loaded = b
                                .build_load(i64t, fp, "fld")
                                .map_err(internal)?
                                .into_int_value();
                            let field_val: BasicValueEnum<'ctx> = match &field_tys[pi] {
                                Ty::Base(TyCon::Int) => loaded.into(),
                                Ty::Base(TyCon::Bool) => b
                                    .build_int_truncate(loaded, ctx.bool_type(), "bt")
                                    .map_err(internal)?
                                    .into(),
                                _ => b
                                    .build_int_to_ptr(loaded, ptrt, "i2p")
                                    .map_err(internal)?
                                    .into(),
                            };
                            match p {
                                CorePat::Var(v) => bindings.push((v.clone(), field_val)),
                                CorePat::Wild => {}
                                CorePat::Ctor(..) => {
                                    return Err(CodegenError::Unsupported(
                                        "nested constructor pattern",
                                    ))
                                }
                                CorePat::Lit(_) => {
                                    return Err(CodegenError::Unsupported("literal pattern"))
                                }
                            }
                        }
                        for (n, v) in &bindings {
                            env.insert(n.clone(), *v);
                        }
                        let v = lower_expr(ctx, func, b, lc, &arm.body, env)?;
                        for (n, _) in &bindings {
                            env.remove(n);
                        }
                        let exit = b
                            .get_insert_block()
                            .ok_or_else(|| internal("builder left no block"))?;
                        b.build_unconditional_branch(join_bb).map_err(internal)?;
                        incoming.push((v, exit));
                    }
                    CorePat::Wild => {
                        b.position_at_end(fallthrough);
                        b.build_unconditional_branch(body_bb).map_err(internal)?;
                        b.position_at_end(body_bb);
                        let v = lower_expr(ctx, func, b, lc, &arm.body, env)?;
                        let exit = b
                            .get_insert_block()
                            .ok_or_else(|| internal("builder left no block"))?;
                        b.build_unconditional_branch(join_bb).map_err(internal)?;
                        incoming.push((v, exit));
                        terminal = true;
                    }
                    CorePat::Var(name) => {
                        b.position_at_end(fallthrough);
                        b.build_unconditional_branch(body_bb).map_err(internal)?;
                        b.position_at_end(body_bb);
                        let prev = env.insert(name.clone(), s.into());
                        let v = lower_expr(ctx, func, b, lc, &arm.body, env);
                        match prev {
                            Some(p) => {
                                env.insert(name.clone(), p);
                            }
                            None => {
                                env.remove(name);
                            }
                        }
                        let v = v?;
                        let exit = b
                            .get_insert_block()
                            .ok_or_else(|| internal("builder left no block"))?;
                        b.build_unconditional_branch(join_bb).map_err(internal)?;
                        incoming.push((v, exit));
                        terminal = true;
                    }
                    CorePat::Lit(_) => return Err(CodegenError::Unsupported("literal pattern")),
                }
            }
            // The default block — reached only if the last arm was a constructor
            // and no tag matched — traps via elya_match_fail, never UB.
            if !terminal {
                b.position_at_end(fallthrough);
                b.build_call(lc.fail, &[], "fail").map_err(internal)?;
                // `elya_match_fail` exits (calls exit(1)); this terminator is never
                // reached — a placeholder, NOT the trap.
                b.build_unreachable().map_err(internal)?;
            }
            b.position_at_end(join_bb);
            let phi = match node_ty {
                BasicTypeEnum::IntType(t) => b.build_phi(t, "mph"),
                BasicTypeEnum::PointerType(t) => b.build_phi(t, "mph"),
                _ => return Err(CodegenError::Unsupported("unrepresentable type")),
            }
            .map_err(internal)?;
            for (v, bbb) in incoming.iter() {
                phi.add_incoming(&[(v, *bbb)]);
            }
            Ok(phi.as_basic_value())
        }
    }
}

/// The shared lowering context: function declarations, the constructor table, and
/// the two runtime externals. Bundled so the lowering fold threads one reference
/// instead of four separate ones.
struct LowerCtx<'ctx> {
    decls: &'ctx HashMap<String, FunctionValue<'ctx>>,
    /// Constructor name -> (tag index within its type, field types). A name
    /// missing here is a constructor of a *parametric* ADT, which Task 2
    /// deferred — refused by name in the Ctor/Match arms, never unwrapped.
    ctors: &'ctx HashMap<String, (usize, Vec<Ty>)>,
    alloc: FunctionValue<'ctx>,
    fail: FunctionValue<'ctx>,
}

/// Fold `core.types` into a flat constructor table: name -> (tag, field types).
/// The tag is the constructor's index within its own type's declaration order.
fn build_ctor_table(core: &CoreModule) -> HashMap<String, (usize, Vec<Ty>)> {
    let mut out = HashMap::new();
    for t in &core.types {
        for (i, c) in t.ctors.iter().enumerate() {
            out.insert(c.name.clone(), (i, c.fields.clone()));
        }
    }
    out
}

/// Build the verified LLVM module for `core` into `ctx`: `@elya_main` lowering
/// the Core body, plus (Task 4) the printf declaration, format-string global,
/// and `@main` shim. Returns the handle so `emit_ir` and `compile_module` share
/// one construction path.
fn build_module<'ctx>(ctx: &'ctx Context, core: &CoreModule) -> Result<Module<'ctx>, CodegenError> {
    // §5.1, FIRST STATEMENT ON PURPOSE. The failure this prevents is LLVM's
    // `report_fatal_error` for a win64 guaranteed tail call: no source span, no
    // `Result`, the process simply dies. It must therefore be impossible for
    // any emission to have begun when this fires — so it is a whole-module scan
    // that runs before the module even exists. Tracked as obligation T1.
    for f in &core.fns {
        if f.params.len() > MAX_PARAMS {
            return Err(CodegenError::Unsupported(
                "function takes more than five parameters",
            ));
        }
    }

    let main = find_main(core)?;
    // §5.5: `main` ALONE. The scope narrows from "every function" (which was
    // trivially just `main` in N1) to "`main`", because `@elya_main`'s signature
    // says i64 and the shim's format string is `%lld`. Applying this per
    // function would refuse every ordinary predicate — see `is_pos` in the
    // corpus — and is the most likely way to implement this section wrong.
    require_int(&main.body.ty)?;

    let i64t = ctx.i64_type();
    let i32t = ctx.i32_type();
    let i8t = ctx.i8_type();
    let ptrt = ctx.ptr_type(AddressSpace::default());
    let module = ctx.create_module("elya");

    // §4.2: declare everything (functions + the two runtime externals), then emit
    // every body. The runtime externals are `ccc` (the C-ABI boundary); everything
    // Elya-internal stays `tailcc`.
    let decls = declare_all(ctx, &module, core)?;
    let ctors = build_ctor_table(core);

    let alloc_ty = ptrt.fn_type(&[i64t.into()], false);
    let alloc = module.add_function("elya_alloc", alloc_ty, None); // ccc
    let fail_ty = ctx.void_type().fn_type(&[], false);
    let fail = module.add_function("elya_match_fail", fail_ty, None); // ccc

    let lc = LowerCtx {
        decls: &decls,
        ctors: &ctors,
        alloc,
        fail,
    };
    let b = ctx.create_builder();
    for f in &core.fns {
        emit_body(ctx, &b, &lc, f)?;
    }
    let elya_main = *decls
        .get("main")
        .ok_or(CodegenError::Unsupported("no `main`"))?;

    // §3.5/§4: the print convention is ONE external symbol (printf) plus ONE
    // generated shim (@main). The shim stays `ccc` — it is the C entry point —
    // and its call to @elya_main is an ordinary call.
    let printf_ty = i32t.fn_type(&[ptrt.into(), i64t.into()], true);
    let printf = module.add_function("printf", printf_ty, None);

    let fmt_bytes: &[u8] = b"%lld\n\0";
    let fmt_const = i8t.const_array(
        &fmt_bytes
            .iter()
            .map(|c| i8t.const_int(*c as u64, false))
            .collect::<Vec<_>>(),
    );
    let fmt = module.add_global(fmt_const.get_type(), Some(AddressSpace::default()), ".fmt");
    fmt.set_initializer(&fmt_const);
    fmt.set_constant(true);
    fmt.set_unnamed_addr(true);

    let shim = module.add_function("main", i32t.fn_type(&[], false), None);
    let shim_entry = ctx.append_basic_block(shim, "entry");
    b.position_at_end(shim_entry);
    let site = b.build_call(elya_main, &[], "v").map_err(internal)?;
    // The shim itself stays `ccc`, but @elya_main is now `tailcc`, so THIS CALL
    // SITE must say so too. A site whose convention disagrees with its callee's
    // declaration is a miscompile, not a verifier error — LLVM will happily emit
    // it. "The shim survives untouched" (§2 Finding 4) means the shim is still
    // `ccc` and its call is not `musttail`; it does NOT mean this line is
    // unchanged.
    site.set_call_convention(TAILCC);
    let v = site
        .try_as_basic_value()
        .left()
        .ok_or_else(|| internal("elya_main did not return a value"))?;
    b.build_call(printf, &[fmt.as_pointer_value().into(), v.into()], "p")
        .map_err(internal)?;
    b.build_return(Some(&i32t.const_int(0, false)))
        .map_err(internal)?;

    module
        .verify()
        .map_err(|e| CodegenError::Verify(e.to_string()))?;
    Ok(module)
}

/// Debugging aid ONLY (spec §7): renders the verified module as textual IR so a
/// failed execution proof is easier to debug. Never asserted on; no snapshot
/// test uses it; its existence is not a proof of anything (§0).
pub fn emit_ir(core: &CoreModule) -> Result<String, CodegenError> {
    let ctx = Context::create();
    let module = build_module(&ctx, core)?;
    Ok(module.print_to_string().to_string())
}

/// Core -> object file on disk. The spine (§3.6): host triple only,
/// `OptimizationLevel::None`, `verify()` before emission.
pub fn compile_module(core: &CoreModule, obj_path: &Path) -> Result<(), CodegenError> {
    let ctx = Context::create();
    let module = build_module(&ctx, core)?;

    Target::initialize_native(&InitializationConfig::default()).map_err(internal)?;
    let triple = TargetMachine::get_default_triple();
    let target = Target::from_triple(&triple).map_err(internal)?;
    let machine = target
        .create_target_machine(
            &triple,
            "",
            "",
            OptimizationLevel::None,
            RelocMode::Default,
            CodeModel::Default,
        )
        .ok_or_else(|| internal("no host target machine"))?;
    machine
        .write_to_file(&module, FileType::Object, obj_path)
        .map_err(internal)
}

/// Object file -> executable, via `clang` (hardcoded, §3.6: LLVM is already a
/// hard prerequisite and clang ships with it). Non-zero exit surfaces clang's
/// stderr in [`CodegenError::Link`].
pub fn link(obj: &Path, exe: &Path) -> Result<(), CodegenError> {
    // N4 (spec §2.1): the runtime is a C file clang compiles and links alongside
    // the object, so the two `ccc` externals resolve.
    let runtime = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/runtime.c");
    let out = std::process::Command::new("clang")
        .arg(obj)
        .arg(&runtime)
        .arg("-o")
        .arg(exe)
        .output()
        .map_err(CodegenError::Io)?;
    if !out.status.success() {
        return Err(CodegenError::Link {
            code: out.status.code(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use elya::span::Span;
    use std::rc::Rc;

    fn int_lit(n: i64) -> CoreExpr {
        CoreExpr {
            span: Span::EMPTY,
            ty: Ty::Base(TyCon::Int),
            kind: CoreKind::Lit(CoreLit::Int(n)),
        }
    }

    fn prim(op: BinOp, l: CoreExpr, r: CoreExpr) -> CoreExpr {
        CoreExpr {
            span: Span::EMPTY,
            ty: Ty::Base(TyCon::Int),
            kind: CoreKind::Prim(op, vec![l, r].into()),
        }
    }

    fn main_fn(body: CoreExpr) -> CoreModule {
        CoreModule {
            fns: vec![CoreFn {
                name: "main".into(),
                params: Rc::from([]),
                body,
            }],
            types: Vec::new(),
        }
    }

    fn if_expr(c: CoreExpr, t: CoreExpr, e: CoreExpr) -> CoreExpr {
        CoreExpr {
            span: Span::EMPTY,
            ty: t.ty.clone(),
            kind: CoreKind::If(Rc::new(c), Rc::new(t), Rc::new(e)),
        }
    }

    fn cmp(op: BinOp, l: CoreExpr, r: CoreExpr) -> CoreExpr {
        CoreExpr {
            span: Span::EMPTY,
            ty: Ty::Base(TyCon::Bool),
            kind: CoreKind::Prim(op, vec![l, r].into()),
        }
    }

    /// Parse → infer → lower a real Elya source through the front end. Used by
    /// every test whose witness must be a program a user could actually write,
    /// rather than a hand-built `CoreModule`.
    fn core_of(src: &str) -> elya::core::CoreModule {
        let session = elya::Session::new();
        let (m, pd) = elya::parse::parse_module(&session, src);
        assert!(pd.is_empty(), "parse: {pd:?}");
        let (diags, table) = elya::types::infer_typed_table(&session, &m);
        assert!(diags.is_empty(), "type errors: {diags:?}");
        elya::core::lower_module(&m, &table).expect("lowers")
    }

    #[test]
    fn corpus_verifies() {
        // Layer 1 (§8): verifier-clean IR for the §5 corpus. Cheap structural
        // teeth — NOT the proof (that is execution in tests/native_codegen.rs).
        // The corpus strings duplicate tests/native_codegen.rs's CORPUS,
        // CONTROL_FLOW_CORPUS, FUNCTION_CORPUS, TAIL_CORPUS, and ADT_CORPUS
        // because integration targets cannot share consts; twenty-two lines of
        // duplication is cheaper than new plumbing.
        let corpus = [
            "pub fn main() { 1 + 2 }\n",
            "pub fn main() {\n  let x = 6\n  let y = 7\n  x * y\n}\n",
            "pub fn main() { (2 + 3) * 4 - 5 }\n",
            "pub fn main() { 3 - 10 }\n",
            "pub fn main() { if 1 < 2 { 10 } else { 20 } }\n",
            "pub fn main() { if 2 < 1 { 10 } else { 20 } }\n",
            "pub fn main() {\n  let x = 5\n  if x > 3 { x * 2 } else { 0 }\n}\n",
            "pub fn main() {\n  let x = 7\n  let a = if x > 0 { if x > 5 { 100 } else { 50 } } else { 0 }\n  let b = if x < 0 { 0 } else { if x > 5 { 7 } else { 3 } }\n  a + b\n}\n",
            "pub fn main() {\n  let a = if 1 < 1 { 1 } else { 0 }\n  let b = if 1 < 2 { 1 } else { 0 }\n  let c = if 1 <= 1 { 1 } else { 0 }\n  let d = if 2 <= 1 { 1 } else { 0 }\n  let e = if 1 > 1 { 1 } else { 0 }\n  let f = if 2 > 1 { 1 } else { 0 }\n  let g = if 1 >= 1 { 1 } else { 0 }\n  let h = if 1 >= 2 { 1 } else { 0 }\n  let i = if 1 == 1 { 1 } else { 0 }\n  let j = if 1 == 2 { 1 } else { 0 }\n  let k = if 1 != 2 { 1 } else { 0 }\n  let m = if 1 != 1 { 1 } else { 0 }\n  let n = if (0 - 1) < 1 { 1 } else { 0 }\n  a + b + c + d + e + f + g + h + i + j + k + m + n\n}\n",
            "pub fn main() { if True == False { 1 } else { 2 } }\n",
            "pub fn main() {\n  let a = if 1 < 2 && 3 > 4 { 1 } else { 0 }\n  let b = if 1 < 2 && 3 < 4 { 1 } else { 0 }\n  let c = if 1 > 2 || 3 > 4 { 1 } else { 0 }\n  let d = if 1 > 2 || 3 < 4 { 1 } else { 0 }\n  a + b + c + d\n}\n",
            "fn add3(x) { x + 3 }\npub fn main() { add3(4) }\n",
            "fn add5(a, b, c, d, e) { a + b + c + d + e }\npub fn main() { add5(1, 2, 3, 4, 5) }\n",
            "fn f(x) {\n  let x = x * 2\n  x + 20\n}\nfn g(x) { x + f(x) }\npub fn main() { g(10) }\n",
            "fn dbl(x) { x * 2 }\npub fn main() { dbl(dbl(3)) + dbl(1) }\n",
            "fn is_pos(n) { n > 0 }\npub fn main() { if is_pos(3) { 1 } else { 0 } }\n",
            "fn sum(n) { if n == 0 { 0 } else { n + sum(n - 1) } }\npub fn main() { sum(100) }\n",
            "fn down(n) { if n == 0 { 0 } else { down(n - 1) } }\npub fn main() { down(1000000) }\n",
            "fn ev(n) { if n == 0 { True } else { od(n - 1) } }\nfn od(n) { if n == 0 { False } else { ev(n - 1) } }\npub fn main() { if ev(1000000) { 1 } else { 0 } }\n",
            "type Opt { None, Some(Int) }\npub fn main() { match Some(42) { None -> 0  Some(x) -> x } }\n",
            "type Nat { Zero, Succ(Nat) }\nfn len(n) { match n { Zero -> 0  Succ(m) -> 1 + len(m) } }\npub fn main() { len(Succ(Succ(Succ(Zero)))) }\n",
            "type T { A, B, C(Int) }\npub fn main() { match C(7) { A -> 1  B -> 2  C(x) -> x } }\n",
        ];
        for src in corpus {
            emit_ir(&core_of(src)).expect("verifier-clean IR");
        }
    }

    #[test]
    fn a_multi_function_module_emits_both_functions() {
        // The headline shape change. `emit_ir` returns IR as a debugging aid
        // only (spec §7) — it is never snapshotted and never asserted on
        // structurally. What is asserted here is that `build_module` accepted a
        // two-function module at all and that the verifier passed it; the
        // behaviour is proven by execution in tests/native_codegen.rs.
        emit_ir(&core_of(
            "fn add3(x) { x + 3 }\npub fn main() { add3(4) }\n",
        ))
        .expect("a two-function module must emit verifier-clean IR");
    }

    #[test]
    fn rejects_a_duplicate_top_level_function_specifically() {
        // Hand-built: the front end rejects duplicate definitions, so no surface
        // program reaches this. The refusal still has to exist, because LLVM
        // would silently uniquify the second symbol rather than complain.
        let mut m = main_fn(int_lit(1));
        m.fns.push(CoreFn {
            name: "main".into(),
            params: Rc::from([]),
            body: int_lit(2),
        });
        let err = emit_ir(&m).unwrap_err();
        assert!(
            matches!(
                err,
                CodegenError::Unsupported("duplicate top-level function")
            ),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_div_specifically() {
        let err = emit_ir(&main_fn(prim(BinOp::Div, int_lit(1), int_lit(2)))).unwrap_err();
        assert!(matches!(err, CodegenError::Unsupported("Div")), "{err:?}");
    }

    #[test]
    fn an_if_diamond_verifies() {
        let m = main_fn(if_expr(
            cmp(BinOp::Lt, int_lit(1), int_lit(2)),
            int_lit(10),
            int_lit(20),
        ));
        emit_ir(&m).expect("the diamond must produce verifier-clean IR");
    }

    #[test]
    fn a_nested_if_in_a_branch_verifies() {
        // The phi trap (§3.2): the inner `if` leaves the builder in ITS join
        // block, so the outer phi must name that block, not `then`. Getting it
        // wrong is a verifier error, which is why this is worth a Layer-1 test
        // even though execution is the real proof.
        let inner = if_expr(
            cmp(BinOp::Gt, int_lit(7), int_lit(5)),
            int_lit(100),
            int_lit(50),
        );
        let m = main_fn(if_expr(
            cmp(BinOp::Gt, int_lit(7), int_lit(0)),
            inner,
            int_lit(0),
        ));
        emit_ir(&m).expect("a nested diamond must produce verifier-clean IR");
    }

    #[test]
    fn strict_and_or_verify() {
        let m = main_fn(if_expr(
            CoreExpr {
                span: Span::EMPTY,
                ty: Ty::Base(TyCon::Bool),
                kind: CoreKind::Prim(
                    BinOp::And,
                    vec![
                        cmp(BinOp::Lt, int_lit(1), int_lit(2)),
                        cmp(BinOp::Gt, int_lit(3), int_lit(4)),
                    ]
                    .into(),
                ),
            },
            int_lit(1),
            int_lit(0),
        ));
        emit_ir(&m).expect("strict and must produce verifier-clean IR");
    }

    #[test]
    fn rejects_eq_on_an_unrepresentable_operand_type() {
        // Eq/Ne are the only fully polymorphic operators in the subset: the
        // checker is happy with `Str == Str` and there is no representation for
        // it. Dispatched on the OPERAND type before the operands are lowered, so
        // the message names the operator (§3.3) rather than the operand. Wrapped
        // in an `if` because a Bool-bodied main is refused earlier, by
        // `require_int`, with a different message.
        let s = CoreExpr {
            span: Span::EMPTY,
            ty: Ty::Base(TyCon::Str),
            kind: CoreKind::Lit(CoreLit::Str("a".into())),
        };
        let bad = CoreExpr {
            span: Span::EMPTY,
            ty: Ty::Base(TyCon::Bool),
            kind: CoreKind::Prim(BinOp::Eq, vec![s.clone(), s].into()),
        };
        let err = emit_ir(&main_fn(if_expr(bad, int_lit(1), int_lit(0)))).unwrap_err();
        assert!(
            matches!(
                err,
                CodegenError::Unsupported("Eq on an unrepresentable operand type")
            ),
            "{err:?}"
        );
    }

    #[test]
    fn accepts_eq_on_bool_operands() {
        // The other side of the same door: Bool is a concrete type with a
        // representation, so `True == False` is an icmp on i1, not a refusal.
        let t = CoreExpr {
            span: Span::EMPTY,
            ty: Ty::Base(TyCon::Bool),
            kind: CoreKind::Lit(CoreLit::Bool(true)),
        };
        let f = CoreExpr {
            span: Span::EMPTY,
            ty: Ty::Base(TyCon::Bool),
            kind: CoreKind::Lit(CoreLit::Bool(false)),
        };
        let m = main_fn(if_expr(cmp(BinOp::Eq, t, f), int_lit(1), int_lit(2)));
        emit_ir(&m).expect("Eq on Bool operands must verify");
    }

    #[test]
    fn rejects_bool_literal_specifically() {
        let e = CoreExpr {
            span: Span::EMPTY,
            ty: Ty::Base(TyCon::Bool),
            kind: CoreKind::Lit(CoreLit::Bool(true)),
        };
        let err = emit_ir(&main_fn(e)).unwrap_err();
        assert!(
            matches!(err, CodegenError::Unsupported("non-Int value")),
            "{err:?}"
        );
    }

    #[test]
    fn a_bool_binding_is_representable_even_though_a_bool_main_is_not() {
        // Layer-1 teeth only — an i1 constant costs no instruction, so this
        // proves the type mapping accepts Bool, not that anything computes with
        // it. The real proof is Task 5's execution corpus. Hand-built because no
        // source program can produce a Bool-typed node until Task 4 lands `if`.
        let m = main_fn(CoreExpr {
            span: Span::EMPTY,
            ty: Ty::Base(TyCon::Int),
            kind: CoreKind::Let(
                "b".into(),
                Rc::new(CoreExpr {
                    span: Span::EMPTY,
                    ty: Ty::Base(TyCon::Bool),
                    kind: CoreKind::Lit(CoreLit::Bool(true)),
                }),
                Rc::new(int_lit(1)),
            ),
        });
        emit_ir(&m).expect("a Bool binding must verify");
    }

    #[test]
    fn rejects_a_string_typed_node_by_name() {
        // The widening is exactly two widths wide. Str is not one of them, and
        // it is refused as an unrepresentable *type*, distinct from the
        // "non-Int value" that guards main's return type.
        let m = main_fn(CoreExpr {
            span: Span::EMPTY,
            ty: Ty::Base(TyCon::Int),
            kind: CoreKind::Let(
                "s".into(),
                Rc::new(CoreExpr {
                    span: Span::EMPTY,
                    ty: Ty::Base(TyCon::Str),
                    kind: CoreKind::Lit(CoreLit::Str("a".into())),
                }),
                Rc::new(int_lit(1)),
            ),
        });
        let err = emit_ir(&m).unwrap_err();
        assert!(
            matches!(err, CodegenError::Unsupported("unrepresentable type")),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_lambda_specifically() {
        let e = CoreExpr {
            span: Span::EMPTY,
            ty: Ty::Base(TyCon::Int),
            kind: CoreKind::Lambda(Rc::from(["x".to_string()]), Rc::new(int_lit(1))),
        };
        let err = emit_ir(&main_fn(e)).unwrap_err();
        assert!(
            matches!(err, CodegenError::Unsupported("Lambda")),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_parameterised_main_specifically() {
        let mut m = main_fn(int_lit(1));
        m.fns[0].params = Rc::from([elya::core::CoreParam {
            name: "x".into(),
            ty: Ty::Base(TyCon::Int),
        }]);
        let err = emit_ir(&m).unwrap_err();
        assert!(
            matches!(err, CodegenError::Unsupported("main takes parameters")),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_a_parametric_adt_constructor_by_name() {
        // `Some` belongs to a parametric ADT (deferred in Task 2), so it is NOT in
        // `types`. Lowering its `Ctor` must refuse by name — never unwrap, never a
        // generic unknown-type error.
        let m = main_fn(CoreExpr {
            span: Span::EMPTY,
            ty: Ty::Base(TyCon::Int),
            kind: CoreKind::Let(
                "o".into(),
                Rc::new(CoreExpr {
                    span: Span::EMPTY,
                    ty: Ty::Con("Option".into(), vec![]),
                    kind: CoreKind::Ctor("Some".into(), Rc::from([int_lit(1)])),
                }),
                Rc::new(int_lit(0)),
            ),
        });
        let err = emit_ir(&m).unwrap_err();
        assert!(
            matches!(err, CodegenError::Unsupported("parametric ADT")),
            "{err:?}"
        );
    }

    #[test]
    fn let_shadowing_restores_prior_binding() {
        // `let x = 1; let x = x + 1; x` — the inner body must see x == 2, and
        // the save/restore discipline must leave no stale binding. Structural
        // check: the fold succeeds and verify passes (value correctness is
        // proven by execution in Task 4's lets case).
        let inner = CoreExpr {
            span: Span::EMPTY,
            ty: Ty::Base(TyCon::Int),
            kind: CoreKind::Var("x".into()),
        };
        let rhs = prim(BinOp::Add, int_lit(1), int_lit(1));
        let outer_body = CoreExpr {
            span: Span::EMPTY,
            ty: Ty::Base(TyCon::Int),
            kind: CoreKind::Let("x".into(), Rc::new(rhs), Rc::new(inner)),
        };
        let body = CoreExpr {
            span: Span::EMPTY,
            ty: Ty::Base(TyCon::Int),
            kind: CoreKind::Let("x".into(), Rc::new(int_lit(1)), Rc::new(outer_body)),
        };
        emit_ir(&main_fn(body)).expect("shadowed let verifies");
    }

    // --- §5: the five refusals ---------------------------------------------
    // Each removes a program from the set BOTH back ends accept, so none of
    // these violates the §3.4 fidelity rule. Silently mis-compiling would.

    #[test]
    fn rejects_six_parameters_before_emission_begins() {
        // §5.1. The most load-bearing refusal in the slice: past five
        // parameters, a win64 guaranteed tail call hits `LLVM ERROR: Can't
        // handle guaranteed tail call under win64 yet`, a `report_fatal_error`
        // with no source span that kills the process outright. No `Result` can
        // catch it, so the scan must complete before any emission starts.
        let err = emit_ir(&core_of(
            "fn six(a, b, c, d, e, f) { a + b + c + d + e + f }\n\
             pub fn main() { six(1, 2, 3, 4, 5, 6) }\n",
        ))
        .unwrap_err();
        assert!(
            matches!(
                err,
                CodegenError::Unsupported("function takes more than five parameters")
            ),
            "{err:?}"
        );
        // Five is the boundary, and it is exercised, not only refused: the
        // `five_params` corpus program compiles and runs in
        // tests/native_codegen.rs.
        assert_eq!(MAX_PARAMS, 5);
    }

    #[test]
    fn a_polymorphic_function_refuses_the_whole_module() {
        // §5.2 / obligation T2. `repr_ty` refuses `Ty::Var(_)`, and `declare_all`
        // runs it over every parameter and body type in the module — so `id`
        // refuses this module even though `main` never calls it. That
        // whole-module strictness is deliberate; the relaxation path (emit only
        // what is reachable from `main`) is recorded as T2, not taken here.
        let err = emit_ir(&core_of("fn id(x) { x }\npub fn main() { 1 }\n")).unwrap_err();
        assert!(
            matches!(err, CodegenError::Unsupported("unrepresentable type")),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_a_computed_callee_specifically() {
        // §5.3, witnessed by a program a user could actually write. The Pratt
        // parser applies the postfix call loop to ANY atom and `(expr)` unwraps
        // with no wrapper node, so this parses to `Call { callee: Lambda, .. }`
        // and lowers to `App(Lambda, ..)` — it really does reach the back end.
        //
        // Order matters and is in our control: `build_elya_call` inspects the
        // callee kind FIRST, so "computed callee" fires before the pre-existing
        // `Lambda` arm of `lower_expr` is ever visited. If this test starts
        // reporting `Unsupported("Lambda")`, that ordering has been inverted.
        let err = emit_ir(&core_of("pub fn main() { (fn(x) { x + 1 })(3) }\n")).unwrap_err();
        assert!(
            matches!(err, CodegenError::Unsupported("computed callee")),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_a_callee_that_is_not_a_top_level_function() {
        // §5.4. Hand-built, because the type checker rejects calling an unbound
        // name in the front end — no surface program can reach this arm today.
        // The refusal still has to exist: `lower_module` is a public API, and a
        // future front-end change must fail loudly here rather than emit a call
        // to a symbol that was never declared.
        let call = CoreExpr {
            span: Span::EMPTY,
            ty: Ty::Base(TyCon::Int),
            kind: CoreKind::App(
                Rc::new(CoreExpr {
                    span: Span::EMPTY,
                    ty: Ty::Base(TyCon::Int),
                    kind: CoreKind::Var("nope".to_string()),
                }),
                Rc::from([]),
            ),
        };
        let err = emit_ir(&main_fn(call)).unwrap_err();
        assert!(
            matches!(
                err,
                CodegenError::Unsupported("callee is not a top-level function")
            ),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_a_function_name_in_value_position() {
        // §5.4's companion. This IS legal Elya — typed_inference's surface4
        // type-checks `let g = worker  g()` — so the program reaches the back
        // end and gets its own message rather than the generic "unbound var",
        // which would point a reader at name resolution instead of at this gap.
        // N5 (closures) is what makes it representable.
        //
        // The `let` binding is lowered before its body, so this message fires
        // first; the `f(1)` call never gets as far as `build_elya_call`.
        let err = emit_ir(&core_of(
            "fn add3(x) { x + 3 }\npub fn main() {\n  let f = add3\n  f(1)\n}\n",
        ))
        .unwrap_err();
        assert!(
            matches!(err, CodegenError::Unsupported("function used as a value")),
            "{err:?}"
        );
    }

    #[test]
    fn a_bool_returning_helper_compiles_but_a_bool_main_still_refuses() {
        // §5.5. `require_int`'s SCOPE narrows from "every function" (trivially
        // just `main` in N1) to "`main` alone". The likely way to get this wrong
        // is to implement it as scope-PRESERVING — applying `require_int` per
        // function — which would refuse every ordinary predicate. Both halves
        // are pinned here, and the first half is exercised end to end by the
        // `bool_across_a_call` corpus program.
        emit_ir(&core_of(
            "fn is_pos(n) { n > 0 }\npub fn main() { if is_pos(3) { 1 } else { 0 } }\n",
        ))
        .expect("a Bool-returning helper is legal");
        let err = emit_ir(&core_of("pub fn main() { 1 < 2 }\n")).unwrap_err();
        assert!(
            matches!(err, CodegenError::Unsupported("non-Int value")),
            "{err:?}"
        );
    }
}
