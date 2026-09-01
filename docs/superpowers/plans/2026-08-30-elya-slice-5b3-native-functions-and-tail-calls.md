# Elya Slice 5b-3 — Native Functions, Calls, and Guaranteed Tail Calls — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Teach the native back end to emit every top-level Elya function, call them, and eliminate tail calls under an LLVM-verifier-enforced `musttail`, so that mutual recursion at one million frames compiles to a native binary that exits 0 with the right answer.

**Architecture:** Parameter types reach the back end by recording each `Spanned<Param>`'s type in the existing per-node type table and carrying it into a new `CoreParam { name, ty }` on `CoreFn`. The back end changes from one-function emission to a two-pass scheme — declare every function first (mangled `elya_*`, calling convention `tailcc`), then emit bodies — so mutual recursion resolves without ordering. Emission splits into `lower_expr` (produces a value) and `lower_tail` (emits a terminator); a call reached through `lower_tail` gets `musttail` and is immediately followed by `ret`, which is exactly the shape LLVM's verifier enforces. Five named refusals fence off everything N2 does not cover, the most important being a pre-emission whole-module scan for arity ≥ 6, which prevents an uncatchable LLVM `report_fatal_error` on win64.

**Tech Stack:** Rust (three-crate workspace: `elya` root / `elya-codegen` / `elya-cli`), inkwell 0.5, LLVM 18.1.6 (vcpkg `x64-windows-static-md-rel`), clang 22.1.8 as link driver, insta for front-end snapshots.

**Spec:** `docs/superpowers/specs/2026-08-29-elya-slice-5b3-native-functions-and-tail-calls-design.md`

## Global Constraints

- **Crate boundary.** LLVM lives *only* in `crates/codegen`. Never add `inkwell`/`llvm-sys` to the root `elya` crate or to `crates/cli` — the 3.73 GiB rlib is the reason the split exists.
- **`CARGO_INCREMENTAL=0` on every cargo invocation.** The incremental cache hangs on this machine.
- **`cargo fmt --all` (write mode) before every gate run.** The gate fmt-*checks* and fails hard.
- **Gate command:** `powershell -NoProfile -File scripts/check.ps1` — the five-stage gate, both configurations, default parallelism. `pwsh` is not installed on this machine; use `powershell`.
- **Proof is execution** (5b-1 §0). No `insta` snapshot of LLVM IR anywhere. No test may skip: no `#[ignore]`, no toolchain-probe early return.
- **Semantic fidelity** (5b-1 §3.4, extended 5b-2 §4.1). Native codegen must be neither *more*- nor *less*-undefined than the tree evaluator. A clean `Unsupported` refusal removes a program from the set *both* back ends accept, so a refusal does **not** violate this rule; silently mis-compiling does.
- **Space fidelity is explicitly NOT preserved for non-tail recursion** (spec §6.3, Limitation L1). The differential harness compares *answers, not resource behavior*. Every non-tail-recursive corpus program is therefore deliberately shallow.
- **Elya spells Bool literals `True` / `False`.** Lowercase `true` lexes as an ordinary identifier and dies at E0200 in the front end — a front-end failure that can masquerade as a back-end bug.
- **Constants pinned by the spec, verbatim:** `TAILCC = 18` (llvm/IR/CallingConv.h `Tail`), `MAX_PARAMS = 5` (the flat cap from §2's probe matrix), `STACK_OVERFLOW = 0xC00000FD` (Windows `STATUS_STACK_OVERFLOW`, `-1073741571` as `i32`).
- **The five refusal messages are exact strings and must be reproduced byte-for-byte:** `"function takes more than five parameters"`, `"unrepresentable type"`, `"computed callee"`, `"callee is not a top-level function"`, `"function used as a value"`, plus the pre-existing `"non-Int value"` and the new `"duplicate top-level function"`.
- **Scratch files go in the scratchpad OUTSIDE the repo.** Never create scratch or debug files under a tracked path.
- **`git add <explicit paths>` only.** Never `git add -A` or `git add .`.
- **Commit trailer:** `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`

---

## File Structure

**Front end (root crate `elya`) — Task 1 only:**

- `src/types.rs` — MODIFY. One insert in the SCC body-inference loop inside `infer_all`, recording each parameter's type at its own span. Nothing else in the file changes; the existing single zonk pass at the end of `infer_all` maps over the whole table, so the new entries resolve for free.
- `src/core.rs` — MODIFY. Adds `pub struct CoreParam`; `CoreFn.params` changes from `Rc<[String]>` to `Rc<[CoreParam]>`; `lower_module` looks each parameter's type up by span; `pretty_typed` renders `&param.name`.
- `tests/typed_inference.rs` — MODIFY. Two new non-snapshot assertion tests. Four of its seven snapshots gain exactly one row each.
- `tests/snapshots/typed_inference__surface{1,2,3,6}*.snap` — MODIFY (one added line each; the other three snapshots must NOT change).
- `tests/core_lowering.rs` — MODIFY. Import gains `TyCon`; one new test asserting a lowered parameter carries its inferred type.

**Back end (`crates/codegen`) — Tasks 2-4:**

- `crates/codegen/src/lib.rs` — MODIFY, heavily. This is the single file that owns Core → LLVM. It gains `mangle`, `TAILCC`, `MAX_PARAMS`, `find_main`, `declare_all`, `emit_body`, `build_elya_call`, and (in Task 4) `lower_tail`; `build_module` is restructured into the two-pass scheme; the inline `mod tests` gains the refusal suite and loses one now-false test.
- `crates/codegen/tests/native_codegen.rs` — MODIFY. The execution proof. Gains `FUNCTION_CORPUS` (six programs), `TAIL_CORPUS` (two programs), the `STACK_OVERFLOW` diagnosis, and the runners that put both corpora through the direct and the differential harness.

The back end stays in one file on purpose: it is ~600 lines, every function in it is part of one pipeline, and the codebase's established shape is one module per compiler stage. Splitting it would scatter a single traversal across files without giving any piece an independent interface.

**Close-out — Task 5:**

- `README.md` — MODIFY (the "subset the backend covers" paragraph).
- `docs/superpowers/specs/2026-08-29-elya-slice-5b3-native-functions-and-tail-calls-design.md` — MODIFY (tick §9's checklist, annotating deviations inline).
- This plan — MODIFY (tick every checkbox).

---

## Task 1: Parameter types reach Core

The back end cannot declare an LLVM function type without knowing each parameter's type. Inference already computes them; it just never records them anywhere the Core lowering can see. This task closes that gap end to end, front-end only — no LLVM code changes.

**Files:**
- Modify: `src/types.rs` — the SCC body-inference loop inside `infer_all` (~line 1905)
- Modify: `src/core.rs` — `CoreFn` (~line 70), `lower_module` (~line 92), `pretty_typed` (~line 239)
- Modify: `crates/codegen/src/lib.rs` — the inline test `rejects_parameterised_main_specifically`
- Test: `tests/typed_inference.rs` (two new tests), `tests/core_lowering.rs` (one new test)
- Modify: `tests/snapshots/typed_inference__surface1_monomorphic_fn.snap`, `…__surface2_polymorphic_fn_has_var_node.snap`, `…__surface3_use_site_instantiation.snap`, `…__surface6_match.snap`

**Interfaces:**
- **Produces**, for Tasks 2-4:
  - `pub struct CoreParam { pub name: String, pub ty: elya::types::Ty }` in `elya::core`, deriving `Clone, Debug`.
  - `CoreFn { pub name: String, pub params: std::rc::Rc<[CoreParam]>, pub body: CoreExpr }`. **There is no return-type field:** `body.ty` *is* the return type, because `lower_block` propagates the block's type onto the synthesized `Let` spine. A second field would be a second source of truth.
  - The invariant that every `Spanned<Param>` span appears in the map returned by `elya::types::infer_typed_table`, carrying a zonked `Ty`.
- **Consumes:** nothing.

**Deliberate asymmetry (spec §3.2, obligation T3):** `CoreKind::Lambda` keeps `Rc<[String]>`. Lambda parameters are *not* recorded, because N2 refuses lambdas outright (§5.3) and N5 (closures) is where that gap gets closed. Do not "fix" it here.

- [x] **Step 1: Write the two failing front-end tests**

Append to `tests/typed_inference.rs`. The file already imports `parse_module`, `Span`, `infer_with_types`, and `Session`, and already defines `is_var`; only `elya::ast::Decl` is new, and it is used inline via its full path so no import line changes.

```rust
// --- Slice 5b-3 §3.2: parameter spans carry their types ----------------------
// These are assertions, not snapshots. The snapshots below prove the table
// *renders* right; these prove the specific key the Core lowering will look up
// is present and resolved. That is the invariant `lower_module` depends on.

#[test]
fn parameter_spans_carry_their_zonked_types() {
    let src = "fn add1(n) { n + 1 }\n";
    let (m, pd) = parse_module(&Session::new(), src);
    assert!(pd.is_empty(), "parse: {pd:?}");
    let (diags, table) = infer_with_types(&Session::new(), &m);
    assert!(diags.is_empty(), "type errors: {diags:?}");
    let elya::ast::Decl::Fn(f) = &m.decls[0].node else {
        panic!("expected a fn decl")
    };
    let p = &f.params[0];
    assert_eq!(&src[p.span.start as usize..p.span.end as usize], "n");
    assert_eq!(
        table.get(&p.span).map(String::as_str),
        Some("Int"),
        "parameter span carries no type: {table:?}"
    );
}

#[test]
fn a_polymorphic_parameter_span_is_a_var_and_survives_zonking() {
    // The zonk half of the invariant. Types are recorded PRE-zonk (record-then-
    // zonk, spec §3); the single pass at the end of `infer_all` maps over the
    // WHOLE table, so a parameter entry resolves like any other. A leaked
    // internal token or an unresolved var would show up here.
    let src = "fn id(x) { x }\n";
    let (m, pd) = parse_module(&Session::new(), src);
    assert!(pd.is_empty(), "parse: {pd:?}");
    let (diags, table) = infer_with_types(&Session::new(), &m);
    assert!(diags.is_empty(), "type errors: {diags:?}");
    let elya::ast::Decl::Fn(f) = &m.decls[0].node else {
        panic!("expected a fn decl")
    };
    let p = &f.params[0];
    let rendered = table.get(&p.span).expect("parameter span carries no type");
    assert!(is_var(rendered), "expected a type variable, got {rendered}");
    // The parameter and the body are the SAME variable, so the same letter must
    // render for both — the cross-node coherence property, at a param span.
    let same: Vec<_> = table.values().filter(|v| *v == rendered).collect();
    assert!(same.len() >= 2, "param and body should share one var: {table:?}");
}
```

- [x] **Step 2: Run the two tests to verify they fail**

```powershell
$env:CARGO_INCREMENTAL="0"; cargo test -p elya --test typed_inference parameter_spans
$env:CARGO_INCREMENTAL="0"; cargo test -p elya --test typed_inference a_polymorphic_parameter_span
```
Expected: both FAIL — `parameter span carries no type` (the span is not a key in the table).

- [x] **Step 3: Record parameter types in `infer_all`**

In `src/types.rs`, inside `infer_all`'s step 2 loop ("infer each body under its params + ambient"), add one insert. Match on this exact existing text:

```rust
            for (p, pty) in f.params.iter().zip(&params) {
                env.insert(
                    &p.node.name,
```

Replace with:

```rust
            for (p, pty) in f.params.iter().zip(&params) {
                // Slice 5b-3 §3.2: record the parameter's type at its OWN span,
                // so `lower_module` can look it up the same way it looks up an
                // expression's. The precedent for recording a non-expression
                // span is the callee-span insert in the `Call` arm. Recorded
                // pre-zonk like every other entry — the single zonk pass at the
                // end of this function maps over the whole table, so these
                // resolve for free.
                inf.node_types.insert(p.span, pty.clone());
                env.insert(
                    &p.node.name,
```

Nothing else in `src/types.rs` changes. In particular, do **not** touch the zonk pass: it already iterates the entire `inf.node_types` map.

- [x] **Step 4: Run the two tests to verify they pass, then the whole target**

```powershell
$env:CARGO_INCREMENTAL="0"; cargo test -p elya --test typed_inference parameter_spans
$env:CARGO_INCREMENTAL="0"; cargo test -p elya --test typed_inference a_polymorphic_parameter_span
```
Expected: both PASS.

```powershell
$env:CARGO_INCREMENTAL="0"; cargo test -p elya --test typed_inference
```
Expected: **exactly four snapshot failures** — `surface1_monomorphic_fn`, `surface2_polymorphic_fn_has_var_node`, `surface3_use_site_instantiation`, `surface6_match`. These four have functions with parameters; `surface4`, `surface5`, and `surface7` do not (surface4's `worker`/`use_it` take none and effect-op params never enter this loop; surface5's `demo()` takes none and its lambda's `x` is deliberately unrecorded, T3).

If any *other* snapshot fails, stop — something beyond the intended change moved.

- [x] **Step 5: Inspect the four pending snapshots by eye**

```powershell
Get-ChildItem tests\snapshots\*.snap.new | ForEach-Object { "`n--- $($_.Name) ---"; Get-Content $_.FullName }
```

Expected: each file is its existing content plus **exactly one** added row, sorted first (a parameter's span precedes its body's):

| file | added row |
|---|---|
| `typed_inference__surface1_monomorphic_fn.snap.new` | ``8..9 `n` : Int`` |
| `typed_inference__surface2_polymorphic_fn_has_var_node.snap.new` | ``6..7 `x` : a`` |
| `typed_inference__surface3_use_site_instantiation.snap.new` | ``6..7 `x` : a`` |
| `typed_inference__surface6_match.snap.new` | ``38..39 `o` : Option(Int)`` |

**Check the variable letters, not just the row count.** `infer_with_types` renders the whole table through ONE shared `Names`, iterating in span order, so a new key sorting first could in principle shift which letter a variable gets. In surface2 and surface3 the parameter is the *same* variable as the already-rendered body node, so no new letter is introduced and none shift — the existing `x : a` rows must still read `a`. If any pre-existing row's letter changed, stop and report it; that is a real behavioral change, not a mechanical one.

- [x] **Step 6: Accept the four snapshots**

Overwrite each `.snap` with its inspected `.snap.new`. Explicit paths, no wildcard, so nothing unreviewed is accepted:

```powershell
Move-Item -Force tests\snapshots\typed_inference__surface1_monomorphic_fn.snap.new tests\snapshots\typed_inference__surface1_monomorphic_fn.snap
Move-Item -Force tests\snapshots\typed_inference__surface2_polymorphic_fn_has_var_node.snap.new tests\snapshots\typed_inference__surface2_polymorphic_fn_has_var_node.snap
Move-Item -Force tests\snapshots\typed_inference__surface3_use_site_instantiation.snap.new tests\snapshots\typed_inference__surface3_use_site_instantiation.snap
Move-Item -Force tests\snapshots\typed_inference__surface6_match.snap.new tests\snapshots\typed_inference__surface6_match.snap
```

Then audit the diff — it must be four files, one added line each, zero removed lines:

```powershell
git diff --stat tests/snapshots/
git diff tests/snapshots/
git status --porcelain tests/snapshots/
```

The last command must show no remaining `.snap.new` entries. Re-run the target to confirm green:

```powershell
$env:CARGO_INCREMENTAL="0"; cargo test -p elya --test typed_inference
```
Expected: PASS.

- [x] **Step 7: Write the failing Core test**

In `tests/core_lowering.rs`, extend the import — `TyCon` is not currently imported:

```rust
use elya::types::{infer_typed_table, Ty, TyCon, TyPrinter};
```

and append:

```rust
// --- Slice 5b-3 §3.3: CoreFn carries its signature --------------------------

#[test]
fn core_parameters_carry_their_inferred_types() {
    // The back end declares an LLVM function type from these. A parameter that
    // reached Core as a bare name would leave the back end guessing.
    let src = "fn add3(x) { x + 3 }\npub fn main() { add3(4) }\n";
    let (core, _table) = lower_src(src);
    let add3 = core
        .fns
        .iter()
        .find(|f| f.name == "add3")
        .expect("add3 lowered");
    assert_eq!(add3.params.len(), 1);
    assert_eq!(add3.params[0].name, "x");
    assert!(
        matches!(add3.params[0].ty, Ty::Base(TyCon::Int)),
        "{:?}",
        add3.params[0].ty
    );
    // The body's root type IS the return type (§3.3) — there is no separate
    // field that could disagree with it.
    assert!(
        matches!(add3.body.ty, Ty::Base(TyCon::Int)),
        "{:?}",
        add3.body.ty
    );
}
```

- [x] **Step 8: Run it to verify it fails**

```powershell
$env:CARGO_INCREMENTAL="0"; cargo test -p elya --test core_lowering core_parameters_carry
```
Expected: FAIL to compile — `no field 'name' on type '&String'` (and `no field 'ty'`), because `CoreFn.params` is still `Rc<[String]>`.

- [x] **Step 9: Add `CoreParam` and carry types through `lower_module`**

In `src/core.rs`, replace the `CoreFn` declaration:

```rust
/// One function parameter, carrying the type inference recorded at its span
/// (Slice 5b-3 §3.3). A named struct rather than a `(String, Ty)` pair because
/// the affine work (4d-2) identified a parameter multiplicity annotation as a
/// plausible future field — a tuple would have to be rewritten to grow one.
#[derive(Clone, Debug)]
pub struct CoreParam {
    pub name: String,
    pub ty: Ty,
}

#[derive(Clone, Debug)]
pub struct CoreFn {
    pub name: String,
    pub params: Rc<[CoreParam]>,
    /// The body's root type IS the return type: `lower_block` propagates the
    /// block type onto the synthesized `Let` spine, so a separate `ret` field
    /// would be a second source of truth (Slice 5b-3 §3.3).
    pub body: CoreExpr,
}
```

(Keep whatever derives and doc comment `CoreFn` already carries above; only the `params` field type and the added `CoreParam` struct are new.)

Replace the body of `lower_module`:

```rust
pub fn lower_module(module: &Module, table: &BTreeMap<Span, Ty>) -> Result<CoreModule, LowerError> {
    let mut fns = Vec::new();
    for d in &module.decls {
        if let Decl::Fn(f) = &d.node {
            let mut params = Vec::with_capacity(f.params.len());
            for p in &f.params {
                // The same tripwire `lower_expr` uses for expression spans: an
                // unrecorded span is a recorder-totality bug, surfaced by name
                // rather than papered over with a fresh variable.
                let ty = table
                    .get(&p.span)
                    .cloned()
                    .ok_or(LowerError::Untyped(p.span))?;
                params.push(CoreParam {
                    name: p.node.name.clone(),
                    ty,
                });
            }
            let body = lower_block(&f.body.node, table)?;
            fns.push(CoreFn {
                name: f.name.clone(),
                params: params.into(),
                body,
            });
        }
    }
    Ok(CoreModule { fns })
}
```

In `pretty_typed`, the parameter-rendering loop pushes each `&String` directly. Change that one line:

```rust
            s.push_str(&param.name);
```

This keeps every `core_lowering__*` snapshot byte-identical: they render params as bare names, e.g. `(fn id (x) (var x : a))`.

Finally, fix the one inline back-end test that constructs params by hand. In `crates/codegen/src/lib.rs`, in `rejects_parameterised_main_specifically`, replace:

```rust
        m.fns[0].params = Rc::from(["x".to_string()]);
```

with:

```rust
        m.fns[0].params = Rc::from([CoreParam {
            name: "x".into(),
            ty: Ty::Base(TyCon::Int),
        }]);
```

and add `CoreParam` to the crate's `elya::core` import list:

```rust
use elya::core::{CoreExpr, CoreFn, CoreKind, CoreLit, CoreModule, CoreParam};
```

`main_fn`'s `CoreModule { fns: vec![CoreFn { …, params: Rc::from([]), … }] }` compiles unchanged — the empty array's element type is inferred from the field.

- [x] **Step 10: Run the affected targets to verify they pass**

```powershell
$env:CARGO_INCREMENTAL="0"; cargo test -p elya --test core_lowering
$env:CARGO_INCREMENTAL="0"; cargo test -p elya --test typed_table
$env:CARGO_INCREMENTAL="0"; cargo test -p elya --lib
$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen
```
Expected: all PASS, with **no snapshot changes in `core_lowering__*`**. Confirm:

```powershell
git status --porcelain tests/snapshots/
```
Expected: only the four `typed_inference__*` files from Step 6 are modified.

- [x] **Step 11: Format and run the full gate**

```powershell
$env:CARGO_INCREMENTAL="0"; cargo fmt --all
powershell -NoProfile -File scripts/check.ps1
```
Expected: green.

- [x] **Step 12: Commit**

```bash
git add src/types.rs src/core.rs tests/typed_inference.rs tests/core_lowering.rs tests/snapshots/typed_inference__surface1_monomorphic_fn.snap tests/snapshots/typed_inference__surface2_polymorphic_fn_has_var_node.snap tests/snapshots/typed_inference__surface3_use_site_instantiation.snap tests/snapshots/typed_inference__surface6_match.snap crates/codegen/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(core): parameter types reach Core as CoreParam (5b-3 Task 1)

Inference records each parameter's type at its own Spanned<Param> span, so
lower_module can look it up the way it looks up an expression's. CoreFn.params
becomes Rc<[CoreParam]>; the body's root type stays the sole return type.

Four typed_inference snapshots gain one row each — the parameter span, sorted
first. No variable letters shift: in the polymorphic surfaces the parameter is
the same variable as the body node already rendered.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Multi-function emission — declare, then emit

The back end stops being a one-function compiler. Two passes, because mutual recursion means a body can call a function whose body has not been emitted yet. Calls are ordinary (`tailcc` convention, no tail-call kind) — `musttail` waits for Task 4, so this task's corpus is deliberately free of deep recursion.

**Files:**
- Modify: `crates/codegen/src/lib.rs` — module doc; imports; new `mangle`/`TAILCC`/`MAX_PARAMS`/`find_main`/`declare_all`/`emit_body`/`build_elya_call`; `validate_module` removed; `build_module` restructured; `lower_expr` threads `decls` and gains its `App` arm; the inline `mod tests` loses one test, gains two, and extracts a helper
- Modify: `crates/codegen/tests/native_codegen.rs` — `FUNCTION_CORPUS`, two runners, one scoping comment

**Interfaces:**
- **Consumes** (Task 1): `elya::core::CoreParam { name, ty }`; `CoreFn.params: Rc<[CoreParam]>`; `CoreFn.body.ty` as the return type.
- **Produces**, for Tasks 3-4:
  - `fn mangle(name: &str) -> String` — `format!("elya_{name}")`
  - `const TAILCC: u32 = 18;`, `const MAX_PARAMS: usize = 5;`
  - `fn find_main(core: &CoreModule) -> Result<&CoreFn, CodegenError>`
  - `fn declare_all<'ctx>(ctx, module, core) -> Result<HashMap<String, FunctionValue<'ctx>>, CodegenError>` — keyed by the **unmangled** Elya name
  - `fn emit_body<'ctx>(ctx, b, decls, f: &CoreFn) -> Result<(), CodegenError>`
  - `fn build_elya_call<'ctx>(ctx, func, b, decls, callee: &CoreExpr, args: &[CoreExpr], env) -> Result<CallSiteValue<'ctx>, CodegenError>`
  - `lower_expr` gains a `decls: &HashMap<String, FunctionValue<'ctx>>` parameter, positioned after `b`
  - In `crates/codegen/src/lib.rs`'s `mod tests`: `fn core_of(src: &str) -> elya::core::CoreModule`
  - In `crates/codegen/tests/native_codegen.rs`: `const FUNCTION_CORPUS: &[(&str, &str, &str)]` — `(tag, source, expected stdout)`, the same 3-tuple shape as `CONTROL_FLOW_CORPUS`

- [x] **Step 1: Write the failing execution test**

In `crates/codegen/tests/native_codegen.rs`, add the corpus and its two runners. Place `FUNCTION_CORPUS` next to `CONTROL_FLOW_CORPUS`.

```rust
/// The 5b-3 §7.1 corpus, non-tail half: (tag, source, expected stdout). Six
/// programs, each aimed at one thing multi-function emission can get wrong.
/// The two deep tail-recursive programs live in TAIL_CORPUS (Task 4) because
/// they only pass once `musttail` is emitted.
const FUNCTION_CORPUS: &[(&str, &str, &str)] = &[
    (
        "two_functions",
        "fn add3(x) { x + 3 }\npub fn main() { add3(4) }\n",
        "7",
    ),
    (
        "five_params",
        "fn add5(a, b, c, d, e) { a + b + c + d + e }\npub fn main() { add5(1, 2, 3, 4, 5) }\n",
        "15",
    ),
    (
        // Environments are per-function: both `f` and `g` bind a parameter
        // named `x`, and `f` shadows its own with a `let`. If the value
        // environment leaked across the call, or the shadow were not restored,
        // this prints something other than 50. f(10) = 10*2 + 20 = 40; g(10) =
        // 10 + f(10) = 50.
        "distinct_envs",
        "fn f(x) {\n  let x = x * 2\n  x + 20\n}\nfn g(x) { x + f(x) }\npub fn main() { g(10) }\n",
        "50",
    ),
    (
        // Calls as operands, including a call whose argument is a call.
        // dbl(dbl(3)) + dbl(1) = 12 + 2 = 14.
        "call_in_operand_position",
        "fn dbl(x) { x * 2 }\npub fn main() { dbl(dbl(3)) + dbl(1) }\n",
        "14",
    ),
    (
        // An i1 crosses the call boundary and is consumed as an `if`
        // condition. Also §5.5's live proof that `require_int` applies to
        // `main` ALONE: `is_pos` returns Bool and must compile.
        "bool_across_a_call",
        "fn is_pos(n) { n > 0 }\npub fn main() { if is_pos(3) { 1 } else { 0 } }\n",
        "1",
    ),
    (
        // Ordinary (non-tail) recursion. Deliberately shallow — spec §6.3
        // Limitation L1: native non-tail recursion grows the machine stack,
        // which is bounded differently from the evaluator's Kont stack, and the
        // harness compares answers, not resource behavior. sum(100) = 5050.
        "shallow_non_tail_recursion",
        "fn sum(n) { if n == 0 { 0 } else { n + sum(n - 1) } }\npub fn main() { sum(100) }\n",
        "5050",
    ),
];

#[test]
fn the_function_corpus_compiles_runs_and_prints_the_expected_answer() {
    let dir = temp_dir("functions");
    for (tag, src, expected) in FUNCTION_CORPUS {
        let core = lower_src(src);
        let exe = compile_and_link(&core, &dir, tag);
        assert_runs(&exe, expected);
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn native_output_matches_the_evaluator_across_the_function_corpus() {
    // The fidelity teeth, extended to N2. The expected strings above are a
    // human's arithmetic; this asserts against what the CEK evaluator actually
    // computes, so a wrong expectation cannot make a wrong compiler look right.
    let dir = temp_dir("differential-functions");
    for (tag, src, _) in FUNCTION_CORPUS {
        let core = lower_src(src);
        let exe = compile_and_link(&core, &dir, tag);
        let out = Command::new(&exe).output().expect("run produced binary");
        assert!(out.status.success(), "{tag}: binary exited {:?}", out.status);
        let native = String::from_utf8_lossy(&out.stdout).trim().to_string();
        assert_eq!(
            native,
            eval_main_int(src),
            "{tag}: native output diverges from the evaluator"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_function_corpus_lowers_to_multi_function_core() {
    // Spec §7.2's recorder-totality assertion for the new key class, stated
    // directly: `lower_src` unwraps `lower_module`, so a `LowerError::Untyped`
    // from a parameter span fails here by name. Also the §7.3 counterpart to
    // `corpus_lowers_to_core_through_the_real_pipeline`, which stays scoped to
    // the single-function CORPUS.
    for (tag, src, _) in FUNCTION_CORPUS {
        let core = lower_src(src);
        assert!(core.fns.iter().any(|f| f.name == "main"), "{tag}: no main");
        for f in &core.fns {
            assert!(
                f.params.len() <= 5,
                "{tag}: {} exceeds the arity cap",
                f.name
            );
        }
    }
    let multi = lower_src(FUNCTION_CORPUS[0].1);
    assert_eq!(multi.fns.len(), 2, "two_functions must lower to two Core fns");
}
```

Add the §7.3 scoping comment to the existing test. In `corpus_lowers_to_core_through_the_real_pipeline`, immediately above its `for` loop, insert:

```rust
    // Slice 5b-3 §7.3: the `core.fns.len() == 1` assertion below is scoped to
    // CORPUS on purpose — the 5b-1 arithmetic programs are single-function and
    // stay that way. Multi-function lowering is asserted by
    // `the_function_corpus_lowers_to_multi_function_core`.
```

- [x] **Step 2: Run it to verify it fails**

```powershell
$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen --test native_codegen the_function_corpus_compiles
```
Expected: FAIL — `Unsupported("multi-function module")` from `validate_module`.

- [x] **Step 3: Restructure the back end into two passes**

All in `crates/codegen/src/lib.rs`.

**3a. Module doc comment.** Its last line currently claims one function. Replace:

```rust
//! comparisons as signed `icmp`, and `&&`/`||` as bit-wise `and`/`or` on i1.
//! Two value widths (i64, i1), one function (`@elya_main`), no effects.
```

with:

```rust
//! comparisons as signed `icmp`, and `&&`/`||` as bit-wise `and`/`or` on i1.
//! Slice 5b-3 adds functions: every top-level fn is emitted, mangled `elya_*`
//! and carrying `tailcc`, in two passes so mutual recursion resolves without
//! ordering. Two value widths (i64, i1), no closures, no effects.
```

**3b. Imports.** Add to the inkwell import block:

```rust
use inkwell::types::BasicMetadataTypeEnum;
use inkwell::values::BasicMetadataValueEnum;
use inkwell::values::CallSiteValue;
```

**3c. Constants and the mangler.** Add near the top of the file, beside the existing helpers:

```rust
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
```

**3d. Replace `validate_module` with `find_main`.** `main` is no longer "the only function there is"; it is one function among many, distinguished only by name.

```rust
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
```

**3e. Pass 1 — declare.**

```rust
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
        let fn_ty = repr_ty(ctx, &f.body.ty)?.fn_type(&params, false);
        let func = module.add_function(&mangle(&f.name), fn_ty, None);
        // NOTE the plural: `set_call_conventions` is the FunctionValue method.
        // The call-site method is `set_call_convention`, singular. Both are
        // needed and they must agree, or the module is wrong.
        func.set_call_conventions(TAILCC);
        decls.insert(f.name.clone(), func);
    }
    Ok(decls)
}
```

**3f. Pass 2 — emit bodies.**

```rust
/// Pass 2 of §4.2. The value environment starts empty and is seeded from the
/// LLVM parameters, so each function gets its own — nothing leaks across a
/// call, which is what `distinct_envs` pins.
fn emit_body<'ctx>(
    ctx: &'ctx Context,
    b: &Builder<'ctx>,
    decls: &HashMap<String, FunctionValue<'ctx>>,
    f: &CoreFn,
) -> Result<(), CodegenError> {
    let func = *decls
        .get(&f.name)
        .ok_or(CodegenError::Unsupported("undeclared function"))?;
    let entry = ctx.append_basic_block(func, "entry");
    b.position_at_end(entry);
    let mut env: HashMap<String, IntValue<'ctx>> = HashMap::new();
    for (i, p) in f.params.iter().enumerate() {
        let v = func
            .get_nth_param(i as u32)
            .ok_or_else(|| internal("declared arity disagrees with Core arity"))?;
        env.insert(p.name.clone(), v.into_int_value());
    }
    let result = lower_expr(ctx, func, b, decls, &f.body, &mut env)?;
    b.build_return(Some(&result)).map_err(internal)?;
    Ok(())
}
```

**3g. The call builder.**

```rust
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
    decls: &HashMap<String, FunctionValue<'ctx>>,
    callee: &CoreExpr,
    args: &[CoreExpr],
    env: &mut HashMap<String, IntValue<'ctx>>,
) -> Result<CallSiteValue<'ctx>, CodegenError> {
    let CoreKind::Var(name) = &callee.kind else {
        return Err(CodegenError::Unsupported("computed callee"));
    };
    let target = *decls
        .get(name)
        .ok_or(CodegenError::Unsupported("callee is not a top-level function"))?;
    let mut vals: Vec<BasicMetadataValueEnum<'ctx>> = Vec::with_capacity(args.len());
    for a in args.iter() {
        // Left to right, matching the evaluator's argument order.
        vals.push(lower_expr(ctx, func, b, decls, a, env)?.into());
    }
    let site = b.build_call(target, &vals, "c").map_err(internal)?;
    // Singular here (CallSiteValue), plural on the declaration (FunctionValue).
    site.set_call_convention(TAILCC);
    Ok(site)
}
```

**3h. Thread `decls` through `lower_expr` and give it the `App` and improved `Var` arms.**

The signature gains one parameter, after `b`:

```rust
fn lower_expr<'ctx>(
    ctx: &'ctx Context,
    func: FunctionValue<'ctx>,
    b: &Builder<'ctx>,
    decls: &HashMap<String, FunctionValue<'ctx>>,
    e: &CoreExpr,
    env: &mut HashMap<String, IntValue<'ctx>>,
) -> Result<IntValue<'ctx>, CodegenError> {
```

Every recursive `lower_expr(ctx, func, b, …)` call inside the function becomes `lower_expr(ctx, func, b, decls, …)` — mechanical, in the `Let`, `Prim`, and `If` arms. Then replace the `Var` arm:

```rust
        CoreKind::Var(x) => match env.get(x) {
            Some(v) => Ok(*v),
            // §5.4's companion: a bare top-level function name in value
            // position. Legal Elya (`let g = worker  g()` type-checks — see
            // typed_inference's surface4), and it is exactly what N5's closures
            // will make representable. Until then it gets its own message
            // rather than the generic "unbound var", because "unbound" would
            // point the reader at name resolution instead of at this gap.
            None if decls.contains_key(x) => {
                Err(CodegenError::Unsupported("function used as a value"))
            }
            None => Err(CodegenError::Unsupported("unbound var")),
        },
```

and replace the `CoreKind::App(..) => Err(CodegenError::Unsupported("App"))` arm:

```rust
        CoreKind::App(callee, args) => {
            // Ordinary (non-tail) position: `tailcc` convention, NO tail-call
            // kind. Task 4 adds the tail-position path.
            let site = build_elya_call(ctx, func, b, decls, callee, args, env)?;
            site.try_as_basic_value()
                .left()
                .map(|v| v.into_int_value())
                .ok_or(CodegenError::Unsupported("call returned no value"))
        }
```

**3i. `build_module`.**

```rust
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
    let module = ctx.create_module("elya");

    // §4.2: declare everything, then emit every body.
    let decls = declare_all(ctx, &module, core)?;
    let b = ctx.create_builder();
    for f in &core.fns {
        emit_body(ctx, &b, &decls, f)?;
    }
    let elya_main = *decls
        .get("main")
        .ok_or(CodegenError::Unsupported("no `main`"))?;

    // §3.5/§4: the print convention is ONE external symbol (printf) plus ONE
    // generated shim (@main). The shim stays `ccc` — it is the C entry point —
    // and its call to @elya_main is an ordinary call.
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
```

- [x] **Step 4: Update the inline test module**

In `crates/codegen/src/lib.rs`'s `mod tests`:

**4a. Delete `rejects_multi_function_module_specifically` entirely.** N2 legalises multi-function modules; the message it asserts no longer exists. This is a removal, not a migration — there is nothing for it to become.

**4b. Extract `core_of` from `corpus_verifies`.** Replace the body of `corpus_verifies`'s `for` loop with a call, and add the helper above it:

```rust
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
```

```rust
        for src in corpus {
            emit_ir(&core_of(src)).expect("verifier-clean IR");
        }
```

**4c. Add two tests pinning the new module shape:**

```rust
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
            matches!(err, CodegenError::Unsupported("duplicate top-level function")),
            "{err:?}"
        );
    }
```

- [x] **Step 5: Run the back-end tests to verify they pass**

```powershell
$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen --lib
$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen --test native_codegen
```
Expected: PASS, including all four 5b-1 tests and all seven 5b-2 control-flow tests **unchanged** — per §7.3, any edit to those corpora is a regression, not a migration.

- [x] **Step 6: Format and run the full gate**

```powershell
$env:CARGO_INCREMENTAL="0"; cargo fmt --all
powershell -NoProfile -File scripts/check.ps1
```
Expected: green.

- [x] **Step 7: Commit**

```bash
git add crates/codegen/src/lib.rs crates/codegen/tests/native_codegen.rs
git commit -m "$(cat <<'EOF'
feat(codegen): multi-function emission, mangled and tailcc (5b-3 Task 2)

Two passes: declare every function first (elya_* mangled, calling convention
tailcc), then emit bodies, so mutual recursion resolves without ordering.
Calls are ordinary here — musttail lands in Task 4.

The print shim stays ccc, but its call to @elya_main now sets tailcc explicitly:
a site whose convention disagrees with its callee's declaration is a miscompile
LLVM will happily emit, not a verifier error.

Six corpus programs run through both the direct and the differential harness.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: The refusal set

Five refusals fence off what N2 does not cover. Every one is a *clean* refusal, so it removes a program from the set both back ends accept — it does not violate §3.4's fidelity rule. Four of the five are witnessed by real Elya programs through the full front end; only the one the type checker makes unreachable is hand-built.

**Files:**
- Modify: `crates/codegen/src/lib.rs` — the inline `mod tests` only. No production code changes: every refusal is already implemented by Task 2's structure. This task is the proof that each one actually fires, with the right message, on the right witness.

**Interfaces:**
- **Consumes** (Task 2): `core_of`, `main_fn`, `int_lit`, `emit_ir`, `MAX_PARAMS`, and the exact refusal strings.
- **Produces:** nothing new. This task adds tests only.

**Why a task and not a step:** these are the boundary of the buildable subset. A reviewer can reject the boundary while accepting the emission scheme, which is exactly the line a task should be drawn on.

- [x] **Step 1: Write the five failing refusal tests**

Append to `crates/codegen/src/lib.rs`'s `mod tests`.

```rust
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
```

- [x] **Step 2: Run them**

```powershell
$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen --lib
```

Expected: PASS. Task 2's structure already implements all five, so these should be green on the first run — that is the point of writing them as a separate, reviewable task rather than folding them into Task 2's implementation steps.

**If any test fails, the fix is in Task 2's code, not in the test.** The two likely failures and their causes:

- `rejects_a_computed_callee_specifically` reports `Unsupported("Lambda")` → `build_elya_call` is not being reached before `lower_expr`'s `Lambda` arm. Check that `lower_expr`'s `App` arm delegates to `build_elya_call` and does not lower the callee as an expression first.
- `rejects_six_parameters_before_emission_begins` reports `Unsupported("unrepresentable type")` or hard-aborts the test process → the arity scan is not the first statement in `build_module`. Move it above `find_main`.

- [x] **Step 3: Format and run the full gate**

```powershell
$env:CARGO_INCREMENTAL="0"; cargo fmt --all
powershell -NoProfile -File scripts/check.ps1
```
Expected: green.

- [x] **Step 4: Commit**

```bash
git add crates/codegen/src/lib.rs
git commit -m "$(cat <<'EOF'
test(codegen): the five N2 refusals, each on its own witness (5b-3 Task 3)

Arity >5 (pre-emission whole-module scan, because the failure it prevents is an
uncatchable LLVM report_fatal_error), polymorphic function, computed callee,
non-top-level callee, and a function name in value position. Four are witnessed
by real Elya programs through the front end; only the one the type checker makes
unreachable is hand-built.

require_int is pinned to main alone, with a Bool-returning helper compiling
alongside it — the scope narrowing §5.5 names as the likely mis-implementation.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Tail position and `musttail` — the slice's headline

Emission splits in two. `lower_expr` produces a value; `lower_tail` emits a terminator. A call reached through `lower_tail` gets `musttail` and is immediately followed by `ret`, which is the exact shape LLVM's verifier enforces. The acceptance criterion is black-box: `ev`/`od` at one million exits 0 with the right answer.

**Files:**
- Modify: `crates/codegen/src/lib.rs` — new `lower_tail`; `emit_body`'s ending; one import
- Modify: `crates/codegen/tests/native_codegen.rs` — `STACK_OVERFLOW`, `diagnose_stack_overflow`, `TAIL_CORPUS`, three runners

**Interfaces:**
- **Consumes** (Task 2): `build_elya_call`, `lower_expr` with its `decls` parameter, `emit_body`, `TAILCC`.
- **Produces:**
  - `fn lower_tail<'ctx>(ctx, func, b, decls, e: &CoreExpr, env) -> Result<(), CodegenError>` — same parameter list as `lower_expr`, but returns `()` and guarantees the current block is terminated on `Ok`.
  - In `crates/codegen/tests/native_codegen.rs`: `const STACK_OVERFLOW: i32`, `fn diagnose_stack_overflow(status: &std::process::ExitStatus, exe: &Path)`, `const TAIL_CORPUS: &[(&str, &str, &str)]`.

**Why `musttail` holds by construction (§4.4), so a failure here is a real bug and not luck:** return types match, because inference unified each tail expression's type with the function's return type and `repr_ty` is a function of the Elya type; conventions match, because everything is `tailcc`; the call is immediately followed by `ret`, because `lower_tail`'s `App` arm emits them adjacently; differing signatures are fine, because §2's probe modules D and J showed `tailcc` + `musttail` is arity- and type-blind at the verifier; and the argument area cannot grow past the win64 gap, because Task 3's `MAX_PARAMS` scan already ran.

- [x] **Step 1: Write the failing tail-call tests**

In `crates/codegen/tests/native_codegen.rs`, add the failure signal, the corpus, and the runners. Put `STACK_OVERFLOW` and `diagnose_stack_overflow` next to `assert_runs`.

```rust
/// Windows STATUS_STACK_OVERFLOW. Observing it from a corpus binary means one
/// thing: a call that must have been eliminated was not. Inert on other
/// platforms, where no exit code collides with it.
const STACK_OVERFLOW: i32 = 0xC00000FDu32 as i32; // -1073741571

/// Spec §7.2: the distinct failure signal, diagnosed BY NAME. Without this, a
/// tail-call regression surfaces as `binary exited ExitStatus(3221225725)` —
/// a number nobody recognizes — instead of naming its own cause.
fn diagnose_stack_overflow(status: &std::process::ExitStatus, tag: &str) {
    if status.code() == Some(STACK_OVERFLOW) {
        panic!(
            "{tag}: STATUS_STACK_OVERFLOW (0x{:08X}). A tail call that `musttail` \
             was supposed to eliminate grew the machine stack instead. This is the \
             tail-call guarantee failing, not a generic crash.",
            STACK_OVERFLOW as u32
        );
    }
}
```

Insert the diagnosis into `assert_runs`, **immediately after the process runs and before the success assertion** — otherwise the generic assertion fires first and the named message never prints:

```rust
fn assert_runs(exe: &Path, expected: &str) {
    let out = Command::new(exe).output().expect("run produced binary");
    diagnose_stack_overflow(&out.status, &exe.display().to_string());
    assert!(out.status.success(), "binary exited {:?}", out.status);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), expected);
    assert!(
        String::from_utf8_lossy(&out.stderr).is_empty(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
```

Add the same line to each of the three differential runners (`…across_the_control_flow_corpus`, `…also_covers_the_arithmetic_corpus`, `…across_the_function_corpus`), immediately after their `let out = Command::new(&exe).output()…;` and before their `assert!(out.status.success(), …)`:

```rust
        diagnose_stack_overflow(&out.status, tag);
```

Then the corpus and its runners:

```rust
/// The 5b-3 §7.1 corpus, tail half: (tag, source, expected stdout). These two
/// are the reason this slice exists. Both recur one million deep; without
/// `musttail` both overflow the 1 MiB Windows stack long before returning.
const TAIL_CORPUS: &[(&str, &str, &str)] = &[
    (
        "deep_self_tail_recursion",
        "fn down(n) { if n == 0 { 0 } else { down(n - 1) } }\npub fn main() { down(1000000) }\n",
        "0",
    ),
    (
        // `main`'s call to `ev` sits in `if`-condition position, so it is an
        // ordinary non-tail call — one frame, which is fine. The million-deep
        // recursion is the ev<->od pair, and both of those calls are in tail
        // position. The `if` wrapper is what keeps `main` Int-returning, which
        // §5.5's `require_int` demands while `ev` itself returns Bool.
        "deep_mutual_tail_recursion",
        "fn ev(n) { if n == 0 { True } else { od(n - 1) } }\n\
         fn od(n) { if n == 0 { False } else { ev(n - 1) } }\n\
         pub fn main() { if ev(1000000) { 1 } else { 0 } }\n",
        "1",
    ),
];

#[test]
fn mutual_tail_recursion_at_one_million_is_eliminated() {
    // THE primary acceptance criterion for this slice.
    //
    // Self-recursion is NOT sufficient evidence. A self-call can be turned into
    // a branch back to the entry block, so `down(1000000)` could pass with no
    // tail-call machinery at all — the compiler would have proved something
    // weaker than what we claim. `ev` and `od` cannot be looped without merging
    // the two functions, so only the mutual case proves that `musttail`, and
    // not an accidental loop rewrite, is what bounds the stack.
    //
    // The proof is black-box: the binary exits 0 and prints the right answer.
    // The distinct failure signal is STATUS_STACK_OVERFLOW, diagnosed by name
    // inside `assert_runs`.
    let (tag, src, expected) = TAIL_CORPUS[1];
    assert_eq!(tag, "deep_mutual_tail_recursion", "TAIL_CORPUS was reordered");
    let dir = temp_dir("mutual-tail");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, tag);
    assert_runs(&exe, expected);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn self_tail_recursion_at_one_million_is_eliminated() {
    // The corroborating case, weaker on its own than the mutual one above but
    // cheap and a useful bisection point: if this passes and the mutual test
    // fails, the two-pass declaration scheme is what broke, not `musttail`.
    let (tag, src, expected) = TAIL_CORPUS[0];
    assert_eq!(tag, "deep_self_tail_recursion", "TAIL_CORPUS was reordered");
    let dir = temp_dir("self-tail");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, tag);
    assert_runs(&exe, expected);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn native_output_matches_the_evaluator_across_the_tail_corpus() {
    // Both bounds, on the same two programs (spec §6.2). `elya run` bounds these
    // by TCE in the CEK machine, enforced at test time by tests/tce.rs's K_MAX;
    // native bounds them by `musttail`, enforced at build time by the LLVM
    // verifier. This asserts the two agree on the ANSWER — §6.3's L1 is explicit
    // that the harness does not compare resource behavior.
    let dir = temp_dir("differential-tail");
    for (tag, src, _) in TAIL_CORPUS {
        let core = lower_src(src);
        let exe = compile_and_link(&core, &dir, tag);
        let out = Command::new(&exe).output().expect("run produced binary");
        diagnose_stack_overflow(&out.status, tag);
        assert!(out.status.success(), "{tag}: binary exited {:?}", out.status);
        let native = String::from_utf8_lossy(&out.stdout).trim().to_string();
        assert_eq!(
            native,
            eval_main_int(src),
            "{tag}: native output diverges from the evaluator"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}
```

- [x] **Step 2: Run the primary test to verify it fails**

```powershell
$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen --test native_codegen mutual_tail_recursion_at_one_million
```
Expected: FAIL with the named diagnosis — `deep_mutual_tail_recursion: STATUS_STACK_OVERFLOW (0xC00000FD). A tail call that 'musttail' was supposed to eliminate grew the machine stack instead.` Task 2 emits ordinary `tailcc` calls; one million of them overflow.

This is the good failure: it proves the failure signal itself works before the fix makes it unreachable.

- [x] **Step 3: Add `lower_tail`**

In `crates/codegen/src/lib.rs`, add the import:

```rust
use inkwell::values::LLVMTailCallKind;
```

and add `lower_tail` beside `lower_expr`:

```rust
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
    decls: &HashMap<String, FunctionValue<'ctx>>,
    e: &CoreExpr,
    env: &mut HashMap<String, IntValue<'ctx>>,
) -> Result<(), CodegenError> {
    match &e.kind {
        CoreKind::If(cond, then_e, else_e) => {
            // NO join block and NO `phi`. Each arm terminates itself, so a tail
            // call inside an arm is immediately followed by its own `ret` —
            // which a join block would break by inserting a branch between them.
            // The `phi` diamond in `lower_expr` stays exercised by the corpus
            // programs whose `if`s sit in `let`-value position.
            let c = lower_expr(ctx, func, b, decls, cond, env)?;
            if c.get_type().get_bit_width() != 1 {
                return Err(CodegenError::Unsupported("non-Bool if condition"));
            }
            let then_bb = ctx.append_basic_block(func, "then");
            let else_bb = ctx.append_basic_block(func, "else");
            b.build_conditional_branch(c, then_bb, else_bb)
                .map_err(internal)?;
            b.position_at_end(then_bb);
            lower_tail(ctx, func, b, decls, then_e, env)?;
            // Position explicitly rather than assuming where the recursive call
            // left the builder: a nested tail `if` leaves it in ITS else block.
            b.position_at_end(else_bb);
            lower_tail(ctx, func, b, decls, else_e, env)?;
            Ok(())
        }
        CoreKind::Let(x, rhs, body) => {
            // The bound value is NOT in tail position; only the body is.
            let v = lower_expr(ctx, func, b, decls, rhs, env)?;
            let prev = env.insert(x.clone(), v);
            let out = lower_tail(ctx, func, b, decls, body, env);
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
            let site = build_elya_call(ctx, func, b, decls, callee, args, env)?;
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
            let v = lower_expr(ctx, func, b, decls, e, env)?;
            b.build_return(Some(&v)).map_err(internal)?;
            Ok(())
        }
    }
}
```

Then change `emit_body`'s last two statements. Replace:

```rust
    let result = lower_expr(ctx, func, b, decls, &f.body, &mut env)?;
    b.build_return(Some(&result)).map_err(internal)?;
    Ok(())
```

with:

```rust
    // Every function body is in tail position by definition; `lower_tail` emits
    // the terminator, so there is no `build_return` here any more.
    lower_tail(ctx, func, b, decls, &f.body, &mut env)
```

- [x] **Step 4: Run the tail tests to verify they pass**

```powershell
$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen --test native_codegen mutual_tail_recursion_at_one_million
$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen --test native_codegen self_tail_recursion_at_one_million
$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen --test native_codegen native_output_matches_the_evaluator_across_the_tail_corpus
```
Expected: all PASS.

- [x] **Step 5: Run every back-end test — routing `main` through `lower_tail` changes existing IR**

```powershell
$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen
```
Expected: PASS, including all 5b-1 and 5b-2 programs unchanged.

**What to expect if something breaks here.** Routing `main`'s body through `lower_tail` means the top-level `if` in several 5b-2 corpus programs no longer emits a `phi` — the two arms now return directly. That is a deliberate coverage shift, not a regression: the `phi` diamond stays exercised by `nested_if` and `predicates`, whose `if`s sit in `let`-value position and therefore still go through `lower_expr`. Since no test snapshots IR, this change is invisible to the suite except as output, which must not move. If a 5b-2 program's output *does* move, the bug is in `lower_tail`'s `If` arm — most likely a missing `position_at_end(else_bb)` after the then-arm recursion.

- [x] **Step 6: Format and run the full gate**

```powershell
$env:CARGO_INCREMENTAL="0"; cargo fmt --all
powershell -NoProfile -File scripts/check.ps1
```
Expected: green.

- [x] **Step 7: Commit**

```bash
git add crates/codegen/src/lib.rs crates/codegen/tests/native_codegen.rs
git commit -m "$(cat <<'EOF'
feat(codegen): guaranteed tail calls via musttail (5b-3 Task 4)

Emission splits: lower_expr produces a value, lower_tail emits a terminator. A
tail-position `if` emits no join block and no phi, so a tail call is immediately
followed by its own ret — the adjacency musttail requires, made structural.

The primary acceptance criterion is the MUTUAL case, not self-recursion: a
self-call can be faked by a branch back to the entry block, but ev/od cannot be
looped without merging them. Both run 1e6 deep and exit 0 with the right answer.

STATUS_STACK_OVERFLOW (0xC00000FD) is diagnosed by name in every runner, so a
tail-call regression names its own cause instead of surfacing as exit 3221225725.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Close-out

The slice is not done when the tests pass; it is done when the documentation stops lying about the subset, the spec's checklist reflects what was actually built, and the graph is current.

**Files:**
- Modify: `crates/codegen/src/lib.rs` — `corpus_verifies`'s corpus and its now-false comment
- Modify: `README.md` — the backend-subset paragraph
- Modify: `docs/superpowers/specs/2026-08-29-elya-slice-5b3-native-functions-and-tail-calls-design.md` — §9's checklist
- Modify: this plan — every checkbox
- Modify: `C:\Users\elakk\.claude\projects\e--Programming-language-with-a-toolchain\memory\next-slice-decision.md` (outside the repo; not committed)

**Interfaces:**
- **Consumes:** `core_of` (Task 2), `FUNCTION_CORPUS` and `TAIL_CORPUS` (Tasks 2 and 4).
- **Produces:** nothing consumed by later tasks.

- [x] **Step 1: Extend the IR smoke corpus**

The Layer-1 structural check in `crates/codegen/src/lib.rs`'s `corpus_verifies` should cover the new programs, as it did in 5b-2. Its comment currently says "eleven lines of duplication"; with eight more programs that is false.

Replace:

```rust
        // The corpus strings duplicate tests/native_codegen.rs's CORPUS and
        // CONTROL_FLOW_CORPUS because integration targets cannot share consts;
        // eleven lines of duplication is cheaper than new plumbing.
```

with:

```rust
        // The corpus strings duplicate tests/native_codegen.rs's CORPUS,
        // CONTROL_FLOW_CORPUS, FUNCTION_CORPUS, and TAIL_CORPUS because
        // integration targets cannot share consts; nineteen lines of
        // duplication is cheaper than new plumbing.
```

and append these eight entries to the `corpus` array, after the eleven that are there:

```rust
            "fn add3(x) { x + 3 }\npub fn main() { add3(4) }\n",
            "fn add5(a, b, c, d, e) { a + b + c + d + e }\npub fn main() { add5(1, 2, 3, 4, 5) }\n",
            "fn f(x) {\n  let x = x * 2\n  x + 20\n}\nfn g(x) { x + f(x) }\npub fn main() { g(10) }\n",
            "fn dbl(x) { x * 2 }\npub fn main() { dbl(dbl(3)) + dbl(1) }\n",
            "fn is_pos(n) { n > 0 }\npub fn main() { if is_pos(3) { 1 } else { 0 } }\n",
            "fn sum(n) { if n == 0 { 0 } else { n + sum(n - 1) } }\npub fn main() { sum(100) }\n",
            "fn down(n) { if n == 0 { 0 } else { down(n - 1) } }\npub fn main() { down(1000000) }\n",
            "fn ev(n) { if n == 0 { True } else { od(n - 1) } }\nfn od(n) { if n == 0 { False } else { ev(n - 1) } }\npub fn main() { if ev(1000000) { 1 } else { 0 } }\n",
```

Run it:

```powershell
$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen --lib corpus_verifies
```
Expected: PASS. This builds IR only — it does not run the two million-deep programs, so it stays fast.

- [x] **Step 2: Patch the README's subset paragraph**

Write this to the scratchpad (NOT under a tracked path) and run it. The `patch` helper fails hard on an unexpected occurrence count, so a stale README cannot be silently half-edited.

Save as `%TEMP%\claude\e--Programming-language-with-a-toolchain\39600ed7-02d6-4c25-b849-bafba4fb8980\scratchpad\t5_closeout.py`:

```python
# -*- coding: utf-8 -*-
import io, sys

def patch(path, old, new, count=1):
    s = io.open(path, encoding='utf-8', newline='').read()
    n = s.count(old)
    if n != count:
        sys.exit("FAIL %s: expected %d occurrence(s), found %d of:\n%s" % (path, count, n, old))
    io.open(path, 'w', encoding='utf-8', newline='').write(s.replace(old, new, count))
    print("ok:", path)

ROOT = "e:/Programming language with a toolchain/"
README = ROOT + "README.md"

patch(README, u'''Output defaults to the input stem plus the platform executable suffix; `-o <out>`
overrides it. As of Slice 5b-2 the backend covers arithmetic and control flow
(`Int` and `Bool`, `let`, `+ - *`, the six comparisons, strict `&&`/`||`, and
`if`/`else`), so anything outside it is rejected by name rather than
mis-compiled \u2014 the tree-walking `elya run` remains the full language. Every
compiled program in the test corpus is additionally checked against what the
evaluator computes, so the two never drift apart silently.''',
u'''Output defaults to the input stem plus the platform executable suffix; `-o <out>`
overrides it. As of Slice 5b-3 the backend covers arithmetic, control flow, and
top-level functions (`Int` and `Bool`, `let`, `+ - *`, the six comparisons,
strict `&&`/`||`, `if`/`else`, and calls to named functions of up to five
parameters), so anything outside it is rejected by name rather than
mis-compiled \u2014 the tree-walking `elya run` remains the full language. Tail
calls are eliminated under a guarantee the LLVM verifier enforces, so mutually
recursive functions recur to any depth in a compiled binary just as they do
under `elya run`. Every compiled program in the test corpus is additionally
checked against what the evaluator computes, so the two never drift apart
silently.''')
```

Run it:

```powershell
python "$env:TEMP\claude\e--Programming-language-with-a-toolchain\39600ed7-02d6-4c25-b849-bafba4fb8980\scratchpad\t5_closeout.py"
git diff README.md
```
Expected: `ok: <README path>`, and a diff touching only that paragraph.

- [x] **Step 3: Run `graphify update` and the full gate**

```powershell
graphify update .
$env:CARGO_INCREMENTAL="0"; cargo fmt --all
powershell -NoProfile -File scripts/check.ps1
```
Expected: gate green, both configurations, default parallelism.

- [x] **Step 4: Tick the spec's §9 checklist**

Walk §9's nineteen boxes one at a time against what actually landed. Change each `- [ ]` to `- [x]`, and where the implementation deviated from the spec's letter, annotate the deviation **inline on that box** and record it in this plan's "Deliberate deviations" section below. Do not tick a box you cannot point at a test for.

The mapping from box to evidence:

| §9 box | evidence |
|---|---|
| Parameter types recorded and survive zonking | `parameter_spans_carry_their_zonked_types`, `a_polymorphic_parameter_span_is_a_var_and_survives_zonking` |
| `CoreParam { name, ty }`, no return-type field | `core_parameters_carry_their_inferred_types` |
| Missing parameter type is `Untyped`, absent on the corpus | `the_function_corpus_lowers_to_multi_function_core` (unwraps `lower_module`) |
| Mangled `elya_*`, `tailcc` | `declare_all`; proven by every binary linking and running |
| Two-pass, mutual recursion resolves, duplicates refused | `mutual_tail_recursion_at_one_million_is_eliminated`, `rejects_a_duplicate_top_level_function_specifically` |
| Tail position threaded | `lower_tail` exists; the whole tail corpus |
| Tail `If` emits no join block and no `phi` | `lower_tail`'s `If` arm; 5b-2 outputs unchanged |
| `musttail` + `tailcc` on tail calls, `tailcc` only otherwise | the tail corpus passes; `module.verify()` would reject a mismatch |
| ≥6 params refused before emission | `rejects_six_parameters_before_emission_begins` |
| Polymorphic function refuses the module | `a_polymorphic_function_refuses_the_whole_module` |
| Non-`Var` callee refused, immediately-invoked lambda witness | `rejects_a_computed_callee_specifically` |
| Function name in value position, own message | `rejects_a_function_name_in_value_position` |
| `require_int` on `main` alone; Bool helper compiles | `a_bool_returning_helper_compiles_but_a_bool_main_still_refuses`, `bool_across_a_call` |
| All eight corpus programs, both harnesses | the six FUNCTION_CORPUS + two TAIL_CORPUS runners, direct and differential |
| Mutual recursion at 1e6 exits 0 with the right answer | `mutual_tail_recursion_at_one_million_is_eliminated` |
| `STATUS_STACK_OVERFLOW` diagnosed by name | `diagnose_stack_overflow`, called from `assert_runs` and all four differential runners |
| 5b-1 and 5b-2 corpora pass unchanged | `git diff` shows no edit to `CORPUS` or `CONTROL_FLOW_CORPUS` |
| No IR snapshots; no test skips | `git grep -n "insta\|#\[ignore\]" crates/codegen` returns nothing |
| Full gate green, both configurations | Step 3 |

Verify the last two mechanically:

```powershell
git diff HEAD~4 -- crates/codegen/tests/native_codegen.rs | Select-String -Pattern "^-.*CORPUS|^-\s+\(\"(spine|lets|nesting|negative|if_true|if_false)"
```
Expected: no output — nothing was removed from the older corpora.

```powershell
git grep -n "ignore\]" -- crates/codegen src tests
git grep -n "assert_snapshot" -- crates/codegen
```
Expected: no output from either.

- [x] **Step 5: Tick every checkbox in this plan**

Append to `t5_closeout.py` and re-run only this part (or run it as a second script):

```python
PLAN = ROOT + "docs/superpowers/plans/2026-08-30-elya-slice-5b3-native-functions-and-tail-calls.md"
s = io.open(PLAN, encoding='utf-8', newline='').read()
n = s.count("- [x] ")
if n == 0:
    sys.exit("FAIL: plan has no unticked checkboxes; already run?")
io.open(PLAN, 'w', encoding='utf-8', newline='').write(s.replace("- [x] ", "- [x] "))
print("ok: ticked %d checkboxes in the plan" % n)
```

- [x] **Step 6: Update the slice ledger in memory**

Update `C:\Users\elakk\.claude\projects\e--Programming-language-with-a-toolchain\memory\next-slice-decision.md`: mark Slice 5b-3 CLOSED with its commit, state what it bought (native mutual recursion at any depth, guaranteed by the LLVM verifier rather than by hope), record obligations T1 (the ≤5 arity cap), T2 (whole-module representability), and T3 (lambda params carry no types) as still open, and record L1 as a named limitation rather than a bug. Name N4 (ADTs + Match, the heap threshold) as the next node and note that the sequencing call is the user's. This file is outside the repo and is not committed.

- [x] **Step 7: Commit**

```bash
git add crates/codegen/src/lib.rs README.md docs/superpowers/specs/2026-08-29-elya-slice-5b3-native-functions-and-tail-calls-design.md docs/superpowers/plans/2026-08-30-elya-slice-5b3-native-functions-and-tail-calls.md
git commit -m "$(cat <<'EOF'
docs(codegen): close out Slice 5b-3 — native functions and tail calls

README describes the subset the backend now actually covers; the spec's §9
checklist is ticked against the tests that prove each box, with deviations
annotated inline; the emit_ir smoke corpus gains the eight new programs.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

## Deliberate deviations

Record any place the implementation departs from the spec's letter, with the reasoning, at the time it happens. Each entry gets a cross-reference from the §9 checkbox it affects. If nothing deviated, say so — an empty section is a claim, and it should be a true one.

No deliberate deviations. The implementation follows the spec's letter: all five refusal messages are the exact strings §5 pins, `TAILCC = 18` / `MAX_PARAMS = 5` / `STACK_OVERFLOW = 0xC00000FD` are the spec's verbatim constants, the two-pass declare-then-body scheme is §4.2, `lower_tail`/`lower_expr` is §4.3, and `require_int` is scoped to `main` alone per §5.5. The seven breaking-change sites the Self-Review names are migrations the spec's §7.3 already required (or the spec did not name), not deviations from its letter.

---

## Self-Review

**1. Spec coverage.** Every §9 box maps to a task and a named test (the table in Task 5 Step 4 is that mapping, written out). §7.3's three required migrations are all covered: `corpus_lowers_to_core_through_the_real_pipeline` is scoped by comment with a multi-function counterpart added (Task 2 Step 1); the `emit_ir` smoke corpus gains the new programs (Task 5 Step 1); the 5b-1 and 5b-2 corpora are asserted *unchanged* by a mechanical `git diff` check (Task 5 Step 4). The spec's §8 obligations are carried as comments at the code that creates them — T1 at the `MAX_PARAMS` scan, T2 at `declare_all` and its refusal test, T3 at Task 1's asymmetry note — rather than only in the spec, so a future reader meets them where they bite.

**2. Seven breaking-change sites the spec does not name, all covered.** `pretty_typed`'s `s.push_str(param)` (Task 1 Step 9); the four `typed_inference` snapshots that gain a row (Task 1 Steps 4-6, with the variable-letter check that makes acceptance an inspection rather than a rubber stamp); `rejects_parameterised_main_specifically`'s hard compile break (Task 1 Step 9); `rejects_multi_function_module_specifically`, which is deleted rather than migrated because the message it asserts ceases to exist (Task 2 Step 4a); the print shim's call site needing `set_call_convention(TAILCC)` (Task 2 Step 3i, with the reasoning inline because "the shim survives untouched" is easy to over-read); the module doc comment and `corpus_verifies`'s "eleven lines" comment, both of which become false (Task 2 Step 3a, Task 5 Step 1); and `tests/core_lowering.rs`'s import lacking `TyCon` (Task 1 Step 7).

**3. Type consistency.** `decls` is `HashMap<String, FunctionValue<'ctx>>` keyed by the **unmangled** name everywhere it appears — `declare_all` produces it, `emit_body`, `build_elya_call`, `lower_expr`, and `lower_tail` all consume it under that name, and mangling happens only at `add_function`. `lower_expr` and `lower_tail` share one parameter list, differing only in return type. `FUNCTION_CORPUS` and `TAIL_CORPUS` are both `&[(&str, &str, &str)]`, matching `CONTROL_FLOW_CORPUS`'s existing shape, so the runners share one loop idiom. `CoreParam` is spelled identically in `src/core.rs`, `tests/core_lowering.rs`, and `crates/codegen/src/lib.rs`. `set_call_convention**s**` (plural, on `FunctionValue`) and `set_call_convention` (singular, on `CallSiteValue`) are each used exactly once and the difference is flagged in a comment at both sites, because the names are one character apart and doing the wrong one silently produces a miscompile.

**4. One thing worth watching during execution.** Task 3's tests are expected to pass on their first run, since Task 2's structure implements all five refusals. That is intentional — the refusal boundary deserves its own reviewable commit — but it means those tests never fail first. The two ways they could be vacuous are named in Task 3 Step 2 along with what to fix, and the arity refusal in particular is checked twice: once for the message, once for `MAX_PARAMS == 5`, so the boundary cannot drift without the test noticing.
