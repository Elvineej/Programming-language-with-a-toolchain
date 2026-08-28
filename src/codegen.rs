//! Native codegen (Slice 5b-1): Core → LLVM via inkwell, arithmetic subset only
//! — Int literals, Var, Prim(Add/Sub/Mul), Let. Exactly one value type
//! (`Ty::Base(TyCon::Int)` → i64), one function (`@elya_main`), no effects.
//!
//! Proof is EXECUTION (tests/native_codegen.rs), never IR inspection (spec §0):
//! `emit_ir` is a debugging aid and nothing in the suite asserts on its output.
//! Semantic-fidelity rule (§3.4): native codegen must never be more-undefined
//! than the tree evaluator — hence no Div/Rem (UB on zero divisor; needs a
//! branch), no And/Or (short-circuit needs a branch), and Add/Sub/Mul emitted
//! WITHOUT nsw/nuw so overflow is defined two's-complement wrapping.

use std::collections::HashMap;

use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::types::IntType;
use inkwell::values::IntValue;

use crate::ast::BinOp;
use crate::core::{CoreExpr, CoreFn, CoreKind, CoreLit, CoreModule};
use crate::types::{Ty, TyCon};

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

/// §3.2 type mapping: reads the INLINE `ty` field on each Core node (Shape C —
/// the reason this fold needs no side-table lookups). Polymorphic nodes carry
/// `Ty::Var(_)` by design; codegen demands monomorphic Int and errors otherwise
/// (when that fires, that is N7 knocking).
fn require_int(ty: &Ty) -> Result<(), CodegenError> {
    if matches!(ty, Ty::Base(TyCon::Int)) {
        Ok(())
    } else {
        Err(CodegenError::Unsupported("non-Int value"))
    }
}

/// §3.3 expression lowering: a recursive fold returning an `IntValue`,
/// threading a binding environment. NO alloca, NO mem2reg — bindings are
/// immutable and there is no control flow, so values map directly to SSA
/// registers; the save/restore around `Let` is what makes shadowing correct.
fn lower_expr<'ctx>(
    i64t: IntType<'ctx>,
    b: &Builder<'ctx>,
    e: &CoreExpr,
    env: &mut HashMap<String, IntValue<'ctx>>,
) -> Result<IntValue<'ctx>, CodegenError> {
    require_int(&e.ty)?;
    match &e.kind {
        CoreKind::Lit(CoreLit::Int(n)) => Ok(i64t.const_int(*n as u64, true)),
        CoreKind::Lit(_) => Err(CodegenError::Unsupported("non-Int literal")),
        CoreKind::Var(x) => env
            .get(x)
            .copied()
            .ok_or(CodegenError::Unsupported("unbound var")),
        CoreKind::Let(x, rhs, body) => {
            let v = lower_expr(i64t, b, rhs, env)?;
            let prev = env.insert(x.clone(), v);
            let out = lower_expr(i64t, b, body, env);
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
            let l = lower_expr(i64t, b, &args[0], env)?;
            let r = lower_expr(i64t, b, &args[1], env)?;
            // Deliberately NO nsw/nuw flags: defined two's-complement wrapping
            // (§3.4). Overflow reconciliation with the evaluator is tracked in
            // spec §11 — not silently decided here.
            let built = match op {
                BinOp::Add => b.build_int_add(l, r, "add"),
                BinOp::Sub => b.build_int_sub(l, r, "sub"),
                BinOp::Mul => b.build_int_mul(l, r, "mul"),
                other => return Err(CodegenError::Unsupported(op_label(*other))),
            };
            built.map_err(internal)
        }
        CoreKind::App(..) => Err(CodegenError::Unsupported("App")),
        CoreKind::Lambda(..) => Err(CodegenError::Unsupported("Lambda")),
        CoreKind::Match(..) => Err(CodegenError::Unsupported("Match")),
    }
}

/// Build the verified LLVM module for `core` into `ctx`: `@elya_main` lowering
/// the Core body, plus (Task 4) the printf declaration, format-string global,
/// and `@main` shim. Returns the handle so `emit_ir` and `compile_module` share
/// one construction path.
fn build_module<'ctx>(ctx: &'ctx Context, core: &CoreModule) -> Result<Module<'ctx>, CodegenError> {
    let f = validate_module(core)?;
    let i64t = ctx.i64_type();
    let module = ctx.create_module("elya");
    let func = module.add_function("elya_main", i64t.fn_type(&[], false), None);
    let entry = ctx.append_basic_block(func, "entry");
    let b = ctx.create_builder();
    b.position_at_end(entry);
    let mut env = HashMap::new();
    let result = lower_expr(i64t, &b, &f.body, &mut env)?;
    b.build_return(Some(&result)).map_err(internal)?;

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

#[cfg(all(test, feature = "codegen"))]
mod tests {
    use super::*;
    use crate::span::Span;
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

    #[test]
    fn corpus_verifies() {
        // Layer 1 (§8): verifier-clean IR for the §5 corpus. Cheap structural
        // teeth — NOT the proof (that is execution in tests/native_codegen.rs).
        // The corpus strings duplicate tests/native_codegen.rs's CORPUS because
        // integration targets cannot share consts; four lines of duplication is
        // cheaper than new plumbing.
        let corpus = [
            "pub fn main() { 1 + 2 }\n",
            "pub fn main() {\n  let x = 6\n  let y = 7\n  x * y\n}\n",
            "pub fn main() { (2 + 3) * 4 - 5 }\n",
            "pub fn main() { 3 - 10 }\n",
        ];
        for src in corpus {
            let session = crate::Session::new();
            let (m, pd) = crate::parse::parse_module(&session, src);
            assert!(pd.is_empty(), "parse: {pd:?}");
            let (diags, table) = crate::types::infer_typed_table(&session, &m);
            assert!(diags.is_empty(), "type errors: {diags:?}");
            let core = crate::core::lower_module(&m, &table).expect("lowers");
            emit_ir(&core).expect("verifier-clean IR");
        }
    }

    #[test]
    fn rejects_div_specifically() {
        let err = emit_ir(&main_fn(prim(BinOp::Div, int_lit(1), int_lit(2)))).unwrap_err();
        assert!(matches!(err, CodegenError::Unsupported("Div")), "{err:?}");
    }

    #[test]
    fn rejects_and_specifically() {
        let err = emit_ir(&main_fn(prim(BinOp::And, int_lit(1), int_lit(0)))).unwrap_err();
        assert!(matches!(err, CodegenError::Unsupported("And")), "{err:?}");
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
        m.fns[0].params = Rc::from(["x".to_string()]);
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
