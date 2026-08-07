# Elya Sub-Slice 3c — Handlers & One-Shot `resume` on the CEK Machine

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make effects *run*. Evaluate `handle e with { … }` on the exact CEK machine Slice 2 built, with **`resume` as a first-class captured continuation** — the perform point splits the persistent `Rc`-frame `Kont` into a materialized prefix (`k_cap`) and a suffix (`k_rest`); `resume(v)` re-pushes `k_cap` over a re-installed handler. **One-shot by default, enforced** (`E0425` on a second resume). Deep handlers only. Multi-shot is 3d; effect-TCE bounds are 3e.

**Architecture:** One prerequisite refactor + the effect machinery, all in `eval.rs`. The machine's `Frame`/`Kont`/`State` currently **borrow** `&'a` AST — so a captured continuation can't outlive the borrow, and `Value` (shared with the tree-walker) can't carry one. Task 1 converts the machine to **own** its AST via cheap `Rc` clones (the 3a `Box`→`Rc` groundwork exists for exactly this), dropping the `'a` lifetime so `Value::Resume(Rc<ResumeData>)` is lifetime-free and fits the shared `Value` enum. Then handlers install a `HandleK` frame; performing an operation walks the `Kont` for the nearest matching `HandleK`, clones the frames above it into an owned `Vec<Frame>`, and runs the clause with `resume` bound; `resume` re-pushes that immutable prefix. **The tree-walker cannot capture continuations (host stack) — it is not an oracle for effect programs (spec §4.6); its `Handle`/`Resume` arm stays an error.**

**Tech Stack:** Rust 2021; existing deps only. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-08-06-elya-slice-3-effects.md` (approved), §4 (machine), §5 (`E0425`), §8 (golden corpus). Grounded in the current `src/eval.rs`: `Value` (lifetime-free, `PartialEq`-derived), `Env` (persistent `Rc<Scope>`), `Frame<'a>` (`#[derive(Clone)]`, borrows `&'a` AST), `KontNode<'a>`/`Kont<'a> = Option<Rc<KontNode>>`, `State<'a>`, the `eval`/`ret`/`step`/`run_loop` loop, `advance_call`/`apply_callee`, and the `Rc::try_unwrap`-else-`clone` frame pop that makes capture sound.

---

## Global Constraints

- **Rust edition 2021**, toolchain `stable`, MSRV 1.75. No new dependencies.
- **Pass-signature / no-globals** unchanged. The machine stays a pure `step` loop over `State`.
- **Module layer map (pinned):** `eval = 6`. No new module; `tests/arch/layering.rs` still guards it.
- **Diagnostic codes:** runtime errors are `E03xx` (existing `rt()` uses `E0300`). **3c adds `E0425`** — "continuation resumed more than once (one-shot handler)". `E0420`–`E0424` (3b, static) are unchanged; a well-typed program never reaches the machine with an unhandled effect (`E0420` caught it), so the machine's "no handler found" path is a **defensive** `E0300`, not an expected error.
- **Ordering discipline (unchanged from 3b):** `cargo fmt --all` first, then `sh scripts/check.sh`, commit **only** on gate exit 0. The `scripts/githooks/pre-commit` fmt guard from 3b Task 0 remains installed. Toolchain PATH prepends `~/.cargo/bin` ([[cargo-on-windows-path]], [[fmt-before-gate]]); push to `origin/main` after each commit ([[github-remote-workflow]]).
- **Effect-free preservation gate (every task):** the cross-check (`cek == tree` on effect-free programs), `examples`, and the **TCE regression** (`peak_kont_depth ≤ K_MAX = 3`) stay green and byte-identical. Task 1 is behavior-preserving; the effect machinery adds new frames only on effect programs, so pure-program frame counts are unchanged.
- **The equivalence net that is *gone* for effect programs is replaced by a deliberate golden corpus (§Testing), not happy-path demos.** Because no host-stack walker can oracle a continuation-capturing program (spec §4.6), effect programs have **no `cek == tree` net**. This raises the bar: the golden corpus is designed to exercise **every distinct handler behavior** — non-resuming (`Exn`), one-shot resume, nested/innermost-matching, tail-resumptive — each an **output-verified** program. This is an explicit deliverable, called out again in Task 5.

---

## File Structure

| File | Responsibility | 3c change |
|---|---|---|
| `src/eval.rs` | evaluators | **all of it**: de-lifetime the CEK machine (own `Rc`); `Value::Resume` + manual `PartialEq`; `HandleK`/`ResumeApply` frames; ops table; perform/capture/clause-invocation; one-shot `resume` re-push + `E0425` |
| `tests/effects_run.rs` | **new** — output-verified golden corpus | new |
| `tests/ui/resume_twice.elya` | **new** — runtime `E0425` fixture (or a `tests/effects_run.rs` case) | new |
| `tests/crosscheck.rs` | unchanged — stays effect-free (the retained net) | none |

No parser/resolver/types change (3a/3b delivered the front end). The tree-walker's `Handle`/`Resume` arm stays the "not evaluated" error (it is not an effect oracle).

---

## Task 1: De-lifetime the CEK machine — `Frame`/`Kont`/`State` own `Rc` AST (behavior-preserving)

**Why first, and why it's the crux:** `Value::Resume` must hold captured frames in a value that lives in an `Env` and is called later. Today `Frame<'a>` borrows `&'a` module AST, so a captured `Vec<Frame<'a>>` would force `Value<'a>` — poisoning the shared, lifetime-free `Value` (used by the tree-walker and `apply_binop`). The 3a `Box`→`Rc` change made every captured AST position `Rc`-cheap **so the machine can own clones instead of borrowing** (spec §4.1). This task does exactly that: drop `'a` from the machine.

**Files:** Modify `src/eval.rs` (the `cek` module).

**The mechanical transform (each `&'a` → an owned `Rc`/`String` clone):**
- `Frame` variants lose `'a`:
  - `BinRight.rhs: &'a Spanned<Expr>` → `Rc<Spanned<Expr>>`
  - `IfBranch.then_blk/else_blk: &'a Block` → `Rc<Spanned<Block>>` (clone `then_block`/`else_block` — already `Rc<Spanned<Block>>` in the AST)
  - `LetCont { name: &'a str, rest: &'a [Spanned<Stmt>], tail: Option<&'a Spanned<Expr>> }` → `name: String` (or `Rc<str>`), `rest: Rc<[Spanned<Stmt>]>`, `tail: Option<Rc<Spanned<Expr>>>`
  - `SeqDrop { rest, tail }` → same owned forms
  - `CallArgs { pending: &'a [Spanned<Expr>] }` → `pending: Rc<[Spanned<Expr>]>` (the AST's `Expr::Call.args` is already `Rc<[…]>`)
- `KontNode`, `Kont`, `State` drop `'a` (own the above). `eval`/`ret`/`step`/`run_loop`/`step_block`/`eval_block_state`/`advance_call`/`apply_callee` lose `'a` on the machine types; `fns: &Fns` stays a **borrow for the `run_loop` scope** (never stored in a frame), so it keeps a lifetime but the frames/kont/state do not.
- **Iteration over owned slices:** `stmts.split_first()` on an `Rc<[…]>` — deref to `&[…]` (`&stmts[..]`), split, and re-wrap the remainder: `rest` becomes `Rc<[…]>` via `Rc::from(&stmts[1..])` **or** carry an index. **Decision (plan): carry `Rc<[…]>` + a `usize` cursor** in `LetCont`/`SeqDrop`/`CallArgs` rather than re-allocating a new `Rc<[…]>` per step — reslicing an `Rc` slice reallocates and would change the per-step cost. A `(Rc<[T]>, usize)` cursor is O(1) and keeps the TCE frame counts identical. (If the cursor complicates the diff, the fallback is `Rc::from(&slice[1..])`; note the allocation, but the TCE test still passes because depth, not allocation, is what `K_MAX` measures.)
- `eval` currently takes `e: &'a Spanned<Expr>`. With owned frames, the natural signature is `eval(fns, e: &Spanned<Expr>, env, k)` where `e` is borrowed from an `Rc` the caller holds; `State::Eval` holds `Rc<Spanned<Expr>>` and the loop borrows it per step. Spell the exact `State::Eval(Rc<Spanned<Expr>>, Env, Kont)` shape at implementation time; the constraint is **no `'a` on `State`**.

- [ ] **Step 1: Transform the types + signatures** per the table above; drop `'a` from `Frame`/`KontNode`/`Kont`/`State` and the machine fns. Function bodies: `apply_callee` clones `fdecl.body.clone()` (an `Rc<Spanned<Block>>`) into the new eval state instead of borrowing `&fdecl.body.node`.
- [ ] **Step 2: Fix the `ret` frame-pop.** The `Rc::try_unwrap(node)`-else-`clone` stays; frames are still `Clone` (now cloning `Rc`s + `Env`, all cheap `Rc` bumps).
- [ ] **Step 3: The gate — behavior-preserving.** `cargo fmt --all && sh scripts/check.sh`: **all existing tests green and byte-identical**, and specifically `tests/tce.rs` still measures `peak_kont_depth ≤ K_MAX = 3` (owning `Rc` instead of borrowing changes no push/pop, so depth is unchanged). If TCE regresses, a reslice-reallocation or an accidental extra frame crept in — fix the machine, do not touch `K_MAX`.
- [ ] **Step 4: Commit + push** (`refactor(eval): CEK machine owns Rc AST (drop 'a) — prep for first-class resume; behavior-preserving`).

---

## Task 2: Effect machine scaffolding — `Value::Resume`, `HandleK`, ops table, `handle` over a pure body

**Files:** Modify `src/eval.rs`. Test: inline `cek_tests` + one `tests/effects_run.rs` case.

**Interfaces (produces):**
```rust
// A first-class, lifetime-free captured continuation.
pub struct ResumeData {
    captured: Vec<Frame>,        // frames above the handler at the perform point (top-first)
    handler: Rc<Handler>,        // the handler to re-install beneath them (deep handler)
    ret_env: Env,                // env for the handler's clauses/return clause
    consumed: std::cell::Cell<bool>, // one-shot flag (Task 4 enforces; 3d relaxes for `multi`)
}
enum Value { …, Resume(Rc<ResumeData>) }   // add to the shared enum
enum Frame {
    …,
    HandleK { handler: Rc<Handler>, env: Env },      // installed by `handle`
    ResumeApply { resume: Value, span: Span },       // evaluating resume's argument
}
```
- **`Value` loses `#[derive(PartialEq)]`; add a hand-written `impl PartialEq for Value`** — base values compare as today; `Resume` compares **`false`** against anything (continuations are not comparable, and the type system never lets a base-typed `==` see one). This keeps `apply_binop`'s `Eq`/`Ne` working without making `ResumeData` derive `PartialEq`.
- **Ops table**, threaded like `fns`: `type Ops<'a> = HashMap<&'a str, String>` (operation name → its effect name), built in `run_module` from `Decl::Effect`. A call whose callee is `Var(name)` with `name ∈ ops` is a **perform** (Task 3).

- [ ] **Step 1: Failing test** (`tests/effects_run.rs`): a handler over a body that performs **no** operation runs the body and applies the return clause:
```rust
// effect Log { fn log(msg: String) -> Unit }
// pub fn main() { io.println(handle "hi" with { Log.log(m) -> resume(Unit)  return(x) -> x }) }
// => "hi\n"   (body is a pure value; return clause is identity-ish)
```
- [ ] **Step 2: Add `Value::Resume` + manual `PartialEq`; add `HandleK`/`ResumeApply` frames; thread the ops table.** No perform yet.
- [ ] **Step 3: Evaluate `handle`.** `Expr::Handle { body, handler } => State::Eval(body, env.clone(), push(Frame::HandleK { handler: handler.clone(), env }, k))`. In `ret`, a value returning **through** a `HandleK` (the body finished with no outstanding operation) runs the **return clause** — bind its binder to the value in `ret_env` and eval its body — or, if absent, `State::Return(v, rest)` (identity). This is the normal-completion path; the perform path is Task 3.
- [ ] **Step 4: Gate + commit.** Pure programs unchanged (no `HandleK` pushed unless a `handle` runs). The scaffolding compiles; the pure-body handle test passes. Commit `feat(eval): Value::Resume + HandleK frame + ops table; handle over a pure body (return clause)`; push.

---

## Task 3: Perform → `Kont` split → clause invocation → one-shot `resume` re-push

**Files:** Modify `src/eval.rs`. Test: `tests/effects_run.rs` (one-shot + non-resuming).

**Perform (spec §4.3), spelled against the real frames.** A call `Expr::Call { callee: Var(op), args }` with `op ∈ ops` evaluates its args (reuse the `CallArgs` machinery with a new `CalleeSlot::Operation { effect, op }`), and when all args are in hand, instead of `apply_callee` applying a function, it **performs**:
1. **Walk the current `Kont` `k` from the top**, cloning each frame into `k_cap: Vec<Frame>` (top-first) until the nearest `HandleK { handler, env }` whose `handler` has a clause for `op` (match on clause `op` name, and effect when the clause qualifies it — `Effect.op`). 
   - `k_cap` = the frames strictly above that `HandleK` (an owned, cloned prefix — cheap: only the frames between the perform and the handler).
   - `k_rest` = the matched `HandleK` node's `rest` (a suffix — already a `Kont`, no copy).
   - No matching `HandleK` before `k` runs out ⇒ **defensive** `E0300` "internal: unhandled effect `{op}` reached the machine" (a well-typed program never hits this — `E0420` is static; §Global Constraints).
2. **Run the clause** `Op(x…) -> body`: build `clause_env = handler_env.extend(params_bound_to_arg_values ++ [("$resume", Value::Resume(rd))])`, where
   `rd = Rc::new(ResumeData { captured: k_cap, handler: handler.clone(), ret_env: handler_env, consumed: Cell::new(false) })`,
   and continue as `State::Eval(clause_body, clause_env, k_rest)` — the clause's result flows to the handler's consumer (deep handler). `$resume` is a reserved binding name (`resume` is contextually reserved, so no user identifier collides).

**`resume` (spec §4.4).** `Expr::Resume { arg }`:
- Look up the current continuation: `let rv = env.get("$resume")` (`Value::Resume`); a missing binding is a defensive `E0300` (resolver's `E0210` makes this unreachable for valid programs).
- `State::Eval(arg, env, push(Frame::ResumeApply { resume: rv, span }, k))`.
- On `ResumeApply` returning `u` (in `ret`): apply the resume —
  1. **One-shot check (Task 4 hardens):** for now, `if rd.consumed.get() { return Err(E0425) } else { rd.consumed.set(true) }`.
  2. **Rebuild the `Kont`, deep-handler:** `Kont' = k_cap ++ [HandleK{handler, ret_env}] ++ k_now`, built bottom-up:
     ```
     let mut k2 = k_now;                       // the resume call's own continuation
     k2 = push(Frame::HandleK { handler, env: ret_env }, k2);
     for f in rd.captured.iter().rev() { k2 = push(f.clone(), k2); }  // captured[0] ends on top
     ```
  3. `State::Return(u, k2)` — the operation's call-site receives `u` and continues from the perform point, with the handler re-installed so later operations are caught again.

- [ ] **Step 1: Failing goldens** (`tests/effects_run.rs`): **one-shot resume** (value passing) and **non-resuming / `Exn`-style** (clause never resumes; captured continuation dropped):
```rust
// one-shot: greet performs Ask; clause resumes with "ada"
// effect Ask { fn ask() -> String }
// fn greet() / {Ask} -> String { "hi " <> ask() }
// main: io.println(handle greet() with { Ask.ask() -> resume("ada")  return(x) -> x }) => "hi ada\n"

// non-resuming (Exn): fail() performs; clause returns without resume
// effect Exn { fn fail() -> String }
// fn risky(b: Int) / {Exn} -> String { if b == 0 { fail() } else { "ok" } }
// main: io.println(handle risky(0) with { Exn.fail() -> "caught"  return(x) -> x }) => "caught\n"
```
- [ ] **Step 2: Implement** `CalleeSlot::Operation`, the perform walk (`k_cap`/`k_rest` split), clause invocation, `$resume` binding, `ResumeApply`, and the resume re-push.
- [ ] **Step 3: Gate + commit.** Goldens pass; pure/effect-free suite unchanged. Commit `feat(eval): perform captures the Kont; one-shot resume re-pushes k_cap (deep handlers)`; push.

---

## Task 4: One-shot enforcement (`E0425`) + nested handlers + tail-resumptive output

**Files:** Modify `src/eval.rs`; add `tests/effects_run.rs` cases + a runtime `E0425` case.

- **`E0425` — resumed more than once (runtime, sound).** The `consumed` `Cell` from Task 3: a second `resume` on the same `ResumeData` is `E0425` "continuation resumed more than once; this handler is one-shot — use `with multi`", pointing at the resume span. **Runtime enforcement is the sound primary** (no false positives). **The spec's "static where syntactically detectable" is deferred and flagged** (design call for the reviewer): a *sound* static check needs control-flow analysis — naive "≥2 `resume` nodes in a clause" false-positives on `if c { resume(a) } else { resume(b) }` (two nodes, one runs). Rather than ship an unsound lint, 3c enforces one-shot at runtime; a narrow, sound static lint (two `resume` in unconditional statement sequence) can land later. In 3c the `with multi` flag is parsed but **not yet honored** — every handler is one-shot-enforced; **3d** flips multi handlers to allow re-entry and adds the output-verified multi-shot demo (spec §9 3d gate).
- **Nested handlers / innermost-matching:** the perform walk already finds the *nearest* matching `HandleK`; a nested program exercises that an inner handler discharges its own effect while an outer catches the other (correct innermost-matching).
- **Tail-resumptive (output correctness only — bounds are 3e):** a clause whose body *is* `resume(…)` (tail position), driven by a small recursion, produces the right output. 3c verifies the **output**; the pinned-`K_MAX_EFF` bounded-depth assertion is **3e** (spec §6) — flagged so tail-resumption isn't mistaken for "bounded" yet.

- [ ] **Step 1: Failing tests.**
  - **Nested** (`tests/effects_run.rs`): two effects, inner+outer handlers, innermost-matching:
    ```
    // effect A { fn a() -> String }  effect B { fn b() -> String }
    // fn both() / {A,B} -> String { a() <> b() }
    // main: io.println(handle (handle both() with { A.a() -> resume("[a]") }) with { B.b() -> resume("[b]") }) => "[a][b]\n"
    ```
  - **Tail-resumptive** (`tests/effects_run.rs`): clause tail is `resume`, small loop, output-verified:
    ```
    // effect Gen { fn yield_() -> Bool }
    // fn run(n: Int) / {Gen} -> String { if n == 0 { "end" } else { if yield_() { run(n - 1) } else { "stop" } } }
    // main: io.println(handle run(3) with { Gen.yield_() -> resume(True) }) => "end\n"
    ```
  - **`E0425` runtime** (`tests/effects_run.rs` expecting `Err` with code `E0425`, or a `tests/ui/resume_twice.elya` fixture): a **non-multi** clause that resumes twice in unconditional sequence:
    ```
    // effect Twice { fn t() -> Int }
    // fn body() / {Twice} -> Int { t() }
    // main: handle body() with { Twice.t() -> { let _ = resume(1)  resume(2) } }  => runtime E0425
    ```
- [ ] **Step 2: Implement** the `E0425` runtime error on the second resume; verify nested innermost-matching and tail-resumptive output fall out of Task 3's machinery (they should need no new code beyond the `E0425` guard — if they don't, the perform walk or re-push is wrong, fix it).
- [ ] **Step 3: Gate + commit.** Commit `feat(eval): one-shot enforcement (E0425 runtime) + nested/tail-resumptive golden coverage`; push.

---

## Task 5: Integration + 3c exit gate (confirm the replacement net)

**Files:** `tests/effects_run.rs` (corpus assembly), confirm `tests/crosscheck.rs` unchanged.

- [ ] **Step 1: The golden corpus is the deliberate replacement for the lost oracle — assert it as a set.** `tests/effects_run.rs` collects, each **output-verified**: **(a)** non-resuming/`Exn`, **(b)** one-shot resume, **(d)** nested/innermost-matching, **(e)** tail-resumptive. (**(c)** multi-shot re-invocation is 3d — explicitly noted as the one corpus row 3c does not yet cover, so its absence is deliberate, not overlooked.) Add a module doc-comment stating: *the tree-walker cannot oracle these (spec §4.6); this corpus is the sole safety net and is built per-behavior, not happy-path.*
- [ ] **Step 2: Confirm the retained net.** `tests/crosscheck.rs` (`cek == tree`) is **unchanged and still green** — it remains the equivalence net for the **effect-free** subset. State in the plan/PR that effect programs are covered by the golden corpus instead, closing the "net that's gone" question the reviewer raised.
- [ ] **Step 3: The 3c exit gate.** `cargo fmt --all && sh scripts/check.sh` fully green: effect programs run and print expected output; one-shot `E0425` fires on double-resume; nested innermost-matching and tail-resumptive outputs correct; **cross-check + examples + `tce.rs` (`K_MAX = 3`) byte-identical** (pure programs unaffected); layering unchanged (`eval = 6`). Push.
- [ ] **Step 4: Commit + push** (`test(effects): 3c golden corpus (non-resuming/one-shot/nested/tail-resumptive) + exit gate`).

---

## Self-Review

**1. Spec coverage (spec §9 sub-slice 3c → tasks).**
- `Rc`-owning machine so `resume` is first-class (spec §4.1–4.2) → **Task 1** (behavior-preserving; the 3a `Box`→`Rc` was the groundwork).
- `HandleK` + install + return clause (spec §4.3) → **Task 2**.
- Perform → `k_cap`/`k_rest` split → clause invocation → deep-handler `resume` re-push (spec §4.3–4.4) → **Task 3**, spelled against the real `Frame`/`Kont`/`Rc::try_unwrap` machinery.
- One-shot `E0425` (spec §5) → **Task 4** (runtime, sound).
- Golden corpus, non-happy-path, output-verified (spec §8) → **Tasks 3–5**; the lost `cek == tree` oracle for effects is explicitly replaced (Task 5 Steps 1–2).

**2. Every deferral flagged.**
- **Multi-shot + `E0426`** → **3d**; in 3c the `multi` flag is parsed but not honored (one-shot enforced for all), and corpus row (c) is explicitly the one 3c omits.
- **Effect-TCE bounds** (tail-resume splice, pinned `K_MAX_EFF`, grow control) → **3e**; 3c verifies tail-resumptive **output**, not bounded depth — flagged so "runs" isn't read as "bounded".
- **Static `E0425` lint** → deferred (sound version needs CFA; runtime enforcement is the sound 3c deliverable). Design call surfaced for the reviewer.
- **Shallow handlers, generic effects, first-class effect values** → still deferred (spec §11).

**3. Design calls for the reviewer.**
- **(a) Slice-reslice vs cursor in de-lifetimed frames** (Task 1): plan picks a `(Rc<[T]>, usize)` cursor to keep per-step cost O(1) and TCE frame counts identical; fallback `Rc::from(&slice[1..])` noted (allocates, but `K_MAX` measures depth, so the TCE test still holds). Flagged in case you prefer the simpler reslice despite the allocation.
- **(b) `E0425` runtime-only in 3c** (Task 4): sound-by-construction; static lint deferred to avoid false positives on branch-guarded resumes. Confirm you're happy with runtime-primary (the spec allows "else a runtime structured error").
- **(c) Handler/op matching key**: match a performed `op` to a clause by op-name, refined by effect-name when the clause qualifies (`Effect.op`). Monomorphic ops make op-name nearly unique; effect-qualified matching removes the ambiguity. Flagged in case you want strict `(effect, op)` always.

**4. Manifesto / no-oracle honesty.** Effect programs gain first-class continuations, which a host-stack walker fundamentally can't mirror — so the `cek == tree` net narrows to effect-free (retained, unchanged) and the golden corpus becomes the effect net. This is stated as a deliberate consequence (spec §4.6), and the corpus is built per-behavior to earn that trust — not a reduction in coverage but a change in kind.

---

## Execution Handoff

Plan complete — **paused for review; no code written.** 3c is where effect semantics first execute (real continuation capture on the persistent `Kont`), so it goes to you before any implementation. On approval, two execution options:

1. **Subagent-Driven** — dispatch a fresh subagent per task, review between tasks.
2. **Inline Execution** — execute Tasks 1–5 in this session with checkpoints.

Which approach — and any changes to the three flagged design calls (5a slice-cursor, 5b `E0425` runtime-only, 5c op-matching key)?
