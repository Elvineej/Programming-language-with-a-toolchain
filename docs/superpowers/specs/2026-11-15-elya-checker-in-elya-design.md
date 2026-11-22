# The checker in Elya (self-hosting part 4d to 4f)

2026-11-15. Status: the print forms of D2 built; 4d and 4e are Codex's jobs, 4f Claude's. Part of R3 item 4 of
`2026-11-14-elya-self-hosting-design.md`, after modules and resolution (4c).

## 0. What it is

What runs after resolution in `elya::front_end_with`, about 8,000 lines of
Rust:

- `affine::analyze` (src/affine.rs, 1,300 lines): a syntactic pass before
  inference. It finds the bindings used many times, what each point holds,
  each handler's resumes, and the borrow facts. These are `affine::Facts`.
- inference (src/types.rs, 6,300 lines): Hindley-Milner with effect rows, row
  variables and inclusions, levels, annotations and kinds. Duplicability is
  required by marks on type variables. It reads `Facts`, which it computes
  with `affine::analyze` on the same resolved module (src/types.rs), and reports the
  checker's errors and the affine ones (E04xx).
- `exhaust::check` (src/exhaust.rs, 440 lines): Maranget's usefulness over
  the `type` declarations, with no inferred types. It runs only on a program
  with no errors.
- `modules::check_grants` (E0606, part of src/modules.rs): what each import
  grants, against the inferred types of the module's `pub` functions.

## 1. Decisions

**D1. Three parts, in order of size and of how much each depends on the
others.**

- **4d, exhaustiveness** (`exhaust::check`): standalone. A Codex job.
- **4e, the affine facts** (`affine::analyze`): standalone. A Codex job.
- **4f, inference and the grants**, using 4e's facts. Claude's: it is the
  hard part, and the dump has to agree on every type of every node.

**D2. Print forms.**

- `elya exhaust --dump <file>` runs `exhaust::check` on the resolved program,
  and prints its diagnostics in `dump_diagnostics`'s form. It does so even
  when the program has type errors, which the front end checks first (it runs
  exhaustiveness only on a program with no errors). The pass reads no types,
  so the dump tests it on more programs. The Elya twin must not copy the
  front end's guard.
- `elya affine --dump <file>` prints `affine::Facts`, one fact a line, each
  table sorted by its key's span (start, then end), every span `start end`:
  - `many <binder> <at> twice|closure <s> <e>|clause <s> <e>`
  - `held <point> <binder> <first use>`, one line per entry, in the entry's
    order (by binder: the pass keeps them in a `BTreeMap`)
  - `resumes <handle> <resume>`, likewise. These entries are in the order the
    pass's backward liveness walk finds them, not in source order, so the
    Elya twin keeps the same walk order rather than sorting.
  - `late <binder> <read> <consuming use>`
  - `given <binder> <use>`
  - `borrow-value <use> <name> <modes as 0s and 1s>`
- Inference already has its form, `elya check --dump` (spec
  2026-11-15-elya-parser-in-elya D6).

When loading or resolving the program reported an error, each of the three
prints those diagnostics alone, as `check --dump` does: there is nothing to
check. A warning's line begins `warning`, not `error`, in every dump
(`dump_diagnostics`). Before this, E0431 and E0426 were printed as `error`;
no test pinned that, and the parser in Elya prints errors only.

**D3. Messages are compared too, byte for byte.** The question was left open
(compare codes and spans only, to save time). It is closed by what the dump
already needs. `check --dump` prints every node's type, so the Elya checker
needs Rust's type printer exactly, fresh-variable names included. A
diagnostic's message is that printer plus fixed words copied from
src/types.rs. Leaving the messages out would save little, and would leave
R2's "byte for byte" unmet.

**D3a. What a diagnostic prints.** `dump_diagnostics` prints a
diagnostic's headline only (`type mismatch`), not its labels, which hold
most of the type printing (`expected Int, found Bool`). So D3 costs even
less than it first seemed.

**D4. Inference's own order.** `check --dump` names type variables afresh on
each line (spec 2026-11-15 D6), so the Elya checker does not have to number
its variables the way Rust does. It must reach the same types, the same
schemes and the same diagnostics in the same order.

## 2. Tests

Each part runs on the parser's corpus, the multi-file programs (4c), and
every program in the tests with an error of that part. Each also gets a
focused set and negative controls, like the earlier parts. Their briefs say
which.

## 3. Measured after

- F1 (2026-11-22): the largest first-order program (285 bytes) takes 1.8 ms
  natively per process, against 0.31 ms for Rust's resolve and check in
  process. The native figure is mostly process start-up and the parse.
