# Multi-shot resumes share the captured chain -- plan

Spec: `docs/superpowers/specs/2026-10-22-elya-multi-shot-sharing-design.md`. Branch
`auto/elya` at `b2ec3cf` (the lock, on `7ac69a6`). Baseline Linux gate: 1086 passed, 106 suites (measured at b2ec3cf).

1. Red tests, `crates/codegen/tests/native_codegen.rs` (native: each counts once):
   - `a_deep_multi_shot_chain_is_shared_not_copied`: N = 8 000, predicted red on `live`
     (about 65 million words against a bound of 512 000);
   - `a_deep_cps_multi_shot_chain_is_shared_not_copied`: N = 2 000, predicted red on
     `freed` (about 2 million blocks against 128 000);
   - `the_parked_deep_multi_shot_program_runs_natively`: predicted red, out of memory.
     Its red run is under `ulimit -v` 4 GiB, so it cannot take the machine down;
   - `a_handler_frame_reachable_only_as_current_survives_collections`: a guard,
     predicted GREEN before the change (the chain keeps the frame). Its red is K4.

   +4.
2. Runtime (`runtime.c`):
   - the static end block and `elya_handle_end_init` (D3);
   - `gc_collect` grays `elya_current_handler` before `gc_mark` (D3; `gc_mark`
     byte-identical);
   - `elya_cont_copy` walks `parent` from `innermost` to `h` (D2).
3. Codegen:
   - `elya_rt_handle_end`, the `tailcc` trampoline (D1);
   - `main` calls `elya_handle_end_init` after `elya_gc_init`;
   - a handle body's region continuation is the end frame (`St::new`); its saved
     bindings are still loaded from its frame.
4. Controls (each reverted, `git diff --quiet` after):
   - K1, the copy returns the continuation unchanged: m3 fails;
   - K2, no `parent` relink: m10 fails;
   - K3, the body's continuation is its frame again: the CPS deep test or a corpus
     row fails;
   - K4, no current-handler root: the D3 guard fails (a crash or a wrong value).
5. The differential sweep (`scripts/sweep.py`) and the 721-program session sweep: no
   new disagreement.

Predicted gate: 1086 + 4 = **1090 passed, 106 suites**.

Measured: **1090 passed, 106 suites**, as predicted. The review's three low notes were fixed without
new tests (N1 strengthened `copies_survive_a_collection_mid_copy`, red first).
