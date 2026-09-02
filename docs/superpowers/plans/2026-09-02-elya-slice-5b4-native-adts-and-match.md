# Elya Slice 5b-4 — Native ADTs and Match, the heap threshold — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Teach the native back end to allocate — and to match over — algebraic data types. A constructor lowers to an `elya_alloc` call plus a tag-and-fields store; a `match` lowers to a sequential tag-dispatch chain whose arms join through a single `phi`; and a failed match traps deterministically rather than hitting `unreachable`. ADTs are the heap threshold: the first values that do not fit a register, proven by execution on a construct → match → extract-an-Int corpus.

**Architecture:** The value representation widens from two widths (`Int`→`i64`, `Bool`→`i1`) to three (`Ty::Con(..)`→`ptr`), which forces `lower_expr`'s fold from `IntValue` to `BasicValueEnum` — done first, alone, as 5b-2's Task 3 did for Bool, so the mechanical widening and the interesting ADT work review separately. The Core IR gains `CoreModule.types` and `CoreKind::Ctor` so construction is syntactically distinct from application. Two new runtime symbols — `elya_alloc` and `elya_match_fail`, both `ccc` C-ABI — come from a tiny C file the `clang` link step now compiles and links.

**Tech Stack:** Rust (three-crate workspace: `elya` root / `elya-codegen` / `elya-cli`), inkwell 0.5, LLVM 18.1.6 (vcpkg `x64-windows-static-md-rel`), clang 22.1.8 as link driver + C driver for the runtime.

**Spec:** `docs/superpowers/specs/2026-09-02-elya-slice-5b4-native-adts-and-match-design.md`

## Global Constraints

- **Crate boundary.** LLVM lives *only* in `crates/codegen`; never add `inkwell`/`llvm-sys` to the root `elya` crate or `crates/cli`.
- **`CARGO_INCREMENTAL=0` on every cargo invocation.**
- **`cargo fmt --all` (write mode) before every gate.** The gate fmt-*checks* and fails hard.
- **Gate command:** `powershell -NoProfile -File scripts/check.ps1` — five stages, both configurations. From Cline, run it detached via `scripts/check-bg.ps1` and check `-Status`.
- **Proof is execution.** No `insta` snapshot of LLVM IR anywhere. No test may skip (`#[ignore]`, toolchain-probe early return).
- **Semantic fidelity** (5b-1 §3.4). Native must be neither more- nor less-undefined than the evaluator. A clean `Unsupported` refusal is fidelity-preserving; silently mis-compiling is not. N4's reflex instance: a failed match *traps*, it never emits `unreachable`.
- **Monomorphic-ADT-only.** The corpus uses types with no type parameters. `List(a)` at `List(Int)` is explicitly deferred (field-type substitution is its own mechanism, landing with parametric-ADTs-in-codegen alongside N7). `Nat = Zero | Succ(Nat)` still proves the recursive-occurrence-is-a-pointer fact.
- **Constants pinned by the spec:** the object header is one `i64` tag word = the constructor's index within its type; field words are 8 bytes; allocation size = `1 + arity`.
- **Two runtime symbols, exactly:** `ptr elya_alloc(i64 words)` and `void elya_match_fail(void)`. Both are `ccc` (the C-ABI boundary); every Elya-internal function and call site stays `tailcc`.
- **Existing refusal strings stay byte-for-byte** where unchanged (`"unrepresentable type"`, `"Match"`, `"callee is not a top-level function"`, `"non-Int value"`).

---

## Task 1: Widen the value representation to a pointer (no new capability)

The mechanical center, reviewed alone. `repr_ty` maps `Ty::Con(..)` to a pointer and `lower_expr` returns `BasicValueEnum` rather than `IntValue`, but no Core node can yet produce an ADT value, so nothing new actually compiles — the 5b-1/5b-2/5b-3 corpora are the regression suite, and they must pass **unchanged**.

**Files:** `crates/codegen/src/lib.rs`.

**Produces:**
- `repr_ty` returns `Result<BasicTypeEnum<'ctx>, CodegenError>` (was `Result<IntType>` — a pointer is not an integer type): `Int → i64`, `Bool → i1`, `Ty::Con(..) → ptr`, everything else still `"unrepresentable type"`.
- `lower_expr` returns `Result<BasicValueEnum<'ctx>, CodegenError>` (was `IntValue`); `lower_tail` stays `Result<(), _>`.

- [x] **Step 1: Widen `repr_ty`.** In `crates/codegen/src/lib.rs`, change the signature to `Result<BasicTypeEnum<'ctx>, _>` and add the `Ty::Con(..)` arm (import `inkwell::types::BasicTypeEnum`):

```rust
fn repr_ty<'ctx>(ctx: &'ctx Context, ty: &Ty) -> Result<BasicTypeEnum<'ctx>, CodegenError> {
    match ty {
        Ty::Base(TyCon::Int) => Ok(ctx.i64_type().into()),
        Ty::Base(TyCon::Bool) => Ok(ctx.bool_type().into()),
        // N4 (spec §1): an ADT value is a pointer to its heap object.
        Ty::Con(..) => Ok(ctx.ptr_type(AddressSpace::default()).into()),
        _ => Err(CodegenError::Unsupported("unrepresentable type")),
    }
}
```

- [x] **Step 2: Adapt the signature sites.** `declare_all` and the print shim built function signatures with `repr_ty(..)?.fn_type(..)` (an `IntType` method). Now match the kind: an integer type → `.into_int_type().fn_type(..)`, a pointer type → `.into_pointer_type().fn_type(..)`. For this task nothing is yet a pointer in a signature, so the pointer arm is written but unreached; it compiles and is exercised in Task 4.

- [x] **Step 3: Widen the fold.** Change `lower_expr`'s return type to `Result<BasicValueEnum<'ctx>, CodegenError>`. Existing `IntValue` returns coerce; the `If` arm's `Ok(phi.as_basic_value().into_int_value())` becomes `Ok(phi.as_basic_value())`; the `App` arm's `.map(|v| v.into_int_value())` becomes `.map(|v| v)`.

- [x] **Step 4: Gate.** `cargo fmt --all`, then the full gate (detached). Expected: green, **no new tests** — plumbing only, the three prior corpora unchanged.

- [x] **Step 5: Commit.**

```text
refactor(codegen): widen the value representation to a pointer (5b-4 Task 1)
```

---

## Task 2: The core reach — `types` + `CoreKind::Ctor` + the lowerer

Construction becomes a distinct Core node, and the module carries the ADT declarations the back end will read. Front end only — codegen still refuses `Ctor`/`Match` until Task 4.

**Files:** `src/core.rs`, `tests/core_lowering.rs`.

**Produces:**
- `CoreModule { fns, types }`; `CoreType { name, ctors: Vec<CoreCtor> }`; `CoreCtor { name, fields: Vec<Ty> }`.
- `CoreKind::Ctor(String, Rc<[CoreExpr]>)`.
- `lower_module` reads `Decl::Type`, elaborates field `TypeAnn` → `Ty` (monomorphic), and emits `Ctor` for a constructor application and for a bare nullary constructor (replacing today's `Var`).

**Field-types source (pinned):** `lower_module(&m, &table)` already receives the whole `&Module`. `Decl::Type(TypeDecl { variants })` carries `VariantDecl { name, fields: Vec<Spanned<TypeAnn>> }` — the field types are *annotations*, not expression spans, so a small local elaboration (base name → `Base`, `Upper(name,args)` → `Con`, recursing) is all that is needed. No signature widening, no `Infer`/scheme exposure; monomorphic-only keeps it param-free.

- [x] **Step 1: Add the Core structs.** In `src/core.rs`, above `CoreModule`:

```rust
#[derive(Clone, Debug)]
pub struct CoreType {
    pub name: String,
    /// Constructors in declaration order — the index IS the tag (spec §2.2).
    pub ctors: Vec<CoreCtor>,
}

#[derive(Clone, Debug)]
pub struct CoreCtor {
    pub name: String,
    /// One type per field; len == arity.
    pub fields: Vec<Ty>,
}
```

Add `pub types: Vec<CoreType>` to `CoreModule`, and `CoreKind::Ctor(String, Rc<[CoreExpr]>)` to the enum.

- [x] **Step 2: A local monomorphic `TypeAnn → Ty` helper.** In `src/core.rs`, add `fn ann_to_ty(a: &TypeAnn) -> Option<Ty>` mapping a base name (`Int`/`Bool`/…) to `Ty::Base`, and `Upper(name, args)` to `Ty::Con(name, args)` recursing. This is the param-free slice of `types.rs::elaborate_adt_ty`; no solver, no `Infer`.

- [x] **Step 3: Emit `Ctor` in `lower_expr`.** At the top of `lower_module`, walk `Decl::Type`, collecting a `ctor_names: HashSet<String>` (arity 0 ⇒ a bare `Var` is a nullary constructor) and the `types: Vec<CoreType>` vector. In `lower_expr`, special-case `Expr::Var(name)` where `name ∈ ctor_names` → `CoreKind::Ctor(name, [])`; and `Expr::Call { callee: Var(name), args }` where `name ∈ ctor_names` → `CoreKind::Ctor(name, args)` with the field expressions lowered. The node's `ty` (already `Ty::Con` at that span) is untouched.

- [x] **Step 4: `pretty_typed` + snapshot migration.** Add the `Ctor` arm (and render the `types` vector) to `pretty_typed`. In `tests/core_lowering.rs`, the ADT construction snapshots move from `App(Var("Some"), …)` to `Ctor`; add a monomorphic case, `type Nat = Zero | Succ(Nat)`, asserting `Succ` fields elaborate to `Ty::Con("Nat", [])`.

- [x] **Step 5: Gate + commit.**

```text
feat(core): ADT types and CoreKind::Ctor reach Core (5b-4 Task 2)
```

---

## Task 3: The two-symbol runtime, `elya_alloc` and `elya_match_fail`

The C-ABI boundary: exactly two external symbols, both `ccc`, defined in a tiny C file the link step now compiles. Nothing Elya-internal changes convention — it stays `tailcc`.

**Files:** `crates/codegen/src/runtime.c` (new), `crates/codegen/src/lib.rs`.

**Pinned boundary (spec §2.1, §5):**
- `ptr elya_alloc(i64 words)` — allocates `words * 8` bytes (zeroed), returns the pointer. `void elya_match_fail(void)` — writes a fixed message to stderr and exits non-zero.
- Both are declared in the module with `ccc`; every Elya function/call site is `tailcc`.
- The `link` invocation grows from `clang obj.o -o exe` to `clang obj.o runtime.c -o exe` (clang compiles the C and links it). The runtime source is located relative to the crate, so both `emit_ir`-free builds and the real link find it.

- [x] **Step 1: Write `crates/codegen/src/runtime.c`.** Two functions, `malloc`-backed, no `free` (allocate-don't-collect):

```c
#include <stdint.h>
#include <stdlib.h>
#include <stdio.h>

/* Allocate `words` 8-byte words, zeroed. Spec §2.1. */
void *elya_alloc(int64_t words) {
    return calloc((size_t)words, 8);
}

/* The deterministic failed-match trap. Spec §5: never `unreachable`. */
void elya_match_fail(void) {
    fputs("elya: match failed (no arm matched)\n", stderr);
    exit(1);
}
```

- [x] **Step 2: Declare the two externals.** In `lib.rs`'s `build_module` (next to the `printf` declaration), add both with `ccc` (the default convention — `None`), leaving Elya functions `tailcc`:

```rust
let alloc_ty = ptrt.fn_type(&[i64t.into()], false);
let _alloc = module.add_function("elya_alloc", alloc_ty, None); // ccc
let fail_ty = ctx.void_type().fn_type(&[], false);
let _fail = module.add_function("elya_match_fail", fail_ty, None); // ccc
```

- [x] **Step 3: Grow the `link` invocation.** In `link(obj, exe)`, add the runtime C source before `-o`:

```rust
let runtime = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/runtime.c");
let out = std::process::Command::new("clang")
    .arg(obj)
    .arg(&runtime)
    .arg("-o")
    .arg(exe)
    ...
```

- [x] **Step 4: Gate + commit.** (No new tests — the symbols are declared but not yet called.)

```text
feat(codegen): the two-symbol runtime, elya_alloc and elya_match_fail (5b-4 Task 3)
```

---

## Task 4: Heap construction and the sequential match chain

The capability lands. `Ctor` allocates and stores; `Match` compiles to the tag-dispatch chain with an N-armed `phi` and a trapping default.

**Files:** `crates/codegen/src/lib.rs`, `crates/codegen/tests/native_codegen.rs`.

**Setup (pinned):** at the top of `build_module`, fold `core.types` into a `HashMap<String, (usize /*tag*/, usize /*arity*/, &CoreCtor)>` keyed by constructor name, so a `Ctor`/`Ctor`-pattern lookup is O(1).

- [x] **Step 1: Lower `Ctor`.** In `lower_expr`, add the `CoreKind::Ctor(name, fields)` arm: build the field values first (each `lower_expr`), then

```rust
let (tag, arity, _) = ctor_table[name];
let words = 1 + arity as u64;
let p = b.build_call(alloc, &[i64t.const_int(words, false).into()], "a")
    .map_err(internal)?.try_as_basic_value().left()
    .ok_or(CodegenError::Unsupported("elya_alloc returned no value"))?
    .into_pointer_value();
// store tag at p[0]
let tag_ptr = unsafe { b.build_gep(p, &[i32t.const_int(0,false), i32t.const_int(0,false)], "tagp") }.map_err(internal)?;
b.build_store(tag_ptr, i64t.const_int(tag as u64, false)).map_err(internal)?;
for (i, f) in fields.iter().enumerate() {
    let fv = lowered_field_values[i];        // BasicValueEnum
    let fp = unsafe { b.build_gep(p, &[i32t.const_int(0,false), i32t.const_int((i+1) as u64, false)], "fp") }.map_err(internal)?;
    // Int → store i64; Bool → zero-extend to i64 word; ADT → store the pointer.
    let word = match f.ty {
        Ty::Base(TyCon::Int) => fv.into_int_value(), /* store */
        Ty::Base(TyCon::Bool) => b.build_int_z_extend(fv.into_int_value(), i64t, "zw").map_err(internal)?.into(),
        _ => fv, // pointer
    };
    b.build_store(fp, word).map_err(internal)?;
}
Ok(p.into())
```

- [x] **Step 2: Lower `Match` — the chain.** In `lower_expr`, replace `CoreKind::Match(..) => Err(..("Match"))` with the sequential chain:

1. Lower the scrutinee once; cast its `ptr` to a `*i64` and `load` the tag once.
2. Create one `match_bb` test block per constructor/arm and a single `join` block (a `phi` of the match's result type, which the checker unified across arms).
3. For a constructor arm, compare `tag == index`, branching to the arm body on equal and to the next test on not-equal. In the arm body, `build_gep` each field, `load` it, bind the pattern's variables into a cloned environment, `lower_expr` the body, and `build_unconditional_branch(join)`.
4. A `Wild`/`Var` arm needs no test: it is the chain's terminal arm (always matches; bind and fall into the body).
5. The final not-equal edge falls to a `default` block (§5) that calls `elya_match_fail`.

- [x] **Step 3: The `phi` with N edges (the 5b-2 trap, generalized).** After lowering every arm, the builder sits at the end of each arm's *exit* block, not the join. Read each back with `get_insert_block()` before branching to the join, then at the join:

```rust
let phi = b.build_phi(result_ty, "m")?;
for (v, exit_bb) in arm_results { phi.add_incoming(&[(&v, exit_bb)]); }
```

The arms' results are all the same LLVM type because the checker unified the match's result type; `repr_ty` makes it one `IntType`/`ptr`. The trapping default block contributes **no** incoming edge.

- [x] **Step 4: The default block traps, never UB.** The fall-through block calls `elya_match_fail` (declared `noreturn`), then `build_unreachable`:

```rust
// default block
b.build_call(match_fail, &[], "fail").map_err(internal)?;
b.build_unreachable().map_err(internal)?;
```

The `unreachable` is a terminator placeholder *after* a `noreturn` call — it is guarded, not the UB the §3.4 reflex forbids. The deterministic behaviour is the `exit(1)` inside `elya_match_fail`; that is what runs and what a regression must observe, never a bare `unreachable` alone.

- [x] **Step 5: Extend the test harness.** In `native_codegen.rs`, add an `ADT_CORPUS: &[(&str, &str, &str)]` (construct → match → extract Int) and a direct runner + a differential runner, reusing `lower_src`/`compile_and_link`/`assert_runs`. Cases: option-style (`Some(n)`/`None`), recursive `Nat = Zero | Succ(Nat)`, and a multi-constructor arity-2 type proving the N-armed `phi`.

- [x] **Step 6: Gate + commit.** Full gate green (the 5b-1/2/3 corpora untouched).

```text
feat(codegen): heap construction and the sequential match chain (5b-4 Task 4)
```

---

## Task 5: Close-out — docs, checklist, ledger

The slice is not done when the tests pass; it is done when the documentation stops lying about the subset and the spec/plan checklists reflect what was built.

**Files:** `crates/codegen/src/lib.rs` (smoke corpus), `README.md`, the spec, this plan; ledger in memory (uncommitted).

- [x] **Step 1: Extend the IR smoke corpus.** In `lib.rs`'s `corpus_verifies`, append the ADT programs (option-style, recursive `Nat`, arity-2) alongside the 5b-1/2/3 strings, so `verify()` teeth exist for `Ctor`/`Match` too.
- [x] **Step 2: Patch the README subset paragraph.** Update "As of Slice 5b-3 …" to "As of Slice 5b-4 …" covering ADTs and `match` (construct → match → extract an Int; strings still refused).
- [x] **Step 3: `graphify update .` + `cargo fmt --all` + the full gate** (detached), expected green, exit 0, no `.snap.new`.
- [x] **Step 4: Tick the spec's §9 checklist** (all boxes now hold by test) and **tick every `- [x]` in this plan**.
- [x] **Step 5: Update the slice ledger** in memory: mark Slice 5b-4 closed, note the arc (N1/N3/N2/N4 landed), record obligations T4 (header promise to GC), T5 (sequential chain is a correctness floor), T6 (literal-pattern match unfocused), and name N5 (closures) as next.
- [x] **Step 6: Commit.**

```text
docs(codegen): close out Slice 5b-4 — native ADTs and match
```

---

## Execution approach

1. **Subagent-driven (recommended)** — a fresh subagent per task, review between tasks.
2. **Inline** — execute tasks here with `executing-plans`, pausing at each gate for review.

Which approach?