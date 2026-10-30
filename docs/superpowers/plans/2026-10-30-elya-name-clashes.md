# Plan: a name clash is reported once, where it was made

Spec `docs/superpowers/specs/2026-10-30-elya-name-clashes-design.md`. Gate before:
1319 passed, 139 suites (059f59e).

1. Red tests first, `tests/name_clashes.rs` (root crate), four tests:
   - an operation clash, through `elya/console` and `elya/host`: predicted red with
     three E0200s labelled in `elya/console.elya`;
   - an effect clash: predicted red with five E0207s;
   - a file's own type while a transitive module declares one: predicted red with
     two E0605s;
   - what stays: E0605 for a name used from a module the file does not import, and
     each clash's one E0209 or E0205 (green before and after).
2. D1 in `modules::Rewriter::expr`, D3 in `Rewriter::visible`, D2 in
   `resolve::check_handler_covers_its_effect`.
3. Controls, each reverted:
   - K1, D1's order back (the root-function check first): the three E0200s again;
   - K2, D2 off: the five E0207s again;
   - K3, D3 off: the two E0605s again.

Predicted gate: 1319 + 4 x 2 (root tests run twice) = **1327 passed, 141 suites**.

The review's fixes add two tests (`every_kind_of_own_name_is_visible`,
`a_name_an_imported_file_declares_is_visible`): predicted gate 1327 + 2 x 2 =
**1331 passed, 141 suites**.
