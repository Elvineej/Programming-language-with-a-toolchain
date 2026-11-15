# Plan: modules and name resolution in Elya (self-hosting part 4c)

Spec: `docs/superpowers/specs/2026-11-15-elya-resolution-in-elya-design.md`.

1. [x] D1, the Rust side: `elya::resolved` (load, then resolve with the R3
   filter, taken out of `front_end_with`, which calls it), `elya::merged_text`,
   and `elya resolve --dump`. Test: `resolve_dump_prints_the_merged_program`
   (an imported file's `Float` at its merged offset; an error's line; exit
   codes). Gate predicted: +2 tests over the last landing (the CLI suite runs
   in configurations A and B), no new suite: 1661 passed, 193 suites;
   measured 1661 and 193. Review: no defect; `front_end_with` unchanged, the
   joined text right for imports of imports, aliases and std modules, the
   output deterministic. It found the spec's "load order" ambiguous (files are
   reached root first, depth first; declarations merge root last), now
   spelled out in D1 and on `merged_text` (comments only after the gate).
2. [ ] D2-D4, `selfhost/resolve.elya` and its tests: Codex, after part 4b,
   to a brief from this spec; checked here, a control re-run.
3. [ ] D5, measured, in the spec's section 3.
