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
//! less-undefined than the tree evaluator. Hence `Int` arithmetic is exact or
//! fails by name on both sides (2026-10-10: overflow and a zero divisor trap
//! through the runtime with the evaluator's words), and `&&`/`||`
//! STRICT rather than short-circuiting — because both of Elya's evaluators are
//! strict, so a short-circuit diamond would make native binaries *less*
//! undefined than `elya run` -- observable now that `/` can fail:
//! `False && 1 / 0 == 0` fails on both sides. Short-circuiting is a front-end
//! question, not a back-end one.

mod anf;
mod any;
mod closure;
mod cps;
mod cps_emit;
mod mono;
mod patterns;
mod reflect;
mod specialize;
mod spill;

use std::collections::{HashMap, HashSet};
use std::path::Path;

use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use inkwell::types::BasicMetadataTypeEnum;
use inkwell::types::BasicTypeEnum;
use inkwell::types::FunctionType;
use inkwell::types::{IntType, PointerType};
use inkwell::values::BasicMetadataValueEnum;
use inkwell::values::BasicValueEnum;
use inkwell::values::CallSiteValue;
use inkwell::values::FunctionValue;
use inkwell::values::IntValue;
use inkwell::values::LLVMTailCallKind;
use inkwell::values::PointerValue;
use inkwell::AddressSpace;
use inkwell::IntPredicate;
use inkwell::OptimizationLevel;

use crate::closure::LambdaSite;
use elya::ast::BinOp;
use elya::core::{CoreExpr, CoreFn, CoreKind, CoreLit, CoreModule, CorePat};
use elya::types::{Ty, TyCon};

#[derive(Debug)]
pub enum CodegenError {
    /// Out-of-subset construct. A typed boundary, not a panic (mirrors
    /// `LowerError::Unsupported`); the payload names the construct.
    Unsupported(&'static str),
    /// A builtin whose owned Core name is outside the supported set.
    UnsupportedBuiltin(String),
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
            CodegenError::UnsupportedBuiltin(name) => {
                write!(f, "codegen: unsupported builtin ({name})")
            }
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
        BinOp::LtF => "LtF",
        BinOp::LeF => "LeF",
        BinOp::GtF => "GtF",
        BinOp::GeF => "GeF",
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
/// two Elya names collide. The separator is a dot (the 2026-10-16 review, M1):
/// with `elya_` an Elya function named `alloc`, `println` or `start` became
/// the runtime's own C symbol and the link failed. No C identifier contains a
/// dot, so no Elya name can collide with runtime.c (`elya_*`), with the
/// symbols this back end generates (`@main`, `@elya_entry`, `@.fmt`) or with
/// `@printf`. `main` mangles to `elya.main`.
fn mangle(name: &str) -> String {
    format!("elya.{name}")
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
///
/// This cap applies to the CONVERTED arity — the arity LLVM actually sees. A
/// lambda gets a separate, lower cap (`MAX_LAMBDA_PARAMS`) because closure
/// conversion prepends the closure pointer, so its source arity is one less.
const MAX_PARAMS: usize = 5;

/// The lambda arity cap (5b-6 §5.1). A lambda of `P` source parameters
/// converts to a lifted function of arity `P + 1` — the closure block is
/// parameter 0 — so the source cap is `MAX_PARAMS - 1`, stated as its own
/// constant because it is refused at its own site with its own message.
///
/// C-ii probe, `scratchpad/n5-indirect-probe`, LLVM 18.1.6 /
/// `x86_64-pc-windows-msvc`, 2026-09-03. Sweeping caller arity C × callee arity
/// K over 1..8 for a `musttail` call under `tailcc`, the indirect-callee matrix
/// is cell-for-cell identical to the direct-callee matrix: K <= 5 compiles for
/// every C; K in {6,7} only when C >= 6; K = 8 only when C >= 8. Every passing
/// cell emits a real tail jump (`jmpq *%rax` for the indirect ones); every
/// failing cell is `LLVM ERROR: Can't handle guaranteed tail call under win64
/// yet`, a `report_fatal_error` that kills the process with no source span.
///
/// This number is MEASURED. Do not raise it to make a program compile; re-run
/// the probe, or lower it.
const MAX_LAMBDA_PARAMS: usize = 4;

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

/// §3.1 type mapping: `Int` -> i64, `Bool` -> i1, `Ty::Con` -> a pointer (N4),
/// and — since N6 — `Str` -> a pointer and `Unit` -> i64. Reads the INLINE `ty`
/// field on each Core node (Shape C — the reason this fold needs no side-table
/// lookups). Everything else is refused by name, `Ty::Var(_)` included; since
/// N7 part 2 (`mono.rs`) no type variable reaches here, so that is a guard.
fn repr_ty<'ctx>(ctx: &'ctx Context, ty: &Ty) -> Result<BasicTypeEnum<'ctx>, CodegenError> {
    match ty {
        Ty::Base(TyCon::Int) => Ok(ctx.i64_type().into()),
        Ty::Base(TyCon::Bool) => Ok(ctx.bool_type().into()),
        // N4 (spec §1): an ADT value is a pointer to its heap object.
        Ty::Con(..) => Ok(ctx.ptr_type(AddressSpace::default()).into()),
        // N5 (5b-6 §3): a function value is a pointer to its closure block.
        Ty::Fn(..) => Ok(ctx.ptr_type(AddressSpace::default()).into()),
        // N6 (spec §1.1, §3): a string is a pointer to its heap block; Unit is the
        // immediate i64 zero, never dereferenced, never traced, never rooted.
        Ty::Base(TyCon::Str) => Ok(ctx.ptr_type(AddressSpace::default()).into()),
        Ty::Base(TyCon::Unit) => Ok(ctx.i64_type().into()),
        // A float is its IEEE-754 bits in an untraced word (spec
        // 2026-10-23-elya-float-natively D1).
        Ty::Base(TyCon::Float) => Ok(ctx.i64_type().into()),
        _ => Err(CodegenError::Unsupported("unrepresentable type")),
    }
}

/// Does a value of this type live on the heap, so the collector must trace it?
///
/// ONE predicate, used by both the constructor descriptor rows and the closure
/// descriptor rows, so a single negative control falsifies both call sites. It
/// must agree with `repr_ty`: exactly the types `repr_ty` represents as a pointer
/// are the types the mask marks traced. `a_captured_closure_is_traced_mask_covers_ty_fn`
/// is the execution proof of the `Ty::Fn` half; `mask_and_repr_agree_on_pointers`
/// pins the correspondence itself.
fn is_heap_ty(ty: &Ty) -> bool {
    matches!(ty, Ty::Con(..) | Ty::Fn(..) | Ty::Base(TyCon::Str))
}

/// The `BasicTypeEnum` -> `FunctionType` step, factored out of `declare_all`
/// because `declare_lifted` (5b-6 §4.3) needs exactly the same match: a lifted
/// lambda body is declared the same way a top-level function is, only with the
/// closure pointer spliced in as parameter 0.
fn fn_type_of<'ctx>(
    ret: BasicTypeEnum<'ctx>,
    params: &[BasicMetadataTypeEnum<'ctx>],
) -> Result<FunctionType<'ctx>, CodegenError> {
    match ret {
        BasicTypeEnum::IntType(t) => Ok(t.fn_type(params, false)),
        BasicTypeEnum::PointerType(t) => Ok(t.fn_type(params, false)),
        _ => Err(CodegenError::Unsupported("unrepresentable type")),
    }
}

/// `main` returns i64: `@elya.main`'s signature says so and the print shim's
/// format string is `%lld`. A `Bool`-bodied main is a representable value in an
/// unrepresentable *place*, so it is refused at the module boundary rather than
/// inside the fold — which is why this message stayed "non-Int value" when the
/// fold widened.
fn require_int(ty: &Ty) -> Result<(), CodegenError> {
    // 5b-8 Task 11: a `Unit` main is also an i64 word (0), so the shim's
    // `%lld` prints it as-is; A9's main ends in `io.println(..)`. A `Bool`
    // main stays refused (an i1 is not the shim's word).
    if matches!(ty, Ty::Base(TyCon::Int) | Ty::Base(TyCon::Unit)) {
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
    cps_fns: &HashSet<String>,
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
        // D14/D16: an effectful function takes its continuation as a trailing
        // pointer and answers with the handle's answer as a word: it never
        // returns its own value, it passes it to the continuation. Its body
        // type is still checked representable, as every function's is.
        let ret = repr_ty(ctx, &f.body.ty)?;
        let fn_ty = if cps_fns.contains(&f.name) {
            params.push(ctx.ptr_type(AddressSpace::default()).into());
            ctx.i64_type().fn_type(&params, false)
        } else {
            fn_type_of(ret, &params)?
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

/// One LLVM function per lambda site. Parameter 0 is the closure pointer — the
/// environment IS the closure (5b-6 §3, C-iii) — then the source parameters in
/// order. `tailcc` on every one of them, for the same reason every top-level
/// Elya function gets it: `musttail`'s convention-match requirement is then true
/// by construction.
fn declare_lifted<'ctx>(
    ctx: &'ctx Context,
    module: &Module<'ctx>,
    sites: &[LambdaSite],
) -> Result<HashMap<String, FunctionValue<'ctx>>, CodegenError> {
    let ptrt = ctx.ptr_type(AddressSpace::default());
    let mut out = HashMap::new();
    for site in sites {
        let mut params: Vec<BasicMetadataTypeEnum<'ctx>> =
            Vec::with_capacity(site.params.len() + 1);
        params.push(ptrt.into());
        for p in site.params.iter() {
            params.push(repr_ty(ctx, &p.ty)?.into());
        }
        // 5b-9b: an effectful lambda takes its continuation last and answers
        // with a word, exactly as an effectful top-level function does.
        let fn_ty = if site.effectful {
            params.push(ptrt.into());
            ctx.i64_type().fn_type(&params, false)
        } else {
            fn_type_of(repr_ty(ctx, &site.ret)?, &params)?
        };
        let f = module.add_function(&mangle(&site.symbol), fn_ty, None);
        f.set_call_conventions(TAILCC);
        out.insert(site.symbol.clone(), f);
    }
    Ok(out)
}

/// Emit one lifted body. Captures are loaded ONCE, at entry, into the same `env`
/// the lowering fold already threads — so from that point a captured name is an
/// ordinary SSA binding and `gc_root_env` roots it exactly as it roots a
/// parameter. That is why the closure pointer itself needs no root inside the
/// body: nothing re-reads it, and a non-recursive lambda never self-calls.
fn emit_lifted<'ctx>(
    ctx: &'ctx Context,
    b: &Builder<'ctx>,
    lc: &LowerCtx<'_, 'ctx>,
    site: &LambdaSite,
) -> Result<(), CodegenError> {
    if site.effectful {
        return cps_emit::emit_cps_lifted(ctx, b, lc, site);
    }
    let func = *lc
        .lifted
        .get(&site.symbol)
        .ok_or(CodegenError::Unsupported("lambda body was never declared"))?;
    let entry = ctx.append_basic_block(func, "entry");
    b.position_at_end(entry);
    let i64t = ctx.i64_type();
    let ptrt = ctx.ptr_type(AddressSpace::default());
    let clos = func
        .get_nth_param(0)
        .ok_or_else(|| internal("lifted body has no environment parameter"))?
        .into_pointer_value();
    let mut env: HashMap<String, BasicValueEnum<'ctx>> = HashMap::new();
    for (i, (name, ty)) in site.captures.iter().enumerate() {
        let cs = unsafe { b.build_gep(i64t, clos, &[i64t.const_int((i + 2) as u64, false)], "cs") }
            .map_err(internal)?;
        let loaded = b
            .build_load(i64t, cs, "cv")
            .map_err(internal)?
            .into_int_value();
        let v = word_to_value(b, loaded, ty, ctx.bool_type(), ptrt)?;
        env.insert(name.clone(), v);
    }
    for (i, p) in site.params.iter().enumerate() {
        let v = func
            .get_nth_param((i + 1) as u32)
            .ok_or_else(|| internal("declared lambda arity disagrees with Core"))?;
        env.insert(p.name.clone(), v);
    }
    lower_tail(ctx, func, b, lc, &site.body, &mut env)
}

/// Pass 2 of §4.2. The value environment starts empty and is seeded from the
/// LLVM parameters, so each function gets its own — nothing leaks across a
/// call, which is what `distinct_envs` pins.
fn emit_body<'ctx>(
    ctx: &'ctx Context,
    b: &Builder<'ctx>,
    lc: &LowerCtx<'_, 'ctx>,
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

/// `<>`, `==` or `!=` on two lowered `String` operands (both emitters). The
/// caller roots what must survive: `<>` allocates (spec
/// 2026-10-15-elya-native-strings D4), `==` does not.
pub(crate) fn string_prim<'ctx>(
    ctx: &'ctx Context,
    b: &Builder<'ctx>,
    lc: &LowerCtx<'_, 'ctx>,
    op: BinOp,
    l: inkwell::values::PointerValue<'ctx>,
    r: inkwell::values::PointerValue<'ctx>,
) -> Result<BasicValueEnum<'ctx>, CodegenError> {
    let i64t = ctx.i64_type();
    match op {
        BinOp::Concat => Ok(b
            .build_call(
                lc.str_concat,
                &[
                    i64t.const_int(lc.string_tag as u64, false).into(),
                    l.into(),
                    r.into(),
                ],
                "cat",
            )
            .map_err(internal)?
            .try_as_basic_value()
            .left()
            .ok_or(CodegenError::Unsupported(
                "elya_str_concat returned no value",
            ))?),
        BinOp::Eq | BinOp::Ne => {
            let w = b
                .build_call(lc.str_eq, &[l.into(), r.into()], "streq")
                .map_err(internal)?
                .try_as_basic_value()
                .left()
                .ok_or(CodegenError::Unsupported("elya_str_eq returned no value"))?
                .into_int_value();
            let pred = if op == BinOp::Eq {
                IntPredicate::NE
            } else {
                IntPredicate::EQ
            };
            Ok(b.build_int_compare(pred, w, i64t.const_zero(), "streqb")
                .map_err(internal)?
                .into())
        }
        _ => Err(CodegenError::Unsupported(op_label(op))),
    }
}

/// `==`/`!=` on a declared type (spec 2026-10-16-elya-structural-equality,
/// D3): heap operands, compared by `elya_eq`'s walk. It allocates nothing.
pub(crate) fn is_data_eq_prim(op: BinOp, args: &[CoreExpr]) -> bool {
    matches!(op, BinOp::Eq | BinOp::Ne) && matches!(args[0].ty, Ty::Con(..))
}

/// `+. -. *. /.`, and `==`/`!=` on two floats (spec 2026-10-23-elya-float-natively
/// D1, D2): operands are words holding IEEE-754 bits.
pub(crate) fn is_float_prim(op: BinOp, args: &[CoreExpr]) -> bool {
    matches!(
        op,
        BinOp::AddF
            | BinOp::SubF
            | BinOp::MulF
            | BinOp::DivF
            | BinOp::LtF
            | BinOp::LeF
            | BinOp::GtF
            | BinOp::GeF
    ) || (matches!(op, BinOp::Eq | BinOp::Ne) && matches!(args[0].ty, Ty::Base(TyCon::Float)))
}

/// A float operation on two words: bitcast to `double`, IEEE arithmetic (no
/// failure: `1.0 /. 0.0` is `inf`, as in the evaluator), bitcast back; `==` is
/// `fcmp oeq` and `!=` is `fcmp une`, so a NaN is unequal to itself and `-0.0`
/// equals `0.0`.
pub(crate) fn float_prim<'ctx>(
    ctx: &'ctx Context,
    b: &Builder<'ctx>,
    op: BinOp,
    l: IntValue<'ctx>,
    r: IntValue<'ctx>,
) -> Result<BasicValueEnum<'ctx>, CodegenError> {
    let f64t = ctx.f64_type();
    let x = b
        .build_bit_cast(l, f64t, "fl")
        .map_err(internal)?
        .into_float_value();
    let y = b
        .build_bit_cast(r, f64t, "fr")
        .map_err(internal)?
        .into_float_value();
    let v = match op {
        BinOp::AddF => b.build_float_add(x, y, "fadd").map_err(internal)?,
        BinOp::SubF => b.build_float_sub(x, y, "fsub").map_err(internal)?,
        BinOp::MulF => b.build_float_mul(x, y, "fmul").map_err(internal)?,
        BinOp::DivF => b.build_float_div(x, y, "fdiv").map_err(internal)?,
        BinOp::Eq | BinOp::Ne | BinOp::LtF | BinOp::LeF | BinOp::GtF | BinOp::GeF => {
            // Ordered comparisons: false with a NaN (spec
            // 2026-10-24-elya-float-part-2 D1); `!=` is unordered-or-unequal.
            let pred = match op {
                BinOp::Eq => inkwell::FloatPredicate::OEQ,
                BinOp::Ne => inkwell::FloatPredicate::UNE,
                BinOp::LtF => inkwell::FloatPredicate::OLT,
                BinOp::LeF => inkwell::FloatPredicate::OLE,
                BinOp::GtF => inkwell::FloatPredicate::OGT,
                _ => inkwell::FloatPredicate::OGE,
            };
            return b
                .build_float_compare(pred, x, y, "fcmp")
                .map(Into::into)
                .map_err(internal);
        }
        other => return Err(CodegenError::Unsupported(op_label(other))),
    };
    b.build_bit_cast(v, ctx.i64_type(), "fw").map_err(internal)
}

/// `float.to_string` on a lowered word: a runtime call that allocates the
/// string (callers root what they need across it).
pub(crate) fn float_to_string<'ctx>(
    ctx: &'ctx Context,
    b: &Builder<'ctx>,
    lc: &LowerCtx<'_, 'ctx>,
    x: BasicValueEnum<'ctx>,
) -> Result<BasicValueEnum<'ctx>, CodegenError> {
    let i64t = ctx.i64_type();
    b.build_call(
        lc.float_to_string,
        &[
            i64t.const_int(lc.string_tag as u64, false).into(),
            x.into_int_value().into(),
        ],
        "ftos",
    )
    .map_err(internal)?
    .try_as_basic_value()
    .left()
    .ok_or(CodegenError::Unsupported(
        "elya_float_to_string returned no value",
    ))
}

/// `float.of_int`, `float.truncate`, `float.round` (spec
/// 2026-10-24-elya-float-part-2 D3, D4), both emitters: `sitofp` (round to
/// nearest, ties to even); the two to `Int` through `elya_float_to_int`, which
/// fails by name on NaN and outside `Int`'s range. Allocates nothing.
pub(crate) fn float_conversion<'ctx>(
    ctx: &'ctx Context,
    b: &Builder<'ctx>,
    lc: &LowerCtx<'_, 'ctx>,
    name: &str,
    x: IntValue<'ctx>,
) -> Result<BasicValueEnum<'ctx>, CodegenError> {
    let i64t = ctx.i64_type();
    match name {
        "float.of_int" => {
            let f = b
                .build_signed_int_to_float(x, ctx.f64_type(), "itof")
                .map_err(internal)?;
            b.build_bit_cast(f, i64t, "fw").map_err(internal)
        }
        "float.truncate" | "float.round" => {
            let n = name.strip_prefix("float.").unwrap_or(name);
            let mode = i64t.const_int(u64::from(n == "round"), false);
            b.build_call(
                lc.float_to_int,
                &[c_string(ctx, lc, n).into(), x.into(), mode.into()],
                "ftoi",
            )
            .map_err(internal)?
            .try_as_basic_value()
            .left()
            .ok_or(CodegenError::Unsupported(
                "elya_float_to_int returned no value",
            ))
        }
        _ => Err(CodegenError::UnsupportedBuiltin(name.to_string())),
    }
}

/// `==`/`!=` on `Unit`: both operands are evaluated, the answer is constant.
pub(crate) fn is_unit_eq_prim(op: BinOp, args: &[CoreExpr]) -> bool {
    matches!(op, BinOp::Eq | BinOp::Ne) && matches!(args[0].ty, Ty::Base(TyCon::Unit))
}

pub(crate) fn unit_eq<'ctx>(ctx: &'ctx Context, op: BinOp) -> BasicValueEnum<'ctx> {
    ctx.bool_type()
        .const_int(u64::from(op == BinOp::Eq), false)
        .into()
}

pub(crate) fn data_eq<'ctx>(
    ctx: &'ctx Context,
    b: &Builder<'ctx>,
    lc: &LowerCtx<'_, 'ctx>,
    op: BinOp,
    l: inkwell::values::PointerValue<'ctx>,
    r: inkwell::values::PointerValue<'ctx>,
) -> Result<BasicValueEnum<'ctx>, CodegenError> {
    let i64t = ctx.i64_type();
    let w = b
        .build_call(
            lc.data_eq,
            &[
                l.into(),
                r.into(),
                i64t.const_int(lc.string_tag as u64, false).into(),
                i64t.const_int(lc.first_closure_tag as u64, false).into(),
            ],
            "dataeq",
        )
        .map_err(internal)?
        .try_as_basic_value()
        .left()
        .ok_or(CodegenError::Unsupported("elya_eq returned no value"))?
        .into_int_value();
    let pred = if op == BinOp::Eq {
        IntPredicate::NE
    } else {
        IntPredicate::EQ
    };
    Ok(b.build_int_compare(pred, w, i64t.const_zero(), "dataeqb")
        .map_err(internal)?
        .into())
}

/// `int.to_string(n)`: a fresh string. Allocates: the caller roots.
pub(crate) fn int_to_string<'ctx>(
    ctx: &'ctx Context,
    b: &Builder<'ctx>,
    lc: &LowerCtx<'_, 'ctx>,
    n: BasicValueEnum<'ctx>,
) -> Result<BasicValueEnum<'ctx>, CodegenError> {
    let i64t = ctx.i64_type();
    b.build_call(
        lc.int_to_string,
        &[
            i64t.const_int(lc.string_tag as u64, false).into(),
            n.into_int_value().into(),
        ],
        "itos",
    )
    .map_err(internal)?
    .try_as_basic_value()
    .left()
    .ok_or(CodegenError::Unsupported(
        "elya_int_to_string returned no value",
    ))
}

/// `time.now_ms()` / `time.sleep_ms(ms)` on lowered operands (both emitters).
/// Neither allocates, so nothing needs rooting. `Unit` is the word 0.
pub(crate) fn time_builtin<'ctx>(
    ctx: &'ctx Context,
    b: &Builder<'ctx>,
    lc: &LowerCtx<'_, 'ctx>,
    name: &str,
    vals: &[BasicValueEnum<'ctx>],
) -> Result<BasicValueEnum<'ctx>, CodegenError> {
    match (name, vals) {
        ("time.now_ms", []) => b
            .build_call(lc.time_now_ms, &[], "now")
            .map_err(internal)?
            .try_as_basic_value()
            .left()
            .ok_or(CodegenError::Unsupported(
                "elya_time_now_ms returned no value",
            )),
        ("time.sleep_ms", [ms]) => {
            b.build_call(lc.time_sleep_ms, &[ms.into_int_value().into()], "")
                .map_err(internal)?;
            Ok(ctx.i64_type().const_int(0, false).into())
        }
        _ => Err(CodegenError::UnsupportedBuiltin(name.to_string())),
    }
}

/// The builtins the native back end implements. `any.*` are the `with any`
/// expansion's (spec 2026-10-17), never written in source.
pub(crate) fn is_native_builtin(name: &str) -> bool {
    matches!(
        name,
        "io.println"
            | "int.to_string"
            | "float.to_string"
            | "float.of_int"
            | "float.truncate"
            | "float.round"
            | "time.now_ms"
            | "time.sleep_ms"
            | "any.show"
            | "any.decode"
            | "any.fail"
            | "any.check"
    ) || name
        .strip_prefix("int.")
        .is_some_and(|n| elya::eval::INT_BINARY_BUILTINS.contains(&n))
}

/// `int.<name>(a, b)` (spec 2026-10-17-elya-unary-and-wrapping D3), both
/// emitters: wrapping arithmetic is LLVM's plain `add`/`sub`/`mul`, bits are
/// `and`/`or`/`xor`, and a shift's amount is checked against `0..=63` first,
/// trapping through `elya_shift_range` with the evaluator's words. Allocates
/// nothing.
pub(crate) fn int_binary<'ctx>(
    ctx: &'ctx Context,
    func: FunctionValue<'ctx>,
    b: &Builder<'ctx>,
    lc: &LowerCtx<'_, 'ctx>,
    name: &str,
    x: IntValue<'ctx>,
    y: IntValue<'ctx>,
) -> Result<BasicValueEnum<'ctx>, CodegenError> {
    let n = name.strip_prefix("int.").unwrap_or(name);
    if matches!(n, "shl" | "shr" | "ushr") {
        let i64t = ctx.i64_type();
        let bad = b
            .build_int_compare(IntPredicate::UGT, y, i64t.const_int(63, false), "shbad")
            .map_err(internal)?;
        let trap = ctx.append_basic_block(func, "shift_range");
        let ok = ctx.append_basic_block(func, "shift_ok");
        b.build_conditional_branch(bad, trap, ok)
            .map_err(internal)?;
        b.position_at_end(trap);
        b.build_call(lc.shift_range, &[c_string(ctx, lc, n).into(), y.into()], "")
            .map_err(internal)?;
        b.build_unreachable().map_err(internal)?;
        b.position_at_end(ok);
    }
    let v = match n {
        "wrapping_add" => b.build_int_add(x, y, "wadd"),
        "wrapping_sub" => b.build_int_sub(x, y, "wsub"),
        "wrapping_mul" => b.build_int_mul(x, y, "wmul"),
        "and" => b.build_and(x, y, "band"),
        "or" => b.build_or(x, y, "bor"),
        "xor" => b.build_xor(x, y, "bxor"),
        "shl" => b.build_left_shift(x, y, "shl"),
        "shr" => b.build_right_shift(x, y, true, "ashr"),
        "ushr" => b.build_right_shift(x, y, false, "lshr"),
        _ => return Err(CodegenError::UnsupportedBuiltin(name.to_string())),
    }
    .map_err(internal)?;
    Ok(v.into())
}

/// A NUL-terminated private constant, for the runtime's messages.
fn c_string<'ctx>(ctx: &'ctx Context, lc: &LowerCtx<'_, 'ctx>, s: &str) -> PointerValue<'ctx> {
    let i8t = ctx.i8_type();
    let bytes: Vec<_> = s
        .bytes()
        .chain(std::iter::once(0))
        .map(|c| i8t.const_int(c as u64, false))
        .collect();
    let arr = i8t.const_array(&bytes);
    let g = lc
        .module
        .add_global(arr.get_type(), Some(AddressSpace::default()), "anymsg");
    g.set_initializer(&arr);
    g.set_constant(true);
    g.set_linkage(Linkage::Private);
    g.set_unnamed_addr(true);
    g.as_pointer_value()
}

/// The `n`-th argument of a builtin, a string literal the expansion wrote.
fn lit_arg(e: &CoreExpr, n: usize) -> Result<&str, CodegenError> {
    match &e.kind {
        CoreKind::Builtin(_, a) => match a.get(n).map(|x| &x.kind) {
            Some(CoreKind::Lit(CoreLit::Str(s))) => Ok(s),
            _ => Err(CodegenError::Unsupported(
                "`any` builtin without its literal",
            )),
        },
        _ => Err(CodegenError::Unsupported(
            "`any` builtin without its literal",
        )),
    }
}

/// `any.show(v, before, after)`, `any.decode(s, display, op)`, `any.check(v)`
/// and `any.fail(s, message)` (spec 2026-10-17 D3/D4) on the lowered first
/// operand `v` (both emitters; only the first operand is lowered, the rest are
/// the expansion's literals). Allocates: the caller roots what it still needs,
/// `v` included.
pub(crate) fn any_builtin<'ctx>(
    ctx: &'ctx Context,
    b: &Builder<'ctx>,
    lc: &LowerCtx<'_, 'ctx>,
    e: &CoreExpr,
    v: BasicValueEnum<'ctx>,
) -> Result<BasicValueEnum<'ctx>, CodegenError> {
    let i64t = ctx.i64_type();
    let CoreKind::Builtin(name, args) = &e.kind else {
        return Err(internal("any_builtin on a non-builtin"));
    };
    let tag = i64t.const_int(lc.string_tag as u64, false);
    match name.as_str() {
        "any.show" => {
            let target = &args[0].ty;
            let node = lc
                .reflect
                .node_of(target)
                .ok_or_else(|| internal("any.show at a type the table lacks"))?;
            let w = value_to_word(b, v, target, i64t)?;
            b.build_call(
                lc.any_show,
                &[
                    tag.into(),
                    lc.any_table.into(),
                    lc.any_names.into(),
                    i64t.const_int(node as u64, false).into(),
                    w.into(),
                    c_string(ctx, lc, lit_arg(e, 1)?).into(),
                    c_string(ctx, lc, lit_arg(e, 2)?).into(),
                ],
                "anyshow",
            )
            .map_err(internal)?
            .try_as_basic_value()
            .left()
            .ok_or(CodegenError::Unsupported("elya_any_show returned no value"))
        }
        "any.decode" => {
            let node = lc
                .reflect
                .node_of(&e.ty)
                .ok_or_else(|| internal("any.decode at a type the table lacks"))?;
            let w = b
                .build_call(
                    lc.any_decode,
                    &[
                        tag.into(),
                        lc.any_table.into(),
                        lc.any_names.into(),
                        i64t.const_int(node as u64, false).into(),
                        v.into(),
                        c_string(ctx, lc, lit_arg(e, 1)?).into(),
                        c_string(ctx, lc, lit_arg(e, 2)?).into(),
                    ],
                    "anydecode",
                )
                .map_err(internal)?
                .try_as_basic_value()
                .left()
                .ok_or(CodegenError::Unsupported(
                    "elya_any_decode returned no value",
                ))?
                .into_int_value();
            word_to_value(
                b,
                w,
                &e.ty,
                ctx.bool_type(),
                ctx.ptr_type(AddressSpace::default()),
            )
        }
        "any.check" => {
            let target = &args[0].ty;
            let node = lc
                .reflect
                .node_of(target)
                .ok_or_else(|| internal("any.check at a type the table lacks"))?;
            let w = value_to_word(b, v, target, i64t)?;
            let r = b
                .build_call(
                    lc.any_check,
                    &[
                        lc.any_table.into(),
                        i64t.const_int(node as u64, false).into(),
                        w.into(),
                    ],
                    "anycheck",
                )
                .map_err(internal)?
                .try_as_basic_value()
                .left()
                .ok_or(CodegenError::Unsupported(
                    "elya_any_check returned no value",
                ))?
                .into_int_value();
            word_to_value(
                b,
                r,
                &Ty::Base(TyCon::Bool),
                ctx.bool_type(),
                ctx.ptr_type(AddressSpace::default()),
            )
        }
        "any.fail" => {
            b.build_call(lc.any_fail, &[c_string(ctx, lc, lit_arg(e, 1)?).into()], "")
                .map_err(internal)?;
            // The runtime exits; a value of the node's type stands in.
            Ok(repr_ty(ctx, &e.ty)?.const_zero())
        }
        _ => Err(CodegenError::UnsupportedBuiltin(name.clone())),
    }
}

/// Is this `Prim` a string operation (`<>`, or `==`/`!=` on `String`s)?
pub(crate) fn is_string_prim(op: BinOp, args: &[CoreExpr]) -> bool {
    match op {
        BinOp::Concat => true,
        BinOp::Eq | BinOp::Ne => matches!(args[0].ty, Ty::Base(TyCon::Str)),
        _ => false,
    }
}

/// One place where an Elya call becomes an LLVM call, used from both `lower_expr`
/// (ordinary position) and, in Task 4, `lower_tail` (tail position). The only
/// difference between the two is the tail-call kind the caller sets afterwards.
///
/// Push `v` onto the shadow stack if — and only if — it is a heap reference,
/// reporting whether it was.
///
/// `repr_ty` gives every ADT a pointer and every scalar an integer, so
/// `is_pointer_value` IS the root predicate: there is nothing to guess. That
/// precision is the whole reason §3 chose a shadow stack over a conservative
/// scan of the machine stack, which could read an `Int` as an address and make
/// native MORE undefined than the evaluator — which 5b-1 §3.4 forbids.
fn gc_root<'ctx>(
    b: &Builder<'ctx>,
    lc: &LowerCtx<'_, 'ctx>,
    v: BasicValueEnum<'ctx>,
) -> Result<bool, CodegenError> {
    if !v.is_pointer_value() {
        return Ok(false);
    }
    b.build_call(lc.gc_push, &[v.into()], "")
        .map_err(internal)?;
    Ok(true)
}

/// Convert values at the shared closure-capture/ADT-field heap-word boundary.
/// Unit already has the i64 word representation, so its conversion is identity.
fn value_to_word<'ctx>(
    b: &Builder<'ctx>,
    v: BasicValueEnum<'ctx>,
    ty: &Ty,
    i64t: IntType<'ctx>,
) -> Result<IntValue<'ctx>, CodegenError> {
    match ty {
        Ty::Base(TyCon::Int) => Ok(v.into_int_value()),
        Ty::Base(TyCon::Unit) => Ok(v.into_int_value()),
        Ty::Base(TyCon::Float) => Ok(v.into_int_value()),
        Ty::Base(TyCon::Bool) => b
            .build_int_z_extend(v.into_int_value(), i64t, "zw")
            .map_err(internal),
        _ => b
            .build_ptr_to_int(v.into_pointer_value(), i64t, "p2i")
            .map_err(internal),
    }
}

/// Restore a typed value from that same word boundary; Unit remains unchanged.
fn word_to_value<'ctx>(
    b: &Builder<'ctx>,
    w: IntValue<'ctx>,
    ty: &Ty,
    boolt: IntType<'ctx>,
    ptrt: PointerType<'ctx>,
) -> Result<BasicValueEnum<'ctx>, CodegenError> {
    match ty {
        Ty::Base(TyCon::Int) => Ok(w.into()),
        Ty::Base(TyCon::Unit) => Ok(w.into()),
        Ty::Base(TyCon::Float) => Ok(w.into()),
        Ty::Base(TyCon::Bool) => b
            .build_int_truncate(w, boolt, "bt")
            .map(Into::into)
            .map_err(internal),
        _ => b
            .build_int_to_ptr(w, ptrt, "i2p")
            .map(Into::into)
            .map_err(internal),
    }
}

/// The prefix of the hidden keys `bind_local` parks shadowed values under. No
/// source identifier can begin with `$`, so `Var` lookup never sees one.
const SHADOW: &str = "$shadow:";

/// What `unbind_local` needs to undo a `bind_local`.
struct Shadowed(Option<String>);

/// Bind `x` to `v`. If that shadows an existing binding, the outer value is
/// NOT moved into a Rust local (where `gc_root_env` cannot see it -- a live
/// heap value left unrooted while the inner scope allocates): it stays in
/// `env` under a hidden key, so every allocation inside the inner scope roots
/// it, and `unbind_local` restores it exactly. Bindings nest LIFO, so the
/// count of hidden keys present is a unique, deterministic suffix.
fn bind_local<'ctx>(
    env: &mut HashMap<String, BasicValueEnum<'ctx>>,
    x: &str,
    v: BasicValueEnum<'ctx>,
) -> Shadowed {
    match env.insert(x.to_string(), v) {
        None => Shadowed(None),
        Some(prev) => {
            let n = env.keys().filter(|k| k.starts_with(SHADOW)).count();
            let key = format!("{SHADOW}{n:06}:{x}");
            env.insert(key.clone(), prev);
            Shadowed(Some(key))
        }
    }
}

fn unbind_local<'ctx>(env: &mut HashMap<String, BasicValueEnum<'ctx>>, x: &str, s: Shadowed) {
    match s.0 {
        None => {
            env.remove(x);
        }
        Some(key) => {
            if let Some(prev) = env.remove(&key) {
                env.insert(x.to_string(), prev);
            }
        }
    }
}

/// Root every heap binding currently in scope, returning how many went on.
///
/// The keys are SORTED first. `HashMap` iteration order varies between
/// processes, so without this the same input would emit different IR run to
/// run — reproducibility is nearly free here and expensive to retrofit.
fn gc_root_env<'ctx>(
    b: &Builder<'ctx>,
    lc: &LowerCtx<'_, 'ctx>,
    env: &HashMap<String, BasicValueEnum<'ctx>>,
) -> Result<usize, CodegenError> {
    let mut names: Vec<&String> = env.keys().collect();
    names.sort();
    let mut n = 0usize;
    for name in names {
        if gc_root(b, lc, env[name])? {
            n += 1;
        }
    }
    Ok(n)
}

/// Pop `n` roots. Every push above is matched by exactly one of these on every
/// path out. An imbalance never fails loudly: it either retains garbage for the
/// life of the process or — the direction that matters — un-roots a value that
/// is still live, which is a silent wrong answer.
fn gc_unroot<'ctx>(
    b: &Builder<'ctx>,
    lc: &LowerCtx<'_, 'ctx>,
    n: usize,
) -> Result<(), CodegenError> {
    for _ in 0..n {
        b.build_call(lc.gc_pop, &[], "").map_err(internal)?;
    }
    Ok(())
}

/// Dispatch one Elya call. A callee that names a LOCAL holds a closure pointer,
/// so the call is indirect; a callee that names a top-level function is direct.
/// `env` is consulted before `decls`, the same order `lower_expr`'s head guard
/// uses, so a local shadowing a top-level name resolves to the local.
///
/// `tail` is not a hint — it selects the root discipline, and both bodies below
/// explain why the two cases cannot be unified.
#[allow(clippy::too_many_arguments)]
fn build_elya_call<'ctx>(
    ctx: &'ctx Context,
    func: FunctionValue<'ctx>,
    b: &Builder<'ctx>,
    lc: &LowerCtx<'_, 'ctx>,
    callee: &CoreExpr,
    args: &[CoreExpr],
    env: &mut HashMap<String, BasicValueEnum<'ctx>>,
    tail: bool,
) -> Result<CallSiteValue<'ctx>, CodegenError> {
    // §9: dispatch is by BINDING, not by type. A callee that is not a name at
    // all — an immediately-applied lambda, say — is still 5b-3 §5.3's computed
    // callee, and refusing it here keeps that cut exactly where N2 drew it.
    // Reading `Ty::Fn` off any callee whatsoever would have moved it silently.
    let CoreKind::Var(name) = &callee.kind else {
        return Err(CodegenError::Unsupported("computed callee"));
    };
    // `env` before `decls`, the same order `lower_expr`'s head guard uses, so a
    // local shadowing a top-level name resolves to the local.
    if env.contains_key(name) {
        return build_closure_call(ctx, func, b, lc, callee, args, env, tail);
    }
    let target = *lc.decls.get(name).ok_or(CodegenError::Unsupported(
        "callee is not a top-level function",
    ))?;
    // D16: an effectful callee takes a continuation; only the CPS emitter can
    // supply one. Reaching here with one is an emitter bug, refused by name.
    if lc.cps_fns.contains(name) {
        return Err(CodegenError::Unsupported(
            "effectful call outside an effectful region",
        ));
    }
    build_direct_call(ctx, func, b, lc, target, args, env, tail)
}

/// The N2 path, unchanged: a statically known `tailcc` callee, called by name.
#[allow(clippy::too_many_arguments)]
fn build_direct_call<'ctx>(
    ctx: &'ctx Context,
    func: FunctionValue<'ctx>,
    b: &Builder<'ctx>,
    lc: &LowerCtx<'_, 'ctx>,
    target: FunctionValue<'ctx>,
    args: &[CoreExpr],
    env: &mut HashMap<String, BasicValueEnum<'ctx>>,
    tail: bool,
) -> Result<CallSiteValue<'ctx>, CodegenError> {
    // Roots, in two layers, and the layering is the whole subtlety.
    //
    // The caller's own bindings are rooted ONLY for a non-tail call. A tail
    // call's frame is dead the instant the call happens — the callee roots its
    // own parameters at its own allocation sites — so rooting them here would
    // be pure cost. It would also be fatal twice over: `musttail` requires the
    // call to be immediately followed by `ret`, leaving nowhere to pop, and a
    // million tail iterations would then grow the shadow stack without bound.
    // That would trade N2's constant-stack guarantee away to fix a GC bug.
    let env_roots = if tail { 0 } else { gc_root_env(b, lc, env)? };
    let mut vals: Vec<BasicMetadataValueEnum<'ctx>> = Vec::with_capacity(args.len());
    let mut arg_roots = 0usize;
    for a in args.iter() {
        // Left to right, matching the evaluator's argument order. Each argument
        // is rooted AS IT IS LOWERED rather than batched afterwards: in
        // `f(g(), h())` the value of `g()` is a temp nothing names, and
        // lowering `h()` can allocate.
        let v = lower_expr(ctx, func, b, lc, a, env)?;
        if gc_root(b, lc, v)? {
            arg_roots += 1;
        }
        vals.push(v.into());
    }
    // Unroot the arguments BEFORE the call, LIFO — they went on last. Handing
    // them over unrooted is safe because the callee roots its parameters at its
    // own first allocation site, and nothing allocates in between.
    gc_unroot(b, lc, arg_roots)?;
    let site = b.build_call(target, &vals, "c").map_err(internal)?;
    // Singular here (CallSiteValue), plural on the declaration (FunctionValue).
    site.set_call_convention(TAILCC);
    // The caller's frame outlives a non-tail call, so its roots come off only
    // now. For a tail call `env_roots` is zero and this emits nothing, which is
    // what leaves the `musttail` call adjacent to its `ret`.
    gc_unroot(b, lc, env_roots)?;
    Ok(site)
}

/// Call through a closure. The signature is read WHOLE off the callee's own
/// recorded `Ty::Fn` — not reconstructed from surrounding context, which is the
/// reconstruction 5b-6 §4 rejected. Parameter 0 is the closure pointer itself.
///
/// Reached only for a `Var` bound in `env`, so the `Ty::Fn` guard below is the
/// second half of §9's predicate, not the whole of it: it catches a local of
/// non-function type in callee position. The syntactic half stays in the
/// dispatcher, which is what keeps 5b-3 §5.3's cut where N2 drew it.
#[allow(clippy::too_many_arguments)]
fn build_closure_call<'ctx>(
    ctx: &'ctx Context,
    func: FunctionValue<'ctx>,
    b: &Builder<'ctx>,
    lc: &LowerCtx<'_, 'ctx>,
    callee: &CoreExpr,
    args: &[CoreExpr],
    env: &mut HashMap<String, BasicValueEnum<'ctx>>,
    tail: bool,
) -> Result<CallSiteValue<'ctx>, CodegenError> {
    let i64t = ctx.i64_type();
    let ptrt = ctx.ptr_type(AddressSpace::default());
    let Ty::Fn(param_tys, _, ret_ty) = &callee.ty else {
        return Err(CodegenError::Unsupported("computed callee"));
    };
    if cps::needs_cps(&callee.ty) {
        return Err(CodegenError::Unsupported(
            "effectful closure call (not yet compiled natively)",
        ));
    }
    if param_tys.len() != args.len() {
        return Err(CodegenError::Unsupported("closure call arity mismatch"));
    }
    let mut sig: Vec<BasicMetadataTypeEnum<'ctx>> = Vec::with_capacity(args.len() + 1);
    sig.push(ptrt.into());
    for t in param_tys.iter() {
        sig.push(repr_ty(ctx, t)?.into());
    }
    let fn_ty = fn_type_of(repr_ty(ctx, ret_ty)?, &sig)?;

    let env_roots = if tail { 0 } else { gc_root_env(b, lc, env)? };
    let clos = lower_expr(ctx, func, b, lc, callee, env)?.into_pointer_value();
    // The closure is rooted UNCONDITIONALLY — including at a tail call, where
    // `env_roots` is 0 by design. It is about to become argument 0, and lowering
    // an argument can allocate, so this is the one root a tail call still needs.
    b.build_call(lc.gc_push, &[clos.into()], "")
        .map_err(internal)?;
    let mut vals: Vec<BasicMetadataValueEnum<'ctx>> = Vec::with_capacity(args.len() + 1);
    vals.push(clos.into());
    let mut arg_roots = 0usize;
    for a in args.iter() {
        let v = lower_expr(ctx, func, b, lc, a, env)?;
        if gc_root(b, lc, v)? {
            arg_roots += 1;
        }
        vals.push(v.into());
    }
    // Word 1 is the code pointer. Loading is not an allocation, so it is safe
    // after the arguments and before the unroot.
    let slot =
        unsafe { b.build_gep(i64t, clos, &[i64t.const_int(1, false)], "cp") }.map_err(internal)?;
    let code = b
        .build_load(i64t, slot, "cw")
        .map_err(internal)?
        .into_int_value();
    let fp = b.build_int_to_ptr(code, ptrt, "i2f").map_err(internal)?;
    gc_unroot(b, lc, arg_roots + 1)?;
    let site = b
        .build_indirect_call(fn_ty, fp, &vals, "ci")
        .map_err(internal)?;
    site.set_call_convention(TAILCC);
    gc_unroot(b, lc, env_roots)?;
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
    lc: &LowerCtx<'_, 'ctx>,
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
            // A shadowed outer binding stays rooted (`bind_local`).
            let shadowed = bind_local(env, x, v);
            let out = lower_tail(ctx, func, b, lc, body, env);
            unbind_local(env, x, shadowed);
            out
        }
        // A call in a match arm is in tail position too (HANDOFF step 1).
        CoreKind::Match(scrutinee, arms) => {
            lower_match(ctx, func, b, lc, scrutinee, arms, None, env)?;
            Ok(())
        }
        CoreKind::App(callee, args) => {
            let site = build_elya_call(ctx, func, b, lc, callee, args, env, true)?;
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

/// A `match`, by tag tests, field loads and a named trap on no match (N6 §8.4).
/// `phi_ty: Some(t)` lowers it as a VALUE: the arms join in a phi of type `t`.
/// `None` lowers it in TAIL position (HANDOFF step 1, 2026-10-05): each arm
/// ends itself through `lower_tail` -- a `musttail` call or a `ret` -- with no
/// join block, exactly as a tail `if`, and the result is `None`. Before, a
/// match in tail position was lowered as a value and a call in an arm was an
/// ordinary call, so a loop through a match grew the machine stack.
#[allow(clippy::too_many_arguments)]
fn lower_match<'ctx>(
    ctx: &'ctx Context,
    func: FunctionValue<'ctx>,
    b: &Builder<'ctx>,
    lc: &LowerCtx<'_, 'ctx>,
    scrutinee: &CoreExpr,
    arms: &[elya::core::CoreArm],
    phi_ty: Option<BasicTypeEnum<'ctx>>,
    env: &mut HashMap<String, BasicValueEnum<'ctx>>,
) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
    // N6 §8.4: a scrutinee must be an ADT (a pointer with a real tag word). A
    // non-Con scrutinee panics `.into_pointer_value()` today; a Str scrutinee
    // would load its tag and fall through to elya_match_fail. Refuse by name.
    if !matches!(scrutinee.ty, Ty::Con(..)) {
        return Err(CodegenError::Unsupported("match scrutinee is not an ADT"));
    }
    let i64t = ctx.i64_type();
    let ptrt = ctx.ptr_type(AddressSpace::default());
    let s = lower_expr(ctx, func, b, lc, scrutinee, env)?.into_pointer_value();
    let tag = b
        .build_load(i64t, s, "tag")
        .map_err(internal)?
        .into_int_value();
    // Value mode joins the arms in a phi; tail mode has NO join block --
    // each arm ends itself (a `musttail` call or a `ret`), as a tail `if`.
    let join_bb = phi_ty.map(|_| ctx.append_basic_block(func, "mjoin"));
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
                        b.build_gep(i64t, s, &[i64t.const_int((pi + 1) as u64, false)], "fp")
                    }
                    .map_err(internal)?;
                    let loaded = b
                        .build_load(i64t, fp, "fld")
                        .map_err(internal)?
                        .into_int_value();
                    let field_val =
                        word_to_value(b, loaded, &field_tys[pi], ctx.bool_type(), ptrt)?;
                    match p {
                        CorePat::Var(v) => bindings.push((v.clone(), field_val)),
                        CorePat::Wild => {}
                        CorePat::Ctor(..) => {
                            return Err(CodegenError::Unsupported("nested constructor pattern"))
                        }
                        CorePat::Lit(_) => {
                            return Err(CodegenError::Unsupported("literal pattern"))
                        }
                    }
                }
                // Bound with `bind_local` and undone in reverse: an arm
                // binder that shadows an outer name used to REMOVE the
                // outer binding after the arm (and leave it unrooted
                // during it).
                let mut shadowed = Vec::with_capacity(bindings.len());
                for (n, v) in &bindings {
                    shadowed.push((n.clone(), bind_local(env, n, *v)));
                }
                let v = arm_body(ctx, func, b, lc, &arm.body, env, join_bb);
                for (n, sh) in shadowed.into_iter().rev() {
                    unbind_local(env, &n, sh);
                }
                if let Some(v) = v? {
                    incoming.push(v);
                }
            }
            CorePat::Wild => {
                b.position_at_end(fallthrough);
                b.build_unconditional_branch(body_bb).map_err(internal)?;
                b.position_at_end(body_bb);
                if let Some(v) = arm_body(ctx, func, b, lc, &arm.body, env, join_bb)? {
                    incoming.push(v);
                }
                terminal = true;
            }
            CorePat::Var(name) => {
                b.position_at_end(fallthrough);
                b.build_unconditional_branch(body_bb).map_err(internal)?;
                b.position_at_end(body_bb);
                let shadowed = bind_local(env, name, s.into());
                let v = arm_body(ctx, func, b, lc, &arm.body, env, join_bb);
                unbind_local(env, name, shadowed);
                if let Some(v) = v? {
                    incoming.push(v);
                }
                terminal = true;
            }
            CorePat::Lit(_) => return Err(CodegenError::Unsupported("literal pattern")),
        }
        // Arms after a catch-all can never match (an E0431 warning, not
        // an error); emitting them branched out of an already-terminated
        // block and LLVM rejected the module (5b-9a review). The CPS twin,
        // `match_dispatch`, stops here too.
        if terminal {
            break;
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
    let (Some(join_bb), Some(node_ty)) = (join_bb, phi_ty) else {
        return Ok(None);
    };
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
    Ok(Some(phi.as_basic_value()))
}

/// The phi's incoming pair: a value and the block it arrives from.
type Incoming<'ctx> = (BasicValueEnum<'ctx>, inkwell::basic_block::BasicBlock<'ctx>);

/// One arm's body. Value mode (`join: Some`): lowered as a value and branched
/// to the join, returning the phi's incoming pair. Tail mode (`join: None`):
/// lowered by `lower_tail`, which terminates the block itself.
fn arm_body<'ctx>(
    ctx: &'ctx Context,
    func: FunctionValue<'ctx>,
    b: &Builder<'ctx>,
    lc: &LowerCtx<'_, 'ctx>,
    body: &CoreExpr,
    env: &mut HashMap<String, BasicValueEnum<'ctx>>,
    join: Option<inkwell::basic_block::BasicBlock<'ctx>>,
) -> Result<Option<Incoming<'ctx>>, CodegenError> {
    let Some(join) = join else {
        lower_tail(ctx, func, b, lc, body, env)?;
        return Ok(None);
    };
    let v = lower_expr(ctx, func, b, lc, body, env)?;
    let exit = b
        .get_insert_block()
        .ok_or_else(|| internal("builder left no block"))?;
    b.build_unconditional_branch(join).map_err(internal)?;
    Ok(Some((v, exit)))
}

/// The binary `Prim` operators on two already-lowered operands. Extracted from
/// `lower_expr` (5b-8 7b-3) so the CPS emitter, which meets operands whose
/// values were computed before a continuation site, applies exactly the same
/// operators -- one table, not two that could drift.
fn prim_values<'ctx>(
    ctx: &'ctx Context,
    func: FunctionValue<'ctx>,
    b: &Builder<'ctx>,
    lc: &LowerCtx<'_, 'ctx>,
    op: BinOp,
    l: IntValue<'ctx>,
    r: IntValue<'ctx>,
) -> Result<BasicValueEnum<'ctx>, CodegenError> {
    // `Int` arithmetic is exact or fails by name (spec
    // 2026-10-10-elya-checked-integer-arithmetic, D3), as in the evaluator:
    // `+ - *` through LLVM's overflow intrinsics, `/` and `%` guarded so that
    // `sdiv`/`srem` never see an input where they are undefined.
    match op {
        BinOp::Add => return checked(ctx, func, b, lc, "llvm.sadd.with.overflow", l, r),
        BinOp::Sub => return checked(ctx, func, b, lc, "llvm.ssub.with.overflow", l, r),
        BinOp::Mul => return checked(ctx, func, b, lc, "llvm.smul.with.overflow", l, r),
        BinOp::Div => {
            let i64t = ctx.i64_type();
            let zero = b
                .build_int_compare(IntPredicate::EQ, r, i64t.const_zero(), "dz")
                .map_err(internal)?;
            trap_if(ctx, func, b, zero, lc.div_zero, "div_zero")?;
            let min = b
                .build_int_compare(
                    IntPredicate::EQ,
                    l,
                    i64t.const_int(i64::MIN as u64, true),
                    "dmin",
                )
                .map_err(internal)?;
            let neg1 = b
                .build_int_compare(IntPredicate::EQ, r, i64t.const_all_ones(), "dneg1")
                .map_err(internal)?;
            let ovf = b.build_and(min, neg1, "dovf").map_err(internal)?;
            trap_if(ctx, func, b, ovf, lc.int_overflow, "div_overflow")?;
            return b
                .build_int_signed_div(l, r, "div")
                .map(|v| v.into())
                .map_err(internal);
        }
        BinOp::Rem => {
            let i64t = ctx.i64_type();
            let zero = b
                .build_int_compare(IntPredicate::EQ, r, i64t.const_zero(), "rz")
                .map_err(internal)?;
            trap_if(ctx, func, b, zero, lc.rem_zero, "rem_zero")?;
            // `x % -1` is 0 for every x, and `srem MIN, -1` is undefined: divide
            // by 1 instead, which gives the same 0 without the undefined input.
            let neg1 = b
                .build_int_compare(IntPredicate::EQ, r, i64t.const_all_ones(), "rneg1")
                .map_err(internal)?;
            let d = b
                .build_select(neg1, i64t.const_int(1, false), r, "rdiv")
                .map_err(internal)?
                .into_int_value();
            return b
                .build_int_signed_rem(l, d, "rem")
                .map(|v| v.into())
                .map_err(internal);
        }
        _ => {}
    }
    // Comparisons are SIGNED: Elya's Int is i64 two's-complement, so
    // `(0 - 1) < 1` must be true. `and`/`or` are strict and bit-wise on
    // i1 because BOTH evaluators are strict (spec §4.1) — a
    // short-circuit diamond here would make native less-undefined than
    // `elya run` (a guarded `/` on the right of `&&` would be skipped).
    let built = match op {
        BinOp::Lt => b.build_int_compare(IntPredicate::SLT, l, r, "lt"),
        BinOp::Le => b.build_int_compare(IntPredicate::SLE, l, r, "le"),
        BinOp::Gt => b.build_int_compare(IntPredicate::SGT, l, r, "gt"),
        BinOp::Ge => b.build_int_compare(IntPredicate::SGE, l, r, "ge"),
        BinOp::Eq => b.build_int_compare(IntPredicate::EQ, l, r, "eq"),
        BinOp::Ne => b.build_int_compare(IntPredicate::NE, l, r, "ne"),
        BinOp::And => b.build_and(l, r, "and"),
        BinOp::Or => b.build_or(l, r, "or"),
        other => return Err(CodegenError::Unsupported(op_label(other))),
    };
    built.map(|v| v.into()).map_err(internal)
}

/// One of LLVM's `*.with.overflow.i64` intrinsics: the result, and a branch
/// to `elya_int_overflow` when its overflow bit is set.
#[allow(clippy::too_many_arguments)]
fn checked<'ctx>(
    ctx: &'ctx Context,
    func: FunctionValue<'ctx>,
    b: &Builder<'ctx>,
    lc: &LowerCtx<'_, 'ctx>,
    intrinsic: &str,
    l: IntValue<'ctx>,
    r: IntValue<'ctx>,
) -> Result<BasicValueEnum<'ctx>, CodegenError> {
    let i64t = ctx.i64_type();
    let decl = inkwell::intrinsics::Intrinsic::find(intrinsic)
        .and_then(|i| i.get_declaration(lc.module, &[i64t.into()]))
        .ok_or_else(|| internal(format!("no intrinsic {intrinsic}")))?;
    let pair = b
        .build_call(decl, &[l.into(), r.into()], "ck")
        .map_err(internal)?
        .try_as_basic_value()
        .left()
        .ok_or_else(|| internal("overflow intrinsic returned nothing"))?
        .into_struct_value();
    let value = b.build_extract_value(pair, 0, "ckv").map_err(internal)?;
    let ovf = b
        .build_extract_value(pair, 1, "cko")
        .map_err(internal)?
        .into_int_value();
    trap_if(ctx, func, b, ovf, lc.int_overflow, "overflow")?;
    Ok(value)
}

/// Branch to a block that calls `trap` (which exits) when `cond` holds; the
/// builder continues in the other block.
fn trap_if<'ctx>(
    ctx: &'ctx Context,
    func: FunctionValue<'ctx>,
    b: &Builder<'ctx>,
    cond: IntValue<'ctx>,
    trap: FunctionValue<'ctx>,
    name: &str,
) -> Result<(), CodegenError> {
    let bad = ctx.append_basic_block(func, name);
    let ok = ctx.append_basic_block(func, "arith_ok");
    b.build_conditional_branch(cond, bad, ok)
        .map_err(internal)?;
    b.position_at_end(bad);
    b.build_call(trap, &[], "trap").map_err(internal)?;
    // The trap exits; a placeholder terminator, not the trap itself.
    b.build_unreachable().map_err(internal)?;
    b.position_at_end(ok);
    Ok(())
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
    lc: &LowerCtx<'_, 'ctx>,
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
        CoreKind::Lit(CoreLit::Float(bits)) => Ok(ctx.i64_type().const_int(*bits, false).into()),
        CoreKind::Lit(CoreLit::Bool(v)) => Ok(node_ty
            .into_int_type()
            .const_int(u64::from(*v), false)
            .into()),
        // N6 §1.5/§6.4: the .rodata bytes are static source data, not heap
        // roots. Build the full byte array explicitly: Inkwell's string-global
        // helper truncates at an embedded NUL, while elya_str_lit copies len.
        CoreKind::Lit(CoreLit::Str(v)) => {
            let i64t = ctx.i64_type();
            let i8t = ctx.i8_type();
            let env_roots = gc_root_env(b, lc, env)?;
            let bytes = v.as_bytes().iter().copied().chain(std::iter::once(0));
            let bytes_const = i8t.const_array(
                &bytes
                    .map(|byte| i8t.const_int(byte as u64, false))
                    .collect::<Vec<_>>(),
            );
            let source = lc.module.add_global(
                bytes_const.get_type(),
                Some(AddressSpace::default()),
                "cstr",
            );
            source.set_initializer(&bytes_const);
            source.set_constant(true);
            source.set_linkage(Linkage::Private);
            source.set_unnamed_addr(true);
            let bytes_ptr = source.as_pointer_value();
            let p = b
                .build_call(
                    lc.str_lit,
                    &[
                        i64t.const_int(lc.string_tag as u64, false).into(),
                        bytes_ptr.into(),
                        i64t.const_int(v.len() as u64, false).into(),
                    ],
                    "str",
                )
                .map_err(internal)?
                .try_as_basic_value()
                .left()
                .ok_or(CodegenError::Unsupported("elya_str_lit returned no value"))?
                .into_pointer_value();
            gc_unroot(b, lc, env_roots)?;
            Ok(p.into())
        }
        CoreKind::Builtin(builtin, args) if builtin.starts_with("time.") => {
            let mut vals = Vec::with_capacity(args.len());
            for a in args.iter() {
                vals.push(lower_expr(ctx, func, b, lc, a, env)?);
            }
            time_builtin(ctx, b, lc, builtin, &vals)
        }
        CoreKind::Builtin(builtin, args) if builtin.starts_with("any.") && !args.is_empty() => {
            let v = lower_expr(ctx, func, b, lc, &args[0], env)?;
            let roots = gc_root_env(b, lc, env)?;
            let own = usize::from(gc_root(b, lc, v)?);
            let r = any_builtin(ctx, b, lc, e, v)?;
            gc_unroot(b, lc, roots + own)?;
            Ok(r)
        }
        CoreKind::Builtin(builtin, args)
            if builtin.starts_with("int.") && builtin != "int.to_string" && args.len() == 2 =>
        {
            let x = lower_expr(ctx, func, b, lc, &args[0], env)?.into_int_value();
            let y = lower_expr(ctx, func, b, lc, &args[1], env)?.into_int_value();
            int_binary(ctx, func, b, lc, builtin, x, y)
        }
        CoreKind::Builtin(builtin, args)
            if matches!(
                builtin.as_str(),
                "float.of_int" | "float.truncate" | "float.round"
            ) && args.len() == 1 =>
        {
            let x = lower_expr(ctx, func, b, lc, &args[0], env)?.into_int_value();
            float_conversion(ctx, b, lc, builtin, x)
        }
        CoreKind::Builtin(builtin, args) if builtin == "int.to_string" && args.len() == 1 => {
            let n = lower_expr(ctx, func, b, lc, &args[0], env)?;
            let roots = gc_root_env(b, lc, env)?;
            let v = int_to_string(ctx, b, lc, n)?;
            gc_unroot(b, lc, roots)?;
            Ok(v)
        }
        CoreKind::Builtin(builtin, args) if builtin == "float.to_string" && args.len() == 1 => {
            let x = lower_expr(ctx, func, b, lc, &args[0], env)?;
            let roots = gc_root_env(b, lc, env)?;
            let v = float_to_string(ctx, b, lc, x)?;
            gc_unroot(b, lc, roots)?;
            Ok(v)
        }
        CoreKind::Builtin(builtin, args) => {
            // Refuse by the actual owned Core name before touching any argument.
            if builtin != "io.println" {
                return Err(CodegenError::UnsupportedBuiltin(builtin.clone()));
            }
            if args.len() != 1 {
                return Err(CodegenError::Unsupported(
                    "io.println takes exactly one argument",
                ));
            }
            let s = lower_expr(ctx, func, b, lc, &args[0], env)?.into_pointer_value();
            b.build_call(lc.println, &[s.into()], "pl")
                .map_err(internal)?;
            Ok(ctx.i64_type().const_int(0, false).into())
        }
        // 5b-8 Task 11: `Unit` is the immediate i64 word 0 (`repr_ty`), never
        // dereferenced, traced or rooted. A9's source needs `resume(Unit)`.
        CoreKind::Lit(CoreLit::Unit) => Ok(ctx.i64_type().const_int(0, false).into()),
        CoreKind::Var(x) => match env.get(x) {
            Some(v) => Ok(*v),
            None => Err(CodegenError::Unsupported("unbound var")),
        },
        CoreKind::Let(x, rhs, body) => {
            let v = lower_expr(ctx, func, b, lc, rhs, env)?;
            // Restore any shadowed binding -- `let x = 1; let x = x + 1` stays
            // correct -- and keep it ROOTED meanwhile (`bind_local`).
            let shadowed = bind_local(env, x, v);
            let out = lower_expr(ctx, func, b, lc, body, env);
            unbind_local(env, x, shadowed);
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
            // Strings (2026-10-15): pointer operands, a runtime call. The left
            // operand is rooted while the right one is lowered (it may
            // allocate), and `<>` roots the environment across its own
            // allocation, as `Ctor` does (spec D4).
            if is_string_prim(*op, args) {
                let l = lower_expr(ctx, func, b, lc, &args[0], env)?;
                let held = gc_root(b, lc, l)?;
                let r = lower_expr(ctx, func, b, lc, &args[1], env)?;
                let roots = if *op == BinOp::Concat {
                    gc_root_env(b, lc, env)?
                } else {
                    0
                };
                let v = string_prim(
                    ctx,
                    b,
                    lc,
                    *op,
                    l.into_pointer_value(),
                    r.into_pointer_value(),
                )?;
                gc_unroot(b, lc, roots + usize::from(held))?;
                return Ok(v);
            }
            // Structural equality (2026-10-16): the left operand is rooted
            // while the right one is lowered (it may allocate); the walk
            // itself allocates nothing.
            if is_data_eq_prim(*op, args) {
                let l = lower_expr(ctx, func, b, lc, &args[0], env)?;
                let held = gc_root(b, lc, l)?;
                let r = lower_expr(ctx, func, b, lc, &args[1], env)?;
                let v = data_eq(
                    ctx,
                    b,
                    lc,
                    *op,
                    l.into_pointer_value(),
                    r.into_pointer_value(),
                )?;
                gc_unroot(b, lc, usize::from(held))?;
                return Ok(v);
            }
            if is_unit_eq_prim(*op, args) {
                lower_expr(ctx, func, b, lc, &args[0], env)?;
                lower_expr(ctx, func, b, lc, &args[1], env)?;
                return Ok(unit_eq(ctx, *op));
            }
            if is_float_prim(*op, args) {
                let l = lower_expr(ctx, func, b, lc, &args[0], env)?.into_int_value();
                let r = lower_expr(ctx, func, b, lc, &args[1], env)?.into_int_value();
                return float_prim(ctx, b, *op, l, r);
            }
            if matches!(op, BinOp::Eq | BinOp::Ne)
                && !matches!(args[0].ty, Ty::Base(TyCon::Int) | Ty::Base(TyCon::Bool))
            {
                return Err(CodegenError::Unsupported(eq_operand_label(*op)));
            }
            let l = lower_expr(ctx, func, b, lc, &args[0], env)?.into_int_value();
            let r = lower_expr(ctx, func, b, lc, &args[1], env)?.into_int_value();
            prim_values(ctx, func, b, lc, *op, l, r)
        }
        CoreKind::App(callee, args) => {
            // Ordinary (non-tail) position: `tailcc` convention, NO tail-call
            // kind. Task 4 adds the tail-position path.
            let site = build_elya_call(ctx, func, b, lc, callee, args, env, false)?;
            site.try_as_basic_value()
                .left()
                .ok_or(CodegenError::Unsupported("call returned no value"))
        }
        CoreKind::Lambda(..) => {
            let i64t = ctx.i64_type();
            // Identity by node address: the pre-pass and this emitter hold the
            // same immutable `&CoreModule`, so a miss is a real bug, not a
            // tolerable absence.
            let key = e as *const CoreExpr as usize;
            let idx = *lc.lambda_index.get(&key).ok_or(CodegenError::Unsupported(
                "lambda site missing from the pre-pass",
            ))?;
            let site = &lc.lambdas[idx];
            let code_fn = *lc
                .lifted
                .get(&site.symbol)
                .ok_or(CodegenError::Unsupported("lambda body was never declared"))?;
            // Same bracket shape as `CoreKind::Ctor`: `elya_alloc` is the one
            // call here that can collect, so the environment is rooted across it.
            let env_roots = gc_root_env(b, lc, env)?;
            let p = b
                .build_call(
                    lc.alloc,
                    &[i64t
                        .const_int((2 + site.captures.len()) as u64, false)
                        .into()],
                    "cl",
                )
                .map_err(internal)?
                .try_as_basic_value()
                .left()
                .ok_or(CodegenError::Unsupported("elya_alloc returned no value"))?
                .into_pointer_value();
            // Word 0: the synthetic tag, so `gc_mark` finds a descriptor row for
            // this block exactly as it does for a constructor (Task 4).
            b.build_store(p, i64t.const_int(site.tag as u64, false))
                .map_err(internal)?;
            // Word 1: the code pointer, stored as a word and NOT traced.
            let cp = unsafe { b.build_gep(i64t, p, &[i64t.const_int(1, false)], "cp") }
                .map_err(internal)?;
            let code = b
                .build_ptr_to_int(code_fn.as_global_value().as_pointer_value(), i64t, "f2i")
                .map_err(internal)?;
            b.build_store(cp, code).map_err(internal)?;
            // Words 2..: the captures, in name order. Values come straight out of
            // `env` — nothing here allocates, so `env_roots` already covers them.
            for (i, (name, ty)) in site.captures.iter().enumerate() {
                let v = *env
                    .get(name)
                    .ok_or(CodegenError::Unsupported("captured name is not in scope"))?;
                let word = value_to_word(b, v, ty, i64t)?;
                let cs =
                    unsafe { b.build_gep(i64t, p, &[i64t.const_int((i + 2) as u64, false)], "cs") }
                        .map_err(internal)?;
                b.build_store(cs, word).map_err(internal)?;
            }
            gc_unroot(b, lc, env_roots)?;
            Ok(p.into())
        }
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
            // Two layers again, and here the second one is what a batch push
            // gets wrong. The live bindings go on first, because lowering a
            // field can itself allocate and those bindings are not otherwise
            // reachable from anything the collector can see. Then each field is
            // rooted AS IT IS LOWERED: in `Pair(Some(1), Some(2))` the value of
            // `Some(1)` is a temp nothing names, and lowering `Some(2)`
            // allocates — a push after the loop would root it only after the
            // collection that could already have freed it.
            let env_roots = gc_root_env(b, lc, env)?;
            let mut vals: Vec<BasicValueEnum<'ctx>> = Vec::with_capacity(fields.len());
            let mut field_roots = 0usize;
            for f in fields.iter() {
                let v = lower_expr(ctx, func, b, lc, f, env)?;
                if gc_root(b, lc, v)? {
                    field_roots += 1;
                }
                vals.push(v);
            }
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
                let word = value_to_word(b, fv, &field_tys[i], i64t)?;
                b.build_store(fp, word).map_err(internal)?;
            }
            // The object now holds the fields, so it roots them. Nothing below
            // this line allocates, so `p` needs no entry of its own — whichever
            // bracket encloses this expression is what covers it.
            gc_unroot(b, lc, field_roots + env_roots)?;
            Ok(p.into())
        }
        CoreKind::Match(scrutinee, arms) => {
            lower_match(ctx, func, b, lc, scrutinee, arms, Some(node_ty), env)?
                .ok_or_else(|| internal("a value match produced no value"))
        }
        // A DIRECT handle (one that leaks no effect) is a nesting native call,
        // wherever it sits (5b-10). A CPS handle is a site of its CPS region and
        // never reaches here from a clean front end; it is refused by name.
        CoreKind::Handle(_) => cps_emit::emit_handle_site(ctx, func, b, lc, e, env),
        // Task 8: a resume inside a clause body (or a lambda in one) is a
        // native nesting call of the continuation (D14).
        CoreKind::Resume(arg) => cps_emit::emit_resume_call(ctx, func, b, lc, e, arg, env),
        // Only CPS regions perform; reaching one here is an emitter bug.
        CoreKind::Perform(_) => Err(CodegenError::Unsupported(
            "perform outside an effectful region",
        )),
    }
}

/// The collector's descriptor table: one `[arity, ptr_mask]` row per tag, in tag
/// order -- the real constructors, then one synthetic row per lambda site, then
/// the string row. Pure: it reads Core and the lambda table and emits nothing,
/// so a unit test can read the very rows `build_module` hands the runtime.
/// The descriptor table and the tags read off it. `frame_tag` and `site_tags`
/// are the single source of truth for 7b-3's emitter (plan: "frame_tag comes
/// from descriptor_rows, never recomputed").
pub(crate) struct Descriptors {
    pub(crate) rows: Vec<u64>,
    pub(crate) frame_tag: usize,
    /// Continuation site key (`ContSite::key`) -> the tag its frames carry.
    pub(crate) site_tags: std::collections::BTreeMap<usize, usize>,
    /// Handle key (`HandlerSite::key`) -> the tag its handler frame carries.
    pub(crate) handler_tags: std::collections::BTreeMap<usize, usize>,
    /// The first handler tag. Handler rows are exactly `[handler_lo, cont_tag)`
    /// (guarded below): `elya_cont_copy` tells a handler frame from a site
    /// frame by that range alone (native multi-shot, spec D3).
    pub(crate) handler_lo: usize,
    /// D13: the continuation object's tag.
    pub(crate) cont_tag: usize,
}

fn descriptor_rows(
    core: &CoreModule,
    lambdas: &[closure::LambdaSite],
    sites: &[cps::ContSite],
    handlers: &[cps::HandlerSite],
    n_real_ctors: usize,
    string_tag: usize,
) -> Result<Descriptors, CodegenError> {
    // 5b-5 §4: one `[arity, ptr_mask]` pair per constructor, in the SAME global
    // tag order `build_ctor_table` assigns — the tag stored in an object's word
    // 0 indexes straight into this table. Bit `i` of the mask is set iff field
    // `i` is a heap pointer, which is what lets the mark phase trace precisely
    // instead of guessing: an `Int` field is never mistaken for a pointer.
    let mut desc: Vec<u64> = core
        .types
        .iter()
        .flat_map(|t| t.ctors.iter())
        .flat_map(|c| {
            let arity = c.fields.len() as u64;
            let mut mask = 0u64;
            for (i, f) in c.fields.iter().enumerate() {
                if is_heap_ty(f) {
                    mask |= 1 << i;
                }
            }
            [arity, mask].into_iter()
        })
        .collect();
    // One SYNTHETIC constructor row per lambda site, continuing the tag
    // numbering. This is what keeps `gc_mark` byte-identical: a closure is just
    // an object whose descriptor happens to have been synthesized rather than
    // declared, so the one function whose failure mode is silent gains no second
    // dispatch path.
    for (i, site) in lambdas.iter().enumerate() {
        // Assigned in `collect_lambdas`'s pre-order; checked here rather than
        // assumed, because a drift between the tag stored in word 0 and the row
        // index would mis-trace silently. A hard error, not a `debug_assert` —
        // release builds must not skip it.
        if site.tag != n_real_ctors + i {
            return Err(CodegenError::Unsupported(
                "lambda tag disagrees with its descriptor row index",
            ));
        }
        // arity = 1 (the code pointer) + the captures.
        desc.push(1 + site.captures.len() as u64);
        // Bit 0 is CLEAR: word 1 is a code pointer into the text segment, not a
        // heap object. Bit j+1 is set iff capture j is a heap value.
        let mut mask: u64 = 0;
        for (j, (_, ty)) in site.captures.iter().enumerate() {
            if is_heap_ty(ty) {
                mask |= 1 << (j + 1);
            }
        }
        desc.push(mask);
    }
    // N6 (§1.2): ONE string row — arity 0, mask 0. Nothing after the tag is a
    // heap reference, so the mark phase traces nothing for a string. The tag is
    // this row's index; the guard below makes "tag agrees with its row index" a
    // compile-time property, exactly as the lambda guard above does.
    if string_tag != desc.len() / 2 {
        return Err(CodegenError::Unsupported(
            "string tag disagrees with its descriptor row index",
        ));
    }
    desc.push(0); // arity = 0: no traced-candidate words follow the tag
    desc.push(0); // mask = 0
                  // N8 §6.1 / §6.3 / A8: ONE row for a captured-continuation frame cell,
                  // `[tag][code_ptr][next]`. Its tag continues the same linear numbering as the
                  // lambda tags and the string tag, and the row is appended AFTER the string
                  // row, so the string guard above still sees the table length it expects. Same
                  // guard shape as the string tag's: "the tag agrees with its row index" is a
                  // compile-time property rather than a comment.
    let frame_tag = string_tag + 1;
    if frame_tag != desc.len() / 2 {
        return Err(CodegenError::Unsupported(
            "frame tag disagrees with its descriptor row index",
        ));
    }
    desc.push(2); // arity = 2: the code pointer and `next` follow the tag
    desc.push(0b10); // bit 0 clear (code pointer, not traced); bit 1 set (`next` is heap)
                     // 5b-8 D10: one row per continuation site that saves anything,
                     // `[tag][code_ptr][next][saved_0..]`, appended after the frame row; a site
                     // that saves nothing shares the frame row above. The tag IS the row index
                     // at the moment the row is pushed, and `site_tags` is the only place an
                     // emitter reads it from -- so there is no second assignment for a guard to
                     // compare against (unlike the lambda tags, assigned in `collect_lambdas`).
                     // A handler frame takes its own row, appended after the site rows
                     // (below).
    let frame_row = |saved: &[(cps::Saved, Ty)], desc: &mut Vec<u64>| -> usize {
        if saved.is_empty() {
            return frame_tag;
        }
        let tag = desc.len() / 2;
        desc.push(2 + saved.len() as u64);
        // Bit 0 clear (code pointer), bit 1 set (`next`), bit j+2 set iff saved
        // value j is a heap value -- the lambda rows' convention shifted by one.
        let mut mask: u64 = 0b10;
        for (j, (_, ty)) in saved.iter().enumerate() {
            if is_heap_ty(ty) {
                mask |= 1 << (j + 2);
            }
        }
        desc.push(mask);
        tag
    };
    let mut site_tags = std::collections::BTreeMap::new();
    for site in sites {
        site_tags.insert(site.key, frame_row(&site.saved, &mut desc));
    }
    // Task 8: a handler frame is `[tag][code_ptr = the return clause][next]
    // [table][parent][saved..]`. `next` is null for a direct handle and the
    // handle's continuation for a CPS one (5b-10; traced, bit 1). Word 3 is the
    // address of the handle's static clause table (text/rodata, not heap), so
    // it is never traced: bit 2 clear. `parent` (5b-10) is the handler a
    // perform this one does not answer goes to (traced, bit 3), and saved
    // value j is bit j+4. Always its own row -- its arity is never the frame
    // row's.
    let mut handler_tags = std::collections::BTreeMap::new();
    let handler_lo = desc.len() / 2;
    for h in handlers {
        handler_tags.insert(h.key, desc.len() / 2);
        desc.push(4 + h.saved.len() as u64);
        let mut mask: u64 = 0b1010;
        for (j, (_, ty)) in h.saved.iter().enumerate() {
            if is_heap_ty(ty) {
                mask |= 1 << (j + 4);
            }
        }
        desc.push(mask);
    }
    // D13: ONE row for the continuation object a clause receives,
    // `[tag][k][h][innermost]`: `k` is the captured frame chain (traced, bit
    // 0); `h` is the frame that answers the perform, and 0 once the
    // continuation is consumed -- the one-shot flag (5b-10; not traced: `h` is
    // a frame on `k`'s chain, so `k` keeps it alive, bit 1 clear); `innermost`
    // is the handler that was current at the perform, which a resume makes
    // current again (traced, bit 2).
    let cont_tag = desc.len() / 2;
    if handler_lo + handlers.len() != cont_tag {
        return Err(CodegenError::Unsupported(
            "handler tags are not contiguous below the continuation tag",
        ));
    }
    desc.push(3);
    desc.push(0b101);
    Ok(Descriptors {
        rows: desc,
        frame_tag,
        site_tags,
        handler_tags,
        handler_lo,
        cont_tag,
    })
}

/// The shared lowering context: declarations, constructor and string tags, and
/// runtime externals. Bundled so the lowering fold threads one reference.
struct LowerCtx<'a, 'ctx> {
    module: &'a Module<'ctx>,
    decls: &'a HashMap<String, FunctionValue<'ctx>>,
    /// Constructor name -> (globally unique tag, field types). A name
    /// missing here is a constructor of a *parametric* ADT, which Task 2
    /// deferred — refused by name in the Ctor/Match arms, never unwrapped.
    ctors: &'a HashMap<String, (usize, Vec<Ty>)>,
    /// Every lambda site in the module, in the pre-order `collect_lambdas` fixed.
    lambdas: &'a [LambdaSite],
    /// Core node address -> index into `lambdas`.
    lambda_index: &'a HashMap<usize, usize>,
    /// `LambdaSite::symbol` -> the declared lifted function.
    pub(crate) lifted: &'a HashMap<String, FunctionValue<'ctx>>,
    /// 5b-9b: each lambda's region (its scope), for effectful lifted bodies.
    pub(crate) lambda_regions: &'a HashMap<usize, cps::LambdaRegion>,
    alloc: FunctionValue<'ctx>,
    str_lit: FunctionValue<'ctx>,
    /// Native strings (2026-10-15): `<>`, `==` on strings, `int.to_string`.
    str_concat: FunctionValue<'ctx>,
    str_eq: FunctionValue<'ctx>,
    /// `elya_eq`: structural equality on data (2026-10-16).
    data_eq: FunctionValue<'ctx>,
    int_to_string: FunctionValue<'ctx>,
    /// `float.to_string` (2026-10-23).
    float_to_string: FunctionValue<'ctx>,
    /// Time (2026-10-15): `time.now_ms`, `time.sleep_ms`.
    time_now_ms: FunctionValue<'ctx>,
    time_sleep_ms: FunctionValue<'ctx>,
    println: FunctionValue<'ctx>,
    string_tag: usize,
    /// The first closure tag (closures follow the real constructors), for
    /// `elya_eq`'s stop at a function (2026-10-23).
    first_closure_tag: usize,
    fail: FunctionValue<'ctx>,
    /// The shadow-stack pair, now live. Every allocation site is dominated by a
    /// push of the enclosing frame's heap bindings, so at the moment
    /// `elya_alloc` may collect, `gc_mark` reads exactly the set of values the
    /// program can still reach a name for.
    gc_push: FunctionValue<'ctx>,
    gc_pop: FunctionValue<'ctx>,
    /// 5b-8 7b-3 (D16): the top-level functions compiled with the CPS
    /// convention -- `(params.., ptr k) -> i64`.
    cps_fns: &'a HashSet<String>,
    /// Every Core node by address, so the CPS emitter can walk a site's
    /// recorded path back up to its region root.
    nodes: &'a HashMap<usize, &'a CoreExpr>,
    /// Continuation sites (D10) and their index by node address.
    sites: &'a [cps::ContSite],
    site_index: &'a HashMap<usize, usize>,
    /// 5b-10: the CPS handles and CPS resumes.
    pub(crate) fx: &'a cps::Fx,
    /// Handles and their index by node address.
    handlers: &'a [cps::HandlerSite],
    handler_index: &'a HashMap<usize, usize>,
    /// The descriptor table's tags: per site, per handler.
    desc: &'a Descriptors,
    /// Site key -> its resumption function `(i64 value, ptr frame) -> i64`.
    resume_fns: &'a HashMap<usize, FunctionValue<'ctx>>,
    /// Handle key -> (body `(ptr frame) -> i64`, return `(i64, ptr frame) -> i64`).
    handler_fns: &'a HashMap<usize, cps_emit::HandlerFns<'ctx>>,
    /// Task 8: every `(effect, op)` the module performs or handles -> its index
    /// in each handle's clause table.
    op_ids: &'a HashMap<(String, String), usize>,
    /// Task 8 runtime: the address of `elya_current_handler`, and
    /// `elya_resume_twice`, `elya_unhandled_effect` (both `ccc`).
    current_handler: inkwell::values::PointerValue<'ctx>,
    /// The address of `elya_handle_end` (runtime.c), which holds the end
    /// frame every handle's body continues to (spec
    /// 2026-10-22-elya-multi-shot-sharing D1).
    handle_end: inkwell::values::PointerValue<'ctx>,
    resume_twice: FunctionValue<'ctx>,
    unhandled: FunctionValue<'ctx>,
    /// Checked `Int` arithmetic's traps (`ccc`, they exit):
    /// `elya_int_overflow`, `elya_div_zero`, `elya_rem_zero`.
    int_overflow: FunctionValue<'ctx>,
    div_zero: FunctionValue<'ctx>,
    rem_zero: FunctionValue<'ctx>,
    /// Native multi-shot (spec D3): `elya_cont_copy(cont, hlo, hhi) -> cont'`
    /// (`ccc`), a fresh continuation over a copy of the captured frames.
    cont_copy: FunctionValue<'ctx>,
    /// `with any` (spec 2026-10-17): the reflection table, its two globals, and
    /// `elya_any_show`, `elya_any_decode`, `elya_any_fail` (`ccc`).
    reflect: &'a reflect::Reflect,
    any_table: PointerValue<'ctx>,
    any_names: PointerValue<'ctx>,
    any_show: FunctionValue<'ctx>,
    any_decode: FunctionValue<'ctx>,
    any_fail: FunctionValue<'ctx>,
    any_check: FunctionValue<'ctx>,
    /// `elya_shift_range(name, amount)` (`ccc`, exits): a shift amount outside
    /// `0..=63` (spec 2026-10-17-elya-unary-and-wrapping D3).
    shift_range: FunctionValue<'ctx>,
    /// `elya_float_to_int(name, bits, round)` (`ccc`; exits on NaN or outside
    /// `Int`, spec 2026-10-24-elya-float-part-2 D4).
    float_to_int: FunctionValue<'ctx>,
}

/// Fold `core.types` into a flat constructor table: name -> (tag, field types).
/// The tag is a GLOBALLY unique id (a running offset across all types), not the
/// index within its own type — a collector must be able to recover a
/// constructor's type from the tag alone (5b-5 spec §4).
fn build_ctor_table(core: &CoreModule) -> HashMap<String, (usize, Vec<Ty>)> {
    let mut out = HashMap::new();
    let mut next = 0usize;
    for t in &core.types {
        for c in t.ctors.iter() {
            out.insert(c.name.clone(), (next, c.fields.clone()));
            next += 1;
        }
    }
    out
}

/// Build the verified LLVM module for `core` into `ctx`: `@elya.main` lowering
/// the Core body, plus (Task 4) the printf declaration, format-string global,
/// and `@main` shim. Returns the handle so `emit_ir` and `compile_module` share
/// one construction path.
fn build_module<'ctx>(ctx: &'ctx Context, core: &CoreModule) -> Result<Module<'ctx>, CodegenError> {
    // N7 part 2: clone generic functions and locals per type instantiation,
    // so no type variable reaches the back end (spec 2026-10-15).
    // Computed callees are named and top-level functions used as values are
    // wrapped in closures, so every call below is by name (spec 2026-10-16).
    let named = anf::run(core);
    let specialized = mono::run(&named)?;
    // N7 part 1: monomorphize rows where a reference needs another convention,
    // and eta-expand upcasts, so every phase below sees types that say the
    // right convention. Pure Core-to-Core; nothing is emitted yet.
    let specialized = specialize::run(&specialized)?;
    // `with any` handlers become plain ones, a clause per handled operation,
    // now that the rows they handle are ground (spec 2026-10-17).
    let specialized = any::run(&specialized)?;
    // N7 part 2: applied parametric types become ground instances, AFTER the
    // rows inside their arguments are substituted (spec 2026-10-15 §2).
    let specialized = mono::instantiate_types(&specialized)?;
    // Nested and literal patterns become the flat matches, `if`s and `let`s
    // both emitters support (spec 2026-10-10-elya-native-pattern-compilation).
    let specialized = patterns::run(&specialized);
    // Past three parameters, the rest travel in one argument record (spec
    // 2026-10-16-elya-argument-records), so no kind exceeds its LLVM cap.
    let specialized = spill::run(&specialized);
    let core = &specialized;

    // §5.1, FIRST CHECK ON THE EMITTED CORE ON PURPOSE. The failure this
    // prevents is LLVM's `report_fatal_error` for a win64 guaranteed tail
    // call: no source span, no `Result`, the process simply dies. It must
    // therefore be impossible for any emission to have begun when this fires
    // — so it is a whole-module scan that runs before the module even exists.
    // Tracked as obligation T1. Since argument records it is a guard: `spill`
    // leaves at most three source parameters anywhere.
    for f in &core.fns {
        if f.params.len() > MAX_PARAMS {
            return Err(CodegenError::Unsupported(
                "function takes more than five parameters",
            ));
        }
    }

    // The closure tags continue the constructor numbering, so `first_tag` is the
    // number of REAL descriptor rows — computed the same way the descriptor table
    // below counts them, from `core.types`.
    let n_real_ctors: usize = core.types.iter().map(|t| t.ctors.len()).sum();
    let lambdas = closure::collect_lambdas(core, n_real_ctors);
    let string_tag = n_real_ctors + lambdas.len();
    // §5.1, and for the same reason the scan above is first: over-cap is a
    // `report_fatal_error` inside LLVM, so it must be refused BEFORE anything is
    // emitted. One site governs the whole module: every `Ty::Fn` value in a
    // whole-module compile originates at a lambda site this loop has seen.
    for site in &lambdas {
        if site.params.len() > MAX_LAMBDA_PARAMS {
            return Err(CodegenError::Unsupported(
                "lambda takes more than four parameters",
            ));
        }
    }
    let lambda_index: HashMap<usize, usize> = lambdas
        .iter()
        .enumerate()
        .map(|(i, s)| (s.key, i))
        .collect();

    // 5b-8 7b-3: which functions are effectful (D16), and the refusals that
    // keep the convention sound -- all before anything is emitted, like the
    // arity caps above. 5b-10: which handles leak an effect (CPS handles) and
    // which resumes are theirs.
    let fx = cps::effect_facts(core);
    let cps_fns = cps_emit::cps_functions(core, &fx);
    cps_emit::prepass(core, &cps_fns)?;
    let sites = cps::collect_sites(core);
    let handlers = cps::collect_handlers(core);
    let lambda_regions = cps::collect_lambda_regions(core);
    let site_index: HashMap<usize, usize> =
        sites.iter().enumerate().map(|(i, s)| (s.key, i)).collect();
    let handler_index: HashMap<usize, usize> = handlers
        .iter()
        .enumerate()
        .map(|(i, h)| (h.key, i))
        .collect();
    let nodes = cps_emit::index_nodes(core);
    let descriptors = descriptor_rows(core, &lambdas, &sites, &handlers, n_real_ctors, string_tag)?;

    let main = find_main(core)?;
    // §5.5: `main` ALONE. The scope narrows from "every function" (which was
    // trivially just `main` in N1) to "`main`", because `@elya.main`'s signature
    // says i64 and the shim's format string is `%lld`. Applying this per
    // function would refuse every ordinary predicate — see `is_pos` in the
    // corpus — and is the most likely way to implement this section wrong.
    require_int(&main.body.ty)?;

    let i64t = ctx.i64_type();
    let i32t = ctx.i32_type();
    let i8t = ctx.i8_type();
    let ptrt = ctx.ptr_type(AddressSpace::default());
    let module = ctx.create_module("elya");

    // §4.2: declare Elya functions and runtime externals, then emit every body.
    // Runtime externals are `ccc` at the C-ABI boundary; Elya functions stay `tailcc`.
    let decls = declare_all(ctx, &module, core, &cps_fns)?;
    let ctors = build_ctor_table(core);

    let alloc_ty = ptrt.fn_type(&[i64t.into()], false);
    let alloc = module.add_function("elya_alloc", alloc_ty, None); // ccc
    let str_lit_ty = ptrt.fn_type(&[i64t.into(), ptrt.into(), i64t.into()], false);
    let str_lit = module.add_function("elya_str_lit", str_lit_ty, None); // ccc
    let str_concat_ty = ptrt.fn_type(&[i64t.into(), ptrt.into(), ptrt.into()], false);
    let str_concat = module.add_function("elya_str_concat", str_concat_ty, None); // ccc
    let str_eq_ty = i64t.fn_type(&[ptrt.into(), ptrt.into()], false);
    let str_eq = module.add_function("elya_str_eq", str_eq_ty, None); // ccc
    let data_eq_ty = i64t.fn_type(&[ptrt.into(), ptrt.into(), i64t.into(), i64t.into()], false);
    let data_eq = module.add_function("elya_eq", data_eq_ty, None); // ccc
    let int_to_string_ty = ptrt.fn_type(&[i64t.into(), i64t.into()], false);
    let int_to_string = module.add_function("elya_int_to_string", int_to_string_ty, None); // ccc
    let float_to_string = module.add_function("elya_float_to_string", int_to_string_ty, None); // ccc
    let time_now_ms_ty = i64t.fn_type(&[], false);
    let time_now_ms = module.add_function("elya_time_now_ms", time_now_ms_ty, None); // ccc
    let time_sleep_ms_ty = ctx.void_type().fn_type(&[i64t.into()], false);
    let time_sleep_ms = module.add_function("elya_time_sleep_ms", time_sleep_ms_ty, None); // ccc
    let println_ty = ctx.void_type().fn_type(&[ptrt.into()], false);
    let println = module.add_function("elya_println", println_ty, None); // ccc
    let fail_ty = ctx.void_type().fn_type(&[], false);
    let fail = module.add_function("elya_match_fail", fail_ty, None); // ccc

    // 5b-5: the collector's four symbols. All `ccc` for the same reason the
    // first two are — this is the C-ABI boundary. Task 3 calls all four:
    // `elya_gc_init` hands over the descriptor table before `@elya.main` runs,
    // the shadow-stack pair brackets every allocation, and the stats dump
    // closes the shim.
    let gc_init_ty = ctx.void_type().fn_type(&[ptrt.into(), i64t.into()], false);
    let gc_init = module.add_function("elya_gc_init", gc_init_ty, None); // ccc
                                                                         // `elya_eq_init`: the float masks `elya_eq` reads (spec
                                                                         // 2026-10-23-elya-float-natively D2), the same shape.
    let eq_init = module.add_function("elya_eq_init", gc_init_ty, None); // ccc
    let gc_push_ty = ctx.void_type().fn_type(&[ptrt.into()], false);
    let gc_push = module.add_function("elya_gc_push", gc_push_ty, None); // ccc
    let gc_pop_ty = ctx.void_type().fn_type(&[], false);
    let gc_pop = module.add_function("elya_gc_pop", gc_pop_ty, None); // ccc
    let gc_report_ty = ctx.void_type().fn_type(&[], false);
    let gc_report = module.add_function("elya_gc_report", gc_report_ty, None); // ccc

    let lifted = declare_lifted(ctx, &module, &lambdas)?;
    let trap_ty = ctx.void_type().fn_type(&[], false);
    let resume_twice = module.add_function("elya_resume_twice", trap_ty, None); // ccc
    let unhandled = module.add_function("elya_unhandled_effect", trap_ty, None); // ccc
    let int_overflow = module.add_function("elya_int_overflow", trap_ty, None); // ccc
    let div_zero = module.add_function("elya_div_zero", trap_ty, None); // ccc
    let rem_zero = module.add_function("elya_rem_zero", trap_ty, None); // ccc
    let cont_copy_ty = ptrt.fn_type(&[ptrt.into(), i64t.into(), i64t.into()], false);
    let cont_copy = module.add_function("elya_cont_copy", cont_copy_ty, None); // ccc

    // `elya_current_handler`: a declaration (no initializer); the definition
    // lives in runtime.c.
    let current_handler = module
        .add_global(ptrt, Some(AddressSpace::default()), "elya_current_handler")
        .as_pointer_value();
    // The end frame (spec 2026-10-22-elya-multi-shot-sharing D1): runtime.c
    // defines the block and `elya_handle_end`, its address; its code is
    // generated here and handed over by `main` through `elya_handle_end_init`
    // (`ccc`).
    let handle_end = module
        .add_global(ptrt, Some(AddressSpace::default()), "elya_handle_end")
        .as_pointer_value();
    let handle_end_init_ty = ctx.void_type().fn_type(&[ptrt.into()], false);
    let handle_end_init = module.add_function("elya_handle_end_init", handle_end_init_ty, None);

    // `with any` (spec 2026-10-17 D3): the reflection table for the types
    // `any.show` and `any.decode` are used at, and its runtime entry points.
    let reflect = reflect::Reflect::of(core, &ctors)?;
    let any_table = {
        let words: Vec<_> = reflect
            .words
            .iter()
            .map(|w| i64t.const_int(*w as u64, true))
            .collect();
        let arr = i64t.const_array(&words);
        let g = module.add_global(
            arr.get_type(),
            Some(AddressSpace::default()),
            "elya.any.types",
        );
        g.set_initializer(&arr);
        g.set_constant(true);
        g.set_linkage(Linkage::Private);
        g.as_pointer_value()
    };
    let any_names = {
        let bytes: Vec<_> = reflect
            .names
            .iter()
            .copied()
            .chain(std::iter::once(0))
            .map(|c| i8t.const_int(c as u64, false))
            .collect();
        let arr = i8t.const_array(&bytes);
        let g = module.add_global(
            arr.get_type(),
            Some(AddressSpace::default()),
            "elya.any.names",
        );
        g.set_initializer(&arr);
        g.set_constant(true);
        g.set_linkage(Linkage::Private);
        g.as_pointer_value()
    };
    let any_show_ty = ptrt.fn_type(
        &[
            i64t.into(),
            ptrt.into(),
            ptrt.into(),
            i64t.into(),
            i64t.into(),
            ptrt.into(),
            ptrt.into(),
        ],
        false,
    );
    let any_show = module.add_function("elya_any_show", any_show_ty, None); // ccc
    let any_decode_ty = i64t.fn_type(
        &[
            i64t.into(),
            ptrt.into(),
            ptrt.into(),
            i64t.into(),
            ptrt.into(),
            ptrt.into(),
            ptrt.into(),
        ],
        false,
    );
    let any_decode = module.add_function("elya_any_decode", any_decode_ty, None); // ccc
    let any_fail_ty = ctx.void_type().fn_type(&[ptrt.into()], false);
    let any_fail = module.add_function("elya_any_fail", any_fail_ty, None); // ccc
    let any_check_ty = i64t.fn_type(&[ptrt.into(), i64t.into(), i64t.into()], false);
    let any_check = module.add_function("elya_any_check", any_check_ty, None); // ccc
    let shift_range_ty = ctx.void_type().fn_type(&[ptrt.into(), i64t.into()], false);
    let shift_range = module.add_function("elya_shift_range", shift_range_ty, None); // ccc
    let float_to_int_ty = i64t.fn_type(&[ptrt.into(), i64t.into(), i64t.into()], false);
    let float_to_int = module.add_function("elya_float_to_int", float_to_int_ty, None); // ccc
    let op_ids = cps_emit::op_ids(core);
    let (resume_fns, handler_fns) =
        cps_emit::declare(ctx, &module, &nodes, &sites, &handlers, &op_ids)?;

    let lc = LowerCtx {
        module: &module,
        decls: &decls,
        ctors: &ctors,
        lambdas: &lambdas,
        lambda_index: &lambda_index,
        lifted: &lifted,
        lambda_regions: &lambda_regions,
        alloc,
        str_lit,
        str_concat,
        str_eq,
        data_eq,
        int_to_string,
        float_to_string,
        time_now_ms,
        time_sleep_ms,
        println,
        string_tag,
        first_closure_tag: n_real_ctors,
        fail,
        gc_push,
        gc_pop,
        cps_fns: &cps_fns,
        fx: &fx,
        nodes: &nodes,
        sites: &sites,
        site_index: &site_index,
        handlers: &handlers,
        handler_index: &handler_index,
        desc: &descriptors,
        resume_fns: &resume_fns,
        handler_fns: &handler_fns,
        op_ids: &op_ids,
        current_handler,
        handle_end,
        resume_twice,
        int_overflow,
        div_zero,
        rem_zero,
        unhandled,
        cont_copy,
        reflect: &reflect,
        any_table,
        any_names,
        any_show,
        any_decode,
        any_fail,
        any_check,
        shift_range,
        float_to_int,
    };
    let b = ctx.create_builder();
    let handle_end_code = cps_emit::emit_handle_end(ctx, &module, &b, current_handler)?;
    for f in &core.fns {
        if cps_fns.contains(&f.name) {
            cps_emit::emit_cps_fn(ctx, &b, &lc, f)?;
        } else {
            emit_body(ctx, &b, &lc, f)?;
        }
    }
    cps_emit::emit_sites_and_handlers(ctx, &b, &lc)?;
    for site in lc.lambdas {
        emit_lifted(ctx, &b, &lc, site)?;
    }
    let elya_main = *decls
        .get("main")
        .ok_or(CodegenError::Unsupported("no `main`"))?;

    // §3.5/§4: the print convention is ONE external symbol (printf) plus ONE
    // generated shim (@main). The shim stays `ccc` — it is the C entry point —
    // and its call to @elya.main is an ordinary call.
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

    let desc = descriptors.rows.clone();
    let n_ctors = (desc.len() / 2) as u64;
    let desc_const = i64t.const_array(
        &desc
            .iter()
            .map(|&d| i64t.const_int(d, false))
            .collect::<Vec<_>>(),
    );
    let desc_global = module.add_global(
        desc_const.get_type(),
        Some(AddressSpace::default()),
        ".gc_desc",
    );
    desc_global.set_initializer(&desc_const);
    desc_global.set_constant(true);
    desc_global.set_unnamed_addr(true);
    // One float mask per declared constructor, in tag order (D2); never empty, so
    // the global always has an address.
    let mut float_masks: Vec<u64> = core
        .types
        .iter()
        .flat_map(|t| t.ctors.iter())
        .map(|c| {
            c.fields
                .iter()
                .enumerate()
                .filter(|(_, f)| matches!(f, Ty::Base(TyCon::Float)))
                .fold(0u64, |m, (i, _)| m | (1 << i))
        })
        .collect();
    let n_masks = float_masks.len() as u64;
    if float_masks.is_empty() {
        float_masks.push(0);
    }
    let masks_const = i64t.const_array(
        &float_masks
            .iter()
            .map(|&m| i64t.const_int(m, false))
            .collect::<Vec<_>>(),
    );
    let masks_global = module.add_global(
        masks_const.get_type(),
        Some(AddressSpace::default()),
        ".eq_float",
    );
    masks_global.set_initializer(&masks_const);
    masks_global.set_constant(true);
    masks_global.set_unnamed_addr(true);

    let entry = module.add_function("elya_entry", i64t.fn_type(&[], false), None); // ccc
    let entry_bb = ctx.append_basic_block(entry, "entry");
    b.position_at_end(entry_bb);
    let site = b.build_call(elya_main, &[], "v").map_err(internal)?;
    // The wrapper itself is `ccc`, but @elya.main is `tailcc`, so THIS CALL
    // SITE must say so too. A site whose convention disagrees with its callee's
    // declaration is a miscompile, not a verifier error — LLVM will happily emit
    // it. The call is not `musttail`: the conventions differ.
    site.set_call_convention(TAILCC);
    let main_value = site
        .try_as_basic_value()
        .left()
        .ok_or_else(|| internal("elya_main did not return a value"))?;
    b.build_return(Some(&main_value)).map_err(internal)?;
    let start_ty = i64t.fn_type(&[ptrt.into()], false);
    let start = module.add_function("elya_start", start_ty, None); // ccc

    let shim = module.add_function("main", i32t.fn_type(&[], false), None);
    let shim_entry = ctx.append_basic_block(shim, "entry");
    b.position_at_end(shim_entry);
    // BEFORE @elya.main: the collector cannot interpret a single object until it
    // has the table, so handing it over is the first thing the process does.
    b.build_call(
        gc_init,
        &[
            desc_global.as_pointer_value().into(),
            i64t.const_int(n_ctors, false).into(),
        ],
        "",
    )
    .map_err(internal)?;
    b.build_call(
        eq_init,
        &[
            masks_global.as_pointer_value().into(),
            i64t.const_int(n_masks, false).into(),
        ],
        "",
    )
    .map_err(internal)?;
    // The end frame's code, before any handle can run (spec
    // 2026-10-22-elya-multi-shot-sharing D3).
    b.build_call(
        handle_end_init,
        &[handle_end_code.as_global_value().as_pointer_value().into()],
        "",
    )
    .map_err(internal)?;
    // The program runs on its own stack (spec 2026-10-16-elya-native-stack):
    // `elya_start` (runtime.c) reserves it, runs `@elya_entry` on a thread that
    // owns it, names an overflow of it, and hands back main's value here, on
    // the process's main thread. `@elya_entry` exists because C cannot call a
    // `tailcc` function: it is `ccc`, and its one call is to `main`'s function.
    let v = b
        .build_call(
            start,
            &[entry.as_global_value().as_pointer_value().into()],
            "v",
        )
        .map_err(internal)?
        .try_as_basic_value()
        .left()
        .ok_or_else(|| internal("elya_start did not return a value"))?;
    b.build_call(printf, &[fmt.as_pointer_value().into(), v.into()], "p")
        .map_err(internal)?;
    // AFTER the answer, so a stats dump can never interleave with the value the
    // execution proofs assert on, and gated on ELY_GC_STATS inside the runtime
    // so stderr stays empty unless a test asks for the counters.
    b.build_call(gc_report, &[], "").map_err(internal)?;
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
    let mut cmd = std::process::Command::new("clang");
    cmd.arg(obj).arg(&runtime).arg("-o").arg(exe);
    // `compile_module` emits with `RelocMode::Default`, which on Linux is
    // non-PIC: a string literal is addressed by an absolute `R_X86_64_32`
    // relocation. Distribution clangs (Ubuntu's among them) link PIE by default
    // and reject that relocation, so link non-PIE here. Linux only, and the
    // emitted code is unchanged: making the object PIC instead would change
    // codegen on every platform (PARKED.md).
    #[cfg(target_os = "linux")]
    cmd.arg("-no-pie");
    // The program's own stack is a thread's (runtime.c `elya_start`).
    #[cfg(unix)]
    cmd.arg("-pthread");
    let out = cmd.output().map_err(CodegenError::Io)?;
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
                row: elya::types::EffectRow::pure(),
                name: "main".into(),
                params: Rc::from([]),
                body,
            }],
            types: Vec::new(),
            generic_types: Vec::new(),
            effect_params_used: Default::default(),
            effects: Default::default(),
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
            row: elya::types::EffectRow::pure(),
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

    // Was `rejects_div_specifically` (5b-1: no Div until a guard existed). The
    // checked-arithmetic decision (2026-10-10) supplies the guard; execution
    // is proved in `tests/native_codegen.rs` (`integer_division_runs_natively`).
    #[test]
    fn div_and_rem_lower_to_verifier_clean_ir() {
        for op in [BinOp::Div, BinOp::Rem] {
            emit_ir(&main_fn(prim(op, int_lit(7), int_lit(2))))
                .unwrap_or_else(|e| panic!("{op:?}: {e:?}"));
        }
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
        // checker is happy with `T == T` for an ADT and there is no native
        // equality for it. Dispatched on the OPERAND type before the operands
        // are lowered, so the message names the operator (§3.3) rather than the
        // operand. Wrapped in an `if` because a Bool-bodied main is refused
        // earlier, by `require_int`, with a different message. (Until
        // 2026-10-15 the operand here was a `String`, until 2026-10-16 an ADT;
        // both compare natively since, and no source program reaches the back
        // end with any other operand type -- functions are E0436, `Float` is
        // refused by Core -- so the guard is exercised on a hand-edited Core:
        // `1 == 2` with its operands retyped as a tuple.)
        let mut core = core_of("pub fn main() -> Int { if 1 == 2 { 1 } else { 0 } }\n");
        fn retype(e: &mut CoreExpr) {
            match &mut e.kind {
                CoreKind::Prim(op, args) if *op == BinOp::Eq => {
                    let mut xs: Vec<CoreExpr> = args.to_vec();
                    for x in xs.iter_mut() {
                        x.ty = Ty::Tuple(Vec::new());
                    }
                    *args = Rc::from(xs);
                }
                CoreKind::If(c, _, _) => retype(Rc::make_mut(c)),
                _ => panic!("unexpected shape: {:?}", e.kind),
            }
        }
        let main = core
            .fns
            .iter_mut()
            .find(|f| f.name == "main")
            .expect("main");
        retype(&mut main.body);
        let err = emit_ir(&core).unwrap_err();
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

    /// 5b-6 §5.1. A lambda converts to a lifted function of arity `P + 1` (the
    /// closure is parameter 0), so the source cap is FOUR, not five.
    ///
    /// C-ii probe, `scratchpad/n5-indirect-probe`, LLVM 18.1.6 /
    /// `x86_64-pc-windows-msvc`, 2026-09-03. Sweeping caller arity C × callee
    /// arity K over 1..8 for a `musttail` call under `tailcc`, the indirect-callee
    /// matrix is cell-for-cell identical to the direct-callee matrix: K <= 5
    /// compiles for every C; K in {6,7} only when C >= 6; K = 8 only when C >= 8.
    /// Every passing cell emits a real tail jump; every failing cell is
    /// `LLVM ERROR: Can't handle guaranteed tail call under win64 yet`, a
    /// `report_fatal_error` that kills the process with no source span. Refusing
    /// at five source parameters is what keeps that unreachable.
    #[test]
    fn a_lambda_with_five_parameters_compiles() {
        // Was `rejects_a_lambda_with_five_parameters`. Since argument records
        // (2026-10-16) the last three travel in one record; expectation changed
        // by that decision.
        let p = |n: &str| elya::core::CoreParam {
            name: n.to_string(),
            ty: Ty::Base(TyCon::Int),
        };
        let e = CoreExpr {
            span: Span::EMPTY,
            ty: Ty::Fn(
                vec![Ty::Base(TyCon::Int); 5],
                elya::types::EffectRow::pure(),
                Box::new(Ty::Base(TyCon::Int)),
            ),
            kind: CoreKind::Lambda(
                Rc::from([p("a"), p("b"), p("c"), p("d"), p("e")]),
                Rc::new(int_lit(1)),
            ),
        };
        let m = main_fn(CoreExpr {
            span: Span::EMPTY,
            ty: Ty::Base(TyCon::Int),
            kind: CoreKind::Let("f".into(), Rc::new(e), Rc::new(int_lit(1))),
        });
        emit_ir(&m).expect("a five-parameter lambda compiles");
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
    fn six_parameters_compile_and_no_kind_can_pass_its_cap() {
        // §5.1. Was `rejects_six_parameters_before_emission_begins`: past five
        // parameters a win64 guaranteed tail call is a `report_fatal_error`
        // with no source span, so a whole-module scan refused the module before
        // emission. Since argument records (2026-10-16) `spill` leaves at most
        // `MAX_SOURCE_PARAMS` source parameters anywhere, and the scan stays
        // as a guard; expectation changed by that decision. The invariant that
        // makes the guard unreachable: the worst kind (an effectful lambda, or
        // a clause: its closure or frame and its continuation) still fits.
        emit_ir(&core_of(
            "fn six(a, b, c, d, e, f) { a + b + c + d + e + f }\n\
             pub fn main() { six(1, 2, 3, 4, 5, 6) }\n",
        ))
        .expect("six parameters compile");
        assert_eq!(MAX_PARAMS, 5);
        const _: () = assert!(spill::MAX_SOURCE_PARAMS + 2 <= MAX_PARAMS);
    }

    #[test]
    fn a_polymorphic_function_is_not_emitted_unless_instantiated() {
        // §5.2 / obligation T2, revised by N7 part 2 (2026-10-15): this test
        // used to assert that the unused generic `id` refused the whole module
        // ("unrepresentable type"). Generic originals are no longer emitted --
        // only their clones, one per instantiation -- so the module compiles,
        // and the module compiles. The whole-module strictness stays for every
        // NON-generic function (each is still emitted and checked).
        emit_ir(&core_of("fn id(x) { x }\npub fn main() { 1 }\n"))
            .expect("the generic original is dropped");
    }

    // ---- 5b-8 7b-3 refusals (D16, D17, and what Task 8 owns) ----------------

    const S_W: &str = "effect S { fn get() -> Int }\n\
                       fn w(b) { if b { get() } else { 2 } }\n";

    // ---- 5b-8 Task 8 refusals (A2, and the clause arity cap) ---------------

    fn handle_expr(clauses: Vec<elya::core::CoreClause>, multi: bool) -> CoreExpr {
        CoreExpr {
            span: Span::EMPTY,
            ty: Ty::Base(TyCon::Int),
            kind: CoreKind::Handle(Rc::new(elya::core::CoreHandle {
                body: Rc::new(int_lit(0)),
                clauses: clauses.into(),
                ret: None,
                is_multi_declared: multi,
                multi,
                any: false,
            })),
        }
    }

    /// Native multi-shot (2026-10-10) replaces 5b-8's
    /// `a_multi_shot_handler_is_refused_by_its_own_name`: the refusal is lifted.
    #[test]
    fn a_multi_shot_handler_compiles() {
        emit_ir(&main_fn(handle_expr(Vec::new(), true))).expect("a multi handle compiles");
    }

    /// Native multi-shot (2026-10-10) replaces 5b-8's
    /// `the_multi_refusal_fires_before_any_clause_body_is_lowered`: with no
    /// handle-node refusal, the poisonous clause body is lowered and refuses
    /// with ITS OWN message.
    #[test]
    fn a_multi_shot_handlers_clause_bodies_are_lowered() {
        let poisonous = CoreExpr {
            span: Span::EMPTY,
            ty: Ty::Base(TyCon::Int),
            kind: CoreKind::Var("unbound".to_string()),
        };
        let clause = elya::core::CoreClause {
            effect: "Flip".to_string(),
            op: "flip".to_string(),
            params: Rc::from([]),
            body: Rc::new(poisonous),
        };
        let err = emit_ir(&main_fn(handle_expr(vec![clause], true))).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unbound"), "{msg}");
        assert!(!msg.contains("multi"), "the handle node was refused: {msg}");
    }

    /// Source -> Core, with the front end allowed to WARN: `with multi` is
    /// deliberately not an error (E0426 stays a warning), so it type-checks
    /// and lowers.
    fn lower_warned_source(src: &str) -> CoreModule {
        let session = elya::Session::new();
        let (m, pd) = elya::parse::parse_module(&session, src);
        assert!(pd.is_empty(), "parse: {pd:?}");
        let (diags, table) = elya::types::infer_typed_table(&session, &m);
        assert!(
            !diags
                .iter()
                .any(|d| d.severity == elya::diag::Severity::Error),
            "type errors: {diags:?}"
        );
        elya::core::lower_module(&m, &table).expect("must lower to Core")
    }

    /// Native multi-shot (2026-10-10) replaces 5b-8's
    /// `a_multi_shot_handler_written_in_source_reaches_the_codegen_refusal`.
    #[test]
    fn a_multi_shot_handler_written_in_source_compiles() {
        let src = "effect multi Flip { fn flip() -> Bool }\n\
                   fn g() -> Int { if flip() { 1 } else { 0 } }\n\
                   pub fn main() -> Int {\n\
                   \x20 handle g() with multi { Flip.flip() -> resume(True) }\n\
                   }\n";
        emit_ir(&lower_warned_source(src)).expect("a multi handle compiles");
    }

    #[test]
    fn an_effect_operation_with_four_parameters_compiles() {
        // Was `an_effect_operation_with_four_parameters_is_refused`: the clause
        // takes the op's arguments, the continuation and the handler frame, and
        // 4 + 2 is past MAX_PARAMS. Since argument records (2026-10-16) the
        // fourth argument travels in a record; expectation changed by that
        // decision. The cap's refusal stays as a guard behind `spill`.
        emit_ir(&core_of(
            "effect E { fn op(a: Int, b: Int, c: Int, d: Int) -> Int }\n\
             fn body() -> Int { op(1, 2, 3, 4) }\n\
             pub fn main() -> Int {\n\
             \x20 handle { body() } with { E.op(a, b, c, d) -> resume(a + d)  return(r) -> r }\n\
             }\n",
        ))
        .expect("a four-argument operation compiles");
    }

    #[test]
    fn an_effect_polymorphic_function_used_at_a_user_effect_compiles() {
        // N7 part 1 (replaces `..._is_refused`, D16's pin): `apply` is
        // compiled direct for its pure uses, and this instantiation at {S}
        // goes to a clone whose rows say S, so it compiles CPS.
        emit_ir(&core_of(&format!(
            "{S_W}fn apply(f) {{ f(1) + 1 }}\n\
             pub fn main() -> Int {{\n\
             \x20 handle {{ apply(fn(x) {{ x * get() }}) }} with {{\n\
             \x20   S.get() -> resume(10)\n    return(r) -> r\n  }}\n}}\n"
        )))
        .expect("a clone of `apply` at {S} compiles");
    }

    #[test]
    fn a_handle_nested_inside_another_handle_compiles() {
        // Slice 5b-10 (replaces `a_handle_nested_inside_another_handle_is_refused`,
        // which pinned D17's refusal): the same program now compiles. The inner
        // handle discharges everything, so it is a direct nesting call.
        emit_ir(&core_of(&format!(
            "{S_W}pub fn main() -> Int {{\n\
             \x20 handle {{\n\
             \x20   handle {{ w(False) }} with {{ S.get() -> resume(1)  return(r) -> r }}\n\
             \x20 }} with {{ S.get() -> resume(2)  return(r) -> r }}\n}}\n"
        )))
        .expect("a nested handle compiles");
    }

    #[test]
    fn a_handle_inside_an_effectful_function_compiles() {
        // Slice 5b-10 (replaces `a_handle_inside_an_effectful_function_is_refused`):
        // `g` performs T outside its handle, so it is CPS, and the handle inside
        // it now compiles there.
        emit_ir(&core_of(&format!(
            "{S_W}effect T {{ fn t() -> Int }}\n\
             fn g() -> Int {{\n\
             \x20 let a = handle {{ w(False) }} with {{ S.get() -> resume(1)  return(r) -> r }}\n\
             \x20 a + t()\n}}\n\
             pub fn main() -> Int {{\n\
             \x20 handle {{ g() }} with {{ T.t() -> resume(1)  return(r) -> r }}\n}}\n"
        )))
        .expect("a handle inside an effectful function compiles");
    }

    #[test]
    fn a_let_bound_effect_polymorphic_lambda_used_at_a_user_effect_compiles() {
        // N7 part 1 (replaces `..._is_refused`, the 5b-9b review's pin: a
        // SIGSEGV where the evaluator printed 8, then refused by name): the
        // use at {S} goes to a local clone `let app.spec.k = ..` bound next
        // to `app`, whose type says S. Execution: `CONVENTIONS` in
        // tests/native_codegen.rs.
        for body in [
            "let app = fn(g) { g() + 1 }  app(fn() { get() })",
            "let app = fn(g) { g() + 1 }  let h = fn() { get() }  app(h) * 2",
        ] {
            emit_ir(&core_of(&format!(
                "{S_W}pub fn main() -> Int {{\n\
                 \x20 handle {{ {body} }} with {{\n\
                 \x20   S.get() -> resume(7)\n    return(r) -> r\n  }}\n}}\n"
            )))
            .unwrap_or_else(|e| panic!("{body}: {e:?}"));
        }
    }

    #[test]
    fn an_effectful_lambda_compiles_with_four_parameters_too() {
        // 5b-9b (replaces `an_effectful_lambda_is_refused_by_name`), and since
        // argument records (2026-10-16) the four-parameter case compiles too:
        // it was refused by name ("effectful lambda takes more than three
        // parameters"); expectation changed by that decision.
        emit_ir(&core_of(&format!(
            "{S_W}pub fn main() -> Int {{\n\
             \x20 handle {{ let f = fn(x) {{ x + get() }}  f(1) }} with {{\n\
             \x20   S.get() -> resume(1)\n    return(r) -> r\n  }}\n}}\n"
        )))
        .expect("an effectful lambda compiles");
        emit_ir(&core_of(&format!(
            "{S_W}pub fn main() -> Int {{\n\
             \x20 handle {{ let f = fn(a, b, c, d) {{ a + b + c + d + get() }}  f(1, 2, 3, 4) }} with {{\n\
             \x20   S.get() -> resume(1)\n    return(r) -> r\n  }}\n}}\n"
        )))
        .expect("a four-parameter effectful lambda compiles");
    }

    #[test]
    fn an_effectful_function_with_five_parameters_compiles() {
        // Was `an_effectful_function_with_five_parameters_is_refused` (D14: the
        // continuation is a sixth LLVM parameter). Since argument records
        // (2026-10-16) it compiles; expectation changed by that decision.
        emit_ir(&core_of(&format!(
            "{S_W}fn five(a, b, c, d, e) {{ a + b + c + d + e + get() }}\n\
             pub fn main() -> Int {{\n\
             \x20 handle {{ five(1, 2, 3, 4, 5) }} with {{\n\
             \x20   S.get() -> resume(1)\n    return(r) -> r\n  }}\n}}\n"
        )))
        .expect("a five-parameter effectful function compiles");
    }

    #[test]
    fn a3_an_unconstrained_polymorphic_effect_is_not_emitted() {
        // A3 (spec §11), measured. A polymorphic effect has no refusal of its
        // own: when `reader` leaves the effect's parameter unconstrained, the
        // parameter survives into `reader`'s type as a `Ty::Var`, and that is
        // refused by the SAME `repr_ty` rule as `id` above (5b-8 plan D2).
        // Constrain it (`get() + 1`) and the measured refusal moves to the
        // monomorphic program's, so the `Ty::Var` is the cause, not the effect.
        let core = core_of(
            "effect State(s) { fn get() -> s  fn set(v: s) -> Unit }\n\
             fn reader() { get() }\n\
             pub fn main() -> Int { 0 }\n",
        );
        let reader = core
            .fns
            .iter()
            .find(|f| f.name == "reader")
            .expect("reader");
        assert!(
            matches!(reader.body.ty, Ty::Var(_)),
            "the unconstrained parameter should reach Core as a Ty::Var: {:?}",
            reader.body.ty
        );
        // N7 part 2 (2026-10-15): `reader` is generic in its result, so it is
        // emitted only as clones, and nothing instantiates it: the module
        // compiles (this asserted the "unrepresentable type" refusal before).
        // An instantiated row-only variable is refused by its own name -- see
        // `mono::tests::a_row_only_type_variable_is_instantiated_from_the_reference`.
        emit_ir(&core).expect("the uninstantiated generic `reader` is not emitted");
    }

    #[test]
    fn a_computed_callee_compiles() {
        // Was `rejects_a_computed_callee_specifically` (§5.3): `(fn(x) { x + 1
        // })(3)` parses to `App(Lambda, ..)` and was refused by name. Since
        // 2026-10-16 (`anf.rs`) a computed callee is let-bound first and called
        // as a local closure; the emitters' "computed callee" refusal stays as
        // the guard behind the pass. Expectation changed by that decision.
        emit_ir(&core_of("pub fn main() { (fn(x) { x + 1 })(3) }\n"))
            .expect("a named callee compiles");
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
    fn a_function_name_in_value_position_compiles() {
        // Was `rejects_a_function_name_in_value_position` (§5.4's companion):
        // `let f = add3` was refused by name. Since 2026-10-16 (`anf.rs`) a
        // top-level function used as a value is wrapped in a closure with no
        // captures; the emitters' "function used as a value" refusal stays as
        // the guard behind the pass. Expectation changed by that decision.
        emit_ir(&core_of(
            "fn add3(x) { x + 3 }\npub fn main() {\n  let f = add3\n  f(1)\n}\n",
        ))
        .expect("a wrapped function value compiles");
    }

    #[test]
    fn concat_verifies() {
        // Was `rejects_concat_by_name` (N6 §8.2) until native strings
        // (2026-10-15): `<>` is a runtime call now, its operands pointers, so the
        // `Prim` arm must not reach `.into_int_value()` for it. BOUND in a `let`
        // whose body is `1`, so `require_int` cannot mask the arm.
        emit_ir(&core_of(
            "pub fn main() {\n  let s = \"a\" <> \"b\"\n  1\n}\n",
        ))
        .expect("`<>` verifies");
    }

    #[test]
    fn rejects_an_unknown_builtin_by_its_owned_name_before_arguments() {
        let poisonous_arg = CoreExpr {
            span: Span::EMPTY,
            ty: Ty::Base(TyCon::Int),
            kind: CoreKind::Var("unbound".to_string()),
        };
        let builtin = CoreExpr {
            span: Span::EMPTY,
            ty: Ty::Base(TyCon::Unit),
            kind: CoreKind::Builtin("io.not_println".to_string(), Rc::from([poisonous_arg])),
        };
        let body = CoreExpr {
            span: Span::EMPTY,
            ty: Ty::Base(TyCon::Int),
            kind: CoreKind::Let("u".to_string(), Rc::new(builtin), Rc::new(int_lit(0))),
        };
        let err = emit_ir(&main_fn(body)).unwrap_err();
        assert!(
            matches!(err, CodegenError::UnsupportedBuiltin(ref name) if name == "io.not_println"),
            "{err:?}"
        );
        assert_eq!(
            err.to_string(),
            "codegen: unsupported builtin (io.not_println)"
        );
    }

    #[test]
    fn io_println_refuses_the_wrong_arity_before_lowering_arguments() {
        let builtin = CoreExpr {
            span: Span::EMPTY,
            ty: Ty::Base(TyCon::Unit),
            kind: CoreKind::Builtin("io.println".to_string(), Rc::from([])),
        };
        let body = CoreExpr {
            span: Span::EMPTY,
            ty: Ty::Base(TyCon::Int),
            kind: CoreKind::Let("u".to_string(), Rc::new(builtin), Rc::new(int_lit(0))),
        };
        let err = emit_ir(&main_fn(body)).unwrap_err();
        assert!(
            matches!(
                err,
                CodegenError::Unsupported("io.println takes exactly one argument")
            ),
            "{err:?}"
        );
    }

    // Was `rejects_a_non_adt_match_scrutinee` (N6 §8.4). Native pattern
    // compilation (2026-10-10) rewrites every match on a non-ADT value into
    // `let`s and `if`s before the emitters run, so this program now compiles;
    // its value is proved by execution (`nested_and_literal_patterns_run_natively`,
    // p2). The emitters keep the refusal as a guard for a match the pass leaves.
    #[test]
    fn a_non_adt_match_scrutinee_is_compiled_away() {
        emit_ir(&core_of("pub fn main() { match 1 { _ -> 2 } }"))
            .expect("a wildcard match on an Int compiles");
    }

    #[test]
    fn eq_on_strings_verifies() {
        // Was `rejects_eq_on_strings_by_name` until native strings (2026-10-15):
        // the allow-list is Int, Bool and String now (byte equality, a runtime
        // call); every other operand type is still refused by name
        // (`rejects_eq_on_an_unrepresentable_operand_type`).
        emit_ir(&core_of(
            "pub fn main() { if \"a\" == \"a\" { 1 } else { 0 } }",
        ))
        .expect("String `==` verifies");
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

    /// `repr_ty` says pointer <=> `is_heap_ty` says traced. If these ever disagree,
    /// some value is passed around as a pointer and never traced (a use-after-free)
    /// or traced without being one (a wild dereference in `gc_mark`). Cheap to check,
    /// and it catches the next `repr_ty` widening that forgets the mask.
    #[test]
    fn mask_and_repr_agree_on_pointers() {
        let ctx = Context::create();
        let cases = [
            Ty::Base(TyCon::Int),
            Ty::Base(TyCon::Bool),
            Ty::Con("List".to_string(), vec![]),
            Ty::Fn(
                vec![Ty::Base(TyCon::Int)],
                elya::types::EffectRow::pure(),
                Box::new(Ty::Base(TyCon::Int)),
            ),
            // N6: the two widths Task 1 admits. Str is a pointer AND traced;
            // Unit is an i64 immediate and must never be traced.
            Ty::Base(TyCon::Str),
            Ty::Base(TyCon::Unit),
        ];
        for ty in cases {
            let repr = repr_ty(&ctx, &ty).expect("every case is representable");
            assert_eq!(
                repr.is_pointer_type(),
                is_heap_ty(&ty),
                "repr_ty and is_heap_ty disagree on {ty:?}"
            );
        }
    }

    #[test]
    fn the_frame_row_follows_the_string_row_with_mask_0b10() {
        // N8 §6.1 / §6.3, A8, and plan D5. A captured-continuation frame cell is
        // `[tag][code_ptr][next]`: arity 2, mask 0b10. Bit 0 is CLEAR because word
        // 1 is a code pointer into the text segment (the lambda rows' own
        // convention); bit 1 is SET because `next` is a heap frame. The spec's
        // §6.3 says 0b11 -- the silent-corruption error D5 records -- and this
        // pins the correction. The frame row is the table's last, right after the
        // string row, so the frame tag is `string_tag + 1`.
        //
        // A real program carrying every earlier kind of row (a constructor with a
        // heap field, and a lambda), so the frame row is placed after all of them.
        let core = core_of(
            "type L { Nil, Cons(Int, L) }\n\
             fn len(l) { match l { Nil -> 0  Cons(_, t) -> 1 + len(t) } }\n\
             pub fn main() -> Int {\n\
             \x20 let f = fn(x) { x + 1 }\n\
             \x20 f(len(Cons(1, Nil)))\n\
             }\n",
        );
        let n_real_ctors: usize = core.types.iter().map(|t| t.ctors.len()).sum();
        let lambdas = closure::collect_lambdas(&core, n_real_ctors);
        assert_eq!(lambdas.len(), 1, "the program must carry one lambda row");
        let string_tag = n_real_ctors + lambdas.len();
        let d = descriptor_rows(&core, &lambdas, &[], &[], n_real_ctors, string_tag).expect("rows");
        let rows = d.rows;
        let frame_tag = string_tag + 1;
        // Task 8 (D13, approved expected-value change): with no sites and no
        // handles, the frame row is followed by exactly ONE more row -- the
        // continuation object's `[3, 0b101]` -- where it used to be the last.
        assert_eq!(
            rows.len() / 2,
            frame_tag + 2,
            "frame row, then only the continuation-object row: {rows:?}"
        );
        assert_eq!(
            d.frame_tag, frame_tag,
            "descriptor_rows reports the frame tag"
        );
        assert_eq!(d.cont_tag, frame_tag + 1);
        assert_eq!(
            &rows[2 * d.cont_tag..],
            &[3, 0b101],
            "continuation object: k and handler traced, consumed flag not"
        );
        assert_eq!(
            &rows[2 * string_tag..2 * string_tag + 2],
            &[0, 0],
            "the string row is unchanged"
        );
        assert_eq!(
            &rows[2 * frame_tag..2 * frame_tag + 2],
            &[2, 0b10],
            "frame row: arity 2, mask 0b10 (D5) -- not the spec's 0b11"
        );
    }

    /// Native multi-shot (spec D3): `elya_cont_copy` tells handler frames from
    /// site frames by tag range alone, so the handler rows must be exactly
    /// `[handler_lo, cont_tag)`.
    #[test]
    fn handler_tags_sit_contiguously_below_the_continuation_tag() {
        let core = core_of(
            "effect multi Flip { fn flip() -> Bool }\n\
             effect Ask { fn ask() -> Int }\n\
             effect Tell { fn tell(v: Int) -> Unit }\n\
             fn inner() {\n\
             \x20 handle {\n\
             \x20   handle { let b = flip()  let y = ask()  let _ = tell(y)  if b { y } else { y + 1 } }\n\
             \x20   with { Tell.tell(v) -> resume(Unit) }\n\
             \x20 } with { Ask.ask() -> resume(5) * 2 }\n\
             }\n\
             pub fn main() -> Int { handle inner() with multi { Flip.flip() -> resume(True) * 1000 + resume(False) } }\n",
        );
        let n_real_ctors: usize = core.types.iter().map(|t| t.ctors.len()).sum();
        let lambdas = closure::collect_lambdas(&core, n_real_ctors);
        let string_tag = n_real_ctors + lambdas.len();
        let sites = cps::collect_sites(&core);
        let handlers = cps::collect_handlers(&core);
        assert_eq!(handlers.len(), 3);
        assert!(!sites.is_empty());
        let d = descriptor_rows(&core, &lambdas, &sites, &handlers, n_real_ctors, string_tag)
            .expect("rows");
        assert_eq!(d.handler_lo, d.cont_tag - 3);
        let mut tags: Vec<usize> = d.handler_tags.values().copied().collect();
        tags.sort_unstable();
        assert_eq!(tags, (d.handler_lo..d.cont_tag).collect::<Vec<_>>());
        assert!(d.site_tags.values().all(|t| *t < d.handler_lo));
    }
}
