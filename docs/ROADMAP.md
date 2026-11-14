# Elya roadmap — the problems it is for

Written 2026-10-09. The maintainer's direction: Elya should solve problems that today's
languages have, with real advantages -- high-tech and experimental, but **working**
("doesn't matter if it is done in an unconventional way, it just needs to work"). The
maintainer delegated the choices; this file records them. `HANDOFF.md` sequences the
work; this file says why.

## The thesis

Every interaction with the outside world is a typed effect, and a handler gives it its
meaning. That one mechanism is the lever for each problem below: the program says *what*
it touches, the handler decides *how* -- for real, simulated, sandboxed, recorded or
replayed.

## Priority problems (chosen 2026-10-09)

### 1. Async without function colouring

**The problem.** In JavaScript, Rust, Python and C#, `async` splits the code into two
worlds: an async function can't be called from a sync one without ceremony, every
higher-order library comes in two copies, and the split spreads virally.

**Elya's answer.** Suspension is an effect. A function that awaits performs `Async`; a
scheduler is a handler that keeps the continuations of suspended computations and resumes
them when their I/O is ready. Nothing changes at the call site, `map` works for both, and
the effect row says exactly which functions can suspend. The native backend already
captures continuations as heap frame chains (5b-8 to 5b-10), which is what a scheduler
stores.

**Route.** (a) An `Async`/`Yield` effect and a round-robin scheduler handler written in
Elya, running in the evaluator and natively. It needs a queue of continuations, so
escaped resumes must work natively (they do since the resume-row fix). (b) Real I/O
readiness through a runtime poll loop. (c) Structured concurrency (a scope handler that
joins its children).

### 2. Supply-chain safety: capabilities per dependency

**The problem.** A package from npm or PyPI can read your SSH keys or phone home, and
nothing in the language stops it. Sandboxes live outside the language.

**Elya's answer.** A dependency's public functions have effect rows. Importing it grants
only the effects you list: a JSON parser that names `Net` in its row fails to link into a
program that did not grant `Net`. Handlers can also attenuate (a `Fs` handler that only
sees one directory).

**Route.** Needs modules first (an import with an effect allow-list), then the check at
the import boundary, then attenuating handlers in the standard library.

### 3. Exact replay and handler-based testing

**The problem.** Flaky tests, bugs that appear only in production, and mocking frameworks
or dependency injection bolted on to make code testable.

**Elya's answer.** Because all outside-world interaction is an effect, a recording handler
can log every effect's answer, and a replay handler can feed the same answers back:
the run is reproduced exactly, bit for bit, anywhere. A test needs no mocks; it handles the
effect differently.

**Route.** A `Record`/`Replay` handler pair in Elya over the existing effects; a CLI flag
(`elya run --record trace`, `--replay trace`); then native support (the effect boundary is
already explicit in native code: every perform is a dispatch).

### Long horizon

Provenance and "why did this happen" (counterfactual) debugging through effects; effects
on bare metal; linear ownership as an alternative to GC; content-addressed compilation.

## A reviewer's six directions, weighed (2026-11-14)

The maintainer received a comment proposing six directions and asked that it be
taken into account. Each is set against what Elya has and what it would cost;
the order below is the order of adoption, not the comment's.

**Adopted now, inside the work in hand.**

- *Ownership in a function's contract* (its direction 1: arguments borrowed,
  consumed or reusable). Elya has consumed (`linear`, E0428/E0429/E0437) and
  reusable; it has no *borrowed*. Self-hosting part 2b needs exactly that: an
  array read must not consume the array. So borrowing is designed there, as a
  parameter mode the affine checker understands, rather than as a special case
  of `array.get`. The closures restriction the comment names (a closure
  capturing an affine value is called by name only) is the next step after it:
  a closure type that says it runs at most once.
- *Differential tests, generated* (its foundation step). The house's oldest
  check is that native code writes what the evaluator writes, on hand-written
  programs. A generator of well-typed programs run through both back ends is a
  Codex job (it has the cloud environment to run it).
- *Published measurements* (its step 4). Codex's benchmark suite
  (`scripts/bench.sh`, eight checksum-checked programs, both back ends) is the
  start; compile time and binary size are added to it.

**Next, after the self-hosted front end (parts 4-6).**

- *The native type-polymorphism gap* (README: code polymorphic in a type
  variable is refused natively). HANDOFF step 3 already; a self-hosted compiler
  will need generic containers, so it is pulled forward if part 4 meets it.
- *Reproducible concurrency* (its direction 4). Record and replay exist for the
  host's answers on both back ends, and Elya's tasks are effects handled by a
  scheduler written in Elya, so a schedule is already a value: recording the
  scheduler's choices, exploring others within a bound and shrinking a failing
  one build on what is there. This is the direction closest to an existing
  strength (roadmap problem 3) and the first of the six to get a spec.
- *An optimizer that explains itself* (its direction 3). The one-shot
  continuation reuse and handler removal it lists are measurable against the
  benchmarks; "why was this allocated" reports before checked evidence.

**Later, and only behind explicit contracts.**

- *Refinement types* (direction 2): a decidable core stays; bounds and units
  as opt-in contracts, a solver timeout never a pass. Not before the switch.
- *Tensors, devices, differentiation* (direction 5) and *checked assistance*
  (direction 6): after arrays, borrowing and the polymorphism gap, which they
  all stand on.

**What decides the order.** Until the switch (self-hosting R4), every change
to the language lands twice, in the Rust compiler and in its Elya twin once
that exists. A type-system feature is therefore cheapest either before the
checker is ported (part 4) or after the switch. Borrowing goes in before;
refinement types wait until after.

## A second review: one research identity (2026-11-14)

A second comment the maintainer received proposes one identity rather than six
directions: *continuation-aware ownership and justified allocation
optimisation for effect handlers*, with self-hosting and runtime correctness
first, and GPU, distribution and further type-system features postponed. It
agrees with the order above and sharpens it.

**Taken as it stands.**

- *Its P0, runtime correctness under reclamation.* It names a real gap, the
  one the buffer's control K3 met the same day: `ELYA_GC_STRESS` poisons a
  freed slot and never reuses it, so a missing root whose slot would be
  reused is not caught. Looked at closely the same day: a stale read already
  meets the poison and a stale reachable pointer already stops the mark; what
  went unseen was a stale *write*. Freed blocks are now checked at every
  sweep, which shows what reuse would, with certainty (HANDOFF step 2).
- *Its order:* P0, then self-hosting (arrays, parser, checker, each
  cross-checked against Rust), then the rest. That is the plan.
- *Its measurements:* allocated bytes and continuation capture/resume latency
  beside wall time, and Koka and Effekt as the comparisons for effect-heavy
  programs, C and Rust for the rest. Added to the benchmark suite's list.

**Taken as the research line, after the switch.** Elya already refuses an
affine value held across a multi-shot point (E0429) and copies continuation
frames on a second resume; the line is to use the same facts to *choose*
representations: a one-shot frame that cannot escape kept off the heap, a
handler with a known target called directly, an affine value updated in
place (the buffer is the first case). Its honesty about prior art stands:
Effekt has constant-time one-shot capture and Koka has reuse, so the claim
would be the combination with static ownership, measured, with a core
calculus stated before anything is called proved.

**Not taken yet.** A machine-checked semantics (its P2) and checked
optimisations (P7) wait for a Core that has stopped moving, which is after
the switch. Unforgeable capability values (P5): Elya's capabilities are
effects granted per import, checked statically; a path-confining handler
(`elya/files`' `under`) names a place and does not guard one, as its own
comment says. Enforcement needs abstract types (HANDOFF step 4) first.

## Language decisions taken (2026-10-08/09)

- **Partial handlers are an error** (`E0207`): a handler covers every operation of its
  effect.
- **Sub-effecting for function values**: a function with a smaller row may be used where a
  bigger one is expected (spec to come; equality today).
- **Type annotations are checked**: parameter, return and `let` annotations will be
  enforced (spec to come; the parser discards them today).
- **`resume` carries its handle's remaining effects** (the resume-row fix).
