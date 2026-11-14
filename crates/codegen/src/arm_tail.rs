//! A `tailcc` call's stacked arguments on 32-bit ARM, made a size LLVM 18's
//! ARM back end agrees with itself about (spec 2026-11-13-elya-boards D4).
//!
//! Under `tailcc` the callee pops its own stacked arguments. LLVM's ARM back end
//! has it pop them rounded up to the data layout's stack alignment, 8 bytes
//! under AAPCS (`ARMISelLowering.cpp`, `LowerFormalArguments`:
//! `StackArgSize = alignTo(StackArgSize, DL.getStackAlignment())`), but has a
//! caller making an ordinary call expect them popped rounded up to 16
//! (`LowerCall`: `CalleePopBytes = canGuaranteeTCO(..) ? alignTo(NumBytes, 16)`).
//! Where the two differ, 1 to 8 bytes past a multiple of 16, the caller's stack
//! pointer comes back 8 bytes off and its `pop {.., pc}` returns into garbage:
//! on QEMU a hard fault at pc 0, the first time `many(fs, 0, 0)` returns. Both
//! lines are the same on LLVM's main branch.
//!
//! So no such call is made. A `tailcc` signature whose stacked arguments end 1
//! to 8 bytes past a multiple of 16 gets one more `i64` parameter, passed as
//! `poison`. It is stacked (once one argument is, every later one is), and it
//! takes the area to the next multiple of 16, where the two roundings agree.
//!
//! This runs over the finished module, after the optimiser: every `tailcc`
//! function and every `tailcc` call is covered however it was made, and no pass
//! sees the unused parameter, so none can take it away again.

use crate::{internal, CodegenError, TAILCC};
use inkwell::attributes::AttributeLoc;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum, FunctionType};
use inkwell::values::{
    BasicMetadataValueEnum, BasicValueEnum, CallSiteValue, FunctionValue, InstructionOpcode,
    InstructionValue,
};

/// The bytes of a call's arguments that AAPCS (soft float) puts on the stack,
/// as LLVM 18 assigns them, or `None` for a parameter type Elya never passes.
///
/// A word (a pointer, an `i1` widened, an `i32`) takes the next of `r0`-`r3`,
/// or 4 bytes of stack. An `i64` takes an even pair, `r0:r1` or `r2:r3`
/// (skipping an odd register), or 8 bytes of stack at an 8-byte offset; and
/// once any argument is stacked, every later one is (LLVM marks `r0`-`r3`
/// used: `CCAssignToStackWithShadow<4, 8, [R0, R1, R2, R3]>`).
pub(crate) fn stacked_bytes(params: &[BasicTypeEnum<'_>]) -> Option<u32> {
    let mut reg = 0u32; // the next core register, r0..r3; 4 when none is left
    let mut stack = 0u32; // the next stacked offset
    for t in params {
        let wide = match t {
            BasicTypeEnum::IntType(i) if i.get_bit_width() == 64 => true,
            BasicTypeEnum::IntType(i) if i.get_bit_width() <= 32 => false,
            BasicTypeEnum::PointerType(_) => false,
            _ => return None,
        };
        if wide {
            reg += reg % 2;
            if reg + 2 <= 4 {
                reg += 2;
            } else {
                reg = 4;
                stack = stack.next_multiple_of(8) + 8;
            }
        } else if reg < 4 {
            reg += 1;
        } else {
            stack += 4;
        }
    }
    Some(stack)
}

/// Whether a `tailcc` call with these parameters is one the two roundings
/// disagree about: 1 to 8 bytes of stacked arguments past a multiple of 16.
fn needs_pad(params: &[BasicTypeEnum<'_>]) -> Result<bool, CodegenError> {
    let bytes = stacked_bytes(params).ok_or(CodegenError::Unsupported(
        "a `tailcc` parameter the ARM call padding cannot place",
    ))?;
    Ok(bytes.next_multiple_of(8) % 16 == 8)
}

fn padded_type<'ctx>(
    ctx: &'ctx Context,
    ret: Option<BasicTypeEnum<'ctx>>,
    params: &[BasicTypeEnum<'ctx>],
) -> FunctionType<'ctx> {
    let mut all: Vec<BasicMetadataTypeEnum<'ctx>> = params.iter().map(|&t| t.into()).collect();
    all.push(ctx.i64_type().into());
    match ret {
        Some(r) => r.fn_type(&all, false),
        None => ctx.void_type().fn_type(&all, false),
    }
}

/// Pads every `tailcc` function and call that needs it (see the module
/// comment). Returns how many functions were padded, for the tests.
pub(crate) fn pad_tail_calls<'ctx>(
    ctx: &'ctx Context,
    module: &Module<'ctx>,
) -> Result<usize, CodegenError> {
    // Each padded function's replacement, made first (bodiless): the calls are
    // rebuilt while every old function still exists.
    let mut moved: Vec<(FunctionValue<'ctx>, FunctionValue<'ctx>)> = Vec::new();
    for f in module.get_functions() {
        if f.get_call_conventions() != TAILCC {
            continue;
        }
        let ty = f.get_type();
        let params = ty.get_param_types();
        if !needs_pad(&params)? {
            continue;
        }
        let g = module.add_function("", padded_type(ctx, ty.get_return_type(), &params), None);
        g.set_linkage(f.get_linkage());
        g.set_call_conventions(TAILCC);
        for a in f.attributes(AttributeLoc::Function) {
            g.add_attribute(AttributeLoc::Function, a);
        }
        for a in f.attributes(AttributeLoc::Return) {
            g.add_attribute(AttributeLoc::Return, a);
        }
        for i in 0..params.len() as u32 {
            for a in f.attributes(AttributeLoc::Param(i)) {
                g.add_attribute(AttributeLoc::Param(i), a);
            }
        }
        moved.push((f, g));
    }

    // Every `tailcc` call that needs it, rebuilt with the padding argument. The
    // callee operand is kept as it is (an old function's pointer is replaced
    // below, with the function), so a direct and an indirect call are one case.
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
        repad_call(ctx, &b, i)?;
    }

    // The bodies move into their replacements, whose parameters take over the
    // old ones' uses, and the replacements take over the functions' own uses
    // (the code pointers closures and frames hold) and then their names.
    let count = moved.len();
    for (f, g) in moved {
        let anchor = ctx.append_basic_block(g, "");
        let mut after = anchor;
        for bb in f.get_basic_blocks() {
            bb.move_after(after)
                .map_err(|()| internal("a block would not move"))?;
            after = bb;
        }
        // SAFETY: `anchor` is empty and nothing branches to it; deleting it
        // leaves the old entry block first, as the entry.
        unsafe { anchor.delete() }.map_err(|()| internal("the anchor block stayed"))?;
        for (old, new) in f.get_param_iter().zip(g.get_param_iter()) {
            match (old, new) {
                (BasicValueEnum::IntValue(o), BasicValueEnum::IntValue(n)) => {
                    o.replace_all_uses_with(n)
                }
                (BasicValueEnum::PointerValue(o), BasicValueEnum::PointerValue(n)) => {
                    o.replace_all_uses_with(n)
                }
                _ => return Err(internal("a padded parameter changed kind")),
            }
        }
        let name = f.get_name().to_str().map_err(internal)?.to_owned();
        f.replace_all_uses_with(g);
        // SAFETY: `f` has no blocks and, after the replacement, no uses.
        unsafe { f.delete() };
        g.as_global_value().set_name(&name);
    }
    Ok(count)
}

/// One `tailcc` call, rebuilt in place with the padding argument if its
/// signature needs one. The signature is the call's own: its arguments' types
/// and its result's (no Elya call is variadic).
fn repad_call<'ctx>(
    ctx: &'ctx Context,
    b: &Builder<'ctx>,
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
    let types: Vec<BasicTypeEnum<'ctx>> = args.iter().map(|v| v.get_type()).collect();
    if !needs_pad(&types)? {
        return Ok(());
    }
    let ret = BasicTypeEnum::try_from(i.get_type()).ok();
    let fn_ty = padded_type(ctx, ret, &types);
    let old = CallSiteValue::try_from(i).map_err(|()| internal("a call is not a call site"))?;
    let mut vals: Vec<BasicMetadataValueEnum<'ctx>> = args.iter().map(|&v| v.into()).collect();
    vals.push(ctx.i64_type().get_poison().into());
    b.position_before(&i);
    let site = b
        .build_indirect_call(fn_ty, callee, &vals, "")
        .map_err(internal)?;
    site.set_call_convention(TAILCC);
    site.set_tail_call_kind(old.get_tail_call_kind());
    for loc in [AttributeLoc::Function, AttributeLoc::Return] {
        for a in old.attributes(loc) {
            site.add_attribute(loc, a);
        }
    }
    for k in 0..types.len() as u32 {
        for a in old.attributes(AttributeLoc::Param(k)) {
            site.add_attribute(AttributeLoc::Param(k), a);
        }
    }
    match (
        old.try_as_basic_value().left(),
        site.try_as_basic_value().left(),
    ) {
        (Some(BasicValueEnum::IntValue(o)), Some(BasicValueEnum::IntValue(n))) => {
            o.replace_all_uses_with(n)
        }
        (Some(BasicValueEnum::PointerValue(o)), Some(BasicValueEnum::PointerValue(n))) => {
            o.replace_all_uses_with(n)
        }
        (None, None) => {}
        _ => return Err(internal("a padded call changed its result")),
    }
    i.erase_from_basic_block();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::stacked_bytes;
    use inkwell::context::Context;
    use inkwell::types::BasicTypeEnum;
    use inkwell::AddressSpace;

    /// AAPCS's placement, case by case (spec 2026-11-13-elya-boards D4). The
    /// QEMU run of `native_boards` is what checks it against LLVM itself.
    #[test]
    fn aapcs_stacks_what_r0_to_r3_cannot_hold() {
        let ctx = Context::create();
        let w: BasicTypeEnum = ctx.i64_type().into();
        let p: BasicTypeEnum = ctx.ptr_type(AddressSpace::default()).into();
        let b: BasicTypeEnum = ctx.bool_type().into();
        let cases: &[(&[BasicTypeEnum], u32)] = &[
            (&[], 0),
            (&[w, w], 0),
            (&[p, w], 0),       // p r0, (r1 skipped), w r2:r3
            (&[p, w, w], 8),    // `many(fs, i, acc)`
            (&[p, p, p, w], 8), // r0-r2, then w cannot split: stacked
            (&[p, p, p, p, p], 4),
            (&[w, w, b], 4),
            (&[w, w, p, w], 16), // p at 0, w at 8
            (&[w, w, w, w, w], 24),
            (&[p, w, p], 4),
        ];
        for (params, bytes) in cases {
            assert_eq!(stacked_bytes(params), Some(*bytes), "{params:?}");
        }
        assert_eq!(stacked_bytes(&[ctx.f64_type().into()]), None);
    }
}
