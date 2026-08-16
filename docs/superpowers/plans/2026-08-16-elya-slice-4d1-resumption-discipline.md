# Elya Slice 4d-1 — Effect Resumption Discipline — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let an effect declare `multi` (may be multi-resumed), default one-shot, and enforce that `with multi` handlers only handle `multi`-declared effects — the static, looked-up resumption fact the affine check (4d-2) will consume.

**Architecture:** A `multi` modifier on the effect declaration (`effect multi Flip`) sets `EffectDecl.is_multi`; an `effect_name -> is_multi` table is built during elaboration; `infer_handle` emits **E0427** when a `with multi` handler is over a one-shot effect. `is_multi` is a declaration field read by name — **not** a row attribute — so `unify_row` and the runtime (`eval.rs`) are untouched.

**Tech Stack:** Rust 2021, `logos`, `ariadne`, `insta`. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-08-16-elya-slice-4d1-resumption-discipline-design.md`

## Global Constraints

- **Rust 2021, MSRV 1.75. No new dependencies. One new diagnostic code: E0427** (the next free effect-diagnostic code — E0420–E0426 taken).
- **Every shell session:** `export PATH="$HOME/.cargo/bin:$PATH"` and `export CARGO_INCREMENTAL=0`.
- **Before every `sh scripts/check.sh`:** run `cargo fmt --all` (fmt-check fails hard).
- **Commits:** end every message with `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`. Use **explicit `git add` paths** (never `git add -A`). Push to `origin/main` after each task. **Gate atomically** — `if sh scripts/check.sh; then git commit …; fi` — so a red gate never commits.
- **HONESTY LINE (from the spec, enforced):** 4d-1 is enabling infrastructure. It makes **no** claim of precise E0426 and **no** affine guarantee (those are 4d-2). E0426 is **unchanged**. Exit criteria must not assert either as a win.
- **THE TWO-SIDED CONFORMANCE CHECK (the proof the rule is right, 4c-2 arity-0-gate shape):** after migration, **every existing `with multi` program still passes with unchanged output** (its effect got the `multi` marker — behavior preserved, since those effects were *already* being multi-resumed; the marker encodes existing intent) **AND** a fresh `with multi` over a non-`multi` effect **fails with E0427** (new discipline enforced). Migration adapts declarations to a now-required keyword; it changes **no assertion**.
- **Runtime untouched:** do not modify `eval.rs`.
- Scratch/debug files go in the session scratchpad **outside** the repo.

---

## File Structure

| File | Responsibility | Change |
|---|---|---|
| `src/ast.rs` | AST | `EffectDecl.is_multi: bool`. |
| `src/parse.rs` | Parser | accept optional `multi` after `effect` (reuse `KwMulti`). |
| `src/types.rs` | Inference | `Infer.effect_multi: HashMap<String, bool>`; populate during effect elaboration; conformance check in `infer_handle` → **E0427**. |
| `tests/effect_syntax.rs`, `tests/effects_run.rs`, `tests/effect_types.rs` | The `with multi` corpus | migrate each `with multi` program's effect decl to `multi` (behavior-preserving). |
| `tests/ui/multi_on_oneshot.elya` + `tests/ui.rs` | Diagnostic fixture | the E0427 negative. |

`eval.rs`, `resolve.rs`, `tests/arch/layering.rs` unchanged.

---

## Task 1: The `multi` declaration modifier + parse

**Files:**
- Modify: `src/ast.rs` (`EffectDecl.is_multi`), `src/parse.rs` (`effect_decl` ~289)
- Test: `src/parse.rs` inline round-trip

**Interfaces:**
- Produces: `EffectDecl { name, params, is_multi: bool, ops }`.

- [ ] **Step 1: Add the AST field.** In `src/ast.rs` `EffectDecl`, add after `name`:

```rust
    /// Resumption discipline (Slice 4d-1): `effect multi Name` may be resumed
    /// more than once; unmarked is one-shot (the default). Looked up by the
    /// `with multi` conformance rule and (later) the affine capture check.
    pub is_multi: bool,
```

- [ ] **Step 2: Parse `multi` after `effect`.** In `src/parse.rs` `effect_decl`, insert right after `self.bump(); // effect`:

```rust
        // Optional resumption modifier: `effect multi Name` (reuses `KwMulti`).
        let is_multi = self.eat(&TokenKind::KwMulti);
```

and set it in the `EffectDecl { … }` this function builds (`is_multi,`).

- [ ] **Step 3: Fix the one direct `EffectDecl` construction.** In `src/ast.rs` the `pretty_prints_effect_decl_and_handle` test builds `Decl::Effect(EffectDecl { name: "Log".into(), params: vec![], ops: … })`; add `is_multi: false,` after `params`. (The parser-based tests use `parse_module` and need no change.)

- [ ] **Step 4: Write the round-trip test.** In `src/parse.rs` tests:

```rust
#[test]
fn effect_multi_modifier_parses() {
    let src = "effect multi Flip { fn flip() -> Bool }\n\
               effect Exn { fn fail() -> Unit }\n\
               pub fn main() { io.println(\"x\") }\n";
    let (m, d) = parse_module(&Session::new(), src);
    assert!(d.is_empty(), "parse: {d:?}");
    let Decl::Effect(flip) = &m.decls[0].node else { panic!("expected effect") };
    let Decl::Effect(exn) = &m.decls[1].node else { panic!("expected effect") };
    assert!(flip.is_multi, "effect multi Flip -> is_multi");
    assert!(!exn.is_multi, "unmarked effect -> one-shot");
}
```

- [ ] **Step 5: Build + run.**

Run: `cargo build 2>&1 | grep -E "^error" | head` → expect none (fix any other `EffectDecl { … }` the compiler flags with `is_multi: false`).
Run: `cargo test --lib effect_multi_modifier_parses 2>&1 | grep -E "test result|FAILED"` → PASS.
Run the full suite: `cargo test 2>&1 | grep -E "test result: FAILED|FAILED" | head || echo GREEN` → all green (no conformance rule yet, so existing `with multi` still compiles).

- [ ] **Step 6: Commit (atomic gate).**

```bash
cargo fmt --all && sh scripts/check.sh && \
git add src/ast.rs src/parse.rs && \
git commit -m "$(printf 'feat(effects): multi modifier on the effect declaration\n\nEffectDecl.is_multi; parser accepts `effect multi Name` (reuses KwMulti); default\none-shot (unmarked). No conformance rule yet -- existing `with multi` still\ncompiles, suite green. The static resumption fact the 4d-2 affine check consumes.\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>')" && \
git push origin main
```

---

## Task 2: The conformance rule (E0427) + the behavior-preserving migration

**Files:**
- Modify: `src/types.rs` (`Infer.effect_multi`; populate in elaboration ~1660; conformance in `infer_handle` ~1000)
- Modify: `tests/effect_syntax.rs`, `tests/effects_run.rs`, `tests/effect_types.rs` (migrate `with multi` decls)
- Create: `tests/ui/multi_on_oneshot.elya`; Modify: `tests/ui.rs`
- Test: `tests/effect_types.rs` (the four conformance cases)

**Interfaces:**
- Consumes: `EffectDecl.is_multi` (Task 1); `handler_effect(&Handler) -> Option<String>` and `handler.multi` (existing).
- Produces: `Infer.effect_multi: HashMap<String, bool>`.

- [ ] **Step 1: Add the lookup table.** In `src/types.rs` `struct Infer`, add near `ops`:

```rust
    /// Effect name -> whether it is declared `multi` (Slice 4d-1). Read by the
    /// `with multi` conformance rule; a declaration fact, NOT a row attribute.
    effect_multi: HashMap<String, bool>,
```

and initialize it in `Infer::new()` (`effect_multi: HashMap::new(),`).

- [ ] **Step 2: Populate it during effect elaboration.** In `src/types.rs` where effect declarations are elaborated (the `for d in &module.decls { if let Decl::Effect(e) = … }` loop that builds `OpInfo`, ~line 1660), add once per effect:

```rust
            inf.effect_multi.insert(e.name.clone(), e.is_multi);
```

- [ ] **Step 3: Write the failing conformance tests.** In `tests/effect_types.rs`:

```rust
#[test]
fn with_multi_over_oneshot_effect_is_e0427() {
    // A `with multi` handler over a one-shot (default) effect is E0427.
    let src = "effect Ask { fn ask() -> String }\n\
               fn greet() { \"hi \" <> ask() }\n\
               pub fn main() {\n\
                 io.println(handle greet() with multi { Ask.ask() -> resume(\"ada\") })\n\
               }\n";
    let d = infer_diags(src);
    assert!(
        d.iter().any(|x| x.code == "E0427"),
        "with multi over a one-shot effect must be E0427: {d:?}"
    );
}

#[test]
fn with_multi_over_multi_effect_is_ok() {
    // Declaring the effect `multi` makes the same handler legal.
    let src = "effect multi Ask { fn ask() -> String }\n\
               fn greet() { \"hi \" <> ask() }\n\
               pub fn main() {\n\
                 io.println(handle greet() with multi { Ask.ask() -> resume(\"ada\") })\n\
               }\n";
    let d = infer_diags(src);
    assert!(
        !d.iter().any(|x| x.code == "E0427"),
        "with multi over a multi effect must be accepted: {d:?}"
    );
}

#[test]
fn plain_handler_over_oneshot_effect_is_ok() {
    // The common case is unchanged: a plain handler over a one-shot effect.
    let src = "effect Ask { fn ask() -> String }\n\
               fn greet() { \"hi \" <> ask() }\n\
               pub fn main() {\n\
                 io.println(handle greet() with { Ask.ask() -> resume(\"ada\") })\n\
               }\n";
    let d = infer_diags(src);
    assert!(
        !d.iter().any(|x| x.code == "E0427"),
        "a plain handler must never be E0427: {d:?}"
    );
}
```

(`infer_diags` is an existing helper in `tests/effect_types.rs`.)

- [ ] **Step 4: Run — expect the first to FAIL** (no conformance rule yet; `with multi` over `Ask` currently type-checks).

Run: `cargo test --test effect_types with_multi_over_oneshot_effect_is_e0427 2>&1 | grep -E "test result|FAILED"`
Expected: FAIL (no E0427 emitted).

- [ ] **Step 5: Implement the conformance rule.** In `src/types.rs` `infer_handle`, right after `let effect = self.handler_effect(handler);`, add:

```rust
        // Conformance (Slice 4d-1): `with multi` is legal only over a `multi`-
        // declared effect. Multi-resuming a one-shot effect is E0427. `is_multi`
        // is looked up by name — no row involvement.
        if handler.multi {
            if let Some(e) = &effect {
                if !self.effect_multi.get(e).copied().unwrap_or(false) {
                    self.diags.push(
                        Diagnostic::error(
                            "E0427",
                            format!("a one-shot effect cannot be handled with `multi`"),
                        )
                        .with_label(span, format!("`{e}` is handled `with multi` here"))
                        .with_help(format!(
                            "effect `{e}` is one-shot (its continuation resumes at most once); declare it `effect multi {e} {{ … }}` to allow multi-shot resumption, or drop `multi`"
                        )),
                    );
                }
            }
        }
```

- [ ] **Step 6: Run the conformance tests — expect PASS; the existing `with multi` corpus is now RED (expected).**

Run: `cargo test --test effect_types with_multi_over_oneshot_effect_is_e0427 with_multi_over_multi_effect_is_ok plain_handler_over_oneshot 2>&1 | grep -E "test result|FAILED"`
Expected: the three conformance tests PASS. The rule now also (correctly) rejects the *existing* `with multi` programs, which declare `Flip` one-shot — they emit E0427 and will be red until migrated in Step 7. That is the rule working, not a regression.

- [ ] **Step 7: The behavior-preserving migration.** Every existing `with multi` program was *already* multi-resuming its effect; declaring that effect `multi` **encodes the intent it already had** and changes no behavior. Find each and mark its effect declaration:

Run: `grep -rn "with multi" tests/` — the sites are in `tests/effect_syntax.rs`, `tests/effects_run.rs`, and `tests/effect_types.rs`, all over the `Flip` effect. In each such program's source string, change its declaration `effect Flip { … }` to `effect multi Flip { … }`. Do **not** touch any assertion — outputs (`TrueFalse`, the `resume(True) <> resume(False)` collect, `tick\ndone\n`, etc.) and the E0426 expectations stay identical (a `with multi` handler over a now-`multi` `Flip` still performs the same effects and still trips E0426 where it did).

- [ ] **Step 8: The E0427 UI fixture.** Create `tests/ui/multi_on_oneshot.elya`:

```elya
effect Ask {
  fn ask() -> String
}
fn greet() { "hi " <> ask() }
pub fn main() {
  io.println(handle greet() with multi { Ask.ask() -> resume("ada") })
}
//~ ERROR[E0427] one-shot
```

Register it in `tests/ui.rs` alongside the other fixtures.

- [ ] **Step 9: Regression + the two-sided proof + exit gate + commit (atomic).**

Run: `cargo test 2>&1 | grep -E "test result: FAILED|FAILED" | head || echo GREEN`
Expected: **all green** — the two-sided proof: every migrated `with multi` program passes with **unchanged output** (behavior preserved), AND `with_multi_over_oneshot_effect_is_e0427` + the fixture show a fresh `with multi` over a one-shot effect fails **E0427** (discipline enforced).

```bash
cargo fmt --all && sh scripts/check.sh && \
git add src/types.rs tests/effect_syntax.rs tests/effects_run.rs tests/effect_types.rs tests/ui/multi_on_oneshot.elya tests/ui.rs && \
git commit -m "$(printf 'feat(effects): with multi conformance rule (E0427); resumption discipline enforced\n\ninfer_handle rejects `with multi` over a one-shot effect with E0427 (is_multi\nlooked up by name -- no row/eval reach). The `with multi` corpus (all over Flip)\nis migrated to `effect multi Flip`, behavior-preserving: those handlers were\nalready multi-resuming, so the marker encodes existing intent -- outputs\nunchanged. Two-sided proof: migrated programs still pass; a fresh `with multi`\nover a one-shot effect is E0427. No precise-E0426 / affine claim (4d-2).\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>')" && \
git push origin main
```

### Exit criterion

`effect multi Flip { … }` parses (`is_multi`); a `with multi` handler over a one-shot effect is **E0427**, over a `multi` effect it type-checks, and a plain handler over either is fine; the migrated `with multi` corpus runs with **unchanged output**; `eval.rs` is untouched; the full suite is green; `cargo fmt --all` + `sh scripts/check.sh` clean. **No claim of precise E0426 and no affine guarantee — those are 4d-2.**

---

## Self-Review

- **Spec coverage:** §2 `multi` modifier → Task 1. §3 conformance rule + E0427 → Task 2 Steps 3–5 + the fixture. §4 (E0425/E0426/generic/rows unchanged) → nothing touched; E0426 corpus migrated but its assertions preserved (Step 7). §5 one new code E0427 → Task 2. §6 pipeline (no `eval`) → honored. §7 migration → Task 2 Step 7. §8 testing (four cases + migration green + runtime unchanged) → Tasks 1–2. §9 build order → the 2-task sequence.
- **Honesty line honored:** the exit criterion and the commit messages claim no precise-E0426 and no affine win; E0426 is only *migrated*, not changed.
- **Two-sided proof explicit:** Step 9 states both sides — migrated programs pass with unchanged output (behavior preserved), and a fresh one-shot `with multi` is E0427 (enforced) — the 4c-2 arity-0-gate shape.
- **Placeholder scan:** none — every step is concrete code or an exact edit instruction (the migration names the files and the exact `effect Flip` → `effect multi Flip` change).
- **Type/name consistency:** `EffectDecl.is_multi`, `Infer.effect_multi`, `handler_effect`, `handler.multi`, E0427 used consistently across tasks; `infer_diags` is an existing `effect_types.rs` helper.
- **Ordering caveat is explicit:** Step 6 flags that implementing the rule turns the existing corpus red until the Step 7 migration — the rule working, not a regression — so the executor isn't surprised.