# Elya Slice 5b-8 — Native Effect Handlers (arc node N8) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Compile Elya's one-shot algebraic effect handlers (`handle { … } with { … }` / `resume`) to native machine code by selective CPS, so a handled effect program produces the same answer natively as under the evaluator, with both heap residency and machine-stack depth proven flat across resume count.

**Architecture:** Selective CPS keyed on the latent effect row already carried by `Ty::Fn(_, EffectRow, _)` — no new `CoreFn` field, no whole-program transform. Every captured continuation frame is a heap cell `[tag][code_ptr][next]`, byte-identical in layout to a one-capture closure (Fork 3(a): "a continuation frame is a closure"), so it reuses the existing closure-call path and `gc_mark` gains **no** case. The handler transfer returns to a trampoline rather than nesting, which is what makes machine-stack flatness a separate, independently provable property from heap flatness.

**Tech Stack:** Rust; `inkwell` over LLVM; `TAILCC` indirect calls; the C runtime at `crates/codegen/src/runtime.c` (shadow-stack tracing collector); the tree-walking CEK evaluator at `src/eval.rs` as the semantic specification; the differential harness in `crates/codegen/tests/native_codegen.rs`.

**Spec:** `docs/superpowers/specs/2026-09-18-elya-slice-5b8-native-effect-handlers-design.md`

---

## Global Constraints

These are project-wide and apply to **every** task below. Each task's requirements implicitly include this section.

- **Gate before every commit.** Run the full 5-stage gate in the **foreground, never piped**: `powershell -NoProfile -File scripts/check.ps1`. Prefix cargo invocations with `$env:CARGO_INCREMENTAL="0"`. Read the **exit code**; commit **only if it is 0**.
- **Never `cat` a gate log.** Grep it for `test result:`, `FAILED`, `panicked`, and the exit code.
- **Explicit `git add` paths — never `git add -A` and never `git add .`**
- **Commit messages via `-F <file>`** written with a **Bash heredoc**, **never** a PowerShell here-string.
- **Commit trailer:** `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`
- **Single-match anchor guards on every scripted edit.** Assert the anchor matched **exactly once** (`count == 1`); be line-ending agnostic. Over-match and under-match both fail silently. Never grep for a control character to measure line endings — measure by byte count.
- **Scratch and debug files go OUTSIDE the repo.** Never create files under tracked paths.
- **Do not lower A9's `N = 1_000_000`.** It is the machine-stack criterion's whole force.
- **Pin A7 by its measurement command, never by a remembered byte count.** Measurement-method drift is the known cause of A7 false alarms.
- **Read files in targeted chunks.** Use `grep` and `sed -n 'X,Yp'` rather than `cat` on `crates/codegen/src/lib.rs` (~2000 lines) or `crates/codegen/tests/native_codegen.rs` (~1200 lines).
- **`cargo fmt --all` before the gate** — the gate fmt-checks and fails hard.
- **Elya spells booleans `True` / `False`.** Lowercase dies at E0200 in the front end and can masquerade as a back-end bug.

---

## Pre-Plan Decisions (settled before planning; do not re-litigate)

Three gates the spec left open were closed by measurement before this plan was written, and two spec errors were found. An executor who re-derives these wastes a task; an executor who trusts the spec verbatim on D5 ships a silent-corruption bug.

### D1 — `effect IO` is refusable, and is refused at the Core boundary (§9.2 resolution (c))

§9.2's premise that `IO` cannot be user-declared is **false**: a user can write `effect IO { … }`, which collides with the builtin `io.*` namespace the selective-CPS partition keys on. Resolution taken: **refuse a user-declared effect named `IO`** this slice, and record front-end reservation of the name as a tracked obligation rather than doing it here.

The refusal lands in `lower_module` (`src/core.rs`), not in `crates/codegen`, because `CoreModule { fns, types }` carries **no effect declarations** — `Decl::Effect` is not re-homed (spec §2). `lower_module` is the last place effect declarations are visible. That makes it `LowerError::Unsupported`, not `CodegenError::Unsupported`.

**Side finding, out of scope, record as an obligation:** an *unhandled* user-declared `IO` slips past `check_main_discharge` silently. That is a pre-existing front-end hole, unrelated to codegen.

### D2 — A3 (polymorphic effects) is a measure-then-assert step with a stated prediction, not a refusal

`instantiate_op` (`src/types.rs:503-518`) instantiates an operation signature **per perform**, while the handle path fixes one shared instantiation per handle scope. Therefore a polymorphic effect **used at a single concrete type should compile**. A3 gets a **positive** test with a written-down prediction, not a refusal test. Only a genuinely unconstrained type parameter survives as `Ty::Var`, and that already meets the pre-existing `"unrepresentable type"` refusal at `crates/codegen/src/lib.rs:203` / `:230`.

### D3 — The pre-zonk hazard is closed negatively; the CPS predicate cannot under-approximate

The freeze at `src/types.rs:1985-1990` maps `inf.resolve(t)` over every table entry **after all SCC groups solve**. `resolve` recurses into `Ty::Fn`'s effect row (`:255`), and `resolve_row` (`:213-242`) unions labels across every bound link. So a row read from the frozen table is fully resolved: the selective-CPS predicate **cannot** under-approximate and miss an effectful function. Generalization is per-SCC-group *before* the freeze, and lowering consumes a whole module *after* `infer_all` returns — there is no "later" in which a row could grow.

### D4 — `is_multi_declared` must be **op-keyed**, not effect-keyed (refines spec §5.3)

§5.3 asserts "`lower_module` knows both the handler's effect name and the module's declarations." **The first half is not always true.** `Handler` has **no effect-name field** and `OpClause.effect` is `Option<String>` (`src/ast.rs:96`). Inference recovers the effect name via `handler_effect` (`src/types.rs:1355-1377`):

```rust
let eff = clause.effect.clone()
    .or_else(|| self.ops.get(&clause.op).map(|o| o.effect.clone()));
```

That `self.ops` fallback is an **inference-internal registry `lower_module` does not have**. For a clause written `get() -> …` with no `State.` prefix, `lower_module` cannot name the effect. So the stamp is derived from **op names**:

```rust
let is_multi_declared = handler.clauses.iter()
    .any(|c| multi_ops.contains(&c.node.op));
```

This is the same shape both existing constructions already use, so it **strengthens** §9.4's extraction case rather than weakening it. Task 1 exists because of this.

### D5 — The frame descriptor mask is `0b10`, **not** §6.3's `0b11` (spec error; report, do not silently patch)

§6.3 specifies the new frame descriptor row as "arity 2, mask `0b11` (both `code_ptr` and `next` are pointers)". **That is wrong, in the silent-corruption direction.** `gc_mark`'s loop is:

```c
for (int64_t f = 0; f < arity; f++) {
    if ((mask >> f) & 1) {
        gc_gray_push((void *)(intptr_t)obj[1 + f]);
    }
}
```

Bit `f` governs word `1 + f`. For `[tag][code_ptr][next]`: bit 0 is `code_ptr`, bit 1 is `next`. The existing lambda-descriptor loop keeps bit 0 **clear** for exactly this reason, and says so in its own comment (`crates/codegen/src/lib.rs:1395-1397`):

> `// Bit 0 is CLEAR: word 1 is a code pointer into the text segment, not a heap object. Bit j+1 is set iff capture j is a heap value.`

With `0b11`, `gc_mark` pushes a **text-segment address** into the gray set, then reads the first eight bytes of machine code as `obj[0]` and traces onward if that garbage lands in `[0, gc_n_ctors)`. Correct value: **arity 2, mask `0b10`.** Per A7's standing rule, spec deviations are **reported, not patched** — Task 6 implements `0b10` and its commit message records the deviation.

### D6 — Two refusal boundaries with two different test homes (easy to blur, structurally real)

| Refusal | Error type | Lives in | Tested in |
|---|---|---|---|
| `effect IO` (D1) | `LowerError::Unsupported` | `src/core.rs` (`lower_module`) | `tests/core_lowering.rs` |
| `with multi` (A2) | `CodegenError::Unsupported` | `crates/codegen/src/lib.rs`, read off the stamped `is_multi_declared` bit | `crates/codegen/tests/native_codegen.rs` — an **execution** test, per §5.4 |
| tag / descriptor-row disagreement (A8) | `CodegenError::Unsupported` | `crates/codegen/src/lib.rs`, the two guards already there | none — **structural**, discharged by the check itself (§11) |

> **Correction to this plan's own first draft.** An earlier revision put the `with multi`
> refusal in `lower_module` as a `LowerError`. That contradicts the approved spec: §5.3
> takes reading **(ii)** — lowering *stamps* an `is_multi_declared: bool` onto the Core
> handle node — and §5.4 puts the *refusal* at codegen, "on the handle node, before
> lowering any clause body." A2 then requires it be proved by execution. Tasks 2, 5 and 8
> below follow the spec; `effect IO` (D1) stays a `LowerError` because `CoreModule` carries
> no effect declarations, so `lower_module` is the last place that can see one.

**There is no existing `LowerError` negative assertion anywhere in the repo** — `tests/core_lowering.rs::lower_src` (`:22-31`) `.expect()`s. Task 2 must therefore add a `lower_err` helper. The `CodegenError` side already has its pattern (`crates/codegen/src/lib.rs:2040-2080`), including the **poisonous-unbound-`Var`** technique that proves a refusal fires *before* its subterms are lowered — which is precisely what §5.4 requires.

### D7 — `$k` is synthesized, not recorded (carry-in 1)

The A-normalization `Let("$k", <resume node>, App(Var("$k"), [s]))` synthesizes a binder whose type is **cloned from the resume node**. `$k` therefore **never appears in `node_types`**, and its absence is **not** a recorder gap. Two precedents: `lower_block`'s `"_"` binder (`src/core.rs:224-225`) and the evaluator's `$`-prefixed internal names (e.g. `$resume`, `src/eval.rs:495-500`). A future recorder-totality audit must not flag this. Task 5 carries the assertion that pins it.

### D8 — Un-refusing `Expr::Block` is a `repr_ty`-widening, so it needs a sweep (carry-in 2)

Removing `Expr::Block(_) => return Err(LowerError::Unsupported("Block"))` (`src/core.rs:347`) is the same shape as 5b-7 Task 1's `repr_ty` widening: it **un-shadows** every downstream site that was protected only by the `Block` refusal firing first. Task 4 sweeps for those sites rather than assuming handler bodies are the only consumer. Confirmed first hit: `src/core.rs:215`'s `"block without tail expression"`. Baseline to diff against: `lower_block`'s four call sites (`:175`, `:308`, `:309`, `:327`).

### D9 — §9.1's "source transfers but the assertion does not" distinction stays intact (carry-in 3)

Measured against the real corpus (`tests/state_effect.rs`), the split is **three-way**, not two-way, and is more favorable than §9.1 assumed:

| Piece of `state_tail_loop_is_bounded` (`:74-91`) | Transfers natively? |
|---|---|
| The **program** `state_tail_loop(n)` (`:59-72`) — `effect State`, a state-passing handler whose clause bodies return functions, `io.println` at the end | **Yes, verbatim.** Every construct is in the 5b-7 subset plus this slice's handlers. |
| The **output** assertion `assert_eq!(out, "x\n")` | **Yes.** `io.println` reaches stdout natively as of 5b-7. |
| The **peak** assertions `small <= K_MAX_STATE` and `small == large` | **No.** `run_peak`'s second return is the evaluator's `peak_kont_depth()` — an interpreter-internal instrument with no native analogue. |

So: the source transfers, the output assertion transfers, and **only the peak assertion is rewritten** — into Task 9 (A4, heap live-set) and Task 10 (A9, machine stack), which measure two *different* things and are exactly the §7.1 coupling hazard made into two tests. Do **not** compress this to "we reused the test."

Two further facts from the same read, both load-bearing:
- The sibling control `non_tail_state_loop_grows_with_length` (`:93+`) puts the recursive call under `<>`. Its **program does not transfer** — native codegen refuses `<>` on strings by name. That is the real "string concatenation barrier," and it applies to the *control*, not the main corpus program.
- `K_MAX_STATE` and `N = 1_000_000` **already appear here** (`:77`). A9's N is not invented for this slice; it is the existing corpus's large-N value. One more reason not to lower it.

---

## File Structure

| File | Responsibility | Tasks |
|---|---|---|
| `src/ast.rs` | Gains `pub fn multi_declared_ops(&Module) -> HashSet<String>` — the single op-keyed "is this op from a `multi` effect" query. | 1 |
| `src/affine.rs` | Loses its local `multi_ops` construction (`:23-36`), calls the helper. Behavior must be unchanged. | 1 |
| `src/types.rs` | **Unchanged.** Its `effect_multi` map is **effect-keyed** (`HashMap<String, bool>`, `:161`/`:183`/`:1834`) and serves E0427; it is a *different index*, not a third copy of the op-keyed set. Do not fold it into the helper — that breaks E0427. | — |
| `src/core.rs` | The `effect IO` refusal; un-refusing `Expr::Block`; the two new `CoreKind` variants (`Handle`, `Resume`); the `is_multi_declared` stamp; the `$k` A-normalization; building the op-keyed multi set in `lower_module`'s pass 1. Also gains `pretty_expr` arms — that match is exhaustive. | 2, 4, 5 |
| `crates/codegen/src/lib.rs` | Frame-cell allocation, the one new descriptor row (arity 2, mask `0b10`), tag extension, both A8 guards; the selective-CPS partition predicate and the transform; handler dispatch, continuation splice, deep re-installation, the trampoline. | 6, 7a, 7b, 8 |
| `crates/codegen/src/runtime.c` | **`gc_mark` must not gain a case (A7).** Any trampoline support lands outside it. | 8 (guarded by A7) |
| `tests/core_lowering.rs` | Gains a `lower_err` helper (the first `LowerError` negative assertion in the repo) plus the refusal, Block-widening, and `$k` proofs. | 2, 4, 5 |
| `tests/effect_syntax.rs` | Gains the focused `multi_declared_ops` test. | 1 |
| `crates/codegen/tests/native_codegen.rs` | The A3 positive measurement, the differential fidelity corpus, the A4 heap-live-set proof, the A9 machine-stack proof, and the negative controls. | 3, 9, 10, 11 |

### Task dependency order

Tasks 1 → 2 → 4 → 5 → 6 → 7a → 7b → 8 are a chain; each needs its predecessor. Tasks 3, 9,
10, 11, 12 are proofs: **3** can run any time after 2; **9**, **10**, **11**, **12** require 8.

**Honest sizing note (§9.3 explicitly asks for this):** step 4 of §9.3's decomposition — the transform — does **not** fit one task. It is split into **7a** (the partition predicate, small and independently testable) and **7b** (the transform itself, the largest task in this plan, carrying an internal checkpoint). If 7b overruns, split it at its checkpoint rather than pushing work into Task 8, because Task 8 is where the trampoline requirement (§7.2) is discharged and it must not absorb unrelated risk.

---

## Verified Interface Reference

Every signature below was read out of the tree at plan time. An implementer who
guesses one of these will write a task that does not compile, so prefer this table
over memory or inference.

**Core IR — `src/core.rs`.** `CoreExpr` is a *struct*, and the variants live on
`CoreKind`. Children are `Rc`, never `Box`:

```rust
pub struct CoreExpr { pub span: Span, pub ty: Ty, pub kind: CoreKind }

pub enum CoreKind {
    Lit(CoreLit), Var(String),
    App(Rc<CoreExpr>, Rc<[CoreExpr]>),
    Builtin(String, Rc<[CoreExpr]>),
    Ctor(String, Rc<[CoreExpr]>),
    Prim(BinOp, Rc<[CoreExpr]>),
    Lambda(Rc<[CoreParam]>, Rc<CoreExpr>),
    Let(String, Rc<CoreExpr>, Rc<CoreExpr>),   // POSITIONAL. No `ty` field.
    If(Rc<CoreExpr>, Rc<CoreExpr>, Rc<CoreExpr>),
    Match(Rc<CoreExpr>, Rc<[CoreArm]>),
}

pub enum LowerError { Unsupported(&'static str), Untyped(Span) }   // derives Clone, Debug, PartialEq
```

**Lowering entry points** — note there is **no `ctx` parameter** anywhere:

```rust
fn lower_expr(e: &Expr, span: Span, table: &BTreeMap<Span, Ty>, ctors: &HashSet<String>) -> Result<CoreExpr, LowerError>
fn lower_block(block: &Block, table: &BTreeMap<Span, Ty>, ctors: &HashSet<String>) -> Result<CoreExpr, LowerError>
pub fn lower_module(module: &Module, table: &BTreeMap<Span, Ty>) -> Result<CoreModule, LowerError>
pub fn pretty_typed(m: &CoreModule, p: &mut TyPrinter) -> String
```

**Surface AST — `src/ast.rs`.** The handler fields are not what an English reading
of the spec suggests:

```rust
pub struct Handler { pub multi: bool, pub clauses: Vec<Spanned<OpClause>>, pub ret: Option<ReturnClause> }
pub struct OpClause { pub effect: Option<String>, pub op: String, pub params: Vec<Spanned<Param>>, pub body: Rc<Spanned<Expr>> }
pub struct ReturnClause { pub binder: String, pub body: Rc<Spanned<Expr>> }
pub struct EffectDecl { pub name: String, pub params: Vec<String>, pub is_multi: bool, pub ops: Vec<Spanned<OpSig>> }
```

Three traps in there: the handler's flag is **`multi`**, while the *declaration*'s is
**`is_multi`**; the return clause is a **distinct `Option` field**, not a clause whose
`op` happens to be `"return"`; and `OpClause::params` is a **`Vec`**, so a single
`binder: Option<String>` is the wrong shape for a clause.

**Codegen test harness — `crates/codegen/tests/native_codegen.rs`.** There is **no
`compile_and_run`**; the real helpers are:

```rust
fn lower_src(src: &str) -> elya::core::CoreModule          // asserts check_source(..).is_ok() FIRST
fn compile_and_link(core: &CoreModule, dir: &Path, tag: &str) -> PathBuf
fn assert_runs(exe: &Path, expected: &str)                  // exit status + trimmed stdout + EMPTY stderr
fn native_text_value(exe: &Path, tag: &str) -> (String, String)   // (printed text, main's returned value)
fn run_with_gc_stats(exe: &Path, tag: &str) -> (String, GcStats)  // GcStats { collections, freed, live }
fn temp_dir(tag: &str) -> PathBuf
const STACK_OVERFLOW: i32 = 0xC00000FDu32 as i32;
```

Because `lower_src` runs the **whole** front end, every program in a codegen test
must pass inference, exhaustiveness and the affine check — not merely parse.

**The Core-lowering harness — `tests/core_lowering.rs`.** `lower_src` there has a
*different* signature and `.expect()`s success, so it cannot host a refusal test:

```rust
fn lower_src(src: &str) -> (CoreModule, BTreeMap<Span, Ty>)   // panics on any error
fn nodes(core: &CoreModule) -> Vec<&CoreExpr>                 // exhaustive match on CoreKind
```

**Three exhaustive matches break by design when `CoreKind` grows.** This is a
feature, not an accident — `crates/codegen/src/closure.rs`'s doc comment says a new
variant "must fail the build here rather than fall into a `_ => {}` and go silently
uncaptured." Task 5 must fix all three in the same commit or the tree will not build:

| Site | Why it must be taught the new variants |
|---|---|
| `crates/codegen/src/closure.rs::fv_walk` | Free variables under a handler would go uncaptured — silently wrong closures |
| `src/core.rs::pretty_expr` | `pretty_typed` is how every Core test reads a module |
| `tests/core_lowering.rs::nodes` | The pre-order walk backing the origin proofs |

**`emit_ir` is not evidence.** Its own doc comment, at `crates/codegen/src/lib.rs`:

> Debugging aid ONLY (spec §7) … Never asserted on; no snapshot test uses it; its
> existence is not a proof of anything (§0).

It also returns `Result<String, CodegenError>`, not `String`. No task in this plan
asserts on its output.

**The gate has two configurations and only one has LLVM.** `scripts/check.ps1` runs
`fmt` → `clippy -p elya -p elya-cli` → `test -p elya -p elya-cli` → `clippy
--workspace --features elya-cli/codegen` → `test --workspace --features
elya-cli/codegen`. Stages 2 and 3 are **LLVM-free**, so anything added under `src/`
must compile and pass without the codegen feature. That is why Tasks 2, 4 and 5 put
their tests in `tests/` and Tasks 3, 6 and 8–12 put theirs in `crates/codegen/tests/`.

---

### Task 1: Extract `multi_declared_ops` (§9.4) and pin the A7 baseline

The op-keyed "is this op from a `multi` effect?" query exists once in `src/affine.rs` and is about to be needed a second time in `src/core.rs` (D4). Extract it **before** there are two copies, not after. Also establishes the A7 baseline measurement that Tasks 6 and 8 re-run.

**Files:**
- Modify: `src/ast.rs` (append the helper at end of file)
- Modify: `src/affine.rs:22-36` (replace the local construction with a call)
- Test: `tests/effect_syntax.rs` (append)

**Interfaces:**
- Consumes: nothing.
- Produces: `pub fn multi_declared_ops(module: &Module) -> std::collections::HashSet<String>` in `src/ast.rs`. Returns **operation** names (e.g. `"flip"`), never effect names. Consumed by `src/affine.rs::check` (this task) and `src/core.rs::lower_module` (Task 2).

- [ ] **Step 1: Write the failing test**

Append to `tests/effect_syntax.rs`:

```rust
#[test]
fn multi_declared_ops_is_keyed_by_operation_name_not_effect_name() {
    // Two `multi` effects (one single-op, one two-op) and one non-`multi`.
    let src = "effect multi Flip { fn flip() -> Bool }\n\
               effect multi Choice { fn pick() -> Int  fn stop() -> Unit }\n\
               effect State { fn get() -> String  fn set(v: String) -> Unit }\n";
    let (m, pd) = parse_module(&Session::new(), src);
    assert!(pd.is_empty(), "parse: {pd:?}");

    let ops = elya::ast::multi_declared_ops(&m);

    // Keyed by OPERATION name, and every op of a multi effect is present —
    // the two-op effect proves the per-effect loop, not just the outer one.
    assert!(ops.contains("flip"), "{ops:?}");
    assert!(ops.contains("pick"), "{ops:?}");
    assert!(ops.contains("stop"), "{ops:?}");

    // Effect names must never leak into the set (this is the D4 distinction:
    // the set is op-keyed, unlike inference's effect-keyed `effect_multi`).
    assert!(!ops.contains("Flip"), "effect name leaked: {ops:?}");
    assert!(!ops.contains("Choice"), "effect name leaked: {ops:?}");

    // Operations of a non-`multi` effect are absent — both of them.
    assert!(!ops.contains("get"), "{ops:?}");
    assert!(!ops.contains("set"), "{ops:?}");

    assert_eq!(ops.len(), 3, "exactly the three multi ops: {ops:?}");
}
```

- [ ] **Step 2: Run it and verify it fails**

```
$env:CARGO_INCREMENTAL="0"; cargo test --test effect_syntax multi_declared_ops
```

Expected: **compile error**, `no function or associated item named 'multi_declared_ops' found`. A compile failure is the correct "fails first" signal here; do not proceed if it fails for any other reason (e.g. a parse assertion firing would mean the syntax above is wrong).

- [ ] **Step 3: Write the minimal implementation**

Append to `src/ast.rs`. Note the **fully-qualified** `std::collections::HashSet` — this deliberately avoids touching `src/ast.rs`'s import block, so the diff is purely additive:

```rust
/// Operation names belonging to a `multi`-declared effect, keyed by **operation**
/// name. Op-keyed and not effect-keyed on purpose: `Handler` carries no
/// effect-name field and `OpClause.effect` is `Option<String>`, so a consumer
/// outside inference cannot always name a clause's effect — it can only ask
/// "is this op from a multi effect?" (Slice 5b-8 D4).
///
/// Deliberately NOT unified with `Infer.effect_multi`, which is effect-keyed
/// (`HashMap<String, bool>`) and serves the `with multi` conformance rule
/// E0427. They are two different indexes, not two copies of one.
pub fn multi_declared_ops(module: &Module) -> std::collections::HashSet<String> {
    let mut ops: std::collections::HashSet<String> = std::collections::HashSet::new();
    for d in &module.decls {
        if let Decl::Effect(e) = &d.node {
            if e.is_multi {
                for op in &e.ops {
                    ops.insert(op.node.name.clone());
                }
            }
        }
    }
    ops
}
```

- [ ] **Step 4: Run it and verify it passes**

```
$env:CARGO_INCREMENTAL="0"; cargo test --test effect_syntax multi_declared_ops
```

Expected: **PASS**.

- [ ] **Step 5: Redirect `affine.rs` at the helper**

In `src/affine.rs`, replace the local construction (the `let mut multi_ops` block through its closing braces, currently `:22-36`) with the call. Use a single-match anchor guard: assert the string `let mut multi_ops: HashSet<String> = HashSet::new();` occurs **exactly once** before editing.

```rust
pub fn check(module: &Module, affine_sites: &HashSet<Span>) -> Vec<Diagnostic> {
    // Operation names belonging to a `multi`-declared effect. One construction,
    // two call sites (Slice 5b-8 §9.4) — this one and `lower_module`'s.
    let multi_ops: HashSet<String> = multi_declared_ops(module);
```

`src/affine.rs` already has `use crate::ast::*;`, so the helper is in scope with no import change.

- [ ] **Step 6: Verify affine behavior is unchanged**

```
$env:CARGO_INCREMENTAL="0"; cargo test --test affine
```

Expected: **PASS, same count as before the edit.** This is a pure refactor; a single changed assertion means the extraction is not behavior-preserving. If E0427 tests break, you folded in `effect_multi` — revert and re-read the File Structure note on `src/types.rs`.

- [ ] **Step 7: Record the A7 baseline**

Run and write the number into the commit message body:

```bash
awk '/^static void gc_mark\(void\) \{/,/^\}/' crates/codegen/src/runtime.c | wc -c
```

This exact command is the A7 criterion for the rest of the slice. **Pin the command, not the number** — a remembered byte count is the known cause of A7 false alarms.

- [ ] **Step 8: Gate and commit**

```
cargo fmt --all
$env:CARGO_INCREMENTAL="0"; powershell -NoProfile -File scripts/check.ps1
```

Read the exit code. Commit only if 0.

```bash
cat > /tmp/elya-msg-t1 <<'MSG'
refactor(ast): one op-keyed multi_declared_ops helper, two call sites

Slice 5b-8 Task 1. `src/core.rs` needs the same op-keyed "is this op from a
multi effect?" query `src/affine.rs` already builds locally, because
`lower_module` cannot name a clause's effect (Handler has no effect field;
OpClause.effect is Option<String>) — so the stamp must be op-derived.

Extracted before the second copy existed, not after. Deliberately NOT merged
with Infer.effect_multi, which is effect-keyed and serves E0427.

A7 baseline (gc_mark, by command not by remembered number):
  awk '/^static void gc_mark\(void\) \{/,/^\}/' crates/codegen/src/runtime.c | wc -c
  => <paste measured bytes>

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
MSG
git add src/ast.rs src/affine.rs tests/effect_syntax.rs
git commit -F /tmp/elya-msg-t1
```

---

### Task 2: Refuse a user-declared `effect IO` (D1), and the repo's first `LowerError` assertion

D1 is not housekeeping — it is what makes Task 7a's partition predicate cheap and
sound. `CoreModule` carries no effect declarations, so codegen cannot ask "is this
label user-declared?"; it can only ask "is this label `IO`?". That question is a
correct proxy for "user-declared" **only if** no user can declare an effect named
`IO`. `lower_module` is the last pass that still sees `Decl::Effect`, so the refusal
belongs there, as a `LowerError`.

**Files:**
- Modify: `src/core.rs`, inside `lower_module`'s pass-1 loop over `module.decls`
- Test: `tests/core_lowering.rs` (append)

**Interfaces:**
- Consumes: nothing from Task 1.
- Produces: the guarantee Task 7a depends on — after `lower_module` returns `Ok`, an
  effect label in any row of that module is user-declared **iff** it is not the
  literal string `"IO"`. No new function.

- [ ] **Step 1: Measure whether the front end already refuses this**

The refusal is only worth adding if `effect IO { ... }` currently survives to
lowering. Write a probe that reports the truth instead of assuming it. Append to
`tests/core_lowering.rs`:

```rust
#[test]
fn probe_user_declared_io_reaches_lowering() {
    let src = "effect IO { fn write(s: String) -> Unit }\n\
               pub fn main() -> Int { 0 }\n";
    let (m, pd) = parse_module(&Session::new(), src);
    let (diags, table) = infer_typed_table(&Session::new(), &m);
    let lowered = lower_module(&m, &table);
    panic!("parse={pd:?}\ninfer={diags:?}\nlower={lowered:?}");
}
```

- [ ] **Step 2: Run it and read the three lines**

Run: `$env:CARGO_INCREMENTAL="0"; cargo test -p elya --test core_lowering probe_user_declared_io -- --nocapture`
Expected: FAIL (it always panics). Read which stage, if any, already rejected it.

Then take one of two branches:

- **Parse or infer already produced a diagnostic** — the refusal exists upstream.
  Delete the probe and append a test asserting *that* diagnostic, named
  `a_user_declared_io_effect_is_rejected_before_lowering`. Skip Steps 3–6; the
  guarantee Task 7a needs already holds, and this becomes a one-test task.
- **Both empty and `lower=Ok(..)`** — nothing refuses it. Continue to Step 3.

- [ ] **Step 3: Replace the probe with the real failing test**

```rust
#[test]
fn a_user_declared_io_effect_is_refused_at_lowering() {
    // `IO` is the one effect label codegen treats as built in (Task 7a reads the
    // partition off `label != "IO"`). A user-declared `IO` makes that read
    // ambiguous, so lowering — the last pass that still sees `Decl::Effect` —
    // refuses it by name.
    let src = "effect IO { fn write(s: String) -> Unit }\n\
               pub fn main() -> Int { 0 }\n";
    let (m, _pd) = parse_module(&Session::new(), src);
    let (_diags, table) = infer_typed_table(&Session::new(), &m);
    assert_eq!(
        lower_module(&m, &table),
        Err(elya::core::LowerError::Unsupported("effect IO")),
    );
}

#[test]
fn an_effect_not_named_io_still_lowers() {
    // Negative control: the refusal is keyed on the NAME, not on effect
    // declarations in general. Without this, an implementation that rejected
    // every `Decl::Effect` would pass the test above.
    let src = "effect State { fn get() -> Int  fn set(v: Int) -> Unit }\n\
               pub fn main() -> Int { 0 }\n";
    let (m, _pd) = parse_module(&Session::new(), src);
    let (_diags, table) = infer_typed_table(&Session::new(), &m);
    assert!(lower_module(&m, &table).is_ok());
}
```

`LowerError` derives `PartialEq`, so `assert_eq!` on the whole `Result` works. This
is the first place in the repo that relies on that derive.

- [ ] **Step 4: Run to verify it fails**

Run: `$env:CARGO_INCREMENTAL="0"; cargo test -p elya --test core_lowering io_effect`
Expected: FAIL — `a_user_declared_io_effect_is_refused_at_lowering` gets `Ok(..)`
where it wanted `Err(..)`. `an_effect_not_named_io_still_lowers` should already PASS.

- [ ] **Step 5: Write the minimal implementation**

In `src/core.rs`, in `lower_module`'s pass-1 loop (the one that already matches
`Decl::Effect` in order to skip it), add the name check:

```rust
        if let Decl::Effect(e) = &d.node {
            // D1 / spec §9.2: codegen decides "does this row mention a
            // user-declared effect?" by asking `label != "IO"`. That read is
            // sound only while `IO` cannot be user-declared, so it is refused
            // here — the last pass that still sees the declaration.
            if e.name == "IO" {
                return Err(LowerError::Unsupported("effect IO"));
            }
        }
```

- [ ] **Step 6: Run to verify both pass**

Run: `$env:CARGO_INCREMENTAL="0"; cargo test -p elya --test core_lowering io_effect`
Expected: PASS, 2 tests.

- [ ] **Step 7: Gate and commit**

Run `powershell -NoProfile -File scripts/check.ps1` in the foreground, read the exit
code, and commit only on `0`.

```bash
git add src/core.rs tests/core_lowering.rs
git commit -F /tmp/msg-t2.txt
```

---

### Task 3: A3 — measure what a polymorphic effect actually does, prediction first

Spec §11 marks **A3 UNCERTAIN**: we believe a polymorphic effect such as
`effect State(s)` is already refused somewhere upstream, but we have not measured
*where*, and an acceptance criterion asserting an unmeasured refusal is a guess.
This task turns A3 into a recorded fact, wherever that fact lands.

**Write the prediction down before running anything.** The prediction: the front end
rejects it, but *not* for the reason an earlier draft of this plan gave. That draft
said `EffectRow`'s labels are plain strings with no slot for a type argument. They
are not — `src/types.rs` carries

```rust
pub struct EffectLabel {
    pub args: Vec<Ty>,
    pub span: Span,
}
```

and `args` is documented as "empty for a monomorphic effect — the arity-0 case that
reproduces Slice-3 behavior". **The slot exists and is deliberately under-used.**

That measurement makes the prediction weaker, not stronger, and it is exactly why
§11 marks A3 ✗ UNCERTAIN. The representation would happily hold `State(s)`; whether
anything refuses it therefore depends on the parser, the declaration checker, or
unification — three places this plan has not read. So the honest prediction is: a
refusal exists somewhere upstream of codegen, with **no confident claim about
where**, and a live possibility that nothing refuses it at all.

If the measurement disagrees, the measurement wins and A3 gets restated to match.
Do not repair the prediction retroactively; record what was predicted and what was
found.

**Files:**
- Test: `tests/effect_syntax.rs` (append) — or
  `crates/codegen/tests/native_codegen.rs`, depending on Step 2's outcome

**Interfaces:**
- Consumes: nothing.
- Produces: one named test pinning A3. No production code.

- [ ] **Step 1: Write the probe**

There is no existing refusal test in `crates/codegen/tests/native_codegen.rs` to
copy, so the probe calls the public front-end entry directly. It deliberately does
**not** use that file's `lower_src`, which `assert!`s the front end succeeded — the
whole question is whether it does:

```rust
#[test]
fn probe_polymorphic_effect_a3() {
    let src = "effect State(s) { fn get() -> s  fn set(v: s) -> Unit }\n\
               pub fn main() -> Int { 0 }\n";
    let front = elya::check_source("a3.elya", src);
    panic!("check_source={front:?}");
}
```

Put the probe in `tests/effect_syntax.rs` (LLVM-free) so Step 2 runs in seconds.

- [ ] **Step 2: Run it and read the outcome**

Run: `$env:CARGO_INCREMENTAL="0"; cargo test -p elya --test effect_syntax probe_polymorphic_effect -- --nocapture`
Expected: FAIL (it always panics). Read whether `check_source` returned `Err`.

- **`Err`** — the prediction held. Continue to Step 3 and assert the diagnostic code.
- **`Ok`** — the front end admits it. Move the probe to
  `crates/codegen/tests/native_codegen.rs`, extend it to call `lower_src` and then
  `compile_module(&core, &temp_dir("a3").join("a3.o"))`, and record which of the two
  refuses. Assert on that refusal instead.
- **Neither refuses** — A3 is not a refusal criterion at all. Stop and report to the
  reviewer before writing more of this slice. A polymorphic effect reaching the CPS
  transform is outside the spec's subset, and that needs a spec amendment, not a
  plan workaround.

- [ ] **Step 3: Write the real assertion**

For the predicted branch. The diagnostic code is the *output* of Step 2 — fill it in
from the measurement, do not guess it:

```rust
#[test]
fn a3_a_polymorphic_effect_is_refused_by_the_front_end() {
    // A3 (spec §11), measured rather than assumed. The subset is monomorphic
    // effects; this pins where a polymorphic one dies so a future slice that
    // lifts the restriction has to delete a test on purpose.
    let src = "effect State(s) { fn get() -> s  fn set(v: s) -> Unit }\n\
               pub fn main() -> Int { 0 }\n";
    let diags = elya::check_source("a3.elya", src).expect_err("A3: must be refused");
    assert!(
        diags.iter().any(|d| d.code == "E0XXX"),   // <- the code measured in Step 2
        "A3 expected the polymorphic-effect diagnostic, got {diags:?}",
    );
}
```

- [ ] **Step 4: Run to verify it passes**

Run the same filter with the final test name.
Expected: PASS, 1 test.

- [ ] **Step 5: Gate and commit**

```bash
git add tests/effect_syntax.rs
git commit -F /tmp/msg-t3.txt
```

Adjust the path if Step 2 sent the test to the codegen suite instead.

---

### Task 4: Un-refuse `Expr::Block` in expression position (§3.1(b)), and sweep what that un-shadows (D8)

`src/core.rs` refuses six surface forms. One of them, `Expr::Block`, is in the way:
spec §3.1(b) verified that a `handle { ... }` body parses through `block_expr()` to
an `Expr::Block`, so Task 5 cannot lower a handle body until this refusal is gone.

Two facts from §3.1 that shape this task, both already verified against the tree:

1. **Clause bodies are not affected.** A clause body is parsed by `self.expr(0)`,
   and every one in the corpus is a bare `resume(...)` — an `Expr::Resume`, refused
   separately and lifted by Task 5, not by this task. A `return(x) -> x` clause body
   is an `Expr::Var`. Neither is a Block. Only the *handle body* is, and only because
   `handle { ... } with` parses it through `block_expr()`:

   ```
   handle {                              <- Expr::Block, refused today
     apply(fn(n) { log(n) }, "hi")
   } with {
     Log.log(m) -> resume(m <> "!")      <- Expr::Resume, Task 5
     return(x) -> x                      <- Expr::Var, already lowers
   }
   ```
   (`tests/effect_closures.rs:31`.) An earlier draft of this plan justified the task
   with clause bodies being lambdas; that was wrong, and the corpus says so.
2. **No test anywhere asserts `Unsupported("Block")`.** Zero test updates, and no
   deletion of a proof.

The D8 sweep is therefore narrow and provable rather than speculative: `lower_block`
emits only `CoreKind::Let` and whatever the tail expression lowers to — **no new
`CoreKind` variant** — so by construction nothing downstream of Core sees a shape it
has not already seen. And `lower_block` never queries the brace span, so no new span
enters the frozen typed table. Both are asserted below rather than argued.

**Files:**
- Modify: `src/core.rs`, the `Expr::Block(_)` arm of `lower_expr`
- Test: `tests/core_lowering.rs` (append)

**Interfaces:**
- Consumes: nothing.
- Produces: `lower_expr` accepts `Expr::Block(b)` by delegating to the existing
  `lower_block(b, table, ctors)`. Signature of neither function changes. Task 5's
  handle-body lowering depends on this.

- [ ] **Step 1: Write the failing test**

Append to `tests/core_lowering.rs`:

```rust
#[test]
fn a_block_in_expression_position_lowers_to_nested_lets() {
    // §3.1(b): `handle { ... }` parses its body through `block_expr()`, so Task 5
    // needs blocks lowerable in expression position. A `let` initializer is the
    // smallest program that reaches the same arm today.
    let src = "pub fn main() -> Int {\n\
               \x20 let x = { let a = 1  a + 2 }\n\
               \x20 x\n\
               }\n";
    let (core, table) = lower_src(src);

    // (a) It lowered at all, and produced only shapes Core already had.
    for n in nodes(&core) {
        assert!(
            !matches!(n.kind, CoreKind::Lit(_)) || n.ty != Ty::Error,
            "no node may carry Ty::Error",
        );
    }

    // (b) The inner block became a `Let` — not a new node kind.
    let lets = nodes(&core)
        .iter()
        .filter(|n| matches!(n.kind, CoreKind::Let(ref name, _, _) if name == "a"))
        .count();
    assert_eq!(lets, 1, "the block's `let a` should survive as a CoreKind::Let");

    // (c) The frozen table gained nothing: every node's span is a span the
    //     inference table already knows. `lower_block` never queries the brace
    //     span, and this is the assertion that keeps it that way.
    for n in nodes(&core) {
        assert!(
            table.contains_key(&n.span),
            "node at {:?} has a span absent from the typed table",
            n.span,
        );
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `$env:CARGO_INCREMENTAL="0"; cargo test -p elya --test core_lowering block_in_expression_position`
Expected: FAIL — `lower_src` panics with
`lowering the corpus subset should succeed: Unsupported("Block")`.

- [ ] **Step 3: Write the minimal implementation**

In `src/core.rs`, replace the refusal arm:

```rust
        Expr::Block(_) => return Err(LowerError::Unsupported("Block")),
```

with the delegation:

```rust
        // §3.1(b): a block in expression position is exactly a function body in
        // expression position — same statements, same tail. `lower_block` already
        // builds the `Let` chain, and it emits no CoreKind that did not already
        // exist, so nothing downstream of Core widens.
        //
        // This arm RETURNS rather than yielding a `CoreKind`: `lower_block` hands
        // back a whole `CoreExpr` carrying the tail expression's own span and type,
        // and reusing those is what keeps the brace span out of the frozen table.
        Expr::Block(b) => return lower_block(b, table, ctors),
```

Note the shape difference from its neighbours. Most arms evaluate to a `CoreKind`
that the function wraps once at the end (`CoreKind::Lambda(ps.into(), Rc::new(b))` at
`src/core.rs:328`). A few return early instead — the `io.println` arm at
`src/core.rs:267` is the precedent. This one is in the second group because it has a
finished `CoreExpr` already.

- [ ] **Step 4: Run to verify it passes**

Run: `$env:CARGO_INCREMENTAL="0"; cargo test -p elya --test core_lowering block_in_expression_position`
Expected: PASS, 1 test.

- [ ] **Step 5: Run the whole Core-lowering suite — the actual sweep**

Run: `$env:CARGO_INCREMENTAL="0"; cargo test -p elya --test core_lowering`
Expected: PASS, every test. This is the D8 sweep: `both_origin_proofs_and_no_token_leak_across_corpus`
walks the whole corpus and would catch a widened node that leaked an untyped span.

- [ ] **Step 6: Gate and commit**

```bash
git add src/core.rs tests/core_lowering.rs
git commit -F /tmp/msg-t4.txt
```

---
### Task 5: Lower `handle`/`resume` into Core, and fix the three matches that break by design

This is the task that grows `CoreKind`. Two new variants, three named structs, and —
because three matches in this repo are deliberately exhaustive with no catch-all —
three call sites that must be updated in the same commit or the build stays red.

The `resume`-as-syntax decision (spec §2, ledger obs 861) is what keeps this small:
`resume` is a form, not a value, so a continuation needs no Core representation here.
Clause bodies are ordinary expressions. No fifth node kind.

**Files:**
- Modify: `src/core.rs` — the `CoreKind` enum, three new structs, `lower_expr`'s
  `Handle`/`Resume`/`Call` arms, and `pretty_expr`
- Modify: `crates/codegen/src/closure.rs:70` — `fv_walk`
- Test: `tests/core_lowering.rs` — the `nodes` walker, plus two new tests

**Interfaces:**
- Consumes: Task 4's `Expr::Block` acceptance — `handle { ... } with` has a Block body
  and cannot lower without it.
- Produces, for Tasks 7a/7b/8 to consume by exact name:

```rust
pub struct CoreHandle {
    pub body: Rc<CoreExpr>,
    pub clauses: Rc<[CoreClause]>,
    pub ret: Option<Rc<CoreReturn>>,
    /// §5.3: lowering STAMPS this bit off `Handler::multi`; §5.4 puts the
    /// REFUSAL in codegen, where an execution test can observe it (D6, A2).
    pub is_multi_declared: bool,
}

pub struct CoreClause {
    pub effect: String,
    pub op: String,
    pub params: Rc<[CoreParam]>,
    pub body: Rc<CoreExpr>,
}

pub struct CoreReturn {
    pub binder: String,
    pub body: Rc<CoreExpr>,
}

// added to CoreKind:
    Handle(Rc<CoreHandle>),
    Resume(Rc<CoreExpr>),
```

- [ ] **Step 1: Write the failing test**

Append to `tests/core_lowering.rs`. The corpus uses `<>`, which codegen refuses by
name — irrelevant here, because this test stops at Core and never reaches codegen:

```rust
#[test]
fn a_handle_lowers_to_core_handle() {
    let src = "effect Log { fn log(msg: String) -> String }\n\
               pub fn main() {\n\
               \x20 let r = handle {\n\
               \x20   log(\"hi\")\n\
               \x20 } with {\n\
               \x20   Log.log(m) -> resume(m)\n\
               \x20   return(x) -> x\n\
               \x20 }\n\
               \x20 io.println(r)\n\
               }\n";
    let (core, _table) = lower_src(src);

    let h = nodes(&core)
        .into_iter()
        .find_map(|n| match &n.kind {
            CoreKind::Handle(h) => Some(h.clone()),
            _ => None,
        })
        .expect("the handle should have lowered to a CoreKind::Handle");

    assert_eq!(h.clauses.len(), 1);
    assert_eq!(h.clauses[0].effect, "Log");
    assert_eq!(h.clauses[0].op, "log");
    assert_eq!(h.clauses[0].params.len(), 1);
    assert_eq!(h.clauses[0].params[0].name, "m");

    // The `with multi` bit is stamped here and refused later (§5.3/§5.4).
    assert!(!h.is_multi_declared, "plain `with` is not multi");

    // `return(x) -> x` is present, and its body is a plain Var — not a lambda.
    let r = h.ret.as_ref().expect("return clause");
    assert_eq!(r.binder, "x");
    assert!(matches!(r.body.kind, CoreKind::Var(ref v) if v == "x"));

    // The clause body is `resume(m)` — a Resume node, not an App of a `resume` Var.
    assert!(matches!(h.clauses[0].body.kind, CoreKind::Resume(_)));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `$env:CARGO_INCREMENTAL="0"; cargo test -p elya --test core_lowering a_handle_lowers`
Expected: FAIL to **compile** — `CoreKind::Handle` does not exist. That is the
intended first failure; the test cannot be written against a type that is absent.

- [ ] **Step 3: Add the Core types**

In `src/core.rs`, next to `CoreArm` and `CoreParam`, add the three structs exactly as
given in **Interfaces** above, each with the same derives the neighbouring structs
carry (read `CoreArm`'s derive line and copy it). Then add the two variants at the
end of `CoreKind`, after `Match`:

```rust
    /// `handle <body> with { .. }` (5b-8 §4). Clause bodies are ordinary
    /// `CoreExpr`: `resume` is a syntactic form, not a value, so a captured
    /// continuation needs no Core node of its own.
    Handle(Rc<CoreHandle>),
    /// `resume(e)` — only well-formed inside a clause body. The type checker
    /// has already established that; Core does not re-check it.
    Resume(Rc<CoreExpr>),
```

- [ ] **Step 4: Build, and fix the three matches that break**

Run: `$env:CARGO_INCREMENTAL="0"; cargo build --workspace`
Expected: FAIL, three non-exhaustive-match errors. They are deliberate — each of
these matches omits a catch-all so that growing `CoreKind` cannot silently skip a
node. Fix all three now; the commit is not splittable, because the build is red until
the last one lands.

**(a) `crates/codegen/src/closure.rs:70`, `fv_walk`.** Free variables of a handle are
the body's, plus each clause's minus that clause's params, plus the return clause's
minus its binder. Under-capture becomes a use-after-free, so the binder handling is
the whole point:

```rust
        CoreKind::Handle(h) => {
            fv_walk(&h.body, scope, out);
            for c in h.clauses.iter() {
                let depth = scope.len();
                for p in c.params.iter() {
                    scope.push(p.name.clone());
                }
                fv_walk(&c.body, scope, out);
                scope.truncate(depth);
            }
            if let Some(r) = &h.ret {
                let depth = scope.len();
                scope.push(r.binder.clone());
                fv_walk(&r.body, scope, out);
                scope.truncate(depth);
            }
        }
        CoreKind::Resume(v) => fv_walk(v, scope, out),
```

**(b) `src/core.rs`, `pretty_expr`.** Follow the printer's existing formatting
conventions for the surrounding arms — read two neighbouring arms and match their
indentation and parenthesisation style. The output feeds `pretty_typed`, which
several tests snapshot, so the shape only has to be stable and readable:

```rust
        CoreKind::Handle(h) => {
            let mut s = format!("(handle {}", pretty_expr(&h.body, p));
            if h.is_multi_declared {
                s.push_str(" multi");
            }
            for c in h.clauses.iter() {
                let ps: Vec<String> = c.params.iter().map(|x| x.name.clone()).collect();
                s.push_str(&format!(
                    " [{}.{}({}) -> {}]",
                    c.effect,
                    c.op,
                    ps.join(", "),
                    pretty_expr(&c.body, p),
                ));
            }
            if let Some(r) = &h.ret {
                s.push_str(&format!(" [return({}) -> {}]", r.binder, pretty_expr(&r.body, p)));
            }
            s.push(')');
            s
        }
        CoreKind::Resume(v) => format!("(resume {})", pretty_expr(v, p)),
```

**(c) `tests/core_lowering.rs`, `nodes`.** The pre-order walker. If it misses the new
children, `both_origin_proofs_and_no_token_leak_across_corpus` stops checking them —
a proof that quietly narrows is worse than one that fails:

```rust
            CoreKind::Handle(h) => {
                go(&h.body, out);
                for c in h.clauses.iter() {
                    go(&c.body, out);
                }
                if let Some(r) = &h.ret {
                    go(&r.body, out);
                }
            }
            CoreKind::Resume(v) => go(v, out),
```

Match the recursion helper's real name and signature — read the existing arms of
`nodes` and follow them exactly.

- [ ] **Step 5: Lower the two forms**

In `src/core.rs`, replace the two refusals at `:348-349`:

```rust
        Expr::Handle { body, handler } => {
            let b = lower_expr(&body.node, body.span, table, ctors)?;
            let mut clauses = Vec::with_capacity(handler.clauses.len());
            for c in handler.clauses.iter() {
                // Same param-type lookup as the Lambda arm at :316-326: the type
                // comes out of the frozen table keyed by the param's own span, and
                // a missing entry is `Untyped`, never a guess.
                let mut ps = Vec::with_capacity(c.node.params.len());
                for p in &c.node.params {
                    let ty = table
                        .get(&p.span)
                        .cloned()
                        .ok_or(LowerError::Untyped(p.span))?;
                    ps.push(CoreParam {
                        name: p.node.name.clone(),
                        ty,
                    });
                }
                clauses.push(CoreClause {
                    effect: c.node.effect.clone(),
                    op: c.node.op.clone(),
                    params: ps.into(),
                    body: Rc::new(lower_expr(
                        &c.node.body.node,
                        c.node.body.span,
                        table,
                        ctors,
                    )?),
                });
            }
            let ret = match &handler.ret {
                Some(r) => Some(Rc::new(CoreReturn {
                    binder: r.binder.clone(),
                    body: Rc::new(lower_expr(&r.body.node, r.body.span, table, ctors)?),
                })),
                None => None,
            };
            CoreKind::Handle(Rc::new(CoreHandle {
                body: Rc::new(b),
                clauses: clauses.into(),
                ret,
                is_multi_declared: handler.multi,
            }))
        }
        Expr::Resume { arg } => {
            CoreKind::Resume(Rc::new(lower_expr(&arg.node, arg.span, table, ctors)?))
        }
```

Check the field names on `OpClause` before compiling this — `c.node.effect` and
`c.node.op` are written from the corpus syntax `Log.log(m)`. If the struct spells
them differently, follow the struct.

- [ ] **Step 6: Run to verify it passes**

Run: `$env:CARGO_INCREMENTAL="0"; cargo test -p elya --test core_lowering`
Expected: PASS, the whole file — including the corpus walker, which now visits handle
subtrees.

- [ ] **Step 7: Write the failing test for an applied resume**

The state-passing encoding applies the resumed computation: `resume(s)(s)`. The CPS
transform in Task 7b needs the resume in a *named* position, so lowering
A-normalizes it. Only an **applied** resume is rewritten — a bare `resume(v)` in tail
position is left exactly as it is:

```rust
#[test]
fn an_applied_resume_is_a_normalized_but_a_bare_one_is_not() {
    // `resume(v)(s)` becomes `let $k = resume(v) in $k(s)`, so Task 7b sees the
    // resume in a named position. `$k` is SYNTHESIZED, not recorded: it has no
    // source span, and its type is taken from the resume node rather than looked
    // up in the frozen table.
    let src = "effect St { fn get() -> Int }\n\
               pub fn main() {\n\
               \x20 let f = handle { get() } with {\n\
               \x20   St.get() -> fn(s) { resume(s)(s) }\n\
               \x20 }\n\
               \x20 io.println(\"done\")\n\
               }\n";
    let (core, table) = lower_src(src);

    let lets: Vec<_> = nodes(&core)
        .into_iter()
        .filter_map(|n| match &n.kind {
            CoreKind::Let(name, v, _) if name == "$k" => Some((n, v.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(lets.len(), 1, "exactly one $k binding");
    assert!(matches!(lets[0].1.kind, CoreKind::Resume(_)), "$k binds the resume");

    // Synthesized, not recorded: `$k`'s own type came from the resume node, and
    // the Let carries the resume's span rather than inventing one.
    assert_eq!(lets[0].0.span, lets[0].1.span);
    assert!(table.contains_key(&lets[0].0.span));
}
```

If the corpus program above is rejected by inference (the `St` handler returns a
function, and effect-row inference is the least settled part of the front end), cut
it down until it type-checks, and record what you cut in the commit message — the
assertion is about the *shape lowering produces*, not about that specific program.

- [ ] **Step 8: Run to verify it fails, then implement**

Run: `$env:CARGO_INCREMENTAL="0"; cargo test -p elya --test core_lowering an_applied_resume`
Expected: FAIL — zero `$k` bindings, because `Expr::Call` lowers the resume callee
through the generic path.

In `src/core.rs`, inside the `Expr::Call { callee, args }` arm, add this **before**
the generic callee lowering and after the existing `Qualified`/ctor special cases:

```rust
            if let Expr::Resume { arg } = &callee.node {
                // A-normalize an APPLIED resume so Task 7b sees it named:
                //     resume(v)(args)  ==>  let $k = resume(v) in $k(args)
                // A bare `resume(v)` never reaches here — it is not a Call.
                let k = lower_expr(&callee.node, callee.span, table, ctors)?;
                let k_ty = k.ty.clone();
                let mut lowered = Vec::with_capacity(args.len());
                for a in args.iter() {
                    lowered.push(lower_expr(&a.node, a.span, table, ctors)?);
                }
                let call = CoreExpr {
                    span,
                    ty: ty.clone(),
                    kind: CoreKind::App(
                        Rc::new(CoreExpr {
                            span: callee.span,
                            ty: k_ty,
                            kind: CoreKind::Var("$k".to_string()),
                        }),
                        lowered.into(),
                    ),
                };
                return Ok(CoreExpr {
                    span: callee.span,
                    ty,
                    kind: CoreKind::Let("$k".to_string(), Rc::new(k), Rc::new(call)),
                });
            }
```

`arg` is unused in the guard — it is destructured only to match the shape; write
`if let Expr::Resume { .. } = &callee.node` if the compiler warns.

- [ ] **Step 9: Run to verify it passes**

Run: `$env:CARGO_INCREMENTAL="0"; cargo test -p elya --test core_lowering`
Expected: PASS, the whole file.

- [ ] **Step 10: Gate and commit**

```bash
git add src/core.rs crates/codegen/src/closure.rs tests/core_lowering.rs
git commit -F /tmp/msg-t5.txt
```

---


### Task 6: Introduce the frame tag and its one descriptor row (A8), and re-measure `gc_mark` (A7)

**Files:**
- Modify: `crates/codegen/src/lib.rs:1253-1255` (tag arithmetic), `:1386-1396` (per-lambda guard), `:1406-1417` (row-count guard and the string row)
- Test: no new committed test — see "Why there is no red step" below
- Measure: `crates/codegen/src/runtime.c` (read only; **must not be edited by this task**)

**Interfaces:**
- Consumes: Task 1's recorded A7 baseline byte count and the exact command that produced it.
- Produces: `frame_tag`, a `usize` in scope in the same function as `string_tag`, equal to
  `string_tag + 1`; and one appended descriptor row `[2, 0b10]`. Task 8 stores `frame_tag`
  into word 0 of every frame cell it allocates.

**A correction this task exists to carry.** The spec's §6.3 says the frame row is
"arity 2, mask `0b11` (both `code_ptr` and `next` are pointers)". That is **wrong
against the code**, and the code is right. The descriptor convention is stated at
`crates/codegen/src/lib.rs:1399-1401`:

> Bit 0 is CLEAR: word 1 is a code pointer into the text segment, not a heap object.
> Bit j+1 is set iff capture j is a heap value.

A code pointer is deliberately **not traced**. So a frame `[tag][code_ptr][next]` is
`arity = 1 + 1 = 2`, `mask = 1 << (0 + 1) = 0b10`. Use `0b10`. Do not "fix" the code to
match the spec; record the spec deviation in the commit message and it will be carried to
close-out.

**Why this is one row and not zero.** Two true statements that sound contradictory:

- The row's **contents** are not novel. `[2, 0b10]` is byte-for-byte the row any
  one-capture lambda whose capture is a heap value already emits. That is why `gc_mark`
  needs no new case: the mark phase reads arity and mask generically, so a shape it
  already traces adds no dispatch path. This is the whole of A7.
- The row **count** still goes up by one. Rows are indexed **positionally by tag**
  (`desc[2*tag]`, `desc[2*tag+1]`), so a distinct tag requires a distinct row even when
  the row duplicates an existing one. This is A8.

An earlier draft of this plan collapsed the two into "a captured frame adds no descriptor
row." That was wrong; §11 lists A7 and A8 as separate criteria of different kinds
(measurement vs structural), and this task discharges both without conflating them.

**Why there is no red step.** Nothing allocates a frame until Task 8, so there is no
user-visible behavior to drive a failing test from. §11 is explicit about the discharge
kind: A8 is "structural (discharged by the check)" and A7 is "measurement". Manufacturing
a red test here would assert tag arithmetic against itself. Instead this task proves the
guard is *live* by breaking it on purpose and reverting (Step 4), which is an observable
result, and proves A7 by re-running Task 1's command (Step 5).

- [ ] **Step 1: Extend the tag arithmetic**

At `crates/codegen/src/lib.rs:1255`, directly under the existing `string_tag` line:

```rust
    let string_tag = n_real_ctors + lambdas.len();
    // N8 §6.1: one synthetic tag for a captured-continuation frame cell,
    // `[tag][code_ptr][next]`. It continues the same linear numbering as the
    // lambda tags and the string tag, so the "tag == row index" property the two
    // guards below enforce extends to it unchanged.
    let frame_tag = string_tag + 1;
```

Leave `string_tag` exactly as it is. Its guard at `:1412` compares against
`desc.len() / 2` *before* the string row is pushed, so inserting the frame row
**after** the string row keeps that comparison correct without editing it.

- [ ] **Step 2: Append the frame descriptor row**

Immediately after the two `desc.push(0);` lines that close out the string row
(`crates/codegen/src/lib.rs:1416-1417`):

```rust
    // N8 §6.3 / A8: ONE frame row, appended after the string row so the string
    // guard above still sees the table length it expects. Same guard shape as the
    // string tag: "the tag agrees with its row index" becomes a compile-time
    // property rather than a comment.
    if frame_tag != desc.len() / 2 {
        return Err(CodegenError::Unsupported(
            "frame tag disagrees with its descriptor row index",
        ));
    }
    desc.push(2); // arity = 2: the code pointer and `next` follow the tag
    desc.push(0b10); // bit 0 clear (code pointer, not traced); bit 1 set (`next` is heap)
```

Note what is **not** here: no change to `gc_mark`, no third kind of guard, and no new
variant on `CodegenError` — the payload string is new but the variant is the existing
`Unsupported(&'static str)`.

- [ ] **Step 3: Build and run the codegen suite**

```
$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen
```

Expected: PASS, with the same test count as before this task. A `frame_tag` that is
computed but not yet stored anywhere will draw an `unused_variable` warning in builds
where Step 1 landed without Step 2 — if you see that warning *after* Step 2, Step 2 did
not actually reference `frame_tag`, and the guard you just added is dead. Fix it before
continuing.

- [ ] **Step 4: Prove the new guard is live, then revert**

Temporarily change Step 1's line to `let frame_tag = string_tag + 2;` and re-run
`cargo test -p elya-codegen`. Expected: failures naming
`frame tag disagrees with its descriptor row index`.

Then restore `string_tag + 1` and re-run to confirm green. **Do not commit the broken
state**, and do not leave a committed test that depends on it — this is a live-guard
check, not a test case.

- [ ] **Step 5: Re-measure A7 and compare to Task 1's baseline**

Run the exact command Task 1 recorded, from the repo root:

```
awk '/^static void gc_mark\(void\) \{/,/^\}/' crates/codegen/src/runtime.c | wc -c
```

Expected: **the identical byte count Task 1 wrote down.** This task does not edit
`runtime.c` at all, so any movement means something else in the working tree touched it.

**If the number moved, §11 A7 governs: the deviation is reported, not patched.** Stop,
do not adjust `runtime.c` to make the number match, and report to the reviewer with both
numbers and `git diff crates/codegen/src/runtime.c`. A7's value comes from it being a
measurement nobody is allowed to tune.

- [ ] **Step 6: Full gate**

```
powershell -NoProfile -File scripts/check.ps1
```

Foreground, unpiped. Read the exit code; continue only on 0.

- [ ] **Step 7: Commit**

```bash
cat > /tmp/msg-t6.txt <<'EOF'
feat(codegen): frame tag and its descriptor row (A8), gc_mark unmoved (A7)

One synthetic tag for a captured-continuation frame cell, continuing the
same linear numbering as the lambda and string tags, plus one descriptor
row and the guard that pins tag == row index.

The row is arity 2, mask 0b10 -- NOT the 0b11 the spec's 6.3 states. Bit 0
is deliberately clear because word 1 is a code pointer into the text
segment and is not traced (lib.rs:1399-1401). The spec deviation is
recorded here rather than patched into the code.

A7 re-measured by Task 1's command; gc_mark byte count unchanged, third
consecutive slice. runtime.c is untouched by this commit.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
git add crates/codegen/src/lib.rs
git commit -F /tmp/msg-t6.txt
```

---

### Task 7a: The selective-CPS partition predicate, in an LLVM-free module

**Files:**
- Create: `crates/codegen/src/cps.rs`
- Modify: `crates/codegen/src/lib.rs:20` (add `mod cps;`)
- Test: `crates/codegen/src/cps.rs`, in its own `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: nothing from earlier tasks. It reads `elya::types::{Ty, EffectRow, RowTail}` only.
- Produces:
  ```rust
  /// True iff a call to a value of this type may need its continuation captured.
  pub fn needs_cps(ty: &Ty) -> bool;
  ```
  Task 7b calls it at every call site, on the **callee's** `CoreExpr.ty`, and on each
  `CoreFn`'s own signature.

**Why a separate module, and why LLVM-free.** `needs_cps` is a pure function of a `Ty`.
Kept out of the LLVM-typed code it needs no `Context`, no target triple, no `clang`, and
no temp directory — so its tests are unit tests that run everywhere in milliseconds.
`crates/codegen/src/lib.rs` is ~2000 lines and `mod closure;` is currently its only
submodule; this is the second, and it follows the same shape.

**The anchor, measured.** `crates/codegen/src/lib.rs:594` already destructures the callee
type and **throws the row away**:

```rust
    let Ty::Fn(param_tys, _, ret_ty) = &callee.ty else {
        return Err(CodegenError::Unsupported("computed callee"));
    };
```

That `_` is the latent effect row of §8.1. This task's whole job is to give that position
a name and a question. Task 7b is what changes the `_`.

**Why `label != "IO"` is sound, and what it depends on.** §8.2: the key is "the row
mentions a *user-declared* effect", not "the row is non-pure" — because `io.println`
performs `{IO}` and would otherwise drag every printing function into CPS (§9.2). Codegen
has no `Decl::Effect` list — `CoreModule` carries no effect declarations — so the only
question it can ask is `label != "IO"`. **That is sound only because Task 2's D1 refuses a
user-declared `effect IO`.** Task 2 is a hard prerequisite of this task, not housekeeping;
if D1 were dropped, a user `effect IO` would be silently classified as the builtin and
compiled without CPS. There is a precedent for treating `"IO"` as a distinguished name:
`src/types.rs:42`, `const OBSERVABLE_EFFECTS: &[&str] = &["IO"]`, used by the E0426 lint.

- [ ] **Step 1: Write the failing tests**

Create `crates/codegen/src/cps.rs` containing **only** the test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use elya::span::Span;
    use elya::types::{EffectLabel, EffectRow, RowTail, Ty, TyCon};
    use std::collections::BTreeMap;

    fn row(labels: &[&str], tail: RowTail) -> EffectRow {
        let mut m = BTreeMap::new();
        for l in labels {
            m.insert(
                (*l).to_string(),
                EffectLabel { args: Vec::new(), span: Span::EMPTY },
            );
        }
        EffectRow { labels: m, tail }
    }

    fn func(r: EffectRow) -> Ty {
        Ty::Fn(vec![Ty::Base(TyCon::Int)], r, Box::new(Ty::Base(TyCon::Int)))
    }

    #[test]
    fn a_pure_function_is_a_direct_call() {
        assert!(!needs_cps(&func(EffectRow::pure())));
    }

    #[test]
    fn a_printing_function_is_still_a_direct_call() {
        // The whole point of 8.2: `{IO}` must NOT select CPS, or every
        // function that prints pays for machinery it cannot use.
        assert!(!needs_cps(&func(row(&["IO"], RowTail::Closed))));
    }

    #[test]
    fn a_user_declared_effect_selects_cps() {
        assert!(needs_cps(&func(row(&["State"], RowTail::Closed))));
    }

    #[test]
    fn a_user_effect_alongside_io_still_selects_cps() {
        assert!(needs_cps(&func(row(&["IO", "State"], RowTail::Closed))));
    }

    #[test]
    fn an_unresolved_tail_selects_cps_conservatively() {
        // Over-CPS costs speed. Under-CPS is a wrong answer. Same asymmetry
        // `fv_walk` cites for over- vs under-capture (closure.rs:66-75).
        let v = RowTail::ErrorRow;
        assert!(needs_cps(&func(row(&[], v))));
    }

    #[test]
    fn a_non_function_type_is_not_a_call_site_at_all() {
        assert!(!needs_cps(&Ty::Base(TyCon::Int)));
    }
}
```

If `RowTail::Open` needs a `RowVar` you cannot construct from outside `types.rs`, drop
`an_unresolved_tail_selects_cps_conservatively` to the `ErrorRow` case only, as written
above — `ErrorRow` is a unit variant and needs no constructor. Do not add a public
constructor to `types.rs` for the sake of a test.

- [ ] **Step 2: Wire the module and watch it fail to compile**

Add to `crates/codegen/src/lib.rs`, immediately after `mod closure;` at `:20`:

```rust
mod cps;
```

Run:

```
$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen --lib cps
```

Expected: **compile error**, `cannot find function 'needs_cps' in this scope`. The tests
cannot run against a function that does not exist; that is the red step.

- [ ] **Step 3: Write the predicate**

At the top of `crates/codegen/src/cps.rs`, above the test module:

```rust
//! The selective-CPS partition (spec §8.1-8.2).
//!
//! Deliberately LLVM-free: this is a pure question about a `Ty`, so it needs no
//! `Context` and its tests are unit tests.

use elya::types::{EffectRow, RowTail, Ty};

/// The one effect label the back end treats as builtin. Sound as a *name* test
/// only because lowering refuses a user-declared `effect IO` (D1, Task 2) --
/// `CoreModule` carries no effect declarations, so the name is all codegen has.
/// Mirrors `src/types.rs:42`'s `OBSERVABLE_EFFECTS`.
const BUILTIN_EFFECT: &str = "IO";

/// True iff a call to a value of this type may need its continuation captured.
///
/// A non-function type is not a call site and answers `false`.
pub fn needs_cps(ty: &Ty) -> bool {
    match ty {
        Ty::Fn(_, row, _) => row_needs_cps(row),
        _ => false,
    }
}

fn row_needs_cps(row: &EffectRow) -> bool {
    if row.labels.keys().any(|l| l != BUILTIN_EFFECT) {
        return true;
    }
    match row.tail {
        RowTail::Closed => false,
        // A row we cannot see the end of might carry a user-declared effect.
        // Answer conservatively: over-CPS costs speed, under-CPS is a wrong
        // answer -- the asymmetry `closure.rs:66-75` states for over- vs
        // under-capture. If the codegen suite starts hitting this arm in
        // practice, that is a signal to investigate why an unresolved row
        // survived inference, NOT a signal to flip the default.
        RowTail::Open(_) | RowTail::ErrorRow => true,
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

```
$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen --lib cps
```

Expected: PASS, 6 tests (or 5 if you dropped the `Open` case per Step 1).

- [ ] **Step 5: Full gate**

```
powershell -NoProfile -File scripts/check.ps1
```

Foreground, unpiped. Read the exit code; continue only on 0.

- [ ] **Step 6: Commit**

```bash
cat > /tmp/msg-t7a.txt <<'EOF'
feat(codegen): the selective-CPS partition predicate

needs_cps reads the latent effect row out of a callee's Ty::Fn -- the
middle field that lib.rs:594 currently destructures as `_`. The key is
"mentions a user-declared effect", not "is non-pure", because io.println
performs {IO} and the non-pure reading drags every printing function into
CPS (spec 8.2 / 9.2).

Testing the name "IO" is sound only because lowering refuses a
user-declared `effect IO` (D1). That refusal is a prerequisite of this
predicate, not housekeeping.

An unresolved row tail answers true: over-CPS costs speed, under-CPS is a
wrong answer.

LLVM-free module so the tests need no Context, triple, or clang.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
git add crates/codegen/src/cps.rs crates/codegen/src/lib.rs
git commit -F /tmp/msg-t7a.txt
```

---

### Task 7b: The effectful calling convention — measure the seam, then build it

**Files:**
- Modify: `crates/codegen/src/cps.rs`, `crates/codegen/src/lib.rs`
- Possibly modify: `crates/codegen/src/runtime.c` — **only outside `gc_mark`** (A7)
- Test: `crates/codegen/tests/native_codegen.rs`

**Interfaces:**
- Consumes: `cps::needs_cps` (Task 7a); `frame_tag` (Task 6); `CoreKind::Handle` /
  `CoreKind::Resume` and the `CoreHandle`/`CoreClause`/`CoreReturn` structs (Task 5).
- Produces: the frame-cell constructor and the effectful-call convention that Task 8's
  dispatch, splice and trampoline are written against. Its exact signatures are **fixed
  by Step 2's measurement** and must be written into the checkpoint report before any
  emission code is committed.

**This is the largest task in the plan and it opens with a measurement, not a test.**
§9.3 asked for honest sizing; this is it. The spec fixes *what* the convention must
achieve (§8.1, §8.3) and *what shape a frame has* (§6.1), but it does not fix **where the
capture/resume machinery lives** — emitted IR or `runtime.c`. That is a real decision with
a real cost either way, and guessing it is how a task like this overruns into Task 8,
which §9.3 explicitly says must not happen because Task 8 carries the trampoline
requirement.

- [ ] **Step 1: Read the two function-emission paths and record what you find**

```
grep -n "fn emit_fn\|fn emit_body\|fn emit_expr\|fn emit_call" crates/codegen/src/lib.rs
sed -n '265,320p' crates/codegen/src/lib.rs
sed -n '690,720p' crates/codegen/src/lib.rs
sed -n '870,900p' crates/codegen/src/lib.rs
```

Write down, in the checkpoint report: the exact signature of the function that emits a
`CoreFn`; whether `CoreKind::App` has one emission site or two (the grep in this plan's
preparation found `CoreKind::App` at both `:697` and `:877` — find out what distinguishes
them before assuming they are the same path); and whether the direct-call path at `:594`
is shared by both.

- [ ] **Step 2: Read the runtime's existing seam**

```
grep -n "^[a-z].*(" crates/codegen/src/runtime.c | head -40
sed -n '150,210p' crates/codegen/src/runtime.c
```

`elya_alloc` is already an extern C function the emitted module calls
(`crates/codegen/src/lib.rs:1293`, `// ccc`). Record: how many such externs exist, how
they are declared to LLVM, and whether any of them currently call back into emitted code.
That last one is the decisive question — a trampoline in `runtime.c` requires calling a
code pointer out of a frame cell, and if no existing extern does that, this task is adding
the first such edge and owes it a note.

- [ ] **Step 3: CHECKPOINT — stop and report before writing emission code**

Report to the reviewer, in one message:

1. The two shapes, named, with the cost you measured for each:
   - **(A) Runtime-helper shape.** Emitted code calls `elya_frame_push`,
     `elya_perform`, `elya_resume` as externs; the cons-list walk and the trampoline loop
     live in `runtime.c`, alongside — but strictly outside — `gc_mark`. Cost: a new
     runtime↔emitted-code edge, and `runtime.c` grows in a file A7 watches (A7 measures
     only the `gc_mark` function body, so growth elsewhere in the file is allowed, but it
     must be *stated*, not slipped in).
   - **(B) Emitted-IR shape.** Every frame push, pop and trampoline iteration is emitted
     inline. Cost: substantially more `lib.rs`, and the trampoline becomes an emitted loop
     in a function whose other basic blocks you are already juggling.
2. Which one the measurement in Steps 1-2 favors, and why.
3. Whether Step 2's "does any extern call back into emitted code" answer changed that.

**Do not proceed past this checkpoint on your own judgment.** If the reviewer is
unavailable, stop here and leave the branch unpicked — an unfinished 7b with a clean
checkpoint is a better handoff than a finished 7b built on a guess, because Task 8's
trampoline is written against whichever convention this step fixes.

- [ ] **Step 4: Write the failing execution test for frame capture**

Whichever branch, the first observable deliverable is identical: a program that captures
one frame compiles, links, and prints the right answer. **Surviving a collection is not
provable here** — this program allocates a handful of words and the collector never runs.
That property belongs to Task 10, which allocates enough to force one.

The helpers, measured rather than assumed:

- `assert_runs(exe: &Path, expected: &str)` — takes a **built executable and the expected
  trimmed stdout**, not a source string, and asserts exit status, stdout and empty stderr
  (`crates/codegen/tests/native_codegen.rs:224`). Main's `Int` reaches stdout because the
  print shim emits it as the final line, so the expected value is a **string**, not an
  exit code.
- `lower_src(src) -> CoreModule` (`:162`), `temp_dir(tag)` (`:20`),
  `compile_and_link(&core, &dir, tag) -> PathBuf` (`:196`).

The effect and handler surface syntax is likewise measured, from
`tests/state_effect.rs:56-71` — note the `fn` keyword inside the `effect` block, which is
easy to drop:

```rust
#[test]
fn a_non_tail_resume_compiles_and_runs() {
    // The smallest program that captures exactly one frame: `get()` is performed
    // in operand position, so `+ 1` is a frame that must be captured and resumed
    // into. NOTE: no `<>` anywhere -- codegen refuses string concatenation by
    // name (spec 9.1), and that refusal applies to test corpora too.
    let src = "effect State { fn get() -> Int }
               fn body() -> Int { get() + 1 }
               pub fn main() -> Int {
                 handle { body() } with {
                   State.get() -> resume(41)
                   return(x) -> x
                 }
               }
";
    let dir = temp_dir("frame-capture");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, "frame-capture");
    assert_runs(&exe, "42");
    std::fs::remove_dir_all(&dir).ok();
}
```

Do not add a second `assert_runs`. If the program does not type-check as written, fix the
**program** against `tests/state_effect.rs`'s known-good forms — never the helper.

- [ ] **Step 5: Run it and confirm the failure is the one you expect**

```
$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen --test native_codegen a_non_tail_resume -- --nocapture
```

Expected before any emission work: a `CodegenError::Unsupported` naming the handle node,
because Task 5 added the Core variants and nothing in codegen matches them yet. **If it
fails any other way — a panic, a `verify()` error, a link failure — that is new
information about a path this plan did not anticipate.** Record it in the checkpoint
report before continuing.

- [ ] **Step 6: Build the frame constructor**

Allocate a frame cell exactly as the closure site at `crates/codegen/src/lib.rs:902-926`
allocates a closure — same `lc.alloc` call, same word-0 tag store, same
`build_ptr_to_int` for the code pointer — with `frame_tag` from Task 6 in word 0 and the
`next` pointer in word 2. The existing site is the template; read it and mirror it rather
than writing a second allocation idiom.

Two things that site does which you must not drop: `gc_root_env` is called **before** the
allocation, because `elya_alloc` can collect and the environment must be rooted across it
(`:900-901`); and word 1 is stored as an integer word, never as a pointer value.

- [ ] **Step 7: Run the test to verify it passes**

```
$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen --test native_codegen a_non_tail_resume -- --nocapture
```

Expected: PASS, stdout `42`.

- [ ] **Step 8: Re-measure A7**

```
awk '/^static void gc_mark\(void\) \{/,/^\}/' crates/codegen/src/runtime.c | wc -c
```

Expected: Task 1's baseline, unchanged. If branch (A) was taken, this is the step that
proves the new runtime helpers landed **outside** `gc_mark`. If the number moved, §11 A7
governs — report, do not patch.

- [ ] **Step 9: Full gate**

```
powershell -NoProfile -File scripts/check.ps1
```

Foreground, unpiped. Read the exit code; continue only on 0.

- [ ] **Step 10: Commit**

```bash
cat > /tmp/msg-t7b.txt <<'EOF'
feat(codegen): frame capture and the effectful calling convention

Frame cells are allocated by the same idiom as closures (lib.rs:902-926):
gc_root_env first, tag in word 0, code pointer stored as an integer word
and not traced, `next` in word 2. That identity is the point -- it is what
keeps gc_mark out of the slice.

Convention shape chosen at the Step 3 checkpoint: <A or B, one line why>.

A7 re-measured; gc_mark byte count unchanged.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
git add crates/codegen/src/lib.rs crates/codegen/src/cps.rs crates/codegen/tests/native_codegen.rs
git commit -F /tmp/msg-t7b.txt
```

Add `crates/codegen/src/runtime.c` to the `git add` line **only** if branch (A) was taken.
Explicit paths; never `-A`.

---

### Task 8: Handler dispatch, the splice, the trampoline, and the `with multi` refusal (A2)

**Files:**
- Modify: `crates/codegen/src/lib.rs`
- Test (refusal, unit): `crates/codegen/src/lib.rs`'s `mod tests` — next to the existing
  refusal precedent at `:2038-2085`. A refusal produces no binary, and the integration
  file's module doc forbids non-executing tests.
- Test (dispatch, execution): `crates/codegen/tests/native_codegen.rs`

**Interfaces:**
- Consumes: `CoreHandle { body, clauses, ret, is_multi_declared }`, `CoreClause { effect,
  op, params, body }`, `CoreReturn { binder, body }` (Task 5); `frame_tag` (Task 6);
  `needs_cps` (Task 7a); the frame constructor and calling convention (Task 7b).
- Produces: a compiling `handle`/`resume`. Tasks 9-12 are proofs over it and add no
  machinery.

**Order within the task is deliberate: the refusal lands first.** §5.4 requires the
`multi` refusal to fire "on the handle node, **before lowering any clause body**", and A2
requires it be proved by **execution**. Writing it first means it is green and committed
before the dispatch machinery exists to complicate it — and it means the refusal is proved
to fire before clause lowering by the strongest possible evidence: at that commit there
*is* no clause lowering.

**What the front end already does, and why it is not enough.** `src/types.rs:1319-1336`
emits **E0426**, a *warning*, when a `with multi` handler's body performs an observable
effect. `tests/effect_types.rs:210` pins it: `"E0426 must be a warning"`. A warning does
not stop compilation. The evaluator, separately, *implements* multi-shot — `src/eval.rs:1000`
skips one-shot enforcement for `with multi`. So all three layers differ on purpose:
front end warns, evaluator runs it, native refuses it. **Do not "unify" them.** In
particular do not promote E0426 to an error; that changes front-end behavior, breaks
`tests/effect_types.rs`, and is outside this slice.

This asymmetry is also why Task 9's differential corpus must contain no `with multi`
program: the evaluator would run it and the native back end would refuse it, and the
harness would report a difference that is the design working correctly.

- [ ] **Step 1: Write the two Core-level refusal tests**

**They go in `crates/codegen/src/lib.rs`'s unit-test module (`mod tests`, opens at
`:1533`) — not in the integration test file.** Three measured facts force this:

1. `crates/codegen/tests/native_codegen.rs` states its own doctrine in its module doc
   (`:1-4`): *"Every test here produces a native binary, RUNS it, and asserts on exit
   status, stdout, and stderr. Proof is execution, never IR inspection."* A refusal
   produces no binary, so it does not belong in that file.
2. That file has **no** error-returning helper and does not even import `CodegenError` —
   `grep -n "unwrap_err\|CodegenError" crates/codegen/tests/native_codegen.rs` returns
   nothing. There is nothing there to reuse.
3. The existing refusal precedent is already in `lib.rs`:
   `rejects_an_unknown_builtin_by_its_owned_name_before_arguments` (`:2038`) and
   `io_println_refuses_the_wrong_arity_before_lowering_arguments` (`:2065`). Both build
   Core **directly** and call `emit_ir(&main_fn(body)).unwrap_err()`.

Follow that precedent exactly. Put these next to it, using the module's existing
`int_lit` (`:1538`) and `main_fn` (`:1554`) builders:

```rust
    #[test]
    fn a_multi_shot_handler_is_refused_by_its_own_name() {
        let handle = CoreExpr {
            span: Span::EMPTY,
            ty: Ty::Base(TyCon::Int),
            kind: CoreKind::Handle(CoreHandle {
                body: Rc::new(int_lit(0)),
                clauses: Rc::from([]),
                ret: None,
                is_multi_declared: true,
            }),
        };
        let err = emit_ir(&main_fn(handle)).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("multi"), "the message must name the feature: {msg}");
    }
```

```rust
    #[test]
    fn the_multi_refusal_fires_before_any_clause_body_is_lowered() {
        // `Var("unbound")` in the clause body would refuse with its OWN message.
        // Seeing "multi" instead proves the handle-node refusal fired first
        // (§5.4). Same poisonous-subterm technique as lib.rs:2040.
        let poisonous = CoreExpr {
            span: Span::EMPTY,
            ty: Ty::Base(TyCon::Int),
            kind: CoreKind::Var("unbound".to_string()),
        };
        let handle = CoreExpr {
            span: Span::EMPTY,
            ty: Ty::Base(TyCon::Int),
            kind: CoreKind::Handle(CoreHandle {
                body: Rc::new(int_lit(0)),
                clauses: Rc::from([CoreClause {
                    effect: "Flip".to_string(),
                    op: "flip".to_string(),
                    params: Rc::from([]),
                    body: Rc::new(poisonous),
                }]),
                ret: None,
                is_multi_declared: true,
            }),
        };
        let err = emit_ir(&main_fn(handle)).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("multi"), "{msg}");
        assert!(
            !msg.contains("unbound"),
            "the clause body was lowered before the refusal fired: {msg}"
        );
    }
```

Adjust the struct literals to whatever field names and types Task 5 actually produced —
that task's **Interfaces / Produces** block is authoritative, not this snippet. Build the
value however Task 5's constructors permit; do not add a constructor just for the test.

- [ ] **Step 2: Add the source-level reachability test**

The two above prove the refusal fires when Core reaches codegen in that shape. They do
**not** prove a programmer can get there from source — and A2 is about a *reachable*
refusal. This third test closes that gap. It also belongs in `lib.rs`'s `mod tests`, which
can reach the front end (`use elya::span::Span;` at `:1535` shows `elya::` paths resolve
there):

```rust
    /// Source -> Core, with the front end allowed to WARN. `with multi` is
    /// deliberately not an error (see this task's preamble), so it type-checks
    /// and lowers; the refusal is codegen's alone.
    fn lower_warned_source(src: &str) -> CoreModule {
        let session = elya::Session::new();
        let (m, pd) = elya::parse::parse_module(&session, src);
        assert!(pd.is_empty(), "parse: {pd:?}");
        let (diags, table) = elya::types::infer_typed_table(&session, &m);
        assert!(
            !diags.iter().any(|d| d.severity == elya::diag::Severity::Error),
            "type errors: {diags:?}"
        );
        elya::core::lower_module(&m, &table).expect("must lower to Core")
    }

    #[test]
    fn a_multi_shot_handler_written_in_source_reaches_the_codegen_refusal() {
        // `effect multi` is required: `with multi` over a one-shot effect is
        // E0427, an ERROR, and would never reach codegen
        // (tests/effect_types.rs:276). No `<>` anywhere -- codegen refuses
        // string concatenation by name (§9.1). No IO inside the handled body,
        // which is what E0426 warns about (tests/effect_types.rs:195).
        let src = "effect multi Flip { fn flip() -> Bool }\n\
                   fn g() -> Int { if flip() { 1 } else { 0 } }\n\
                   pub fn main() -> Int {\n\
                     handle g() with multi { Flip.flip() -> resume(True) }\n\
                   }\n";
        let err = emit_ir(&lower_warned_source(src)).unwrap_err();
        assert!(err.to_string().contains("multi"), "{err:?}");
    }
```

Two things to check rather than assume, both cheap:

- **The `fn` keyword inside `effect` is mandatory** — `effect multi Flip { fn flip() -> Bool }`
  (`tests/effect_types.rs:292`). Dropping it is the easiest way to make this test fail for
  the wrong reason.
- If the program does not type-check as written, adapt it toward
  `with_multi_over_multi_effect_is_ok` (`tests/effect_types.rs:290-296`), which is a
  known-accepted `with multi` program — but strip its `<>`. Fix the **program**, never the
  helper, and never by promoting E0426.

If `Severity` or the diag module path differs, `grep -n "pub enum Severity" src/` settles
it in one command.

- [ ] **Step 3: Run all three tests and verify they fail**

```
$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen --lib multi -- --nocapture
```

Note `--lib`, not `--test native_codegen`: these are unit tests now.

Expected: FAIL. At this point `CoreKind::Handle` has no codegen arm at all, so the error
will not name `multi` yet.

- [ ] **Step 4: Write the refusal**

In the codegen expression match, as the **first** thing the `CoreKind::Handle` arm does —
before touching `h.body`, `h.clauses`, or `h.ret`:

```rust
        CoreKind::Handle(h) => {
            // 5.4 / A2: refuse on the handle node, BEFORE any clause body is
            // lowered, keyed on the bit Task 5 stamped off `Handler::multi`.
            // The message names the feature, not the machinery. Same shape as
            // io.println's arity refusal (lib.rs:794-808).
            if h.is_multi_declared {
                return Err(CodegenError::Unsupported(
                    "multi-shot handler (`with multi`)",
                ));
            }
            return Err(CodegenError::Unsupported("handle"));
        }
```

The second `return` is a **deliberate temporary floor**, not a placeholder: it keeps the
module compiling while Steps 1-3's refusal tests go green, and Step 7 replaces it with the
dispatch. It is removed inside this same task, before the task's commit.

- [ ] **Step 5: Run the tests to verify they pass**

```
$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen --lib multi -- --nocapture
```

Expected: PASS, all three.

- [ ] **Step 6: Gate and commit the refusal on its own**

```
powershell -NoProfile -File scripts/check.ps1
```

Then:

```bash
cat > /tmp/msg-t8a.txt <<'EOF'
feat(codegen): refuse `with multi` on the handle node (A2)

Keyed on the is_multi_declared bit lowering stamps off Handler::multi, and
fired before any clause body is lowered -- proved by a control whose clause
body contains a construct that would itself be refused with a different
message.

Three layers differ on purpose and stay that way: the front end warns
(E0426, a warning by test), the evaluator implements multi-shot
(eval.rs:1000), the native back end refuses it. E0426 is NOT promoted to an
error.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
git add crates/codegen/src/lib.rs
git commit -F /tmp/msg-t8a.txt
```

- [ ] **Step 7: Write the failing dispatch test — deep re-installation (A6)**

This is the §4 point-4 property and the one most likely to be got wrong: a handler resumed
inside a loop must perform more than once and find **the same handler** each time. No `<>`
anywhere.

```rust
#[test]
fn a_deep_handler_is_reinstalled_on_every_resume() {
    // Three performs, each finding the handler again: 2 + 2 + 2. The recursive
    // call sits under `+`, so each `get()` captures a real frame. The `fn`
    // keyword inside `effect` is mandatory (tests/state_effect.rs:57).
    let src = "effect State { fn get() -> Int }\n\
               fn loop_body(n) { if n == 0 { 0 } else { get() + loop_body(n - 1) } }\n\
               pub fn main() -> Int {\n\
                 handle { loop_body(3) } with {\n\
                   State.get() -> resume(2)\n\
                   return(x) -> x\n\
                 }\n\
               }\n";
    let dir = temp_dir("deep-reinstall");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, "deep-reinstall");
    assert_runs(&exe, "6");
    std::fs::remove_dir_all(&dir).ok();
}
```

The assertion is on **stdout**, not on an exit status: `assert_runs(exe: &Path, expected: &str)`
compares `String::from_utf8_lossy(&out.stdout).trim()` (`native_codegen.rs:224-234`), and
main's `Int` reaches stdout through the print shim. A run printing `2` means the handler
was found once and then lost — that is the deep re-installation bug, and this expectation
is calibrated to distinguish it from success.

- [ ] **Step 8: Run it and verify it fails on the temporary floor**

```
$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen --test native_codegen a_deep_handler_is_reinstalled -- --nocapture
```

Expected: FAIL with `Unsupported("handle")` — Step 4's floor.

- [ ] **Step 9: Replace the floor with dispatch**

Per §8.3, and against the convention Task 7b's checkpoint fixed:

1. Install a handler record: the `(effect, op)` pairs from `h.clauses`, the clause code
   pointers, and the handler's own captured environment.
2. Emit `h.body` under it.
3. On a perform of a matching `(effect, op)`: capture frames into the §6 cons list, then
   invoke the clause with the reified continuation and the handler's environment.
4. On `resume`: splice `k_cap ++ [handler] ++ k_now` — **handler beneath the captured
   frames**, per §4 point 4. This ordering is what Step 7's test measures; getting it
   backwards yields 2 instead of 6.
5. On the body's normal completion, run `h.ret` if present, binding `r.binder`; if absent,
   the body's value is the result (§7's identity case, already the evaluator's rule).

The `(effect, op)` match is on **two names** and nothing else — §8.3 is explicit that
nothing here needs a row at runtime. Do not reach for `EffectRow` in emitted code.

- [ ] **Step 10: Run the dispatch test to verify it passes**

```
$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen --test native_codegen a_deep_handler_is_reinstalled -- --nocapture
```

Expected: PASS, status 6.

- [ ] **Step 11: Confirm the refusal tests still pass**

```
$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen --lib multi -- --nocapture
```

Expected: PASS, all three. Step 9 removed the temporary floor but must not have removed
the `multi` branch above it. If `the_multi_refusal_fires_before_any_clause_body_is_lowered`
now reports `unbound`, the refusal has drifted below the clause lowering — fix the order,
do not relax the assertion.

- [ ] **Step 12: Re-measure A7**

```
awk '/^static void gc_mark\(void\) \{/,/^\}/' crates/codegen/src/runtime.c | wc -c
```

Expected: Task 1's baseline, unchanged. Report, do not patch.

- [ ] **Step 13: Full gate**

```
powershell -NoProfile -File scripts/check.ps1
```

Foreground, unpiped. Read the exit code; continue only on 0.

- [ ] **Step 14: Commit**

```bash
cat > /tmp/msg-t8b.txt <<'EOF'
feat(codegen): handler dispatch, continuation splice, deep re-installation

Install a handler record, run the body under it, capture frames into the
cons list on a matching perform, and on resume splice
k_cap ++ [handler] ++ k_now -- handler BENEATH the captured frames, so a
resume inside a loop finds the same handler every time.

The (effect, op) match is on two names; no effect row exists at runtime.

Calibrated: the re-installation test returns 6 on success and 2 if the
handler is found once and lost, so it distinguishes the bug from the fix
rather than merely passing.

A7 re-measured; gc_mark byte count unchanged.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
git add crates/codegen/src/lib.rs crates/codegen/tests/native_codegen.rs
git commit -F /tmp/msg-t8b.txt
```

---

### Task 9: The handler corpus — A1, value and exact bytes

**Files:**
- Modify: `crates/codegen/tests/native_codegen.rs`

**Interfaces:**
- Consumes: a compiling `handle`/`resume` (Task 8); `Expr::Block` in expression position
  (Task 4 — one clause body below is a block); the existing harness helpers `temp_dir`
  (`:20`), `lower_src` (`:162`), `compile_and_link` (`:196`), `assert_runs` (`:224`),
  `native_text_value` (`:254`), `eval_main_int` (`:553`), `eval_main_text` (`:570`).
- Produces: `const HANDLER_CORPUS: &[(&str, &str, &str)]` — the same 3-tuple shape as
  `CONTROL_FLOW_CORPUS`, `(tag, src, expected)`. Task 12's dispatch controls are measured
  against two of its rows and against Task 8's A6 test; Task 12 consumes this constant.

**This task adds no machinery.** Every line of it is a proof over what Task 8 built. If a
row does not compile, that is a Task 8 defect surfacing here — fix Task 8, do not weaken
the row.

**Four hard constraints on the corpus, each measured, none stylistic.** A row that
violates any of them fails in the harness rather than in the compiler, which is the worst
possible place to spend an execution:

1. **Main must evaluate to an `Int`.** Both oracles panic otherwise:
   `eval_main_int` (`:553`) and `eval_main_text` (`:570`) each end
   `other => panic!("corpus main must evaluate to an Int, got {other:?}")`.
2. **No `<>`.** §9.1: native codegen refuses string concatenation by name. A row
   containing it never reaches execution.
3. **No `with multi`.** The evaluator runs it (`src/eval.rs:1000`), the native back end
   refuses it (Task 8), and the differential harness would report that difference as a
   divergence — which is the design working correctly. Task 8's own prose says this.
4. **Zero diagnostics.** `lower_src` (`:162`) asserts the diagnostic vector is empty, and
   E0426 is a *warning* that lands in that vector. This is the same constraint as (3),
   arriving one layer earlier.

**On `-> Int`.** The existing corpus writes `pub fn main() {` unannotated (`:130`, `:406`),
but Task 8's A6 test writes `pub fn main() -> Int {`. Annotate here. Inference across a
`handle` node is new in this slice and is not the thing these rows are measuring; an
annotation costs nothing and removes a way for a proof to fail for an unrelated reason.

- [ ] **Step 1: Predict the three corpus values, in writing, before measuring**

Write these three predictions into `/tmp/t9-predictions.txt` before running anything.
This is Task 3's discipline applied again: a value that is *measured first and asserted
second* proves only that the harness is self-consistent.

```
ask-nontail  -> 3
two-ops      -> 14
two-handles  -> 21
```

The reasoning, so a mismatch can be diagnosed rather than merely accommodated:

- `ask-nontail`: one perform. `resume(2)` runs the rest of `one()`, which is just the
  hole, so the body returns `2`; `return(x) -> x` passes it through; the clause body is
  `1 + resume(2)` = `3`.
- `two-ops`: `a()` performs, `resume(10)` continues into `10 + b()`; `b()` performs,
  `resume(4)` continues into `10 + 4` = `14`, which `return` passes through. That `14` is
  the value of the inner `resume`, hence of the `b` clause, hence of the outer `resume`,
  hence of the `a` clause. If dispatch ignored the op name and always took the first
  clause, both performs would resume with `10` and the answer would be `20`.
- `two-handles`: `1 + 20`. If the handler record were installed once globally instead of
  scoped to its body, the second `handle` would reuse the first clause and the answer
  would be `2`.

- [ ] **Step 2: Add the corpus constant**

Place it next to `PRINTING_CORPUS` (`:136-155`), matching its column-aligned style.

```rust
/// A1. Three shapes, each chosen for one property dispatch can get wrong:
/// a NON-TAIL resume (the value flows back through `+` in the clause body),
/// TWO ops under one handler (the `(effect, op)` match must read the op name),
/// and TWO sequential handles (the handler record must be scoped to its body).
/// No `<>` (§9.1 refuses it), no `with multi` (the evaluator would run it and
/// the back end refuses it), and every main is Int-valued because both oracles
/// panic otherwise (`:553`, `:570`).
const HANDLER_CORPUS: &[(&str, &str, &str)] = &[
    (
        "ask-nontail",
        "effect Ask { fn ask() -> Int }\n\
         fn one() { ask() }\n\
         pub fn main() -> Int {\n\
           handle { one() } with {\n\
             Ask.ask() -> 1 + resume(2)\n\
             return(x) -> x\n\
           }\n\
         }\n",
        "3",
    ),
    (
        "two-ops",
        "effect Two { fn a() -> Int  fn b() -> Int }\n\
         fn both() { a() + b() }\n\
         pub fn main() -> Int {\n\
           handle { both() } with {\n\
             Two.a() -> resume(10)\n\
             Two.b() -> resume(4)\n\
             return(x) -> x\n\
           }\n\
         }\n",
        "14",
    ),
    (
        "two-handles",
        "effect Ask { fn ask() -> Int }\n\
         fn one() { ask() }\n\
         pub fn main() -> Int {\n\
           let x = handle { one() } with { Ask.ask() -> resume(1)  return(v) -> v }\n\
           let y = handle { one() } with { Ask.ask() -> resume(20)  return(v) -> v }\n\
           x + y\n\
         }\n",
        "21",
    ),
];
```

Two details are load-bearing and are copied from measured sources, not invented. The `fn`
keyword inside `effect { ... }` is mandatory (`tests/state_effect.rs:57`). Two statements
on one line separated by two spaces is the house source style (`native_codegen.rs:995`,
`:1048`).

- [ ] **Step 3: Write the execution test**

```rust
#[test]
fn the_handler_corpus_compiles_and_runs() {
    let dir = temp_dir("handler-corpus");
    for (tag, src, expected) in HANDLER_CORPUS {
        let core = lower_src(src);
        let exe = compile_and_link(&core, &dir, tag);
        assert_runs(&exe, expected);
    }
    std::fs::remove_dir_all(&dir).ok();
}
```

- [ ] **Step 4: Run it, and read a failure as a prediction miss before reading it as a bug**

```
$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen --test native_codegen the_handler_corpus -- --nocapture
```

Expected: PASS.

If a row's *value* is wrong, do not edit the expectation. Run the evaluator on that row
first — Step 5's differential is the arbiter, and it compares against the evaluator, which
is this project's reference semantics:

```
$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen --test native_codegen native_output_matches_the_evaluator_across_the_handler -- --nocapture
```

If the evaluator agrees with the prediction and native does not, the defect is in Task 8.
If the evaluator disagrees with the prediction, the prediction was wrong: correct the
constant to the evaluator's value and **record the miss in the commit message** — a wrong
prediction about handler semantics is exactly the kind of thing this slice exists to find
out, and it is worth a sentence.

- [ ] **Step 5: Write the differential test (A1, the value half)**

Follow `native_output_matches_the_evaluator_across_the_control_flow_corpus` (`:595-616`)
exactly, including the `_` on the third tuple field — the expectation is deliberately
unused here, because the point of a differential is that the *evaluator* supplies the
answer.

```rust
#[test]
fn native_output_matches_the_evaluator_across_the_handler_corpus() {
    let dir = temp_dir("differential-handler");
    for (tag, src, _) in HANDLER_CORPUS {
        let core = lower_src(src);
        let exe = compile_and_link(&core, &dir, tag);
        let out = Command::new(&exe).output().expect("run produced binary");
        diagnose_stack_overflow(&out.status, tag);
        assert!(out.status.success(), "{tag}: binary exited {:?}", out.status);
        let native = String::from_utf8_lossy(&out.stdout).trim().to_string();
        assert_eq!(native, eval_main_int(src), "{tag}: native output diverges from the evaluator");
    }
    std::fs::remove_dir_all(&dir).ok();
}
```

- [ ] **Step 6: Run it**

```
$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen --test native_codegen native_output_matches_the_evaluator_across_the_handler -- --nocapture
```

Expected: PASS.

- [ ] **Step 7: Write the exact-bytes half of A1**

A1 asks for "both the value and the exact bytes written." Steps 3-6 pin the value. The
bytes need a program that *writes* something, and it is worth more if the writing happens
**inside the clause body**, where it also witnesses that the body ran once per perform.

This is a standalone test rather than a corpus row because `HANDLER_CORPUS` is a 3-tuple
and a printing row needs four fields (`PRINTING_CORPUS`, `:136`). Duplicating the corpus
into a 4-tuple to carry one row would cost more than it explains.

```rust
/// A1's second half: the exact bytes. The `io.println` sits INSIDE the clause
/// body, so the text also witnesses that the body ran once per perform -- two
/// lines, in order. §7.1's splitter (`:244`) separates that text from main's
/// Int, and BOTH halves are compared, so neither an empty-vs-empty text check
/// nor a right-text/wrong-value result can pass vacuously.
#[test]
fn a_printing_clause_body_matches_the_evaluator_byte_for_byte() {
    let src = "effect Ask { fn ask() -> Int }\n\
               fn twice() { ask() + ask() }\n\
               pub fn main() -> Int {\n\
                 handle { twice() } with {\n\
                   Ask.ask() -> { io.println(\"asked\")  resume(1) }\n\
                   return(x) -> x\n\
                 }\n\
               }\n";
    let dir = temp_dir("handler-printing");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, "handler-printing");
    let (text, value) = native_text_value(&exe, "handler-printing");
    assert_eq!(text, eval_main_text(src), "native println text diverges from the evaluator");
    assert_eq!(value, eval_main_int(src), "native main value diverges from the evaluator");
    assert_eq!(text, "asked\nasked\n", "the clause body must run once per perform, in order");
    std::fs::remove_dir_all(&dir).ok();
}
```

The third assertion is not redundant with the first. The first proves native and the
evaluator agree; the third proves they agree on *the right thing*. If both layers silently
dropped the `println`, the first assertion would compare `""` to `""` and pass — the
vacuity `PRINTING_CORPUS`'s own comment (`:287-290`) warns about.

The clause body here is a **block in expression position**, which is exactly what Task 4
un-refused. If this reports `Unsupported`, Task 4's sweep missed a site; fix it there.

- [ ] **Step 8: Run it**

```
$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen --test native_codegen a_printing_clause_body -- --nocapture
```

Expected: PASS, text `asked\nasked\n`, value `2`.

- [ ] **Step 9: Full gate**

```
powershell -NoProfile -File scripts/check.ps1
```

Foreground, unpiped. Read the exit code; continue only on 0.

- [ ] **Step 10: Commit**

```bash
cat > /tmp/msg-t9.txt <<'EOF'
test(codegen): the handler corpus -- A1, value and exact bytes

Three rows, each chosen for one property dispatch can get wrong: a non-tail
resume, two ops under one handler, two sequential handles. Values were
predicted in writing before being measured.

The exact-bytes half puts io.println inside the clause body, so the text also
witnesses that the body ran once per perform. Both halves of the split are
compared, and the text is additionally pinned to a literal, so a silenced
println on both layers cannot pass as an empty-vs-empty agreement.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
git add crates/codegen/tests/native_codegen.rs
git commit -F /tmp/msg-t9.txt
```

---

### Task 10: The heap half of Invariant N8-1 — A4 settles, A5 moves

**Files:**
- Modify: `crates/codegen/tests/native_codegen.rs`

**Interfaces:**
- Consumes: Task 8's dispatch; `run_with_gc_stats` (`:915`) and `GcStats { collections,
  freed, live }` (`:872`); `temp_dir`, `lower_src`, `compile_and_link`.
- Produces: nothing other tasks consume. Task 12's tracing controls are measured against
  this task's two tests.

**What A4 does and does not claim.** §7.3 ground 2 is explicit: `collections > 0` and
`live > 0` are **not** Invariant N8-1 guards and cannot be. They are guards that `live` was
*computed at all* — a run with zero collections reports a `live` of whatever the field
defaults to, and asserting on that number would be asserting on nothing. The invariant
claim is the *settling*: four readings of `live` across an 8× spread of N that do not grow
with N. **No constant is pinned.** A pinned constant would break on every unrelated
allocation change and would tell you nothing about whether the live set is bounded.

**Why this is not what A9 measures.** A4 watches the heap; A9 watches the machine stack
(Task 11). §7.1's coupling hazard says neither subsumes the other: a build can free its
frames promptly and still overflow the stack, and it can hold a bounded stack while
leaking frames. Both are required, and they are deliberately separate tasks.

- [ ] **Step 1: Write the A4 test**

Four N, `200_000 / 25_000 = 8`. The perform sits under a `let _`, so each `tick()`
captures a real frame, and the recursive call is in **tail** position, so the frames must
not accumulate.

```rust
/// A4, the heap half of Invariant N8-1. Four N over an 8x spread. `live` is
/// the LEVEL -- visible words still live at the end of the LAST collection --
/// where `freed` and `words_since_gc` are flows (5b-6 §11, obligation T7).
///
/// NO CONSTANT IS PINNED. The claim is that the level does not grow with N.
/// `collections > 0` and `live > 0` are guards that `live` was computed at
/// all, NOT invariant guards -- §7.3, ground 2, which says in terms that they
/// cannot be.
#[test]
fn a_tail_resuming_handler_settles_its_live_set() {
    let dir = temp_dir("handler-settles");
    let mut levels = Vec::new();
    for n in [25_000, 50_000, 100_000, 200_000] {
        let src = format!(
            "effect Tick {{ fn tick() -> Int }}\n\
             fn spin(n) {{ if n == 0 {{ 0 }} else {{ let _ = tick()  spin(n - 1) }} }}\n\
             pub fn main() -> Int {{\n\
               handle {{ spin({n}) }} with {{\n\
                 Tick.tick() -> resume(1)\n\
                 return(x) -> x\n\
               }}\n\
             }}\n"
        );
        let tag = format!("settles-{n}");
        let core = lower_src(&src);
        let exe = compile_and_link(&core, &dir, &tag);
        let (stdout, stats) = run_with_gc_stats(&exe, &tag);
        assert_eq!(stdout, "0", "{tag}: the loop must run to completion");
        assert!(stats.collections > 0, "{tag}: no collection happened, so `live` measures nothing");
        assert!(stats.live > 0, "{tag}: live=0 means the level was never computed");
        levels.push((n, stats.live));
    }
    let (_, first) = levels[0];
    for (n, live) in &levels {
        assert_eq!(
            *live, first,
            "live set must not grow with N (a growing level is a frame leak): {levels:?}, diverged at N={n}"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}
```

- [ ] **Step 2: Run it. Four equal levels, or stop.**

```
$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen --test native_codegen a_tail_resuming_handler_settles -- --nocapture
```

Expected: PASS, four equal levels.

**If the four levels are not equal: stop and report. Do not weaken the assertion.** There
is no pre-authorized fallback here, and that is deliberate. The strength of this
instrument is that four equal readings pin no constant — there is nothing in it to nudge.
A bound admitted in advance reintroduces exactly the slack the equality removed, and
worse, it converts a signal into a pass.

Unequal readings are **information**, and this plan does not know ahead of time which kind:

- a real frame leak — the thing Invariant N8-1 exists to catch;
- a threshold interaction — `live` is sampled at the *last* collection, and where that
  collection lands relative to the loop's iteration varies with N;
- something about frame-list residency this slice has not yet understood.

Report the four measured numbers, say which N diverged and by how much, and let the next
decision be made **after** seeing why equality failed. A sub-linear bound may well turn
out to be the right answer — as a conclusion drawn from the measurement, never as a branch
authorized before it.

- [ ] **Step 3: Write the A5 control — a growing control must move the instrument**

Same effect, same handler, one change: the recursive call moves **under `+`**, so it is no
longer in tail position and each level holds a pending frame. Written without `<>` (§9.1).

An instrument that cannot be moved is not an instrument. Step 1 asserts a level does not
grow; this asserts that a program which *should* grow it does.

```rust
/// A5. The same handler with the recursive call under `+` instead of in tail
/// position: each level holds a pending frame, so the captured chain grows
/// with N and the level must move. STRICT inequality -- an instrument that a
/// deliberately-growing control cannot move is measuring nothing.
///
/// No `<>` anywhere (§9.1 refuses it, and this must reach execution).
#[test]
fn a_growing_control_moves_the_live_set() {
    let dir = temp_dir("handler-grows");
    let mut levels = Vec::new();
    for n in [5_000, 40_000] {
        let src = format!(
            "effect Tick {{ fn tick() -> Int }}\n\
             fn spin(n) {{ if n == 0 {{ 0 }} else {{ tick() + spin(n - 1) }} }}\n\
             pub fn main() -> Int {{\n\
               handle {{ spin({n}) }} with {{\n\
                 Tick.tick() -> resume(1)\n\
                 return(x) -> x\n\
               }}\n\
             }}\n"
        );
        let tag = format!("grows-{n}");
        let core = lower_src(&src);
        let exe = compile_and_link(&core, &dir, &tag);
        let (stdout, stats) = run_with_gc_stats(&exe, &tag);
        assert_eq!(stdout, n.to_string(), "{tag}: each of the N performs resumes with 1");
        assert!(stats.collections > 0, "{tag}: no collection happened, so `live` measures nothing");
        levels.push((n, stats.live));
    }
    assert!(
        levels[1].1 > levels[0].1,
        "a growing control must move the live set, or A4's settling proves nothing: {levels:?}"
    );
    std::fs::remove_dir_all(&dir).ok();
}
```

The `stdout` assertion is `n`: each of the N performs resumes with `1`, and the `+` chain
sums them. That is a second, independent check that the control really did perform N times
rather than short-circuiting.

- [ ] **Step 4: Run it**

```
$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen --test native_codegen a_growing_control_moves -- --nocapture
```

Expected: PASS, `levels[1].1 > levels[0].1`.

**If this reports `STATUS_STACK_OVERFLOW`** — `diagnose_stack_overflow` names it, which is
why it is in the helper — the non-tail chain outran the machine stack at 40 000. Halve
both N to `2_500 / 20_000`, preserving the 8× ratio, and note the halving in the commit
message. A5 fixes no N; it fixes a *direction*.

- [ ] **Step 5: Full gate**

```
powershell -NoProfile -File scripts/check.ps1
```

Foreground, unpiped. Read the exit code; continue only on 0.

- [ ] **Step 6: Commit**

```bash
cat > /tmp/msg-t10.txt <<'EOF'
test(codegen): the heap half of Invariant N8-1 -- A4 settles, A5 moves

Four N over an 8x spread, asserting the live LEVEL does not grow. No constant
is pinned: a pinned number breaks on unrelated allocation changes and says
nothing about boundedness. collections > 0 and live > 0 are retained only as
guards that the level was computed at all -- per §7.3 ground 2, they cannot be
invariant guards.

A5 is the control that makes A4 mean something: the same handler with the
recursive call under `+` instead of in tail position must move the instrument,
strictly.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
git add crates/codegen/tests/native_codegen.rs
git commit -F /tmp/msg-t10.txt
```

---

### Task 11: The machine-stack half of Invariant N8-1 — A9 at N = 1 000 000

**Files:**
- Modify: `crates/codegen/tests/native_codegen.rs`

**Interfaces:**
- Consumes: Task 8's dispatch; native strings and `io.println` (slice 5b-7);
  `native_text_value` (`:254`) and through it `split_text_and_value` (`:244`);
  `diagnose_stack_overflow` (`:212`).
- Produces: nothing other tasks consume.

**This is the one row of §9.1's fidelity corpus that transfers.** `state_tail_loop`
(`tests/state_effect.rs:57-72`) contains no `<>`, so its *source* compiles natively as
written. Its *peak* assertions do not transfer — `run_peak` is an evaluator instrument —
and §9.1's distinction between the two is deliberate. Keep it: copy the source, do not
copy the assertions, and do not "port" `run_peak`.

**The N must not be lowered.** §12 says lowering it silently converts A9 into a tautology:
a state-passing tail loop at small N completes on any implementation, including one that
grows the stack linearly. Task 12's fifth control demonstrates precisely that. If the test
overflows at 1 000 000, the finding is *a tail-call regression*, which is what
`diagnose_stack_overflow` exists to name — not a reason to pick a smaller number.

**Platform note — Linux (added 2026-10-02; nothing above is changed).** On Linux a stack
overflow shows up as **signal 11 (SIGSEGV)**, not as Windows' `STATUS_STACK_OVERFLOW`.
Measured in the cloud container (Ubuntu 24.04, clang 18.1.3, 8 MiB stack): a C program
that recurses without bound dies with `Segmentation fault`, shell status `139`, and Rust's
`ExitStatus` reports `code() == None`, `signal() == Some(11)`. `diagnose_stack_overflow`
matches only `code() == Some(0xC00000FD)`, so on Linux it stays silent, and a tail-call
regression surfaces as the generic `binary exited …` assertion — unnamed. **The helper must
recognise SIGSEGV by name (e.g. `std::os::unix::process::ExitStatusExt::signal()`) before A9
means anything on Linux.** One caveat for whoever writes that: SIGSEGV is not unique to stack
overflow the way `0xC00000FD` is — a collector or codegen fault raises the same signal — so
the Linux diagnosis names a *likely* cause, not a certain one.

**A measured deviation from A9's wording, reported rather than patched.** A9 says the
program runs "through `assert_runs`, printing exactly `x`". Against the code, it cannot:

1. `state_tail_loop`'s main ends in `io.println(program("init"))`, so main returns `Unit`.
2. `Unit` is an `i64` word in native codegen (`crates/codegen/src/lib.rs:202`).
3. The print shim prints main's word **unconditionally** — `b"%lld\n\0"` (`:1349`) through
   `build_call(printf, ...)` (`:1461`), with no Unit special case.

So native stdout is `x\n<unit word>\n`, and `assert_runs` — which compares
`stdout.trim()` against one expected string (`:224-234`) — cannot match it. Use
`native_text_value` instead and assert on the **text** half. This is *stronger* than A9
asks, not weaker: it pins the trailing newline, which is what "differential bytes" means,
where `.trim()` would have discarded it. The value half is left unpinned because a Unit
word reaching stdout is a shim artifact with nothing to say about the machine stack.

**Do not change the source to dodge this.** The source is precisely the thing that
transfers. This deviation is reported the same way §6.3's `0b11`-vs-`0b10` descriptor
mismatch was: recorded in the commit message, code left alone.

- [ ] **Step 1: Copy the source verbatim from the evaluator test**

Read it first, so what lands here is the source as it actually is, not as remembered:

```
sed -n '57,72p' tests/state_effect.rs
```

- [ ] **Step 2: Write the test**

```rust
/// A9, the machine-stack half of Invariant N8-1 -- the other half from Task
/// 10's heap measurement, and §7.1 says neither subsumes the other.
///
/// The source is §9.1's one transferring row, copied verbatim from
/// `tests/state_effect.rs:57-72`: it contains no `<>`, so it compiles
/// natively as written. Its PEAK assertions do not transfer (`run_peak` is an
/// evaluator instrument) and are deliberately not ported.
///
/// DEVIATION FROM A9, REPORTED NOT PATCHED. A9 prescribes `assert_runs`
/// "printing exactly `x`". It cannot: main ends in io.println so it returns
/// Unit, Unit is an i64 word (`lib.rs:202`), and the shim prints main's word
/// unconditionally (`lib.rs:1349`, `:1461`). Native stdout is therefore
/// "x\n<unit word>\n". §7.1's splitter separates them; the TEXT half is
/// asserted, which is stronger than A9 asks because it pins the newline that
/// `assert_runs`'s `.trim()` would have discarded. The value half is a shim
/// artifact and says nothing about the machine stack, so it is not pinned.
///
/// N MUST NOT BE LOWERED. At small N this loop completes on an implementation
/// that grows the stack linearly, which makes the test a tautology (§12).
/// Task 12's fifth control demonstrates exactly that.
#[test]
fn a_state_passing_tail_loop_is_bounded_natively_at_a_million() {
    let src = "effect State { fn get() -> String  fn set(v: String) -> Unit }\n\
               fn loop(n) { if n == 0 { get() } else { let _ = set(\"x\")  loop(n - 1) } }\n\
               pub fn main() {\n\
                 let program = handle { loop(1000000) } with {\n\
                   State.get() -> fn(s) { (resume(s))(s) }\n\
                   State.set(v) -> fn(s) { (resume(Unit))(v) }\n\
                   return(x) -> fn(s) { x }\n\
                 }\n\
                 io.println(program(\"init\"))\n\
               }\n";
    let dir = temp_dir("state-tail-million");
    let core = lower_src(src);
    let exe = compile_and_link(&core, &dir, "state-tail-million");
    let (text, _unit_word) = native_text_value(&exe, "state-tail-million");
    assert_eq!(
        text, "x\n",
        "the state-passing tail loop must run to completion at N = 1_000_000 and print exactly one line"
    );
    std::fs::remove_dir_all(&dir).ok();
}
```

Note `pub fn main()` here carries **no** `-> Int` annotation, unlike Task 9's corpus: main
returns `Unit`, and the annotation would be wrong. This row is copied, not authored.

- [ ] **Step 3: Run it**

```
$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen --test native_codegen a_state_passing_tail_loop -- --nocapture
```

Expected: PASS, text `x\n`.

A `STATUS_STACK_OVERFLOW` here is `diagnose_stack_overflow` doing its job: it means a tail
call that `musttail` was supposed to eliminate grew the stack instead — a Task 8 defect in
the splice or the trampoline. Fix it there. **Do not lower N.**

- [ ] **Step 4: Full gate**

```
powershell -NoProfile -File scripts/check.ps1
```

Foreground, unpiped. Read the exit code; continue only on 0.

- [ ] **Step 5: Commit**

```bash
cat > /tmp/msg-t11.txt <<'EOF'
test(codegen): the machine-stack half of Invariant N8-1 -- A9 at N = 1_000_000

§9.1's one transferring row, source copied verbatim from
tests/state_effect.rs:57-72. Its peak assertions are an evaluator instrument
and are deliberately not ported; §9.1's source-transfers-but-assertion-doesn't
distinction is kept intact.

Deviation from A9, reported not patched: A9 prescribes assert_runs "printing
exactly x", which is impossible against the code -- main returns Unit,
Unit is an i64 word (lib.rs:202), and the shim prints main's word
unconditionally (lib.rs:1349, :1461), so stdout is "x\n<unit word>\n".
§7.1's splitter is used instead and the TEXT half is asserted, which pins the
newline that assert_runs's .trim() would have discarded. The source is
unchanged, because the source is the thing that transfers.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
git add crates/codegen/tests/native_codegen.rs
git commit -F /tmp/msg-t11.txt
```

---

### Task 12: Negative controls — five, each failing differently

**Files:**
- Modify: `crates/codegen/tests/native_codegen.rs` (doc-comments only)
- Temporarily modify, then revert: `crates/codegen/src/lib.rs`

**Interfaces:**
- Consumes: Tasks 8-11's tests, unchanged. This task breaks the *implementation*, observes,
  and reverts.
- Produces: nothing other tasks consume. This is the last task.

**§9.3 step 7's pattern, and why this task has almost no net diff.** Controls are "built,
observed failing *differently*, reverted in the commit that adds the proofs they control."
Here the proofs already landed in Tasks 8-11, so the reverts leave no implementation diff.
What the commit *does* carry is the record: a doc-comment on each proof naming the
predicate its control trips. That is the form the precedent takes — `native_codegen.rs:1073`
and `:1107` are exactly such comments, and they are the durable half of the exercise. A
control you ran and did not write down is a control the next reader cannot trust.

**"Failing differently" is the whole point.** Five controls that all produce the same
failure would prove one thing five times. Each entry below names the predicate it trips
and the distinct symptom it must produce. **If a control passes, or fails with the wrong
symptom, that is a finding** — it means the proof is not measuring what it claims — and
it must be reported, not quietly re-rolled.

- [ ] **Step 1: Control 1a — invert the splice order**

In Task 8's Step 9 point 4, change `k_cap ++ [handler] ++ k_now` to
`[handler] ++ k_cap ++ k_now`.

- Predicate tripped: deep re-installation, §4 point 4 — the handler must sit *beneath* the
  captured frames so it is found again on the next perform.
- Run: `$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen --test native_codegen a_deep_handler_is_reinstalled -- --nocapture`
- Required symptom: the A6 test prints **2**, not 6 — the handler was found once and then
  lost. Task 8's Step 7 prose predicts this exact number, which is what makes it a
  calibration rather than a guess.
- Revert before continuing.

- [ ] **Step 2: Control 1b — break the second arm of the `(effect, op)` match**

Make dispatch always select clause 0 rather than matching the op name.

- Predicate tripped: the two-name match in Task 8's Step 9 point 3.
- Run: `$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen --test native_codegen the_handler_corpus -- --nocapture`
- Required symptom: the `two-ops` row prints **20**, not 14 — both performs resumed with
  `10`. Task 9's Step 1 predicts this number for this failure.
- **Distinctness check:** this must *not* also break `ask-nontail`, which has one clause and
  is therefore blind to this defect. If `ask-nontail` fails too, something broader broke
  and the control is not isolating what it claims.
- Revert before continuing.

- [ ] **Step 3: Control 2a — drop the frame descriptor row**

Remove the frame's row from the descriptor table (Task 6).

- Predicate tripped: `gc_mark`'s `tag >= gc_n_ctors` skip, `crates/codegen/src/runtime.c:141`
  — a tag with no row is skipped, so the frame is never traced. This is the same predicate
  the closure-capture control at `:1073` trips.
- Run: `$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen --test native_codegen a_deep_handler_is_reinstalled -- --nocapture`
- Required symptom: a **wrong value**, not a crash. `:1080`'s comment is explicit about the
  shape of this failure — "This fails SILENTLY on the un-fixed build — it prints a wrong
  `Int`, it does not crash — which is why the assertion is on the VALUE." Expect the same
  here.
- Revert before continuing.

- [ ] **Step 4: Control 2b — clear mask bit 1**

Leave the row in place; clear the bit that marks the frame's `next` field as a heap
pointer, taking the mask from `0b10` to `0b00`.

- Predicate tripped: the descriptor table's pointer-mask test. Per the measured convention,
  bit 0 is clear (the code pointer) and bit j+1 is set iff capture j is heap; the frame's
  one capture is `next`, so bit 1 is the tail of the captured chain.
- Run: `$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen --test native_codegen a_growing_control_moves -- --nocapture`
- Required symptom: **different from 2a.** 2a loses the whole frame; 2b keeps the frame and
  loses everything behind it, so it needs a *long* chain to show — which is why it is run
  against Task 10's growing control rather than against the A6 test. Expect a wrong value
  or a fault on the deep chain while a one-frame program still passes.
- **Distinctness check:** re-run `a_deep_handler_is_reinstalled` under this control. A
  short chain should be far less affected. If both controls produce identical failures on
  identical tests, the pair is not testing two things.
- Revert before continuing.

- [ ] **Step 5: Control 3 — lower A9's N, and watch the test keep passing**

This one breaks the *test*, not the implementation, and its required symptom is that
**nothing fails**.

Temporarily change Task 11's `loop(1000000)` to `loop(1000)`.

- Predicate tripped: none — that is the finding. §12 says lowering N "silently converts A9
  into a tautology," and this demonstrates it rather than asserting it.
- Run: `$env:CARGO_INCREMENTAL="0"; cargo test -p elya-codegen --test native_codegen a_state_passing_tail_loop -- --nocapture`
- Required symptom: **PASS**. A state-passing tail loop at N = 1 000 completes on an
  implementation that grows the stack linearly, so at that N the test distinguishes
  nothing. This is the concrete reason the N is fixed and not a tuning knob.
- Revert before continuing. **Confirm the revert restored `1000000`** — this is the one
  control whose un-reverted state leaves a green, worthless test, which is strictly worse
  than a red one.

- [ ] **Step 6: Record what each control proved**

Add one doc-comment per proof, in the `:1073`/`:1107` form — name the predicate, name the
symptom, say what a differently-shaped test could not have shown. On
`a_deep_handler_is_reinstalled` (Task 8), `the_handler_corpus` (Task 9),
`a_growing_control_moves_the_live_set` (Task 10), and
`a_state_passing_tail_loop_is_bounded_natively_at_a_million` (Task 11).

Example, on the A9 test, appended to the doc block Task 11 wrote:

```rust
/// Control 3 confirmed the tautology directly: with N lowered to 1_000 this
/// test still PASSES, which is why the N is fixed rather than tuned.
```

Write what was actually observed. If a control's symptom differed from the prediction
above, the comment records the observed symptom and the commit message records the
discrepancy.

- [ ] **Step 7: Confirm the working tree carries no control**

```
git diff --stat
```

Expected: `crates/codegen/tests/native_codegen.rs` only, doc-comments only. If
`crates/codegen/src/lib.rs` appears, a revert was missed — find it before gating, because
the gate may well pass with a control still in place.

- [ ] **Step 8: Full gate**

```
powershell -NoProfile -File scripts/check.ps1
```

Foreground, unpiped. Read the exit code; continue only on 0.

- [ ] **Step 9: Commit**

```bash
cat > /tmp/msg-t12.txt <<'EOF'
test(codegen): record the five negative controls for the handler proofs

Each control was built, observed failing DIFFERENTLY, and reverted. Two
dispatch controls (inverted splice order -> A6 prints 2; first-clause-always
-> two-ops prints 20), two tracing controls (no descriptor row -> gc_mark's
tag >= gc_n_ctors skip, silently wrong value; mask bit 1 cleared -> row
present but the chain tail untraced, which only a long chain can show), and
one control on the test itself: lowering A9's N to 1_000 leaves it green,
demonstrating §12's tautology rather than asserting it.

The net diff is doc-comments, because the proofs these control already landed
in Tasks 8-11. The record is the durable half -- a control that was run and
not written down is one the next reader cannot trust.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
git add crates/codegen/tests/native_codegen.rs
git commit -F /tmp/msg-t12.txt
```

---

## Self-Review

**1. Spec coverage.** Every row of §11's acceptance table maps to a task:

| Criterion | Discharged by | Kind |
|---|---|---|
| A1 — non-tail `State` program, value **and** exact bytes | Task 9 (corpus + differential + printing clause body) | differential execution |
| A2 — `with multi` refused by a message naming `multi`, before any clause body lowers | Task 8 (Steps 1-6; refusal lands *first*, so at that commit there is no clause lowering) | execution |
| A3 — polymorphic effect refused, **✗ UNCERTAIN** in the spec | Task 3 (measure first, prediction written down before measuring) | execution |
| A4 — live set settles over an 8× spread, no constant pinned | Task 10 Steps 1-2 | execution |
| A5 — a growing control moves the instrument, written without `<>` | Task 10 Steps 3-4 | execution |
| A6 — deep handler resumed in a loop finds the same handler each time | Task 8 Steps 7-10 | differential execution |
| A7 — `gc_mark` byte-identical, measured not eyeballed | Task 1 (baseline pinned by command) + Task 6 Step 12 (re-measure) | measurement |
| A8 — the `:1391` / `:1412` guards extended to the frame tag | Task 6 | structural |
| A9 — `state_tail_loop` natively at N = 1 000 000 | Task 11 | execution |
| §9.3 step 7 — negative controls, failing differently, reverted | Task 12 | procedure |
| §9.2 — whether `IO` can be user-declared | Task 2 (D1) | execution |
| §9.4 — extract `multi_declared_ops` | Task 1 | refactor |

No spec section is left without a task. §12's two "open for decision before planning"
items are both settled in the Pre-Plan Decisions section and implemented in Tasks 1 and 2.

**2. Placeholder scan.** Searched for `TBD`, `TODO`, `implement later`, "appropriate error
handling", "add validation", "handle edge cases", "similar to Task N", and steps that
describe without showing. None remain. Two places look like escapes and are not — each
is a *bounded* branch with a stated trigger, a stated bound, and a reporting obligation:

- Task 10 Step 4's N halving on stack overflow — preserves the 8× ratio; A5 fixes a
  direction, not an N.
- Task 9 Step 4's "if the prediction missed" branch — the evaluator is the arbiter, and
  the miss is recorded.

**A4 has no fallback, by decision.** An earlier draft pre-authorized weakening Task 10
Step 2's equality to `live(8x) < 2 * live(1x)` if the four readings disagreed. That was
revoked: the instrument's force is that four equal readings pin no constant, and a bound
admitted in advance reintroduces the slack the equality removed while converting a signal
into a pass. Unequal readings stop the task and get reported. A sub-linear bound may still
be the right answer — reached after seeing why equality failed, not before.

Two known spec-vs-code deviations are carried as *reported findings* rather than patched:
§6.3's frame mask (`0b11` in the spec, `0b10` against the measured descriptor convention,
recorded in Task 6) and A9's `assert_runs` prescription (impossible against `lib.rs:202`,
`:1349`, `:1461`; recorded in Task 11).

**3. Type and name consistency.** Checked across tasks:

- `CoreHandle { body, clauses, ret, is_multi_declared }`, `CoreClause { effect, op, params,
  body }`, `CoreReturn { binder, body }` — defined in Task 5, consumed with the same field
  names in Tasks 6, 7b and 8.
- `needs_cps` — produced by Task 7a, consumed by Tasks 7b and 8 under that name.
- `multi_declared_ops` — extracted in Task 1, referenced in Task 5 and Task 8 under that
  name.
- `frame_tag` — produced by Task 6, consumed by Tasks 7b and 8.
- `HANDLER_CORPUS` — produced by Task 9 as `&[(&str, &str, &str)]`, consumed by Task 12
  against its `two-ops` and `ask-nontail` rows.
- `lower_warned_source` — introduced in Task 8 (because `lower_src` asserts the diagnostic
  vector is empty and E0426 is a warning that lands in it); not used elsewhere.
- Harness helpers are used with their measured signatures throughout: `assert_runs(&Path,
  &str)` asserts on **stdout** (`:224`), never on an exit status; `native_text_value`
  returns `(String, String)` (`:254`); `run_with_gc_stats` returns `(String, GcStats)`
  with `collections`, `freed`, `live` (`:872`, `:915`).
- Test-filter commands match their test names, and the two that target unit tests use
  `--lib` rather than `--test native_codegen` (Task 8 Steps 6, 11) because the refusal
  tests live in `lib.rs`'s `mod tests`.

---

## Execution Handoff

Plan complete and saved to
`docs/superpowers/plans/2026-09-18-elya-slice-5b8-native-effect-handlers.md`. Two execution
options:

**1. Subagent-Driven (recommended)** — a fresh subagent per task, review between tasks,
fast iteration. This plan suits it: twelve tasks, each ending in its own gate and commit,
with explicit Interfaces blocks so a subagent that has never seen the neighbouring tasks
still knows the names and types it must produce. Tasks 1-4 are independent of the dispatch
machinery and land early; Tasks 9-12 are pure proofs over Task 8 and add no machinery, so a
failure there localises immediately.

**2. Inline Execution** — execute tasks in this session using `superpowers:executing-plans`,
batching with checkpoints for review.

Which approach?
