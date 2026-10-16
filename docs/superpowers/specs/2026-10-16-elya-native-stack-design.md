# Deep non-tail recursion natively: the program's own stack

**Status:** done (2026-10-16). HANDOFF step 1. Design choices are Claude's under
the maintainer's delegation (rule 2), recorded here.

## 0. Measured first

At `ab791b6` (gate 1033 / 99), Linux x86-64, `ulimit -s` 8192 (8 MiB), debug `elya`:

| Program | evaluator (`elya run`) | native (`elya build`) |
|---|---|---|
| m1 `len` (non-tail `1 + len(t)`) over 100 000 | runs | 100000 |
| m2 `len` over 200 000 | runs (3.8 s) | SIGSEGV, rc 139, no message |
| m3 `len` over 1 000 000 | runs (20 s) | SIGSEGV |
| m4 `len` over 10 000 000 | runs (3 min 22 s) | SIGSEGV |
| m5 `foldr` with a lambda over 1 000 000 | runs | SIGSEGV |
| m6 a clause with a non-tail `resume` over 1 000 000 performs | runs | SIGSEGV |
| m7 effectful non-tail recursion (`get() + loop(n - 1)`) over 1 000 000 | runs | 1000000 |

The native depth limit of m1-m4 by bisection: about **130 000** frames, so about 64 bytes
per `len` frame at `OptimizationLevel::None`. Every native program runs on the process's
main thread, whose stack is whatever the OS gives it: 8 MiB here (the `ulimit`), **1 MiB
on Windows** (the linker's default reserve; not measured here: no Windows host -- so about
16 000 frames there by the same frame size, an estimate for the Windows gate to confirm).
m7 survives because effectful code is CPS: its frames are heap objects.

The evaluator's continuation is on the heap, so its depth is bounded only by memory.
The gap is therefore a real divergence: programs the evaluator runs die natively, and die
with an unnamed signal (PARKED.md, "Native stack overflow is an unnamed SIGSEGV").

## 1. Options

- (a) Leave the OS stack and only name the overflow. Keeps a limit of 16 000 frames on
  Windows and a limit that depends on the user's shell (`ulimit -s`) on Linux. Rejected:
  the same binary would run or not depending on how it is started.
- (b) Heap frames for deep recursion (CPS every non-tail call, or segmented stacks with a
  check in every prologue). Matches the evaluator exactly but changes every direct
  function's convention and costs a check or an allocation per call; segmented stacks
  need stack switching LLVM cannot express without inline assembly per platform.
  Rejected for now: a large change for a limit that (c) moves past any practical depth.
- (c) **The program's own stack (chosen).** The runtime reserves one stack for the
  program -- a fixed, documented size, **1 GiB** by default -- with a guard region below
  it, and runs `@elya_main` on a thread that owns that stack. Reservation is address
  space only: pages are committed as the recursion touches them, so a shallow program
  pays nothing. Reaching the guard is a **named** failure:
  `elya: native stack overflow: recursion deeper than the 1024 MiB stack` on stderr, exit
  status 1, like every other runtime trap; output already written is flushed first.

(c) is the stricter of the two that fit: one documented limit on every platform and in
every shell, independent of `ulimit` and of the linker, and never an unnamed signal.

### Decisions inside (c)

- **D1. The size is fixed, not derived from the machine's memory.** Deriving it from
  physical memory would mirror the evaluator's bound, but the same program would then
  overflow on one machine and not on another -- the opposite of exact replay. 1 GiB is
  about 16 million `len` frames, past the depth at which the evaluator itself takes
  minutes (m4). *Corrected by the review (M2):* a reservation is not free everywhere.
  Under `ulimit -v` below about 1 GiB, or Linux strict overcommit
  (`vm.overcommit_memory=2`, where `MAP_NORESERVE` is ignored) on a small machine, every
  program -- hello world included -- now stops with `elya: cannot reserve a stack of
  1024 MiB (ELYA_STACK_MIB sets a smaller one)`, exit 1, where it ran before. Kept: a
  silent fallback would make the limit depend on the machine again (D2), and the
  message names the remedy. Recorded as a decision for the maintainer.
- **D2. `ELYA_STACK_MIB` overrides it** (a whole number of MiB, 1 to 1 048 576). Read by
  the runtime at start, like `ELY_GC_STATS`. Anything else is a named error before the
  program runs (`elya: ELYA_STACK_MIB must be a whole number of MiB from 1 to 1048576`,
  exit 1), never silently the default. If the reservation itself fails, that is named
  too (`elya: cannot reserve a stack of N MiB`), never a fallback to a smaller one (the
  limit stays the one documented).
- **D3. Existing native tests keep the stack they were written against.** Every test
  that ran a binary on the 8 MiB Linux main stack now sets `ELYA_STACK_MIB=8`, so a tail
  call that stops being eliminated still overflows at a million steps, as before (now by
  name). On Windows the tests get 8 MiB instead of 1 MiB; every tail test runs a million
  steps, far past 8 MiB of frames, so none of them loses its power. Only the new deep
  tests use the default.
- **D4. Where it lives.** The runtime gains `elya_start(entry)`; the shim calls it in
  place of calling `main`'s function directly. `entry` is a new `ccc` function
  `@elya_entry` emitted beside the shim, whose only job is the `tailcc` call to `main`'s
  function (C cannot call a `tailcc` function). Elya functions are now mangled
  `elya.<name>` (was `elya_<name>`): with the old prefix a function named `start` became
  the runtime's `elya_start` and the link failed -- as `alloc` and `println` already did
  (review M1, pre-existing for those). No C identifier contains a dot. The shim still prints the value and the GC report on the
  main thread after the program's thread has finished; the collector is untouched
  (`gc_mark` byte-identical): its roots are the shadow stack, never the machine stack,
  and only one thread runs Elya code at a time.
- **D5. The guard.** POSIX: the stack is one `mmap` (`MAP_NORESERVE` where it exists)
  whose lowest 1 MiB is `PROT_NONE`, given to `pthread_attr_setstack`. A `SIGSEGV`
  handler on an alternate signal stack checks the faulting address: inside the guard, it
  `siglongjmp`s back to the thread's entry, which prints the diagnostic and `exit(1)`s
  (flushing stdout); anywhere else, it restores the default action and returns, so a
  genuine fault still kills with signal 11 as today. The guard is 1 MiB, not one page,
  so a large frame cannot step over it. Windows: `CreateThread` with
  `STACK_SIZE_PARAM_IS_A_RESERVATION`, `SetThreadStackGuarantee` so the handler has room,
  and a vectored exception handler for `EXCEPTION_STACK_OVERFLOW` that flushes, prints the
  same line and exits 1. Linking adds `-pthread` on POSIX.
- **D6. Known residue.** An overflow inside `fwrite` (a print at the deepest point) is
  interrupted mid-call; the flush that follows is on the same thread, so it does not
  deadlock, but that one line may be cut short. Recorded, not fixed.

## 2. Evidence

- `native_codegen.rs::DEEP` -- m3, m5, m6 at 1 000 000 with the default stack (red today:
  SIGSEGV); the differential against the evaluator at 200 000 for m2 and m5.
- A named overflow: unbounded non-tail recursion after one `io.println` under
  `ELYA_STACK_MIB=8` prints the line, then the diagnostic, exit 1 (red today: SIGSEGV,
  the line lost or kept by luck, no message).
- The knob: m2 overflows by name under `ELYA_STACK_MIB=8` and runs under the default;
  `ELYA_STACK_MIB=0` and `=lots` are named errors (red today: the variable is ignored).
- Negative controls: (K1) the handler treats every fault as foreign -> the named test
  fails with signal 11; (K2) the shim calls `@elya_main` directly again -> the DEEP rows
  fail with SIGSEGV; (K3) the runtime ignores `ELYA_STACK_MIB` -> the knob test fails.
- Gate before the review: 1037 passed, 99 suites (predicted 1037).

## 3. The independent review

Built the change and `HEAD` side by side: deep recursion at 1 000 000 through `len`,
lambdas, mutual recursion, a non-tail resume, a multi-shot copy of a million frames and
GC churn, all correct; named overflows at 1, 2 and 8 MiB in each shape, also in the
middle of printing (to a file and a pipe) and of allocating; every runtime trap and every
example byte-identical to `HEAD`; D3 weakens no test. Fixed, red first:

- **M1 (medium).** `fn start` no longer linked (a new runtime symbol in the namespace
  `mangle` produced). Fixed by the `elya.` prefix (D4);
  `elya_functions_may_be_named_like_runtime_symbols` names `alloc`, `println`, `start`,
  `entry`, `gc_push` and `current_handler` (red: three multiple definitions).
- **M2 (medium).** Every program reserves 1 GiB: under `ulimit -v 900000` hello world
  fails. Kept as decided, D1 corrected, the message names the remedy.
- **L1 (low).** A SIGSEGV sent by `kill`/`raise` was swallowed once (the handler relied
  on the faulting instruction running again). It is now re-raised.
  `foreign_faults_still_kill_by_their_signal` (a C harness: a null load, `raise(SIGSEGV)`,
  `raise(SIGBUS)`; red: the raised rows exited 0) also pins L2, a genuine fault still
  dying by its signal with nothing on stderr.
- **L3.** The differential deep test now asserts an empty stderr; the test harness's
  overflow diagnosis says what an overflow looks like now.
- Not this slice's (parked): a multi-shot handler with a non-tail resume over deep
  non-tail recursion runs out of memory at 20 000 deep, on `HEAD` too.

Predicted gate after the review: 1037 + 2 = **1039 passed, 99 suites**.
