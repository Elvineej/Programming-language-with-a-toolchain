# Plan: the checker in Elya (self-hosting parts 4d to 4f)

Spec: `docs/superpowers/specs/2026-11-15-elya-checker-in-elya-design.md`.

1. [x] D2's print forms: `elya exhaust --dump`, `elya affine --dump`
   (`elya::check_dump::affine_facts`), and a warning printed as `warning`
   by `dump_diagnostics`. Test: `exhaust_and_affine_dumps` in
   crates/cli/tests/run_cli.rs (each of the six fact kinds; a warning's line).
   Gate predicted: +2 (the CLI suite runs in configurations A and B), no new
   suite: 1663 passed, 193 suites; measured 1663 and 193. Review: no defect;
   every field printed, nothing ordered by hashing (md5 equal over repeated
   runs), the same input as inference's, the `warning` change breaking no
   pinned output. Its notes, now in the spec: the exhaust dump runs on
   ill-typed programs on purpose; `resumes` keeps the walk's order; inference
   analyzes the same module. Docs only after the gate.
2. [ ] 4d, `selfhost/exhaust.elya`: Codex, after 4c.
3. [ ] 4e, `selfhost/affine.elya`: Codex, after 4d.
4. [ ] 4f, inference in Elya: Claude. Its own plan, once 4e's facts are in
   hand.
