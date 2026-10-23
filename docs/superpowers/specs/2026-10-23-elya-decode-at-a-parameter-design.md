# Decoding at an effect's own parameter: decided, the refusal stays

**Status:** decided, no code change (2026-10-23, branch `auto/elya`, a scheduled
run). HANDOFF step 1. The choice is Claude's under the maintainer's delegation
(rule 2), recorded here so the question is not reopened without new facts.

## 0. Measured (at 6baf595, gate 1193 / 116)

`effect St(s) { fn get() -> s  fn put(x: s) -> Unit }`, `prog() / {St(Int)}`:

| program | evaluator | native |
|---|---|---|
| a recorder (`op.show`, `op.perform`) over `St(Int)` | `St.put(4)`, `St.get()`, 4 | the same |
| a replayer decoding `"3"` for both operations | `` `3` is not a value of type `Unit`, for an answer of `St.put` `` | the same words |
| a replayer over `get` alone | "the answer type `s` mentions the effect's parameter `s`, which is not known at run time, for an answer of `St.get`" | the same words |

So recording a parametric effect works on both sides; replaying an answer whose
type is the parameter is refused by name on both sides, at run time, at the
`op.decode`. Nothing else is lost.

## 1. The options and the decision

- **A. Decode at the instance, on both sides.** Natively the instance is known (a
  clause copy per label instance after specialization). The evaluator does not
  monomorphize: a perform inside `fn g() / {St(a)} -> a { get() }` has the answer
  type `a` at its site, and the instance exists only in the caller's types, which
  the evaluator never sees. Carrying the instance would need runtime type
  witnesses passed through every effect-polymorphic function, a change to the
  evaluator's calling convention for one builtin.
  - A partial A (the evaluator decodes when the perform SITE's answer type is
    ground in the typed table, refuses otherwise) cannot be matched natively:
    the native expansion is per label instance, not per perform site, so two sites
    of `St(Int)`, one generic in source, share one clause. The two sides would
    disagree on the generic site, and native code must be neither more nor less
    defined than the evaluator (spec 5b-1 §3.4).
- **B. Refuse statically in `elya check`.** At a `with any` handle the handled row
  is usually a row variable (a recorder or replayer is a generic library
  function), and the parametric label appears only at the caller's
  instantiation; the operation may also reach `op.decode` through a helper. A
  check at the handle site therefore catches only the direct, non-generic case,
  and a check at instantiation needs an effect-flow analysis of operation values
  (E0434 has the same limit, HANDOFF's open F4).
- **C. Keep the run-time refusal, by name, on both sides.** It is sound (no
  unchecked data enters at a typed answer), it is identical on both sides, and it
  names the cause.

**Decision: C.** A is unsound to approximate and too large to do properly for its
one use; B would be a partial check that looks complete. The refusal's words stay
as they are.

**Recorded as the way forward if replay of parametric effects is ever needed:**
runtime type witnesses for effect parameters (A), designed together with the
capability work of modules part 2, which also needs to know a module's effect
instances at its import.

No test changes; the gate stays 1193 / 116.
