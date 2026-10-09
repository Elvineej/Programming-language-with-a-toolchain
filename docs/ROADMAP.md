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

## Language decisions taken (2026-10-08/09)

- **Partial handlers are an error** (`E0207`): a handler covers every operation of its
  effect.
- **Sub-effecting for function values**: a function with a smaller row may be used where a
  bigger one is expected (spec to come; equality today).
- **Type annotations are checked**: parameter, return and `let` annotations will be
  enforced (spec to come; the parser discards them today).
- **`resume` carries its handle's remaining effects** (the resume-row fix).
