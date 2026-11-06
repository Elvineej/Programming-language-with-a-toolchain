# The affine guarantee across functions

**Status:** designed (2026-11-06, on `main`, an interactive session). HANDOFF
step 1, the maintainer's priority (2026-10-30: "the affine resource guarantee
is incomplete"). The design choices are Claude's under the maintainer's
delegation (rule 2). It closes the two obligations slice 4d-2 recorded
(`affine-callee-duplication-obligation`, soundness;
`affine-intraprocedural-overapprox`, completeness; 4d-2 spec §6).

## 0. Measured first

`elya check` and `elya run --value` at `main` (af3be7e's front end, release
CLI), every program over the prelude `linear type File { File(Int) }` and
`fn read(f: File) -> Int { match f { File(n) -> n } }`; a value above the
number of `File`s read means one was read twice. The programs are in the
scratchpad's `aff/` (`gen.py` .. `gen4.py`); each goes into `tests/affine.rs`.

| | program | `check` | run | should be |
|---|---|---|---|---|
| U1 | `fn dup(f: File) -> Int { read(f) + read(f) }` | clean | 2 | refused |
| U2 | the same, unannotated | clean | 2 | refused |
| U3 | `let g = fn() { read(f) }`, `g() + g()` | clean | 2 | refused |
| U4 | `let b = Box(f)`, `open(b) + open(b)` (`Box` not `linear`) | clean | 2 | refused |
| U5 | `let g = fn() { f }`, `read(g()) + read(g())` | clean | 2 | refused |
| U6 | `both(f, f)` | E0428 | | refused |
| U7 | `fn loop(f: File, n: Int)` re-using `f` in its recursive arm | clean | 2 | refused |
| U8 | `let g = f`, `read(g) + read(f)`, `f` a parameter | clean | 2 | refused |
| U9 | `match Box(File(1)) { Box(f) -> read(f) + read(f) }` | clean | 2 | refused |
| U10 | used in a `handle`'s body and in a clause | E0428 | | refused |
| U11 | used in a clause only, the body performing twice | clean | 2 | refused |
| U12 | held across a call of a function performing a `multi` effect | clean | 2 | refused |
| U13 | a parameter held across a `multi` perform in its own function | clean | 2 | refused |
| U14 | `fn after(g, x) { let _ = g()  x }` given a `multi`-performing `g` | clean | 2 | refused |
| U15 | `fn twice(g, x) { g(x) + g(x) }`, `twice(read, File(1))` | clean | 2 | refused |
| U16 | a closure capturing `f` given to `fn twice(g) { g() + g() }` | clean | 2 | refused |
| U17 | `fn dupper() { fn(x) { let _ = x  x } }`, `dupper()(File(1))` | clean | 1 | refused |
| U18 | `fn mk(x) { fn() { x } }`, the closure called twice | clean | 2 | refused |
| U19 | a closure capturing `f` stored in a `Box`, taken out, called twice | clean | 2 | refused |
| U20 | used in a `handle`'s body and its return clause | E0428 | | refused |
| U23 | a `multi` perform, then the continuation called through a lambda | E0429 | | refused |
| U24 | `fn each(xs, k)` over a list of `File`s, `k` performing `multi` | clean | 6 | refused |
| S1 | one use in each arm of an `if` | E0428 | | accepted |
| S2 | one use in each arm of a `match` | E0428 | | accepted |
| S3 | a closure capturing `f` called once | clean | 1 | accepted |
| S4 | used in one arm, not the other | clean | 1 | accepted |
| S5 | a parameter used once | clean | 1 | accepted |
| S6 | used once after a `handle` whose clause resumes twice | E0429 | | accepted |
| S7 | a closure capturing `f` called once in each arm | clean | 1 | accepted |
| S8 | `fn app(g, x) { g(x) }`, `app(read, File(1))` | clean | 1 | accepted |
| S9 | used in the return clause only | clean | 2 (`1 + 1`) | accepted |
| S10 | `each` over a list of `File`s, a pure `k` | clean | 3 | accepted |

Seventeen programs that read a `File` twice check clean and run; three sound
ones are refused. A one-shot continuation resumed twice (directly, or through
a lambda) stops at run time (E0425) without running again: one-shot effects
duplicate nothing.

Where the checker (`src/affine.rs`, after inference) fell short: it counted
`Var`s of `let` bindings whose type is `linear`-headed, in one function body,
summing every arm; parameters, pattern binders and containers were not
tracked; a closure's uses counted once at its definition however often it ran;
a clause's once; a call was one use whatever the callee did with its argument;
and E0429 marked what was live at a `multi` perform written in the same
function only, through to the function's end.

## 1. Decisions

**D1. Duplicable types.** A type is *duplicable* unless it is a `linear` type
or holds one: a declared type is duplicable when its arguments and its
constructors' fields are (deep: `Box(File)` is not, nor `type H { H(File) }`);
a tuple when its elements are. Function types are duplicable (D4 says why
that is sound). A type variable is duplicable when what it becomes is: a
variable required duplicable is *marked*, as `==`'s data variables are
(structural equality D1): binding a marked variable checks what it is bound
to (marking the variables in it), and instantiating a scheme that quantifies
a marked variable marks the fresh copy. So a generic function's duplication
of its argument is part of its type, checked where it is instantiated, and
through any number of generic or higher-order calls.

*Considered:* per-function parameter multiplicities (0/1/many) computed by a
fixpoint over the call graph, checked at each call. It misses a function
used as a value (`twice(read, ..)`, a function returned by another): the
callee is not known where the argument is. Marks travel with the types, so
they reach wherever the function goes.

**D2. Multiplicity, counted from the syntax, by arm.** Every local binding (a
function's, a lambda's or a clause's parameter, a `let`, a pattern variable,
a return clause's binder) has a multiplicity, 0, 1 or *many*:
- uses in sequence add; the arms of an `if` or a `match` take the larger;
- a use inside a lambda counts as often as the lambda runs: a lambda bound by
  `let g = fn ..` runs as often as `g` is called, and `g` used other than as
  a callee (passed, stored, returned, aliased) counts as run many times; a
  lambda called where it is written runs once; any other lambda many times;
- a use inside an operation clause counts many (a clause runs once per
  perform); inside the return clause once.

A binding used many times must be duplicable (D1): its type is checked, or
its variables marked, where it is bound.

**D3. Multi-shot points, by row.** A *point* is a call, a `resume`, or a whole
`handle`; its row is what it may perform: a call's callee row, an
operation's effect, `op.perform`'s row, and for a `handle` (and each
`resume` in its clauses) the residual of its body, its return clause's row
and its clauses' rows. A binding bound before a point and used after it in
the same body (a function's, a lambda's, a clause's; a `handle` body's
continues into its return clause) is *held* there. If the point's row has an
effect declared `multi`, a held binding must be duplicable. If the row ends
in a row variable, the requirement is attached to the variable (*held
types*): when the variable takes a `multi` effect, it applies; when it is
bound to a row ending in another variable, it moves there; instantiating a
scheme that quantifies the variable copies it, with the scheme's variables
renamed. So `each(xs, k)` holds `t` across `k(h)` only if `k` performs a
`multi` effect where `each` is called (S10 accepted, U24 refused).

A `handle` delimits: what its body's points hold is what the rest of the
body and its return clause use; what follows the `handle` is held by the
`handle` itself, a point whose row has no handled effect (S6 accepted).

**D4. A function value is duplicable.** A lambda's captures are counted by
its calls where they are known (D2: let-bound and called by name, or called
where it is written) and as used many times anywhere else. So every closure
that reaches a parameter, a container or a function's result captures only
duplicable values, and copying it copies nothing affine. For D3, a use of a
let-bound lambda is a use of everything it captures (through the lambdas it
captures). The cost, recorded: a closure capturing an affine value is called
by name, directly; passing it to a function that calls it once is refused.

**D5. `with any`.** An operation handled by a `with any` handler must take
duplicable arguments: `op.perform(o)` performs it again with them. Checked
where E0434 checks that they are data.

**D6. Errors.**
- **E0428** (kept): a binding used more than once, its type affine; labels
  the use that made it many (the second, or the use in a closure that runs
  more than once, or in a clause).
- **E0429** (kept): a binding held across a multi-shot point; labels its
  first use after the point and the point.
- **E0437** (new): the same through a generic function: a marked variable or
  a held type of a scheme met an affine type where the scheme was
  instantiated; labels the instantiation and the use in the function. Also
  D5's operation.

**D7. Where it runs.** A syntactic pass (`src/affine.rs`, rewritten) runs
before inference: which bindings are used many times (with the use) and
what each point holds (with the first use after). Inference imposes them as
it binds names, infers points and finishes a `handle`. The diagnostics are
kept apart and reported after exhaustiveness, as before, and only on a
program with no type error. The held sets are computed only for a program
declaring a `multi` effect.

**D8. What a `let` generalized holds.** A generalized `let`'s quantified
variables are not checked (D2, D3): its value is syntactic, so no part of it
is of a quantified type (`let n = Nil` used twice duplicates nothing).

## 2. The guarantee

A value of a `linear` type, and any value holding one, is used at most once
on every run: within a function, through any function it is passed to
(generic, higher-order, returned or stored), through closures and handler
clauses, and across the continuation of a `multi` effect, however it is
resumed. One-shot continuations run at most once (E0425 at run time).

Refused although sound (recorded, by choice):
- a `multi` effect handled one-shot still makes its performs multi-shot points;
- an operation clause runs many times (a body that performs once still
  refuses a clause's use of an outer affine value);
- a closure capturing an affine value cannot be passed on (D4), nor aliased;
- a declared type's arguments all count as held (a phantom parameter makes
  `Tag(File)` not duplicable);
- `let _ = x` is a use.

## 3. Limits recorded

- The held sets are copied at every point: a long block holding many
  bindings across many points, in a program with a `multi` effect, is
  quadratic. No program in the suite comes near.
- An E0428 found by a variable bound later in the same function or group
  labels the use, not the binding that made the type affine.

## 4. Predictions

To be written in the plan.
