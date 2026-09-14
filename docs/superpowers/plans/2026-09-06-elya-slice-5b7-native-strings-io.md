# Slice 5b-7 — Native Strings and `io.println` (arc node N6) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Compile Elya string literals to tagged, GC-traced heap blocks, and `io.println` to a native call that writes text to stdout — then hold that text against what the CEK evaluator buffered for the same source, with two negative controls proving the comparison has teeth.

**Architecture:** A string is one flat heap block `[tag][len][bytes…]` allocated by `elya_alloc` and described by one reserved descriptor row (`arity = 0, mask = 0`), so the collector traces it with byte-identical `gc_mark` code. The source bytes live in `.rodata` and are copied into the heap block by `elya_str_lit`; `elya_println` writes `len` bytes plus a newline to **stdout, never stderr**. `repr_ty(Str)` becomes a pointer and `repr_ty(Unit)` becomes `i64` zero — the load-bearing surprise that lets a program *sequence* a `println` at all. `io.println` reaches the back end through one recorder fix in the type checker plus a distinct `CoreKind::Builtin` node (the same "syntactically distinct from application" shape `Ctor` set in 5b-4).

**Tech Stack:** Rust 2021 (workspace `elya` / `elya-codegen` / `elya-cli`), inkwell 0.5 over LLVM 18.1.6, `clang` as link driver and C driver for `crates/codegen/src/runtime.c`, target `x86_64-pc-windows-msvc`.

**Spec:** `docs/superpowers/specs/2026-09-06-elya-slice-5b7-native-strings-io-design.md` (committed at `1578db9`).

> **Provenance and corrections (2026-09-13).** This plan was never committed; the drive holding it
> failed and the copy restored here is a PhotoRec carve of the **pre-correction draft**
> (36,611 B, byte-clean, no corrected copy survived). Three corrections that had been applied to
> the working copy were lost and have been re-derived from the committed source at `1578db9`
> rather than from memory. The draft was internally inconsistent on the load-bearing point:
> *"The comparison rule, locked to spec §7.1"* rests the entire differential split on
> **`require_int` stays — `main` returns Int**, while Task 1's `rejects_concat_by_name` handed
> `main` a `String` body. Task 1 Step 7 is new for the same reason class: Step 1's widening
> relocates an already-committed test's refusal, which the draft never accounted for.

## Global Constraints

- **Crate discipline.** `elya` (repo root `src/`) must stay LLVM-free — configuration A of the gate builds it with no `llvm_sys` anywhere in the graph. All LLVM lives in `crates/codegen`. The recorder fix (Task 4) lands in `src/types.rs`; the `Builtin` node lands in `src/core.rs`; neither may import inkwell.
- **The gate is five stages, run as one command** — `powershell -NoProfile -File scripts/check.ps1`: `cargo fmt --all -- --check` → clippy A (`-p elya -p elya-cli --all-targets -- -D warnings`) → test A (`-p elya -p elya-cli`) → clippy B (`--workspace --all-targets --features elya-cli/codegen -- -D warnings`) → test B (`--workspace --features elya-cli/codegen`). The one definition of the gate is `scripts/check.ps1`.
- **`CARGO_INCREMENTAL=0` for every cargo invocation.** The incremental cache hangs on this machine.
- **Run `cargo fmt --all` in *write* mode before every gate run.** Stage 1 is a `--check` and fails hard on a single stray space.
- **Proof is execution.** No `insta` snapshots of LLVM IR, no `#[ignore]`, no test that asserts on the shape of emitted IR. Every guarantee this slice claims gets a built-and-run proof, and the two differential negative controls are built, run, and shown **failing differently** before their green counts.
- **Never edit a test or an expected value to make something pass**, and **never nudge a constant** (`GC_MAX_WORDS`, `GC_THRESHOLD_WORDS`, `MAX_PARAMS`, `MAX_LAMBDA_PARAMS`) to make a test pass.
- **Atomic commit per task.** Gate first, commit **only if exit 0**, never commit red. **Explicit `git add <paths>`, never `-A`.** Commit trailer `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`. Push after each task commit. Pause and report counts after every task and wait for go-ahead.
- **Single-match anchor guards on every scripted edit** (assert `count == 1`), and make them line-ending agnostic — that guard has caught three real errors in the last two slices. Hand-written Rust edits are guarded by the compiler's exhaustiveness (a new `CoreKind` variant makes every exhaustive `match` fail to build, which *is* the sweep).
- **Long builds are detached.** The gate and any `cargo test`/`cargo build` that links llvm-sys take minutes. From this environment they are started detached via `scripts/check-bg.ps1` (which shells out to `scripts/check.ps1` — one definition of the gate) and checked with `scripts/check-bg.ps1 -Status` on a later turn. A human at their own terminal runs `powershell -NoProfile -File scripts/check.ps1` directly.
- **Elya surface syntax, as the corpus spells it** (`crates/codegen/tests/native_codegen.rs`): block statements and match arms are **whitespace/newline separated, never comma separated**; constructor lists inside a `type` declaration **are** comma separated; boolean literals are `True`/`False`.

## Task 1 — Widen the value representation and re-shield what it un-shadows

`repr_ty` currently refuses everything it does not map to a width; that refusal has been silently protecting downstream sites that are not themselves total (spec §8.1). This task admits `Str` and `Unit`, adds the string's descriptor row, and immediately re-shields the three sites the widening exposes: `<>` (`BinOp::Concat`), a non-`Ty::Con` match scrutinee, and — pinned by test — `==`/`!=` on strings.

**Files:** `crates/codegen/src/lib.rs`, `crates/codegen/src/runtime.c`.

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces:
  - `repr_ty`: `Ty::Base(TyCon::Str) => ptr`, `Ty::Base(TyCon::Unit) => i64`; `Ty::Con(..)` and `Ty::Fn(..)` unchanged.
  - `is_heap_ty`: `matches!(ty, Ty::Con(..) | Ty::Fn(..) | Ty::Base(TyCon::Str))`.
  - One string descriptor row `[arity = 0, mask = 0]` appended after the lambda rows, with a drift guard of the same shape the lambda rows use.
  - The `Prim` arm refuses `BinOp::Concat` by name **before** operand lowering; the `Match` arm refuses a non-`Ty::Con` scrutinee by name.
  - The `runtime.c` comment next to `gc_descriptors` states the widened contract: `arity` is the count of traced-candidate words after the tag, **not** the block size (`gc_sweep` reads `meta`, never the descriptor).

### The sweep, spelled out (spec §8.1)

`repr_ty`'s widening un-shadows every site that dispatched on `Ty` assuming a refusal fired first. Those sites, in full: `repr_ty` (`lib.rs:184`), `is_heap_ty` (`:204`), `require_int` (`:228`), the Eq/Ne operand guard (`:735-739`), and the four word-conversion sites of §8.3 (Task 2). Of the operators whose operands are lowered with `.into_int_value()` at `:740-741`, only `Concat` types a pointer operand (`src/types.rs:796` gives it `String × String → String`); `Eq`/`Ne` are already refused by name at `:735-739`, and every other operator is Int-monomorphic so a `Str`/`Unit` operand cannot reach it. A non-`Ty::Con` scrutinee (`lib.rs:949`) is the remaining `.into_*`-on-value site: Int scrutinees panic today, and a `Str` scrutinee would **not** panic (a string block has a tag word) but would fall through to `elya_match_fail` — wrong behaviour, not a crash. `fn_type_of` (`:216`) and the two `phi` joins (`:877`, `:1083`) already accept `IntType`/`PointerType` and need no change.

### Steps

- [ ] **Step 1: Widen `repr_ty` and `is_heap_ty`.** In `lib.rs:183-193` add two arms before the `_`:

  ```rust
  // N6 (spec §1.1, §3): a string is a pointer to its heap block; Unit is the
  // immediate i64 zero, never dereferenced, never traced, never rooted.
  Ty::Base(TyCon::Str) => Ok(ctx.ptr_type(AddressSpace::default()).into()),
  Ty::Base(TyCon::Unit) => Ok(ctx.i64_type().into()),
  ```

  and widen `is_heap_ty` (`:203-205`):

  ```rust
  fn is_heap_ty(ty: &Ty) -> bool {
      matches!(ty, Ty::Con(..) | Ty::Fn(..) | Ty::Base(TyCon::Str))
  }
  ```

- [ ] **Step 2: The descriptor row + drift guard.** In `build_module`, after the lambda-row loop (`lib.rs:1305`) and before `let n_ctors = (desc.len() / 2) as u64;` (`:1306`):

  ```rust
  // N6 (§1.2): ONE string row — arity 0, mask 0. Nothing after the tag is a
  // heap reference, so the mark phase traces nothing for a string. The tag is
  // this row's index; the guard below makes "tag agrees with its row index" a
  // compile-time property, exactly as the lambda guard above does.
  let string_tag = n_real_ctors + lambdas.len();
  if string_tag != desc.len() / 2 {
      return Err(CodegenError::Unsupported(
          "string tag disagrees with its descriptor row index",
      ));
  }
  desc.push(0); // arity = 0: no traced-candidate words follow the tag
  desc.push(0); // mask = 0
  ```

- [ ] **Step 3: Refuse `<>` by name.** In the `Prim` arm, immediately after the Eq/Ne guard (`lib.rs:735-739`) and before the operand lowering at `:740-741`, add:

  ```rust
  // N6 §8.2: `<>` is String × String → String; its operands are pointers, so
  // `.into_int_value()` at the lines below would PANIC (inkwell, not a Result).
  // Refused by name, reusing op_label's existing "Concat" message.
  if matches!(op, BinOp::Concat) {
      return Err(CodegenError::Unsupported(op_label(*op)));
  }
  ```

- [ ] **Step 4: Refuse a non-`Ty::Con` match scrutinee.** At the top of the `CoreKind::Match` arm (`lib.rs:946`), before the `.into_pointer_value()` at `:949`:

  ```rust
  // N6 §8.4: a scrutinee must be an ADT (a pointer with a real tag word). A
  // non-Con scrutinee panics `.into_pointer_value()` today; a Str scrutinee
  // would load its tag and fall through to elya_match_fail. Refuse by name.
  if !matches!(scrutinee.ty, Ty::Con(..)) {
      return Err(CodegenError::Unsupported("match scrutinee is not an ADT"));
  }
  ```

- [ ] **Step 5: The arity contract comment.** In `runtime.c`, the comment block above `gc_descriptors` (near `runtime.c:19-25`) gains one sentence stating that `arity` is "the count of traced-candidate words following the tag, NOT the block's size" and that `gc_sweep` reads `Block::meta`, never the descriptor.

- [ ] **Step 6: Extend `mask_and_repr_agree_on_pointers`** (`lib.rs:1952-1972`) — add `Ty::Base(TyCon::Str)` (pointer, heap) and `Ty::Base(TyCon::Unit)` (i64, not heap) to the `cases` array. This is the one test that would otherwise let a future `repr_ty` widening drift from the mask.

- [ ] **Step 7: Re-point `rejects_a_string_typed_node_by_name`** (`lib.rs:1690-1712`). Step 1 moves
  this test's refusal, so the test moves with it. This is **not** an edit-to-make-something-pass
  (Global Constraints): the widening relocating this boundary is the specified behaviour, and the
  test's job is to keep pinning whatever the current boundary is, by name.

  Today the `Str`-typed `Let` binding is refused by `repr_ty`'s `_` arm (`:191`) with
  `Unsupported("unrepresentable type")` — the **type** boundary. Once Step 1 gives
  `Ty::Base(TyCon::Str)` a width, `repr_ty` succeeds, the node falls through to the
  `match &e.kind` beneath it, and `CoreKind::Lit(_)` (`:705`) refuses with
  `Unsupported("non-Int literal")` — the **literal** boundary. Same program, same
  refusal-by-name discipline, one boundary further in.

  - Expected value: `Unsupported("unrepresentable type")` → `Unsupported("non-Int literal")`.
  - Rename to `rejects_a_string_literal_until_the_allocator_lands` — the old name asserts a
    *type* refusal that will no longer be the one firing.
  - Doc comment: drop "Str is not one of them, and it is refused as an unrepresentable *type*";
    record instead that `Str` now **has** a width and the unbuilt thing is the literal's
    allocation. Keep the existing contrast against `require_int`'s `"non-Int value"` — that
    distinction survives the move and is still worth pinning.
  - Add: **Task 3 lifts this refusal entirely.**

  Task 3 must delete this test in the same commit that adds the `Lit(CoreLit::Str(v))` arm — a
  test named "until the allocator lands" that outlives the allocator is a lie. Add that deletion
  to Task 3's Step 1 checklist when you get there.

- [ ] **Step 8: Refusal tests** (in `lib.rs` `mod tests`, beside `rejects_a_function_name_in_value_position`):
  - `rejects_concat_by_name` — `emit_ir(&core_of("pub fn main() {\n  let s = \"a\" <> \"b\"\n  1\n}\n"))`
    errors with `Unsupported("Concat")`. **`main` must return Int.** With the concatenation as
    `main`'s own body, `main` types as `String` and `require_int` (`lib.rs:227`) refuses with
    `"non-Int value"` before the `Prim` arm is ever reached — the test would be pinning the
    return-type guard while claiming, by name, to pin the Concat guard. Binding the
    concatenation in a `let` whose body is `1` keeps `require_int` satisfied so the refusal
    under test is the only one that can fire. **Doc comment must state that this test fails on
    the un-refused build with the `Concat` message, not with `"non-Int value"`** — if it ever
    starts reporting the latter, the test has stopped testing anything.
  - `rejects_a_non_adt_match_scrutinee` — `emit_ir(&core_of("pub fn main() { match 1 { _ -> 2 } }"))` errors with `Unsupported("match scrutinee is not an ADT")`.
  - `rejects_eq_on_strings_by_name` — `emit_ir(&core_of("pub fn main() { if \"a\" == \"a\" { 1 } else { 0 } }"))` errors with the existing `eq_operand_label` message (this already fires today; the test pins it against regressions).

  These three fire at `emit_ir`/compile time, so the whole gate (including `cargo test`) is their proof — no execution corpus entry is involved.

- [ ] **Step 9: Gate, then commit.** `cargo fmt --all`, then the full gate. Commit with `git add crates/codegen/src/lib.rs crates/codegen/src/runtime.c`, message `feat(codegen): represent Str as a heap pointer and Unit as i64`, trailer `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`. Push. Pause.

## Task 2 — One word-conversion helper pair, with the `Unit` arm

Spec §8.3 is non-negotiable: the four ad-hoc `match ty { Int => …, Bool => …, _ => ptrtoint/inttoptr }` blocks must collapse into **one pair** of helpers so that ONE negative control covers all four sites. Widening `repr_ty` to admit `Unit` (Task 1) un-shadows exactly these sites: a `Unit` stored as a heap word is an `IntValue`, so the current `_ => build_ptr_to_int(v.into_pointer_value(), …)` **panics on the store side**; on the read side the word is reinterpreted as a pointer. `Str` already lands in `_` and is correct (a string *is* a pointer).

**Files:** `crates/codegen/src/lib.rs`.

**Interfaces:**
- Consumes: the widened `repr_ty`/`is_heap_ty` from Task 1.
- Produces:
  - `fn value_to_word<'ctx>(b, v, ty, i64t) -> Result<IntValue<'ctx>, CodegenError>` — Int/Bool arms lifted verbatim from the four current store sites; a new `Ty::Base(TyCon::Unit) => v.into_int_value()` arm (identity, the value is already the i64 zero); `_ => build_ptr_to_int`.
  - `fn word_to_value<'ctx>(b, w, ty, i64t, ptrt) -> Result<BasicValueEnum<'ctx>, CodegenError>` — Int/Bool arms lifted verbatim from the four current read sites; the symmetric `Unit => w.into()` arm; `_ => build_int_to_ptr`.
  - The four sites (closure-capture read `lib.rs:328-343`, closure-capture store `822-834`, Ctor-field store `929-938`, Ctor-field read `1000-1007`) become one-line calls.

### Why this is provably behavior-preserving at Task 2

The four sites are exercised today by the ADT and closure corpora with `Int`, `Bool`, and pointer (`Ty::Con`/`Ty::Fn`) fields/captures — each arm of the helpers is hit by an existing execution test, so the refactor is proven by the unchanged green run. The two new `Unit` arms are dead until a `Unit` value can be built (§3: only `io.println` produces one), so they cannot fail the gate now; their execution proof is the Unit-field + Unit-capture test in Task 6. The single-match anchor-guarded refactor must leave `mask_and_repr_agree_on_pointers`, the ADT/ADT corpus, and the closure corpus all green.

### Steps

- [ ] **Step 1: Write the helpers** (place near `gc_root`, `lib.rs:395-438`, which already owns `i64t`/`ptrt`/`ctx`-style plumbing). The `Bool` branch needs the bool type — pass `ctx` or a `bool_type` parameter rather than reconstructing it.
- [ ] **Step 2: Splice the four sites** — replace each inline `match` with the matching helper call.
- [ ] **Step 3: Gate, then commit.** `git add crates/codegen/src/lib.rs`, message `refactor(codegen): one word<->value helper pair for the four conversion sites`, trailer `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`. Push. Pause.

## Task 3 — The string-literal arm and `elya_str_lit`

A `CoreKind::Lit(CoreLit::Str(v))` node must become an allocation site: `elya_alloc` can collect mid-evaluation, so the literal arm takes the same `gc_root_env`/`gc_unroot` bracket the `Ctor` arm does (`lib.rs:905-944`). The bytes are a `.rodata` constant global produced by `build_global_string_ptr` (creates a private constant global; NUL-terminated automatically) and are **not** roots — they are not heap objects, and `gc_root`'s `is_pointer_value` check would mis-handle them, which is exactly why §1.5 says "what is heap-allocated is the block, what is static is the source bytes."

**Files:** `crates/codegen/src/runtime.c`, `crates/codegen/src/lib.rs`.

**Interfaces:**
- Consumes: the widened `repr_ty`/`is_heap_ty` + the string descriptor row + `string_tag` from Task 1.
- Produces:
  - `runtime.c`: `void *elya_str_lit(int64_t tag, const char *bytes, int64_t len)` — computes `words = 2 + ((len + 1) + 7) / 8`, calls `elya_alloc`, writes `tag` to `p[0]`, `len` to `p[1]`, `memcpy`s `len` bytes into `p[2]`, relies on `elya_alloc`'s zeroing for the NUL (the `+1` in sizing reserves it even when `len % 8 == 0`). Needs `#include <string.h>`.
  - `LowerCtx`: two new fields — `str_lit: FunctionValue<'ctx>` and `string_tag: usize` (`= n_real_ctors + lambdas.len()`).
  - `build_module`: declares `elya_str_lit` as `ccc` (`ptrt.fn_type(&[i64.into(), ptrt.into(), i64.into()], false)`), stores both new fields into `lc`.
  - `lower_expr`: a new `CoreKind::Lit(CoreLit::Str(v))` arm (before the `Lit(_)` catch-all at `:705`) — `gc_root_env`, `build_global_string_ptr`, `build_call(lc.str_lit, [string_tag, bytes_ptr, len])`, `gc_unroot`, return the pointer.

### Step

- [ ] **Step 1: Write the literal arm, declare the symbol, wire the context.** The `CoreKind::Lit(CoreLit::Str(v))` arm:

  ```rust
  // N6 §1.5/§6.4: a string literal allocates a heap block. The bytes are a
  // .rodata constant global (NOT a root — not a heap object); elya_str_lit copies
  // them into the block. elya_alloc may collect here, so root the env across it,
  // exactly as the Ctor arm does.
  CoreKind::Lit(CoreLit::Str(v)) => {
      let i64t = ctx.i64_type();
      let env_roots = gc_root_env(b, lc, env)?;
      let bytes_ptr = b
          .build_global_string_ptr(v, "cstr")
          .map_err(internal)?;
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
  ```

- [ ] **Step 2: Write `elya_str_lit`** in `runtime.c` (after `elya_alloc`, before `elya_match_fail`). Add `#include <string.h>` at the top.

- [ ] **Step 3: Test.** Add a corpus entry to `native_codegen.rs` (a new `PRINTING_CORPUS`, or extend one):
  - `let s = "hi"` → `pub fn main() { let s = "hi" in 42 }` expected stdout `"42"`. This proves the literal arm allocates, roots, and runs without crashing (content is not yet observable without `println`).

- [ ] **Step 4: Gate, then commit.** `git add crates/codegen/src/lib.rs crates/codegen/src/runtime.c crates/codegen/tests/native_codegen.rs`, message `feat(codegen): string literals are heap blocks via elya_str_lit`, trailer `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`. Push. Pause.

## Task 4 — `io.println` reaches the back end and prints

This is the slice's headline cut (spec §0 item 4, §4–§6.4). It is split across the type checker (one recorder line), Core (one distinct node + lowering + two analysis arms), and the back end (one `CoreKind` arm + one C symbol). The recorder fix mirrors the operation-callee template 20 lines below it (`types.rs:1136-1157`) verbatim — same `node_types.insert`, same `{IO}` closed row, same `callee.span` key.

**Files:** `src/types.rs`, `src/core.rs`, `crates/codegen/src/lib.rs`, `crates/codegen/src/runtime.c`, `crates/codegen/src/closure.rs`, `tests/core_lowering.rs`, `crates/codegen/tests/native_codegen.rs`.

**Interfaces:**
- Consumes: `Str`→pointer/heap (Task 1), the string literal arm + `lc.str_lit`/`lc.string_tag` (Task 3).
- Produces:
  - `types.rs`: exactly one `self.node_types.insert(callee.span, Ty::Fn(vec![Ty::str()], <{IO} closed row>, Box::new(Ty::unit())))` in the `io.println` builtin branch of `infer_call` — inserted after `add_effect`, before `return Ty::unit()`.
  - `core.rs`: `CoreKind::Builtin(String, Rc<[CoreExpr]>)`; the `Expr::Call` arm intercepts `Expr::Qualified { module:"io", name:"println" }` BEFORE its existing Var/ctor path and lowers to `Builtin("io.println", args)`.
  - `core.rs` `pretty_expr`: a `Builtin` rendering arm.
  - `closure.rs`: `CoreKind::Builtin(_, args)` arms in `fv_walk` (71) and `collect_in` (167) — recurse into `args`; the name is not a `Var`, so it is not captured.
  - `tests/core_lowering.rs`: the `walk` helper (35) gains a `Builtin` arm so the test build stays exhaustive.
  - `lib.rs`: `LowerCtx.println` (declared `ccc`); a `CoreKind::Builtin(builtin, args)` arm in `lower_expr` that refuses any name `!= "io.println"` by name, lowers the single arg, calls `lc.println`, and returns `i64 0` (Unit is `repr_ty`'s i64 zero, §3).
  - `runtime.c`: `void elya_println(void *s)` — reads `tag`/length from the block, `fwrite`s `len` bytes to **stdout**, then `fputc('\n', stdout)`. stderr untouched.
  - `native_codegen.rs`: a sibling helper `native_text_value(exe, tag) -> (String, String)` (§7.1 split), leaving `assert_runs` (203-215) untouched.

### The exhaustive-`CoreKind`-match sweep

Adding a variant is the *intended* guard: the compiler fails `crates/codegen/src/lib.rs` (`lower_expr`, `:697`), `crates/codegen/src/closure.rs` (`fv_walk` `:71`, `collect_in` `:167`), `src/core.rs` (`pretty_expr`, `:406`), and `tests/core_lowering.rs` (`walk`, `:35`) — every one of which is exhaustive with no `_ =>`. That is the single-match anchor: each `match` refuses to compile until its `Builtin` arm is added, so "forgot one site" is impossible by construction. The `ClosureExpr` test corpus in `closure.rs::tests` (which exercises `collect_lambdas` and `free_vars` over every `CoreKind`) then proves the two analysis arms are right.

### The recorder fix, exactly (spec §4.2/§4.3)

The builtin branch of `infer_call` (`types.rs:1094-1103`) currently returns `Ty::unit()` without ever recording the callee node, so `node_types[callee.span]` is **absent, not mistyped** (spec §4.1). Insert two lines — build the callee type with `{IO}` in its row, then record it — before the `return`:

```rust
                // §4.2: record the callee node against its span, mirroring the
                // operation-callee branch below — {IO} (not pure) in its row (§4.3).
                let mut io_label = BTreeMap::new();
                io_label.insert(
                    "io".to_string(),
                    EffectLabel { args: Vec::new(), span: callee.span },
                );
                // ... or "IO" per the effect's name as add_effect records it
                self.node_types.insert(
                    callee.span,
                    Ty::Fn(
                        vec![Ty::str()],
                        EffectRow { labels: io_label, tail: RowTail::Closed },
                        Box::new(Ty::unit()),
                    ),
                );
```

(Drop-in on `EffectLabel`/`EffectRow`/`RowTail`/`BTreeMap`, all already in scope from the `1141` precedent.) This is an invariant fix, not an inference change: no new unification, no diagnostic change. Its N6 proof is structural — §4.1's `LowerError::Untyped` failure no longer reaches the Builtin node because the node's call-form intercepts the callee — plus `tests/effect_types.rs:36-41` remaining green (the enclosing scheme still renders `fn() / {IO} -> Unit`).

> **Note for implementer on the effect label name:** `add_effect(amb, "IO", ...)` is what the branch already calls, and `OBSERVABLE_EFFECTS` keys on `"IO"`. The recording here should key the row label on the same string the effect discharge later matches — verify the label matches what `add_effect`/`handle` use, rather than guessing `"io"`. (This is exactly the kind of detail the compiler won't catch; the anchor is `src/types.rs:1136-1157`'s `op_labels.insert`.)

### Steps

- [ ] **Step 1: The recorder fix.** In `types.rs`, inside the `io.println` branch of `infer_call` (`:1094-1102`), after `add_effect` and before `return Ty::unit()`, insert the callee-type recording exactly as shown above.
- [ ] **Step 2: The `Builtin` Core node.** In `src/core.rs`:
  - Add `Builtin(String, Rc<[CoreExpr]>)` to the `CoreKind` enum (`:27-50`), with a doc comment noting it is syntactically distinct from `App` the way `Ctor` is.
  - In `lower_expr`'s `Expr::Call` arm (`:259`), insert the `Expr::Qualified { module, name }` check **before** the `Expr::Var` ctor check, lowering to `CoreKind::Builtin(format!("{module}.{name}"), args)`.
  - Add a `CoreKind::Builtin` arm to `pretty_expr` (`:406`) mirroring `Ctor`.
- [ ] **Step 3: The analysis arms.** In `closure.rs`, add `CoreKind::Builtin(_, args)` arms to `fv_walk` (`:71`) and `collect_in` (`:167`) that recurse into `args`. Add the arm to `tests/core_lowering.rs` `walk` (`:35`). The compiler's exhaustiveness errors on each are the anchor.
- [ ] **Step 4: The codegen arm.** In `lib.rs`, declare `elya_println` as `ccc` in `build_module` and store it in `LowerCtx.println`. Add a `CoreKind::Builtin(builtin, args)` arm to `lower_expr` (after the `Str` literal arm from Task 3):

  ```rust
  CoreKind::Builtin(builtin, args) => {
      // N6 §4.4: NOT App — no closure block, no Ty::Fn value in flight.
      if builtin != "io.println" {
          return Err(CodegenError::Unsupported(builtin));
      }
      if args.len() != 1 {
          return Err(CodegenError::Unsupported("io.println takes exactly one argument"));
      }
      // The arg is a String (a pointer). elya_println allocates nothing, so
      // the argument needs no shadow-stack bracket — only allocation sites do.
      let i64t = ctx.i64_type();
      let s = lower_expr(ctx, func, b, lc, &args[0], env)?.into_pointer_value();
      b.build_call(lc.println, &[s.into()], "pl").map_err(internal)?;
      // io.println returns Unit = i64 zero (§3).
      Ok(i64t.const_int(0, false).into())
  }
  ```

- [ ] **Step 5: `elya_println` in `runtime.c`.** After `elya_gc_report` (end of file), write it reading the length from the block and writing to **stdout** only:

  ```c
  /* N6 §6.2: stdout, never stderr. stderr is the collector's diagnostic
   * channel (elya_gc_report, gated on ELY_GC_STATS); the empty-stderr
   * assertion in assert_runs is what keeps it honest. */
  void elya_println(void *s) {
      int64_t *p = (int64_t *)s;
      int64_t len = p[1];
      const char *bytes = (const char *)&p[2];
      fwrite(bytes, 1, (size_t)len, stdout);
      fputc('\n', stdout);
  }
  ```

- [ ] **Step 6: Tests (prove the end-to-end path).** Add a `PRINTING_CORPUS` and the `native_text_value` sibling helper (§7.1 split) to `native_codegen.rs`:
  - `io.println("hello") 42` → text `"hello\n"`, value `"42"`.
  - `io.println("hello") io.println("world") 7` → text `"hello\nworld\n"`, value `"7"` (§7.3: more than one line).
  - `io.println("") 9` → text `"\n"` (the empty-string case §7.1's `strip_suffix` exists for), value `"9"`.

  Each asserts `(text, value) == expected` and that stderr is empty.

- [ ] **Step 7: Gate, then commit.** `git add src/types.rs src/core.rs crates/codegen/src/lib.rs crates/codegen/src/runtime.c crates/codegen/src/closure.rs tests/core_lowering.rs crates/codegen/tests/native_codegen.rs`, message `feat(codegen): io.println reaches the back end and writes stdout`, trailer `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`. Push. Pause.

## Task 5 — The text-comparing differential and the direction-(b) control

With printing working, the headline proof becomes a comparison: the text the **native** binary writes to stdout (minus the trailing value line) must equal the text the **CEK evaluator** buffered (`Interp::output()`, `src/eval.rs:195`) for the same source. This is the inverse of the empty-string case the split handles.

**Files:** `crates/codegen/tests/native_codegen.rs`.

**Interfaces:**
- Consumes: `native_text_value` from Task 4, the printing corpus.
- Produces:
  - `eval_main_text(src) -> String` — a sibling of `eval_main_int` (416-433) that binds the `Interp` from `run_module_value` (instead of discarding it with `let (_, v) =`) and returns `interp.output()`. It still asserts `main` is an `Int` (so the value line exists to strip): the evaluator prints only via `Interp::println` (`src/eval.rs:201-204`, which buffers), so its `output()` is exactly the native text.
  - `native_output_matches_the_evaluator_across_the_printing_corpus` — the differential loop: for each `PRINTING_CORPUS` entry, compare `native_text_value` against `eval_main_text`; assert stderr is empty.
  - The `direction (b)` execution control — the headline acceptance test (§2, §10 item 3).

### The comparison rule, locked to spec §7.1

`native_text_value` (Task 4) strips exactly one trailing newline (`strip_suffix('\n')`) and `rsplit_once('\n')`s the rest into `(text, value)`. The differential compares `text` to `eval_main_text(src)`. The split is mechanical **because** `require_int` stays (§6.3): `main` returns Int, the shim prints exactly one `%lld` line last — so there is always exactly one value line to peel.

### Direction (b): the hazard with teeth (§2, §10 item 3)

This is the execution proof that `is_heap_ty`'s widening is load-bearing. Build (do not write a test that asserts on survival or exit status) — assert on the **string content read back**:

```
/// N6 §2 / §10.3 — direction (b). A String in an ADT field is traced ONLY by the
/// descriptor mask; with is_heap_ty un-widened the mask bit stays clear and the
/// block is swept while live, then overwritten by the next compatible allocation.
/// The value read back is WRONG (not a crash — the collector recycles, never unmaps).
fn a_string_in_an_adt_field_survives_collection() { ... }
```

The program: declare `type Box { Mk(String) }`, build `Mk("hello")`, force a collection by crossing `GC_THRESHOLD_WORDS` with subsequent allocations (steal the loop idiom from `an_unbounded_allocating_loop_collects_and_frees`, native_codegen.rs:768+ — do **not** lower `GC_THRESHOLD_WORDS`; the constant is pinned), then `match b { Mk(s) -> io.println(s) }` and read the field back. Expected: `"hello"`. The proof runs in three gates: (1) green on this slice's HEAD — string survives; (2) revert **only** `Ty::Base(TyCon::Str)` out of `is_heap_ty`, run, observe the wrong value (a recycled string, not a crash); (3) restore, green again. Step 2 is the negative control that falsifies the "is_heap_ty widening is unnecessary" claim.

### Steps

- [ ] **Step 1: `eval_main_text`.** Mirror `eval_main_int`, but bind `let (interp, v) = elya::eval::run_module_value(&m)` and `return interp.output().to_string();` (still panic if `v` is not `Int`, to keep the value line guaranteed).
- [ ] **Step 2: The differential loop.** `native_output_matches_the_evaluator_across_the_printing_corpus` over `PRINTING_CORPUS`.
- [ ] **Step 3: The direction-(b) test (as above, three gates).**
- [ ] **Step 4: Gate, then commit.** `git add crates/codegen/tests/native_codegen.rs`, message `test(codegen): differential text check vs the evaluator + direction-(b) control`, trailer `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`. Push. Pause.

## Task 6 — Negative controls, the `Unit` word-conversion proofs, and review checks

This task does not add feature code. It proves the slice's guarantees have teeth, in spec §7.2's order: the two differential negative controls are **built, run, and shown failing differently** before the green is counted; the `Unit` arms get their execution proof (which needed `println`, i.e. Task 4); and the two review-only guarantees (`gc_mark` byte-identical, `run_with_gc_stats` consistency) are discharged.

**Files:** `crates/codegen/src/runtime.c`, `crates/codegen/tests/native_codegen.rs`, `crates/codegen/src/lib.rs` (no net change).

### The two negative controls (spec §7.2, §10 item 2)

Both are demonstrated by a one-line sabotage of `elya_println` in `runtime.c`, run, observed, then restored — the differential's green only counts *after* both are shown to fail distinctly.

- [ ] **C5-a — `elya_println` writes nothing.** Sabotage: make `elya_println` a no-op (`(void)s;`). Run the differential. **Expected failure signature:** a **text mismatch** (`"hello\n"` ≠ `""` on the `text` arm) — specifically *not* exit status, specifically *not* the value line. (If it failed on the value line, the §7.1 split is wrong; if on exit status, the test measures the wrong thing.)
- [ ] **C5-b — `elya_println` writes to stderr instead of stdout.** Sabotage: swap `stdout` → `stderr` for the `fwrite`/`fputc`. Run the differential. **Expected failure signature:** `assert_runs`/`native_text_value`'s empty-stderr assertion fires (stdout is empty so the text is `""` *and* stderr is non-empty) — a **distinct** failure from C5-a.
- [ ] **Distinctness check.** C5-a fails on the `text` comparison; C5-b fails on the stderr-is-empty assertion. If both fail the same way, neither is discriminating — do not count green. Restore `elya_println`, run the differential green. No commit of the sabotage (it is demonstrated, never committed).

### The `Unit` word-conversion arm (§8.3, §10 item 8)

The two `Unit` arms the Task 2 helpers gained are dead until a `Unit` value can be built. Now they are exercised, and *only* through the helpers (four inline arms would have left three unproven — spec §8.3):

- [ ] **Unit-typed ADT field.** `type Box { Mk(Unit) }` is not constructible without a `Unit` value, so build it through `io.println`: `let u = io.println("x") in Mk(u)` (or `match` a field). Lowered and run, asserting the `Unit` word read back is `0` (e.g. a field typed `Unit` stored and re-read produces no crash and the surrounding Int result is intact).
- [ ] **Unit-typed closure capture.** `let u = io.println("x") in let f = fn(_) { u } in f(1)` — `u` (Unit) is captured; the capture-store arm converts the i64 zero word without panicking, and the capture-read arm re-reads it without an `inttoptr`. Assert via the program compiling and running.
- [ ] **Sabotage variant (optional, strongest).** Revert **only** the `Unit` arm from `word_to_value`/`value_to_word` (back to the `_ => inttoptr/ptrtoint` path) and show the Unit-field program panics the compiler (`into_pointer_value()` on an `IntValue`) — proving the arm is reached through the shared helpers and that the negative control covers all four sites.

### Review-only checks

- [ ] **§8.5 — `run_with_gc_stats` consistency.** `run_with_gc_stats` (`native_codegen.rs:733-765`) trims `.trim()`s stdout into one string. A printing program run under `ELY_GC_STATS=1` must still split correctly via the §7.1 rule. Add one printing corpus entry under `ELY_GC_STATS` and confirm the counters parse and the text/value split holds. (§7.3 says "do not lower `GC_THRESHOLD_WORDS`" — this gate must not trip the threshold, so keep allocations trivial.)
- [ ] **§9 item 9 — `gc_mark` is byte-identical.** `git diff HEAD~1 -- crates/codegen/src/runtime.c` over the `gc_mark` region (`runtime.c:139-157`) must be empty — a string is a leaf with `arity = 0`, so the mark phase traces nothing for it and gains no second dispatch path. (This is a review check, as in 5b-6: a non-empty diff means the representation is wrong, not that `gc_mark` should be patched.)

- [ ] **Gate, then commit.** `git add crates/codegen/src/runtime.c crates/codegen/tests/native_codegen.rs`, message `test(codegen): negative controls for the string/io differential + Unit conversion proofs`, trailer `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`. Push. Pause.

## Acceptance

The slice is done when all five gate stages are green and these hold, each by execution (item 9 excepted, as a review check).

| Claim | Proof |
|---|---|
| A string is a tagged, traced heap block | direction-(b) test `a_string_in_an_adt_field_survives_collection` — green on HEAD, wrong value with `is_heap_ty` un-widened |
| `io.println` writes its text to stdout | `native_output_matches_the_evaluator_across_the_printing_corpus` (text arm) |
| `io.println` never writes stderr | `assert_runs`/`native_text_value` empty-stderr assertion holds across the printing corpus; C5-b makes it explicit |
| `main` still returns one `%lld` line, last | `require_int` unchanged; §7.1 `strip_suffix` + `rsplit_once` always find it |
| `<>` is refused by name, not a panic | `rejects_concat_by_name` (Task 1) |
| `==`/`!=` on strings refused by name | `rejects_eq_on_strings_by_name` (Task 1) |
| A non-ADT match scrutinee is refused | `rejects_a_non_adt_match_scrutinee` (Task 1) |
| A builtin other than `io.println` is refused | `CoreKind::Builtin` arm refuses by name (Task 4) |
| The tag/row agreement cannot drift | compile-time `Unsupported("string tag disagrees with its descriptor row index")` (Task 1) |
| `repr_ty` and the mask cannot drift | `mask_and_repr_agree_on_pointers` gains `Str` and `Unit` (Task 1) |
| `Unit` is representable as i64 zero; sequencing works | `io.println("hello") 42` etc. sequence a side effect (Tasks 4/5) |
| The four word-conversion sites share one pair of helpers | single refactor of `value_to_word`/`word_to_value`; `Unit`-field and `Unit`-capture execution tests prove both arms (Task 6) |
| `gc_mark` gained no dispatch path | `git diff` over `runtime.c:139-157` is empty (review check, Task 6) |
| `run_with_gc_stats` stays consistent with the split rule | one printing entry under `ELY_GC_STATS=1` parses counters and splits correctly (Task 6) |
| The differential is non-vacuous | C5-a (text mismatch) and C5-b (stderr assertion) demonstrated failing **differently** before the green counts (Task 6) |









