# Elya — Slice 5b-1 Implementation Plan: Native Codegen, the Arithmetic MVP

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prove the native back-end spine exists end to end: an Elya source file becomes a native executable that runs and prints the right integer. The slice is done when a process the compiler produced has exited `0` after writing `3
` to stdout, and a test asserted that.

**Architecture:** A new leaf module `src/codegen.rs` behind a `codegen` cargo feature, consuming `core::CoreModule` (Core's first real consumer) and producing an object file via inkwell's `TargetMachine`, linked by shelling out to `clang`. One print convention (`@main` shim calling `printf("%lld
")`) makes results observable. Proof is execution (`tests/native_codegen.rs` runs every produced binary and asserts exit status + stdout + empty stderr); there is **no insta snapshot of LLVM IR anywhere** and **no test can skip**.

**Tech Stack:** Rust 2021 (MSRV 1.75); `inkwell = "0.5"` as an **optional** dependency behind `features.codegen`, pinned to exactly one `llvmNN-M` feature string determined empirically in Task 1; `clang` on PATH as linker driver. No other new dependencies.

**Spec:** [docs/superpowers/specs/2026-08-23-elya-slice-5b1-native-codegen-arith-mvp-design.md](../specs/2026-08-23-elya-slice-5b1-native-codegen-arith-mvp-design.md) — the plan argues from the spec; executors read both.

## Global Constraints

Every task's requirements implicitly include this section.

- **Rust 2021, MSRV 1.75.** The only sanctioned new dependency this slice is `inkwell` (optional, feature-gated). No `tempfile` (hand-rolled ~10-line unique temp dir), no new dev-dependencies.
- **Command prelude (this machine is Windows 11 / PowerShell).** Prefix every cargo invocation with `$env:CARGO_INCREMENTAL="0";` — the incremental cache hangs on this machine. Example: `$env:CARGO_INCREMENTAL="0"; cargo test --features codegen --test native_codegen`.
- **The gate is `scripts/check.ps1`** (this machine's twin of `scripts/check.sh`; both exist today and are kept byte-equivalent in behavior). After Task 5 it runs, in order: `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --all`, `cargo clippy --all-targets --features codegen -- -D warnings`, `cargo test --features codegen --test native_codegen`. **Run `cargo fmt --all` (write mode) before the gate** — the gate fmt-_checks_ and fails hard on any drift.
- **Clippy is `-D warnings`** in _both_ feature configurations (spec risk #8: feature-gated code must not rot).
- **Atomic commits, never red.** Structure each commit as: run the gate; `if ($LASTEXITCODE -eq 0) { git add <explicit paths>; git commit …; git push origin main }` — never commit a red tree.
- **Explicit paths only.** Stage with `git add <explicit paths>`; **never** `git add -A` / `git add .`.
- **Commit trailer** (last line of every commit message): `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`
- **Push `origin/main` after each task commit.**
- **Non-negotiable boundary (spec §0, §1, §6):**
  - Proof is **execution**, never IR inspection. No insta snapshot of LLVM IR exists in this slice. Nothing asserts on `emit_ir` output.
  - **No test may skip**: no `#[ignore]`, no toolchain-probe early-return, no soft pass. If the toolchain is absent, the build or test fails loudly.
  - **Task 1 is a hard prerequisite gate that writes zero Elya code.** If it cannot go green, **STOP and report** — do not route around it, do not switch to textual `.ll` output. That fork is the human's decision.
  - Subset is arith-only: `Int` literals, `Var`, `Prim(Add/Sub/Mul)`, `Let`, one print. **No Div/Rem/And/Or/Bool** (they need branches — deferred to N3). Arithmetic is emitted **without** `nsw`/`nuw` (defined wrapping, §3.4).
  - Native codegen must never be more-undefined than the tree evaluator.
  - The front end and Core are **not modified** (§6.2) — except recorder-gap fixes demanded by real reach in Task 2 (§6.3: expect zero; fix with the perform-callee shape, never audit speculatively).
  - Default build/test pull in **no LLVM**: `cargo build` / `cargo test --all` with no features behave exactly as today; every existing snapshot unchanged.

## File Structure

| File                                              | Responsibility                                                                                                                                                 | Task       |
| ------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------- |
| `Cargo.toml` (modify)                             | Optional `inkwell` dep + `codegen` feature (§2.3).                                                                                                             | 1          |
| `Cargo.lock` (regenerated)                        | Locks inkwell/llvm-sys transitively.                                                                                                                           | 1          |
| `tests/native_codegen.rs` (create)                | The execution proof. Task 1 seeds it with `toolchain_smoke`; Tasks 2/4/5 extend it. Whole file gated `#![cfg(feature = "codegen")]`.                           | 1, 2, 4, 5 |
| `src/lib.rs` (modify)                             | `#[cfg(feature = "codegen")] pub mod codegen;` (alphabetical slot between `ast` and `core`).                                                                   | 3          |
| `src/codegen.rs` (create)                         | The whole back end: `CodegenError`, module-shape validation, the Core→LLVM fold, `emit_ir` (debugging aid ONLY), `compile_module`, `link`, layer-1 unit tests. | 3, 4       |
| `src/main.rs` (modify)                            | `#[cfg(feature = "codegen")]` `build` subcommand.                                                                                                              | 5          |
| `scripts/check.sh` + `scripts/check.ps1` (modify) | Two added lines each (§2.4) — the codegen feature is clipped and tested on every gate.                                                                         | 5          |

**Two deliberate deviations from the spec's letter, flagged for review:**

1. **`toolchain_smoke` lives in `tests/native_codegen.rs` from Task 1** (not a separate file), so the gate line `cargo test --features codegen --test native_codegen` is identical from Task 1 onward and the smoke test remains as a permanent regression tooth.
2. **Both `check.sh` and `check.ps1` get the two extra lines.** Spec §7 lists only `check.sh`; on this Windows machine the operative gate is `check.ps1`. Extending both keeps the §2.4 invariant ("the gate runs the codegen feature") true on the machine where development happens, and matches existing repo practice (both scripts exist and mirror each other today).

---

### Task 1: Toolchain prerequisite gate _(no Elya code)_

Isolates the entire novel risk — "does this machine have a usable LLVM?" — before any design commitment. Writes zero Elya code: hand-builds a trivial LLVM module via inkwell, emits an object, links with `clang`, runs it, asserts stdout is `3`.

**Files:**

- Modify: `Cargo.toml`
- Regenerated: `Cargo.lock`
- Create: `tests/native_codegen.rs`

- [ ] **Step 1: Probe the machine for LLVM dev libraries**

Run each and record output:

```powershell
clang --version
llvm-config --version          # may not exist on Windows — that alone is not fatal
Get-ChildItem "C:\Program Files\LLVM\lib\LLVMCore*" -ErrorAction SilentlyContinue
$env:LLVM_SYS_181_PREFIX; $env:LLVM_SYS_191_PREFIX   # any pre-set prefixes?
```

Decision tree:

- If `llvm-config` exists on PATH → proceed to Step 2 with the version it prints.
- Else if `C:\Program Files\LLVM\lib\` contains static libraries (`LLVMCore.lib`, `LLVMSupport.lib`, …) → set `LLVM_SYS_<NNN>_PREFIX=C:\Program Files\LLVM` (where `<NNN>` matches the installed major/minor without the dot, e.g. `181` for 18.1) and proceed. Persist the variable user-level (`setx`) so the gate works in fresh shells, AND note it in the commit message body so the requirement is recorded.
- Else → **STOP and report.** The official LLVM Windows distribution may lack the static libs `llvm-sys` links against. Options (obtain a dev-library-bearing distribution, or build LLVM from source) are the human's fork to re-open. Do not route around it; do not switch to textual `.ll`.

- [ ] **Step 2: Determine the inkwell feature string empirically**

The `llvmNN-M` feature must match the installed LLVM exactly (zero or two such features fail to build). Edit `Cargo.toml` with the candidate matching Step 1's version. **Empirically resolved (Task 1, this machine):** inkwell 0.5 spells its LLVM 18 feature `llvm18-0` (→ `llvm-sys-180`, which targets the 18.1.x library line); there is no `llvm18-1`. The vcpkg LLVM build only compiles x86 targets, so inkwell's default `target-all` must be dropped (`default-features = false`) or linking fails with unresolved `LLVMInitialize<OtherTarget>*` symbols:

```toml
[dependencies]
logos = "0.14"
ariadne = "0.4"
inkwell = { version = "0.5", default-features = false, features = ["llvm18-0", "target-x86"], optional = true }

[features]
codegen = ["dep:inkwell"]
```

Run: `$env:CARGO_INCREMENTAL="0"; cargo build --features codegen`
Expected: compiles (slowly — llvm-sys links a large native library). If llvm-sys' build script errors stating a version/env mismatch, correct the feature string (or the `LLVM_SYS_<NNN>_PREFIX` value) and retry. Record the final working pair (feature string, prefix env var) — they are load-bearing facts for this repo.

> **Task 1 outcome — the working pair on this machine (load-bearing):**
> LLVM comes from **vcpkg**, port `llvm@18.1.6`, triplet `x64-windows-static-md-rel`
> (custom release-only triplet at `C:\vcpkg\triplets\x64-windows-static-md-rel.cmake`
> with `VCPKG_BUILD_TYPE=release`; the stock static-md triplet fails at link with
> `LNK1140: limit exceeded for program database` in the debug config).
> Prefix: `LLVM_SYS_180_PREFIX=C:\vcpkg\installed\x64-windows-static-md-rel` (persisted user-level via `setx`).
> Three bridges make vcpkg's layout acceptable to llvm-sys, which expects a
> `bin|include|lib` prefix: `bin\llvm-config.exe` copied from `tools\llvm\`,
> and junctions `C:\vcpkg\installed\{include\llvm, include\llvm-c, lib}` → the
> triplet tree (llvm-config has `C:\vcpkg\installed` baked in as its prefix).
> Link driver: winget's clang 22.1.8 (`C:\Program Files\LLVM\bin`, added to User
> PATH); the driver's version need not match the dev-lib version.

- [ ] **Step 3: Write the smoke test**

Create `tests/native_codegen.rs`:

```rust
//! Slice 5b-1 — the execution proof. Every test here produces a native binary,
//! RUNS it, and asserts on exit status, stdout, and stderr. Proof is execution,
//! never IR inspection: no insta snapshot of LLVM IR exists anywhere in this
//! slice, and no test may skip (no #[ignore], no toolchain-probe early return).
#![cfg(feature = "codegen")]

use std::path::PathBuf;
use std::process::Command;

use inkwell::context::Context;
use inkwell::targets::{FileType, InitializationConfig, RelocMode, Target, TargetMachine};
use inkwell::AddressSpace;
use inkwell::OptimizationLevel;

/// A unique per-test directory under the OS temp dir — never a tracked path
/// (spec §5). Keyed by process id plus a caller-supplied tag; removed by the
/// caller on success. No `tempfile` dependency: this is the whole harness.
fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("elya-codegen-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

#[test]
fn toolchain_smoke() {
    // Task 1 (spec §9): the entire novel risk isolated before any Elya code —
    // hand-build a trivial LLVM module (no Core, no Elya): @elya_main returns
    // the constant 3; the generated @main shim prints it via printf("%lld
").
    // Verifies the module, emits an object, links with clang, runs the binary,
    // and asserts stdout is "3" — proving object emission, C-runtime linking,
    // and the %lld round-trip on THIS platform before anything depends on them.
    let ctx = Context::create();
    let i64t = ctx.i64_type();
    let i32t = ctx.i32_type();
    let i8t = ctx.i8_type();
    let module = ctx.create_module("smoke");

    let fn_ty = i64t.fn_type(&[], false);
    let elya_main = module.add_function("elya_main", fn_ty, None);
    let entry = ctx.append_basic_block(elya_main, "entry");
    let b = ctx.create_builder();
    b.position_at_end(entry);
    b.build_return(Some(&i64t.const_int(3, true))).unwrap();

    // One external symbol (printf) and one format-string global — the §4 runtime.
    let ptrt = ctx.ptr_type(AddressSpace::default());
    let printf_ty = i32t.fn_type(&[ptrt.into(), i64t.into()], true);
    let printf = module.add_function("printf", printf_ty, None);

    let fmt_bytes: &[u8] = b"%lld
\0";
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
        .build_call(elya_main, &[], "v")
        .unwrap()
        .try_as_basic_value()
        .left()
        .expect("elya_main returns a value");
    b.build_call(printf, &[fmt.as_pointer_value().into(), v.into()], "p").unwrap();
    b.build_return(Some(&i32t.const_int(0, false))).unwrap();

    module.verify().expect("hand-built module verifies");

    Target::initialize_native(&InitializationConfig::default()).expect("init native target");
    let triple = TargetMachine::get_default_triple();
    let target = Target::from_triple(&triple).expect("host target");
    let machine = target
        .create_target_machine(
            &triple,
            "",
            "",
            OptimizationLevel::None,
            RelocMode::Default,
            inkwell::targets::CodeModel::Default,
        )
        .expect("host target machine");

    let dir = temp_dir("toolchain-smoke");
    let obj = dir.join("smoke.o");
    let exe = dir.join(format!("smoke{}", std::env::consts::EXE_SUFFIX));
    machine.write_to_file(&module, FileType::Object, &obj).expect("emit object");

    let linked = Command::new("clang").arg(&obj).arg("-o").arg(&exe).output().expect("spawn clang");
    assert!(
        linked.status.success(),
        "clang failed: {}",
        String::from_utf8_lossy(&linked.stderr)
    );

    let out = Command::new(&exe).output().expect("run produced binary");
    assert!(out.status.success(), "binary exited {:?}", out.status);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "3");
    assert!(
        String::from_utf8_lossy(&out.stderr).is_empty(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::remove_dir_all(&dir).ok();
}
```

Notes for the implementer: inkwell 0.5 builders are fallible (`Result`) and pointers are opaque (`ctx.ptr_type(AddressSpace::default())`). If a specific call's error type differs from the sketch (e.g. `write_to_file` returning a typed error instead of `String`), adapt the `expect`/mapping minimally — the shape of the test must not change.

- [ ] **Step 4: Run the smoke test**

Run: `$env:CARGO_INCREMENTAL="0"; cargo test --features codegen --test native_codegen`
Expected: PASS — a process this repo produced printed `3`.

- [ ] **Step 5: Run the full gate in BOTH configurations**

Run: `$env:CARGO_INCREMENTAL="0"; cargo fmt --all; ./scripts/check.ps1`
Then: `$env:CARGO_INCREMENTAL="0"; cargo clippy --all-targets --features codegen -- -D warnings`
Expected: PASS. Without the feature, nothing changed (the test file is fully cfg'd out; no LLVM in the default build).

- [ ] **Step 6: Commit and push**

```powershell
$env:CARGO_INCREMENTAL="0"
cargo fmt --all
./scripts/check.ps1
if ($LASTEXITCODE -eq 0) {
  git add Cargo.toml Cargo.lock tests/native_codegen.rs
  git commit -m "feat(codegen): toolchain smoke — inkwell emits, clang links, binary prints 3 (5b-1 Task 1)" -m "The hard prerequisite gate (spec §9): hand-built LLVM module (@elya_main returning 3 + printf shim), verified, emitted via TargetMachine, linked with clang, executed, stdout asserted == '3'. Proves object emission, C-runtime linking, and %lld round-trip on this platform before any Core-to-LLVM code exists. inkwell is optional behind the codegen feature; the default build pulls in no LLVM. Working pair: <feature string> + <LLVM_SYS prefix if set>." -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
  git push origin main
}
```

**Stop condition:** if Steps 1–4 cannot be made green, **stop and report** with the exact probe outputs and llvm-sys build errors. Do not route around it. The textual-`.ll` fallback is the user's fork to re-open.

---

### Task 2: Core reach measurement _(no codegen)_

Runs the four §5 corpus programs through the real pipeline plus `lower_module`, asserting all four produce a `CoreModule`. Measures §6.3 rather than auditing: an `Untyped` error is a recorder gap on this slice's actual path (fix in inference with the perform-callee shape — record the synthesized node's type at its span); an `Unsupported` means the subset was drawn wrong (narrow the corpus, never widen `core.rs`). Expected outcome: zero gaps closed.

**Files:**

- Modify: `tests/native_codegen.rs` (append)

- [ ] **Step 1: Append the pipeline-lowering test**

Add to the top import block:

```rust
use elya::core::lower_module;
use elya::parse::parse_module;
use elya::Session;
```

Append at the bottom of the file:

```rust
/// The §5 corpus: (tag, source). Four cases so the proof is not "a program
/// that prints a hardcoded 3".
const CORPUS: &[(&str, &str)] = &[
    ("spine", "pub fn main() { 1 + 2 }
"),
    (
        "lets",
        "pub fn main() {
  let x = 6
  let y = 7
  x * y
}
",
    ),
    ("nesting", "pub fn main() { (2 + 3) * 4 - 5 }
"),
    ("negative", "pub fn main() { 3 - 10 }
"),
];

/// Parse → full front-end check → raw type table → lower. Runs the REAL
/// pipeline: `check_source` exercises resolve + inference + exhaustiveness +
/// affinity exactly as `elya check` does; only the table/lowering half is
/// repeated here because `front_end` is private and the frozen table is the
/// public accessor's product.
fn lower_src(src: &str) -> elya::core::CoreModule {
    assert!(
        elya::check_source("corpus.elya", src).is_ok(),
        "front end rejected corpus program: {src}"
    );
    let session = Session::new();
    let (m, pd) = parse_module(&session, src);
    assert!(pd.is_empty(), "parse: {pd:?}");
    let (diags, table) = elya::types::infer_typed_table(&session, &m);
    assert!(diags.is_empty(), "type errors: {diags:?}");
    lower_module(&m, &table).expect("corpus program must lower to Core")
}

#[test]
fn corpus_lowers_to_core_through_the_real_pipeline() {
    // Task 2 (spec §6.3): measure, don't audit. Each corpus program must reach
    // CoreModule unchanged. LowerError::Untyped(span) would be a recorder gap ON
    // THIS SLICE'S PATH — fix it in inference (record the synthesized node's type
    // at its span, the perform-callee shape from 5a-2 Task 3). LowerError::
    // Unsupported means the subset was drawn wrong — narrow the corpus, never
    // widen core.rs. Expected: all four lower today, zero gaps closed.
    for (tag, src) in CORPUS {
        let core = lower_src(src);
        assert_eq!(core.fns.len(), 1, "{tag}: expected exactly one fn");
        assert_eq!(core.fns[0].name, "main", "{tag}");
        assert!(core.fns[0].params.is_empty(), "{tag}");
    }
}
```

- [ ] **Step 2: Run it**

Run: `$env:CARGO_INCREMENTAL="0"; cargo test --features codegen --test native_codegen`
Expected: PASS (both tests). If `Untyped(span)` fires: fix the recorder gap in `src/types.rs` inference (record the synthesized node's type at its span), re-run, and state the fix explicitly in the commit body. If `Unsupported` fires: narrow the corpus and report — do not touch `core.rs`.

- [ ] **Step 3: Full gate, both configurations**

Run: `$env:CARGO_INCREMENTAL="0"; cargo fmt --all; ./scripts/check.ps1` and the codegen clippy line.
Expected: PASS.

- [ ] **Step 4: Commit and push**

```powershell
$env:CARGO_INCREMENTAL="0"
cargo fmt --all
./scripts/check.ps1
if ($LASTEXITCODE -eq 0) {
  git add tests/native_codegen.rs
  git commit -m "test(codegen): corpus reaches Core through the real pipeline (5b-1 Task 2)" -m "Measures §6.3 instead of auditing: all four arith-corpus programs pass the full front end (check_source) and lower_module unchanged. Recorder-totality class touched: ZERO gaps (expected outcome — 5a-2's surface-1 snapshot already proved Lit/Var/Prim lower)." -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
  git push origin main
}
```

---

### Task 3: Core→LLVM for the arithmetic subset

Implements §3.1–§3.3 and §3.5's `@elya_main`: module-shape validation, the `Int → i64` type mapping reading inline `ty` fields, the recursive fold with save/restore `Let` shadowing discipline, and `emit_ir` (debugging aid ONLY). Layer-1 unit tests: verifier-clean corpus, six specific `Unsupported` rejections, nothing panics.

**Files:**

- Modify: `src/lib.rs`
- Create: `src/codegen.rs`

- [ ] **Step 1: Declare the module**

In `src/lib.rs`, insert between `pub mod ast;` and `pub mod core;` (alphabetical: `codegen` sorts before `core`):

```rust
#[cfg(feature = "codegen")]
pub mod codegen;
```

- [ ] **Step 2: Check the layering test's expectations**

Read `tests/arch/layering.rs`. If it enumerates modules/allowed references explicitly, register `codegen` at the layer above `core` (it references `ast`, `types`, `span`, `core` — all ≤ core's layer). If it only forbids upward references, no edit is needed. Any edit to `tests/arch/layering.rs` is additive registration only — never a relaxation.

- [ ] **Step 3: Write `src/codegen.rs`**

Create `src/codegen.rs`:

```rust
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
use std::rc::Rc;

use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::types::IntType;
use inkwell::values::IntValue;
use inkwell::AddressSpace;

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
    Link { code: Option<i32>, stderr: String },
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
/// message, never a panic). Kept total over the AST operator set.
fn op_label(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "Add",
        BinOp::Sub => "Sub",
        BinOp::Mul => "Mul",
        BinOp::Div => "Div",
        BinOp::Rem => "Rem",
        BinOp::AddF => "AddF",
        BinOp::DivF => "DivF",
        BinOp::Eq => "Eq",
        BinOp::Ne => "Ne",
        BinOp::Lt => "Lt",
        BinOp::Le => "Le",
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
fn build_module<'ctx>(
    ctx: &'ctx Context<'ctx>,
    core: &CoreModule,
) -> Result<Module<'ctx>, CodegenError> {
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

    module.verify().map_err(|e| CodegenError::Verify(e.to_string()))?;
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
```

- [ ] **Step 4: Add the layer-1 unit tests**

Append to `src/codegen.rs`:

```rust
#[cfg(all(test, feature = "codegen"))]
mod tests {
    use super::*;
    use crate::span::Span;

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
            "pub fn main() { 1 + 2 }
",
            "pub fn main() {
  let x = 6
  let y = 7
  x * y
}
",
            "pub fn main() { (2 + 3) * 4 - 5 }
",
            "pub fn main() { 3 - 10 }
",
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
            kind: CoreKind::Lambda(Rc::from(["x"]), Rc::new(int_lit(1))),
        };
        let err = emit_ir(&main_fn(e)).unwrap_err();
        assert!(matches!(err, CodegenError::Unsupported("Lambda")), "{err:?}");
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
        m.fns[0].params = Rc::from(["x"]);
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
```

Implementer notes: if `BinOp` carries variants beyond the fourteen listed (e.g. `Gt`/`Ge`), `op_label` must gain arms — the compiler will point at the non-exhaustive match.

- [ ] **Step 5: Run the unit tests**

Run: `$env:CARGO_INCREMENTAL="0"; cargo test --features codegen --lib codegen`
Expected: PASS (eight tests).

- [ ] **Step 6: Full gate, both configurations**

Run: `$env:CARGO_INCREMENTAL="0"; cargo fmt --all; ./scripts/check.ps1` and the codegen clippy line. Confirm `cargo test --all` (no features) is untouched and every existing snapshot is byte-identical.
Expected: PASS.

- [ ] **Step 7: Commit and push**

```powershell
$env:CARGO_INCREMENTAL="0"
cargo fmt --all
./scripts/check.ps1
if ($LASTEXITCODE -eq 0) {
  git add src/lib.rs src/codegen.rs
  git commit -m "feat(codegen): Core-to-LLVM fold for the arithmetic subset (5b-1 Task 3)" -m "New leaf module consuming core::CoreModule (its first real consumer). Reads inline Shape-C ty fields; Int->i64 only; recursive SSA fold with Let save/restore shadowing discipline; no alloca/mem2reg. Add/Sub/Mul emitted WITHOUT nsw/nuw (defined wrapping, §3.4); Div/Rem/And/Or/App/Lambda/Match rejected with specific Unsupported messages — never a panic. emit_ir is a debugging aid only; nothing asserts on IR. Layer-1 teeth: corpus verifies, six rejections pinned." -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
  git push origin main
}
```

---

### Task 4: Shim, object, link, and the execution proof

The `printf` declaration, format-string global, `@main` shim, `TargetMachine` object emission (`compile_module`), `link`, and all four §5 cases through the library API. **This is the slice's milestone.**

**Files:**

- Modify: `src/codegen.rs`
- Modify: `tests/native_codegen.rs`

- [ ] **Step 1: Extend `build_module` with the shim (diff against Task 3 state)**

In `src/codegen.rs`, extend the import block:

```rust
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use inkwell::OptimizationLevel;
use std::path::Path;
```

Replace the tail of `build_module` (the part after `build_return`):

```rust
    b.build_return(Some(&result)).map_err(internal)?;

    // §3.5/§4: the print convention is ONE external symbol (printf) plus ONE
    // generated shim (@main). Elya's namespace stays clean for N2; deleting the
    // convention later is deleting a function, not unpicking a fold.
    let i32t = ctx.i32_type();
    let i8t = ctx.i8_type();
    let ptrt = ctx.ptr_type(AddressSpace::default());
    let printf_ty = i32t.fn_type(&[ptrt.into(), i64t.into()], true);
    let printf = module.add_function("printf", printf_ty, None);

    let fmt_bytes: &[u8] = b"%lld
\0";
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
    b.build_return(Some(&i32t.const_int(0, false))).map_err(internal)?;

    module.verify().map_err(|e| CodegenError::Verify(e.to_string()))?;
    Ok(module)
```

(`build_module`'s signature is unchanged: `-> Result<Module<'ctx>, CodegenError>`.)

- [ ] **Step 2: Add `compile_module` and `link`**

Append to `src/codegen.rs` (after `emit_ir`):

```rust
/// Core -> object file on disk. The spine (§3.6): host triple only,
/// OptimizationLevel::None, verify() before emission.
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
    machine.write_to_file(&module, FileType::Object, obj_path).map_err(internal)
}

/// Object file -> executable, via `clang` (hardcoded, §3.6: LLVM is already a
/// hard prerequisite and clang ships with it). Non-zero exit surfaces clang's
/// stderr in CodegenError::Link.
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
```

- [ ] **Step 3: Write the four execution cases**

Extend `tests/native_codegen.rs` — add to the import block:

```rust
use std::path::Path;
```

Append at the bottom:

```rust
/// Compile + link through the LIBRARY API into `dir`. Task 5 adds the CLI path.
fn compile_and_link(core: &elya::core::CoreModule, dir: &Path, tag: &str) -> PathBuf {
    let obj = dir.join(format!("{tag}.o"));
    let exe = dir.join(format!("{tag}{}", std::env::consts::EXE_SUFFIX));
    elya::codegen::compile_module(core, &obj).expect("compile_module");
    elya::codegen::link(&obj, &exe).expect("link");
    exe
}

/// The three required assertions per case (§5): exit status, stdout, empty stderr.
fn assert_runs(exe: &Path, expected: &str) {
    let out = Command::new(exe).output().expect("run produced binary");
    assert!(out.status.success(), "binary exited {:?}", out.status);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), expected);
    assert!(
        String::from_utf8_lossy(&out.stderr).is_empty(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn spine_prints_three() {
    let core = lower_src(CORPUS[0].1);
    let dir = temp_dir("spine");
    let exe = compile_and_link(&core, &dir, CORPUS[0].0);
    assert_runs(&exe, "3"); // the spine
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn lets_and_mul_print_forty_two() {
    let core = lower_src(CORPUS[1].1);
    let dir = temp_dir("lets");
    let exe = compile_and_link(&core, &dir, CORPUS[1].0);
    assert_runs(&exe, "42"); // Let, Var, Mul
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn nesting_and_sub_print_fifteen() {
    let core = lower_src(CORPUS[2].1);
    let dir = temp_dir("nesting");
    let exe = compile_and_link(&core, &dir, CORPUS[2].0);
    assert_runs(&exe, "15"); // nesting, precedence, Sub
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn negative_result_prints_minus_seven() {
    let core = lower_src(CORPUS[3].1);
    let dir = temp_dir("negative");
    let exe = compile_and_link(&core, &dir, CORPUS[3].0);
    assert_runs(&exe, "-7"); // signed negatives survive %lld
    std::fs::remove_dir_all(&dir).ok();
}
```

- [ ] **Step 4: Run the proof**

Run: `$env:CARGO_INCREMENTAL="0"; cargo test --features codegen --test native_codegen`
Expected: PASS — six tests; four binaries ran and printed `3`, `42`, `15`, `-7`. **This is the slice's milestone.**

If a case fails: reproduce with `emit_ir` (debugging aid) by hand if needed, fix the fold/emission, re-run. Never weaken an assertion; never inspect IR as a substitute for the failing run.

- [ ] **Step 5: Full gate, both configurations**

Run: `$env:CARGO_INCREMENTAL="0"; cargo fmt --all; ./scripts/check.ps1` and the codegen clippy line.
Expected: PASS.

- [ ] **Step 6: Commit and push**

```powershell
$env:CARGO_INCREMENTAL="0"
cargo fmt --all
./scripts/check.ps1
if ($LASTEXITCODE -eq 0) {
  git add src/codegen.rs tests/native_codegen.rs
  git commit -m "feat(codegen): printf shim, object emission, link — four binaries run (5b-1 Task 4)" -m "THE MILESTONE: four corpus programs compiled through the library API run natively and print 3, 42, 15, -7 — asserted on exit status, stdout, and empty stderr. One external symbol (printf), one generated @main shim calling @elya_main; verify() before TargetMachine object emission; clang link with stderr surfaced in CodegenError::Link. Artifacts live in unique pid-keyed temp dirs, removed on success. No IR snapshots; no skippable tests." -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
  git push origin main
}
```

---

### Task 5: `elya build` and the CLI-driven proof

The `build` subcommand, one §5 case re-run through the real `elya` binary, and the gate extension (§2.4).

> **Step 0 (added 2026-08-28, carried from Task 3): split codegen into its own crate — do
> this FIRST, before the rest of Task 5.**
>
> `codegen` is currently a feature on the `elya` *lib*, so all 24 integration tests plus
> the lib and bin test binaries link `llvm_sys` — even though only
> `tests/native_codegen.rs` touches inkwell. The vcpkg `x64-windows-static-md-rel` LLVM 18
> is bundled into `libllvm_sys-*.rlib` at **3.73 GiB** (1719 `.obj` members), so on an
> 8-core / 16 GiB machine parallel link jobs exhaust memory. It shows up nondeterministically
> as either `crate llvm_sys required to be available in rlib format, but was not found in
> this form` or `LINK : fatal error LNK1102: out of memory`, and it survives a pristine-target
> rebuild — it is not stale `target/` state and not a feature-resolution bug.
>
> Tasks 3 and 4 work around it by building the feature with `-j 2`. Task 5 must not inherit
> that cap: it already touches the `elya build` wiring and adds the codegen gate stages, so
> the split lands before the gate depends on it. After the split, only the codegen crate's
> own tests link LLVM and the `-j 2` cap is dropped everywhere — the gate's codegen stages
> in Step 4 run at default parallelism.
>
> Dynamic LLVM (inkwell `llvm18-0-prefer-dynamic` / `llvm18-0-force-dynamic`) was considered
> and rejected: a dynamic LLVM dependency for the shipped compiler is a product decision,
> not a build-speed fix. Held in reserve only if the split proves insufficient.

**Files:**

- Modify: `src/main.rs`
- Modify: `tests/native_codegen.rs`
- Modify: `scripts/check.sh`, `scripts/check.ps1`

- [ ] **Step 1: Add the subcommand to `src/main.rs`**

Diff against current `main.rs`:

```rust
------- SEARCH
        Some("run") => cmd(&args, true),
        Some("check") => cmd(&args, false),
        _ => {
            eprintln!("usage: elya <run|check> <file.elya>");
            ExitCode::from(2)
        }
=======
        Some("run") => cmd(&args, true),
        Some("check") => cmd(&args, false),
        #[cfg(feature = "codegen")]
        Some("build") => build_cmd(&args),
        _ => {
            eprintln!("usage: elya <run|check|build> <file.elya>");
            ExitCode::from(2)
        }
+++++++ REPLACE
```

And append:

```rust
/// `elya build <file.elya> [-o <out>]` (Slice 5b-1, §7): front end exactly as
/// `elya check` (same diagnostics, same warnings), then Core lowering, object
/// emission, and clang link. Default output: input stem + platform exe suffix.
#[cfg(feature = "codegen")]
fn build_cmd(args: &[String]) -> ExitCode {
    let Some(path) = args.get(2) else {
        eprintln!("error: missing file path");
        return ExitCode::from(2);
    };
    let out_path = match (args.get(3).map(String::as_str), args.get(4)) {
        (Some("-o"), Some(o)) => Some(std::path::PathBuf::from(o)),
        (None, _) => None,
        _ => {
            eprintln!("usage: elya build <file.elya> [-o <out>]");
            return ExitCode::from(2);
        }
    };
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: cannot read {path}: {e}");
            return ExitCode::from(2);
        }
    };
    if let Err(diags) = elya::check_source(path, &text) {
        eprint!("{diags}");
        return ExitCode::FAILURE;
    }
    surface_warnings(path, &text);

    let session = elya::Session::new();
    let (module, pd) = elya::parse::parse_module(&session, &text);
    debug_assert!(pd.is_empty(), "check_source passed but parse failed: {pd:?}");
    let (diags, table) = elya::types::infer_typed_table(&session, &module);
    debug_assert!(diags.is_empty(), "check_source passed but inference errored: {diags:?}");
    let core = match elya::core::lower_module(&module, &table) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: core lowering failed: {e:?}");
            return ExitCode::FAILURE;
        }
    };

    let exe = out_path.unwrap_or_else(|| {
        std::path::PathBuf::from(path).with_extension(std::env::consts::EXE_EXTENSION)
    });
    let obj = std::env::temp_dir().join(format!(
        "elya-build-{}-{}.o",
        std::process::id(),
        exe.file_stem().and_then(|s| s.to_str()).unwrap_or("out")
    ));
    if let Err(e) = elya::codegen::compile_module(&core, &obj) {
        eprintln!("error: {e}");
        return ExitCode::FAILURE;
    }
    let linked = elya::codegen::link(&obj, &exe);
    let _ = std::fs::remove_file(&obj);
    if let Err(e) = linked {
        eprintln!("error: {e}");
        return ExitCode::FAILURE;
    }
    println!("built {}", exe.display());
    ExitCode::SUCCESS
}
```

- [ ] **Step 2: Add the CLI-driven test**

Append to `tests/native_codegen.rs`:

```rust
#[test]
fn elya_build_cli_produces_runnable_binary() {
    // The strongest case drives the REAL CLI (§5): the proof covers the
    // user-facing entry point, not just the library API.
    let dir = temp_dir("cli");
    let src = dir.join("prog.elya");
    std::fs::write(&src, CORPUS[0].1).expect("write corpus source");
    let exe = dir.join(format!("prog{}", std::env::consts::EXE_SUFFIX));
    let out = Command::new(env!("CARGO_BIN_EXE_elya"))
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("spawn elya build");
    assert!(
        out.status.success(),
        "elya build failed:
stdout: {}
stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_runs(&exe, "3");
    std::fs::remove_dir_all(&dir).ok();
}
```

- [ ] **Step 3: Run it**

Run: `$env:CARGO_INCREMENTAL="0"; cargo test --features codegen --test native_codegen`
Expected: PASS — seven tests; the CLI-produced binary prints `3`.

- [ ] **Step 4: Extend the gate (both twins)**

`scripts/check.sh` becomes:

```sh
#!/usr/bin/env sh
set -e
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all
cargo clippy --all-targets --features codegen -- -D warnings
cargo test --features codegen --test native_codegen
```

`scripts/check.ps1` gains the same two lines (each followed by its `if ($LASTEXITCODE -ne 0) { exit 1 }` guard, matching the file's existing style).

- [ ] **Step 5: Full gate — now the extended gate itself**

Run: `$env:CARGO_INCREMENTAL="0"; cargo fmt --all; ./scripts/check.ps1`
Expected: PASS — five stages green, including both clippy configurations and the execution proof.

- [ ] **Step 6: Commit and push**

```powershell
$env:CARGO_INCREMENTAL="0"
cargo fmt --all
./scripts/check.ps1
if ($LASTEXITCODE -eq 0) {
  git add src/main.rs tests/native_codegen.rs scripts/check.sh scripts/check.ps1
  git commit -m "feat(codegen): elya build subcommand + CLI-driven proof; gate runs codegen (5b-1 Task 5)" -m "elya build <file.elya> [-o <out>] runs the front end exactly as elya check (diagnostics and warnings surfacing unchanged), lowers to Core, emits an object, links with clang; default output is the input stem + platform exe suffix. The proof covers the real entry point: a test invokes CARGO_BIN_EXE_elya, builds prog.elya, runs the binary, asserts '3'. check.sh/check.ps1 now clip and test BOTH feature configurations with -D warnings, so the codegen feature cannot rot." -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
  git push origin main
}
```

---

## Self-Review

**Spec coverage (§12 checklist → tasks):**

- inkwell optional behind `codegen`; default build LLVM-free — Task 1 (Cargo.toml), verified every task's gate.
- Hand-built module compiles/links/runs/prints `3` before any Core→LLVM code — Task 1.
- Corpus reaches `CoreModule` via the real pipeline; recorder-gap count stated as fact — Task 2 (expected: zero).
- `module.verify()` per corpus program — Task 3 (`corpus_verifies`) and structurally inside `compile_module`.
- Specific `Unsupported` rejections, nothing panics — Task 3 (six pinned messages; `validate_module`/`require_int`/fold arms are total).
- Exactly one external symbol + one shim; no runtime library; no front-end change — Task 4 (printf + `@main` only; `src/main.rs` gains only a cfg'd subcommand).
- Four binaries print `3`, `42`, `15`, `-7`, asserted on status/stdout/stderr — Task 4.
- At least one case drives `elya build` end to end — Task 5.
- No insta snapshot of LLVM IR — structural: the only renderer is `emit_ir`, documented as a debugging aid, never asserted on.
- No test can skip — no `#[ignore]` anywhere; the feature-gated file is exercised by the gate's explicit `--features codegen` lines.
- `cargo test --all` (no features) stays green; snapshots unchanged — asserted at every task's gate step.
- Gate clips/tests both configurations — Task 5 (both script twins).
- Div/Rem/And/Or rejected with §3.4 rationale recorded — Task 3 (code comments cite §3.4; commit body restates it).

**Known risks carried into implementation:** inkwell 0.5 API drift (fallible builders, opaque pointers, exact error types of `verify`/`write_to_file`) — adapt mappings minimally, never the test shapes; `BinOp` may have variants beyond the fourteen listed (`op_label` must stay total); the arch layering test may need additive `codegen` registration (Task 3 Step 2); `pub fn main()` without an effect row is expected to type-check per spec §4 (`check_main_discharge` constrains only the effect row) — Task 2 falsifies this first if wrong.

**Deliberate deviations flagged for reviewer sign-off:** (1) `toolchain_smoke` seeded into `tests/native_codegen.rs` in Task 1; (2) both `check.sh` and `check.ps1` extended.
