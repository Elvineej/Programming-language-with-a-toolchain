//! Native codegen: Core → LLVM via inkwell. Slice 5b-1 covered the arithmetic
//! subset (Int literals, Var, Prim(Add/Sub/Mul), Let); Slice 5b-2 adds control
//! flow — `If` as a three-block diamond joined by `phi`, `Bool` as i1, the six
//! comparisons as signed `icmp`, and `&&`/`||` as bit-wise `and`/`or` on i1.
//! Two value widths (i64, i1), one function (`@elya_main`), no effects.
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
use inkwell::types::IntType;
use inkwell::values::FunctionValue;
use inkwell::values::IntValue;
use inkwell::AddressSpace;
use inkwell::IntPredicate;
use inkwell::OptimizationLevel;

use elya::ast::BinOp;
use elya::core::{CoreExpr, CoreFn, CoreKind, CoreLit, CoreModule};
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

/// §3.1 module shape: exactly one function, named `main`, zero parameters.
/// Rejections are errors, not silent skips. N2 lifts the first and third.
fn validate_module(core: &CoreModule) -> Result<&CoreFn, CodegenError> {
    if core.fns.len() > 1 {
        return Err(CodegenError::Unsupported("multi-function module"));
    }
    let f = core
        .fns
        .first()
        .ok_or(CodegenError::Unsupported("no `main`"))?;
    if f.name != "main" {
        return Err(CodegenError::Unsupported("no `main`"));
    }
    if !f.params.is_empty() {
        return Err(CodegenError::Unsupported("main takes parameters"));
    }
    Ok(f)
}

/// §3.1 type mapping, widened for N3: `Int` -> i64, `Bool` -> i1. Reads the
/// INLINE `ty` field on each Core node (Shape C — the reason this fold needs no
/// side-table lookups). Everything else is refused by name, `Ty::Var(_)`
/// included; when that fires, that is N7 knocking.
///
/// A toe-in, not the value-representation decision: two integer widths is the
/// least that lets a branch have a condition. Heap values arrive with N4.
fn repr_ty<'ctx>(ctx: &'ctx Context, ty: &Ty) -> Result<IntType<'ctx>, CodegenError> {
    match ty {
        Ty::Base(TyCon::Int) => Ok(ctx.i64_type()),
        Ty::Base(TyCon::Bool) => Ok(ctx.bool_type()),
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

/// §3.3 expression lowering: a recursive fold returning an `IntValue`, threading
/// a binding environment. NO alloca, NO mem2reg — bindings are immutable and
/// values map directly to SSA registers; the save/restore around `Let` is what
/// makes shadowing correct. Each node's width comes from its own inline type, so
/// an i1 and an i64 register coexist without a wrapper enum: inkwell's
/// `IntValue` already carries its width.
fn lower_expr<'ctx>(
    ctx: &'ctx Context,
    func: FunctionValue<'ctx>,
    b: &Builder<'ctx>,
    e: &CoreExpr,
    env: &mut HashMap<String, IntValue<'ctx>>,
) -> Result<IntValue<'ctx>, CodegenError> {
    let node_ty = repr_ty(ctx, &e.ty)?;
    match &e.kind {
        CoreKind::Lit(CoreLit::Int(n)) => Ok(node_ty.const_int(*n as u64, true)),
        CoreKind::Lit(CoreLit::Bool(v)) => Ok(node_ty.const_int(u64::from(*v), false)),
        CoreKind::Lit(_) => Err(CodegenError::Unsupported("non-Int literal")),
        CoreKind::Var(x) => env
            .get(x)
            .copied()
            .ok_or(CodegenError::Unsupported("unbound var")),
        CoreKind::Let(x, rhs, body) => {
            let v = lower_expr(ctx, func, b, rhs, env)?;
            let prev = env.insert(x.clone(), v);
            let out = lower_expr(ctx, func, b, body, env);
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
            let l = lower_expr(ctx, func, b, &args[0], env)?;
            let r = lower_expr(ctx, func, b, &args[1], env)?;
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
            built.map_err(internal)
        }
        CoreKind::App(..) => Err(CodegenError::Unsupported("App")),
        CoreKind::Lambda(..) => Err(CodegenError::Unsupported("Lambda")),
        CoreKind::If(cond, then_e, else_e) => {
            let c = lower_expr(ctx, func, b, cond, env)?;
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
            let tv = lower_expr(ctx, func, b, then_e, env)?;
            // THE TRAP (§3.2): a nested `if` inside this branch left the builder
            // in ITS join block, not in `then_bb`. `phi` names the block control
            // actually flows FROM, so read the exit block back from the builder
            // instead of assuming it is the block we positioned at.
            let then_exit = b
                .get_insert_block()
                .ok_or(CodegenError::Unsupported("builder left no block"))?;
            b.build_unconditional_branch(join_bb).map_err(internal)?;

            b.position_at_end(else_bb);
            let ev = lower_expr(ctx, func, b, else_e, env)?;
            let else_exit = b
                .get_insert_block()
                .ok_or(CodegenError::Unsupported("builder left no block"))?;
            b.build_unconditional_branch(join_bb).map_err(internal)?;

            b.position_at_end(join_bb);
            // Exactly two incoming values, always: the AST's `else_block` is not
            // an Option, so there is no one-armed `if` to synthesize a Unit
            // branch for. Both branches carry the same type — the checker
            // unified them — so one phi type is correct.
            let phi = b.build_phi(tv.get_type(), "iftmp").map_err(internal)?;
            phi.add_incoming(&[(&tv, then_exit), (&ev, else_exit)]);
            Ok(phi.as_basic_value().into_int_value())
        }
        CoreKind::Match(..) => Err(CodegenError::Unsupported("Match")),
    }
}

/// Build the verified LLVM module for `core` into `ctx`: `@elya_main` lowering
/// the Core body, plus (Task 4) the printf declaration, format-string global,
/// and `@main` shim. Returns the handle so `emit_ir` and `compile_module` share
/// one construction path.
fn build_module<'ctx>(ctx: &'ctx Context, core: &CoreModule) -> Result<Module<'ctx>, CodegenError> {
    let f = validate_module(core)?;
    // §3.1: `main` returns i64. The fold now speaks two widths, so this is the
    // one place that still insists on Int.
    require_int(&f.body.ty)?;
    let i64t = ctx.i64_type();
    let module = ctx.create_module("elya");
    let func = module.add_function("elya_main", i64t.fn_type(&[], false), None);
    let entry = ctx.append_basic_block(func, "entry");
    let b = ctx.create_builder();
    b.position_at_end(entry);
    let mut env = HashMap::new();
    let result = lower_expr(ctx, func, &b, &f.body, &mut env)?;
    b.build_return(Some(&result)).map_err(internal)?;

    // §3.5/§4: the print convention is ONE external symbol (printf) plus ONE
    // generated shim (@main). Elya's namespace stays clean for N2; deleting the
    // convention later is deleting a function, not unpicking a fold.
    let i32t = ctx.i32_type();
    let i8t = ctx.i8_type();
    let ptrt = ctx.ptr_type(AddressSpace::default());
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
    let v = b
        .build_call(func, &[], "v")
        .map_err(internal)?
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
    let out = std::process::Command::new("clang")
        .arg(obj)
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

    #[test]
    fn corpus_verifies() {
        // Layer 1 (§8): verifier-clean IR for the §5 corpus. Cheap structural
        // teeth — NOT the proof (that is execution in tests/native_codegen.rs).
        // The corpus strings duplicate tests/native_codegen.rs's CORPUS and
        // CONTROL_FLOW_CORPUS because integration targets cannot share consts;
        // eleven lines of duplication is cheaper than new plumbing.
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
        ];
        for src in corpus {
            let session = elya::Session::new();
            let (m, pd) = elya::parse::parse_module(&session, src);
            assert!(pd.is_empty(), "parse: {pd:?}");
            let (diags, table) = elya::types::infer_typed_table(&session, &m);
            assert!(diags.is_empty(), "type errors: {diags:?}");
            let core = elya::core::lower_module(&m, &table).expect("lowers");
            emit_ir(&core).expect("verifier-clean IR");
        }
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
    fn rejects_multi_function_module_specifically() {
        let mut m = main_fn(int_lit(1));
        m.fns.push(CoreFn {
            name: "other".into(),
            params: Rc::from([]),
            body: int_lit(2),
        });
        let err = emit_ir(&m).unwrap_err();
        assert!(
            matches!(err, CodegenError::Unsupported("multi-function module")),
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
}
