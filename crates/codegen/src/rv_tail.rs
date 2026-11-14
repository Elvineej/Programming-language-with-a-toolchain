//! Guaranteed tail calls on 32-bit RISC-V, where LLVM 18 has no `tailcc`
//! (spec 2026-11-14-elya-riscv-board D1).
//!
//! LLVM 18's RISC-V back end accepts the C, Fast and GHC conventions only
//! (`RISCVISelLowering.cpp`, `LowerFormalArguments`: `report_fatal_error(
//! "Unsupported calling convention")`; GHC needs the F and D extensions). For
//! any of them a `musttail` call needs its caller's and callee's types to be the
//! same (the verifier exempts only `tailcc` and `swifttailcc`), and the back end
//! makes it a tail call only when no argument is stacked
//! (`isEligibleForTailCallOptimization`: "Do not tail call opt if the stack is
//! used to pass parameters"), or stops the process.
//!
//! So on RISC-V every Elya function has one type: twelve 32-bit words in, one
//! 64-bit word out, under `fastcc`, which passes integers in thirteen registers
//! in LLVM 18 (`a0`-`a7`, `t2`-`t6`; `getFastCCArgGPRs`) and in twelve on LLVM's
//! main branch (which gives `t2` up for Zicfilp): twelve fit both. A parameter
//! takes one word (a pointer, a Bool zero-extended) or two (an `Int`, low half
//! first), in order; the words left over are `poison`; the result is a pointer
//! by `ptrtoint`, a Bool zero-extended, an `Int` as it is. One type makes every
//! `musttail` valid, and with every argument in a register the back end makes
//! every one a tail call. The largest Elya signatures fit: five `Int`s and a
//! continuation (eleven words), a clause of an operation of five `Int`s
//! (twelve). A function that needs more words is refused by name.
//!
//! Like `arm_tail`, this runs over the finished module, after the optimiser:
//! every `tailcc` function and call is covered however it was made, and the
//! optimiser saw the real types.

use crate::{internal, CodegenError, TAILCC};
use inkwell::attributes::AttributeLoc;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::types::{BasicMetadataTypeEnum, BasicTypeEnum, FunctionType};
use inkwell::values::LLVMTailCallKind;
use inkwell::values::{
    BasicMetadataValueEnum, BasicValueEnum, CallSiteValue, FunctionValue, InstructionOpcode,
    InstructionValue, IntValue,
};

/// The words every function takes: as many of `fastcc`'s argument registers as
/// every LLVM since 18 gives it on RV32.
pub(crate) const WORDS: usize = 12;

/// LLVM's `fastcc` (llvm/IR/CallingConv.h: `Fast = 8`).
const FASTCC: u32 = 8;

fn word_type<'ctx>(ctx: &'ctx Context) -> FunctionType<'ctx> {
    let w: BasicMetadataTypeEnum<'ctx> = ctx.i32_type().into();
    ctx.i64_type().fn_type(&[w; WORDS], false)
}

fn too_many() -> CodegenError {
    CodegenError::Unsupported("a function whose parameters take more than 12 words on RISC-V")
}

/// A value as the words it is passed in.
fn to_words<'ctx>(
    b: &Builder<'ctx>,
    ctx: &'ctx Context,
    v: BasicValueEnum<'ctx>,
    out: &mut Vec<BasicMetadataValueEnum<'ctx>>,
) -> Result<(), CodegenError> {
    let i32t = ctx.i32_type();
    match v {
        BasicValueEnum::IntValue(i) if i.get_type().get_bit_width() == 64 => {
            let lo = b.build_int_truncate(i, i32t, "lo").map_err(internal)?;
            let hi = b
                .build_right_shift(i, ctx.i64_type().const_int(32, false), false, "h")
                .map_err(internal)?;
            let hi = b.build_int_truncate(hi, i32t, "hi").map_err(internal)?;
            out.push(lo.into());
            out.push(hi.into());
        }
        BasicValueEnum::IntValue(i) if i.get_type().get_bit_width() < 32 => {
            out.push(b.build_int_z_extend(i, i32t, "w").map_err(internal)?.into());
        }
        BasicValueEnum::IntValue(i) if i.get_type().get_bit_width() == 32 => out.push(i.into()),
        BasicValueEnum::PointerValue(p) => {
            out.push(b.build_ptr_to_int(p, i32t, "w").map_err(internal)?.into());
        }
        _ => {
            return Err(CodegenError::Unsupported(
                "a value RISC-V's one function type cannot carry",
            ))
        }
    }
    Ok(())
}

/// The words a value of `ty` takes.
fn words_of(ty: BasicTypeEnum<'_>) -> Result<usize, CodegenError> {
    match ty {
        BasicTypeEnum::IntType(t) if t.get_bit_width() == 64 => Ok(2),
        BasicTypeEnum::IntType(t) if t.get_bit_width() <= 32 => Ok(1),
        BasicTypeEnum::PointerType(_) => Ok(1),
        _ => Err(CodegenError::Unsupported(
            "a value RISC-V's one function type cannot carry",
        )),
    }
}

/// A value of `ty` from its words, taken from the front of `words`.
fn from_words<'ctx>(
    b: &Builder<'ctx>,
    ctx: &'ctx Context,
    words: &mut impl Iterator<Item = IntValue<'ctx>>,
    ty: BasicTypeEnum<'ctx>,
) -> Result<BasicValueEnum<'ctx>, CodegenError> {
    let i64t = ctx.i64_type();
    let mut next = || {
        words
            .next()
            .ok_or_else(|| internal("a parameter's word is missing"))
    };
    match ty {
        BasicTypeEnum::IntType(t) if t.get_bit_width() == 64 => {
            let lo = b
                .build_int_z_extend(next()?, i64t, "lo")
                .map_err(internal)?;
            let hi = b
                .build_int_z_extend(next()?, i64t, "hi")
                .map_err(internal)?;
            let hi = b
                .build_left_shift(hi, i64t.const_int(32, false), "h")
                .map_err(internal)?;
            Ok(b.build_or(hi, lo, "v").map_err(internal)?.into())
        }
        BasicTypeEnum::IntType(t) if t.get_bit_width() < 32 => Ok(b
            .build_int_truncate(next()?, t, "v")
            .map_err(internal)?
            .into()),
        BasicTypeEnum::IntType(t) if t.get_bit_width() == 32 => Ok(next()?.into()),
        BasicTypeEnum::PointerType(t) => Ok(b
            .build_int_to_ptr(next()?, t, "v")
            .map_err(internal)?
            .into()),
        _ => Err(CodegenError::Unsupported(
            "a value RISC-V's one function type cannot carry",
        )),
    }
}

/// A result as the one word every function answers.
fn result_word<'ctx>(
    b: &Builder<'ctx>,
    ctx: &'ctx Context,
    v: BasicValueEnum<'ctx>,
) -> Result<IntValue<'ctx>, CodegenError> {
    let i64t = ctx.i64_type();
    match v {
        BasicValueEnum::IntValue(i) if i.get_type().get_bit_width() == 64 => Ok(i),
        BasicValueEnum::IntValue(i) => b.build_int_z_extend(i, i64t, "r").map_err(internal),
        BasicValueEnum::PointerValue(p) => b.build_ptr_to_int(p, i64t, "r").map_err(internal),
        _ => Err(CodegenError::Unsupported(
            "a value RISC-V's one function type cannot carry",
        )),
    }
}

/// A result of `ty` from that word.
fn from_result<'ctx>(
    b: &Builder<'ctx>,
    w: IntValue<'ctx>,
    ty: BasicTypeEnum<'ctx>,
) -> Result<BasicValueEnum<'ctx>, CodegenError> {
    match ty {
        BasicTypeEnum::IntType(t) if t.get_bit_width() == 64 => Ok(w.into()),
        BasicTypeEnum::IntType(t) => Ok(b.build_int_truncate(w, t, "v").map_err(internal)?.into()),
        BasicTypeEnum::PointerType(t) => {
            Ok(b.build_int_to_ptr(w, t, "v").map_err(internal)?.into())
        }
        _ => Err(CodegenError::Unsupported(
            "a value RISC-V's one function type cannot carry",
        )),
    }
}

/// Gives every `tailcc` function and call RISC-V's one function type (see the
/// module comment). Returns how many functions changed, for the tests.
pub(crate) fn uniform_tail_calls<'ctx>(
    ctx: &'ctx Context,
    module: &Module<'ctx>,
) -> Result<usize, CodegenError> {
    let word_fn = word_type(ctx);
    // Each function's replacement, bodiless: the calls are rebuilt first.
    let mut moved: Vec<(FunctionValue<'ctx>, FunctionValue<'ctx>)> = Vec::new();
    for f in module.get_functions() {
        if f.get_call_conventions() != TAILCC {
            continue;
        }
        let mut words = 0;
        for t in f.get_type().get_param_types() {
            words += words_of(t)?;
        }
        if words > WORDS {
            return Err(too_many());
        }
        let g = module.add_function("", word_fn, None);
        g.set_linkage(f.get_linkage());
        g.set_call_conventions(FASTCC);
        for a in f.attributes(AttributeLoc::Function) {
            g.add_attribute(AttributeLoc::Function, a);
        }
        moved.push((f, g));
    }

    let mut calls: Vec<InstructionValue<'ctx>> = Vec::new();
    for f in module.get_functions() {
        for bb in f.get_basic_blocks() {
            let mut at = bb.get_first_instruction();
            while let Some(i) = at {
                at = i.get_next_instruction();
                if i.get_opcode() == InstructionOpcode::Call
                    && CallSiteValue::try_from(i)
                        .map_err(|()| internal("a call is not a call site"))?
                        .get_call_convention()
                        == TAILCC
                {
                    calls.push(i);
                }
            }
        }
    }
    let b = ctx.create_builder();
    for i in calls {
        recall(ctx, &b, word_fn, i)?;
    }

    let count = moved.len();
    for (f, g) in moved {
        // The parameters, from their words, in a block of their own ahead of
        // the old entry.
        let words = ctx.append_basic_block(g, "words");
        let first = f.get_first_basic_block();
        let mut after = words;
        for bb in f.get_basic_blocks() {
            bb.move_after(after)
                .map_err(|()| internal("a block would not move"))?;
            after = bb;
        }
        b.position_at_end(words);
        let mut slots = g.get_param_iter().map(|p| p.into_int_value());
        for old in f.get_param_iter() {
            let v = from_words(&b, ctx, &mut slots, old.get_type())?;
            match (old, v) {
                (BasicValueEnum::IntValue(o), BasicValueEnum::IntValue(n)) => {
                    o.replace_all_uses_with(n)
                }
                (BasicValueEnum::PointerValue(o), BasicValueEnum::PointerValue(n)) => {
                    o.replace_all_uses_with(n)
                }
                _ => return Err(internal("a parameter changed kind")),
            }
        }
        match first {
            Some(entry) => {
                b.build_unconditional_branch(entry).map_err(internal)?;
            }
            None => {
                // A declaration: no body to move.
                unsafe { words.delete() }.map_err(|()| internal("an empty block stayed"))?;
            }
        }
        // Every result as a word.
        for bb in g.get_basic_blocks() {
            let Some(t) = bb.get_terminator() else {
                continue;
            };
            if t.get_opcode() != InstructionOpcode::Return || t.get_num_operands() == 0 {
                continue;
            }
            let Some(v) = t.get_operand(0).and_then(|e| e.left()) else {
                continue;
            };
            if matches!(v, BasicValueEnum::IntValue(i) if i.get_type().get_bit_width() == 64) {
                continue;
            }
            b.position_before(&t);
            let w = result_word(&b, ctx, v)?;
            b.build_return(Some(&w)).map_err(internal)?;
            t.erase_from_basic_block();
        }
        let name = f.get_name().to_str().map_err(internal)?.to_owned();
        f.replace_all_uses_with(g);
        // SAFETY: `f` has no blocks and, after the replacement, no uses.
        unsafe { f.delete() };
        g.as_global_value().set_name(&name);
    }
    Ok(count)
}

/// One `tailcc` call, rebuilt as a `fastcc` call of the one type. A `musttail`
/// call's `ret` returns the word itself: nothing may stand between them.
fn recall<'ctx>(
    ctx: &'ctx Context,
    b: &Builder<'ctx>,
    word_fn: FunctionType<'ctx>,
    i: InstructionValue<'ctx>,
) -> Result<(), CodegenError> {
    let n = i.get_num_operands();
    let mut args: Vec<BasicValueEnum<'ctx>> = Vec::with_capacity(n as usize);
    for k in 0..n.saturating_sub(1) {
        let v = i.get_operand(k).and_then(|e| e.left());
        args.push(v.ok_or_else(|| internal("a call argument is not a value"))?);
    }
    let Some(BasicValueEnum::PointerValue(callee)) = i.get_operand(n - 1).and_then(|e| e.left())
    else {
        return Err(internal("a call's callee is not a pointer"));
    };
    let old = CallSiteValue::try_from(i).map_err(|()| internal("a call is not a call site"))?;
    let kind = old.get_tail_call_kind();
    b.position_before(&i);
    let mut words: Vec<BasicMetadataValueEnum<'ctx>> = Vec::with_capacity(WORDS);
    for a in args {
        to_words(b, ctx, a, &mut words)?;
    }
    if words.len() > WORDS {
        return Err(too_many());
    }
    while words.len() < WORDS {
        words.push(ctx.i32_type().get_poison().into());
    }
    let site = b
        .build_indirect_call(word_fn, callee, &words, "")
        .map_err(internal)?;
    site.set_call_convention(FASTCC);
    site.set_tail_call_kind(kind);
    let word = site
        .try_as_basic_value()
        .left()
        .ok_or_else(|| internal("a call of the one type has no word"))?
        .into_int_value();
    let value = old.try_as_basic_value().left();
    if kind == LLVMTailCallKind::LLVMTailCallKindMustTail {
        let ret = i
            .get_next_instruction()
            .filter(|r| r.get_opcode() == InstructionOpcode::Return)
            .ok_or_else(|| internal("a musttail call not followed by its ret"))?;
        b.position_before(&ret);
        b.build_return(Some(&word)).map_err(internal)?;
        ret.erase_from_basic_block();
        i.erase_from_basic_block();
        return Ok(());
    }
    if let Some(v) = value {
        let back = from_result(b, word, v.get_type())?;
        match (v, back) {
            (BasicValueEnum::IntValue(o), BasicValueEnum::IntValue(n)) => {
                o.replace_all_uses_with(n)
            }
            (BasicValueEnum::PointerValue(o), BasicValueEnum::PointerValue(n)) => {
                o.replace_all_uses_with(n)
            }
            _ => return Err(internal("a call's result changed kind")),
        }
    }
    i.erase_from_basic_block();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::uniform_tail_calls;
    use crate::{CodegenError, TAILCC};
    use inkwell::context::Context;
    use inkwell::types::BasicMetadataTypeEnum;

    /// Twelve words fit (five Ints and two pointers); thirteen (six Ints and a
    /// pointer) are refused by name rather than stacked, where no tail call is
    /// possible (spec 2026-11-14-elya-riscv-board D1).
    #[test]
    fn twelve_words_fit_and_thirteen_are_refused() {
        let ctx = Context::create();
        for (ints, fits) in [(5, true), (6, false)] {
            let module = ctx.create_module("m");
            let mut params: Vec<BasicMetadataTypeEnum> = vec![ctx.i64_type().into(); ints];
            params.push(ctx.ptr_type(inkwell::AddressSpace::default()).into());
            if fits {
                params.push(ctx.ptr_type(inkwell::AddressSpace::default()).into());
            }
            let f = module.add_function("f", ctx.i64_type().fn_type(&params, false), None);
            f.set_call_conventions(TAILCC);
            let b = ctx.create_builder();
            b.position_at_end(ctx.append_basic_block(f, "e"));
            b.build_return(Some(&f.get_nth_param(0).expect("a parameter")))
                .expect("ret");
            let r = uniform_tail_calls(&ctx, &module);
            if fits {
                assert_eq!(r.ok(), Some(1));
                module.verify().expect("verifies");
            } else {
                assert!(
                    matches!(r, Err(CodegenError::Unsupported(m)) if m.contains("12 words")),
                    "{r:?}"
                );
            }
        }
    }
}
