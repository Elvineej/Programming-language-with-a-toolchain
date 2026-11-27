# Argument records in the back end in Elya

2026-11-27. Status: built. Design choices are Claude's (rule 2).

## 0. Measured first

Windows CI on the landing of self-hosting parts 5 and 6: stage B8 (the
self-hosted sources built by the back end in Elya) failed in `clang`,
"Can't handle guaranteed tail call under win64 yet". The back end in Elya
passed every parameter as itself, and the compiler's own sources have
functions of up to twelve; LLVM 18 cannot guarantee a `musttail` call under
win64 to a function of more than five parameters unless its caller has as
many (the probe matrix in `crates/codegen/src/lib.rs`, `MAX_LAMBDA_PARAMS`).
The corpus comparison passed there only because no corpus program in the
stage has such a function. Linux was unaffected.

## 1. Decision

**D1. The Rust back end's rule, unchanged** (spec
2026-10-16-elya-argument-records, option (b)): a function, lambda or
function type of more than three parameters keeps its first two and carries
the rest in ONE record, a constructor of a synthesized ground type named by
what its fields look like (a function type by its parameters and result,
never its row: `mono.key_of`). Built at the call (an ordinary constructor,
rooted as any other), taken apart by a one-arm `match` at the callee's
entry. Three, not five, so the two back ends shape calls alike and any
later convention (a continuation, a handler frame) still fits; on every
platform, so Linux runs the shape Windows needs.

**D2. Where:** a Core-to-Core pass, `spill` in `selfhost/emit.elya`, after
monomorphization and before emission. It rewrites every node's type, every
parameter, every constructor's fields, every function's and lambda's entry
and every call; a constructor of more than three fields used as a value is
written as its lambda first, so that lambda is spilled too. The records are
those of every function type any node, parameter or field mentions, and of
every function's own parameter list (a function never called is still
written), in order of first appearance.

**D3. Refused by name:** a parametric type whose field is a function of more
than three parameters (a record is a ground type; Rust spills after
specializing, the back end in Elya instantiates parametric types while
emitting). No self-hosted source has one.

## 2. Evidence

Focused programs #60-#66 of `tests/support/selfhost_backend.rs`: a
six-parameter tail loop, a five-parameter lambda through a higher-order
function with strings in the record, a five-field constructor as a value and
a field holding a four-parameter function, an instance of a parametric type
holding four-parameter functions, mutual tail calls of eight parameters, the
refusal (D3), and a four-parameter function never called (which, missed,
broke `selfhost/core.elya`'s build: the first version collected records only
from types the nodes mention). Each runs also collecting at every
allocation. B8 runs on Windows again.
