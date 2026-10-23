# Modules, part 2: the effect allow-list at the import

**Status:** in progress (2026-10-24, branch `auto/elya`, a scheduled run). HANDOFF
step 1, ROADMAP priority 2 (capabilities per dependency). The design choices are
Claude's under the maintainer's delegation (rule 2), recorded here.

## 0. Measured first (at 37846fa, gate 1209 / 119)

Five modules beside each program, `elya run --value`:

| program | module | today |
|---|---|---|
| m1 `import steal`, `steal.parse("{}")` | `pub fn parse` reads the clock and prints "phoning home" | runs: prints, 3 |
| m2 `import steal / {}` | the same | E0606 "an import's effect list is not checked yet" |
| m3 `import mk`, `mk.later()()` | `pub fn later() -> fn() -> Int` returns a closure that prints | runs: prints, 7 |
| m4 `import net`, handles `Net` | `pub fn fetch` performs its own `Net` | 42 |
| m5 `import quiet` | performs `Net` inside and handles it there | 42 |

So nothing stops a dependency from doing I/O (m1, m3): the ROADMAP's problem.

What the front end knows: every function's generalized type after inference,
whose rows name the effects it performs that are not handled inside it (`IO`
for `io.*` and `time.*`, which no handler can handle; user effects by name).
A function type inside a data type's field has the row written in the
declaration (an unwritten declaration row is empty).

## 1. Decisions

- **D1. What a module can do is what its interface exposes, positively.** The
  effects a module `M` can perform on its importer's behalf are the effect
  labels at POSITIVE positions of its `pub` functions' inferred types: the
  function's own row; a returned function's row (recursively); a function
  given to a callback (a parameter's parameter, doubly negative); and function
  fields of data at a positive position (each declared field type, at the
  data's polarity, transitively through the declared types). A label in a
  parameter's own row is the caller's closure performing it, not `M`: not
  counted. An effect `M` performs and handles inside itself never reaches its
  interface (m5): handling is attenuation, and needs no grant.
  - `IO` counts like every user effect: a capability is any effect, and a user
    effect a dependency performs is given its meaning by whoever handles it,
    which may be another dependency doing real I/O.
  - *Rejected:* every label anywhere in the interface (both polarities). A
    handler-library (`pub fn sandbox(body: fn() / {Net} -> Int)`) would need
    `Net` granted though it only receives it.
  - *Rejected:* checking a module's function bodies for performs. Inference's
    rows are already exactly what escapes; a second analysis could disagree.
- **D2. An import grants exactly its list; no list grants nothing.**
  `import a/b / {E, IO}`; `import a/b` is `import a/b / {}`. When `M`'s
  interface exposes an effect not granted, the import is **E0606** "module
  `a/b` can perform `IO`, which this import does not grant", labelled at the
  import, with the `pub` function that exposes it named in the help. One
  diagnostic per (import, effect).
  - *Rejected:* no list meaning "everything", for compatibility. It is the
    default the problem exists in; with the strict default a dependency's
    authority is always written where it is imported. No program in the corpus
    imports an effectful module (the modules tests' modules are pure or
    performance-free in their interfaces; measured when the code lands, §2).
- **D3. Transitivity is by interface.** `M`'s own imports are checked the same
  way against `M`'s lists. Whatever `M` lets escape from a dependency is in `M`'s
  own interface, so the root's grant to `M` bounds it: a module cannot launder
  an ungranted effect through another module.
- **D4. The list names visible effects.** Each name in a list is `IO` or an
  effect visible in the importing file (declared there or in a module it
  imports, part 1 D3); anything else is **E0607** "no effect `X` here". A name
  listed twice is E0607 too ("listed twice").
- **D5. The standard modules take no list.** `import elya/io / {IO}` is **E0608**
  "a standard module takes no effect list": builtins stay in scope without an
  import (part 1) and the root program, which runs them, holds every
  capability. Whether builtins must be imported is left to a later step.
- **D6. The root holds every capability.** Nothing checks the root's own
  functions; the allow-list is about dependencies.

## 2. Evidence (planned)

`tests/modules.rs` gains (root crate, counted twice):
1. `an_import_grants_only_its_list`: m1 E0606 for `IO`; m1 with `/ {IO}` runs;
   m3 (a returned closure) E0606; m4 E0606 for `Net`, and runs with `/ {Net}`;
   m5 checks with no list.
2. `a_callback_s_effect_is_not_the_module_s`: a handler library taking
   `fn() / {Net} -> Int` checks with no list; one calling a callback with a
   closure that prints needs `IO`; data with an `IO` closure field returned
   needs `IO`.
3. `grants_are_transitive`: `lib` imports `steal / {IO}` and re-exposes it: the
   root's `import lib` is E0606; `lib` handling or hiding it (pure interface)
   checks.
4. `the_list_names_visible_effects`: E0607 unknown and duplicate; E0608 on
   `elya/io`.
Negative controls: polarity ignored (every label counted): test 2's handler
library is E0606; the check off: test 1's m1 runs.
