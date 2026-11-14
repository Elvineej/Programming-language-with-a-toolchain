# Benchmarks

From the repository root, run `sh scripts/bench.sh` (five samples per backend),
`sh scripts/bench.sh --runs 3`, or `sh scripts/bench.sh --compare benches/results/OLD.json`.
The comparison command takes a new measurement, writes its JSON, then compares it
with the saved run. Use an idle machine and the same toolchain when comparing.
The existing Rust, LLVM 18 and clang native-build prerequisites apply; the runner
uses POSIX shell utilities, with no jq, Python or other added dependency.

Each sample must print its pinned `// checksum:` integer and return `Unit`.
The native executable prints the return value as a final `0`; `elya run --value`
prints that same sentinel. Both complete outputs must match the oracle byte for
byte. A wrong checksum or nonzero exit fails the suite; it cannot become a timing
result. Each evaluator sample has a 30-second deadline. A timeout is reported as a
skip, with no successful evaluator result or cross-backend correctness claim.
Compilation is outside the timed region; evaluator times include loading,
parsing and type checking. Both backends execute exactly the same workload.

| Program | Work measured | Expected checksum |
| --- | --- | ---: |
| allocation_churn | 24,000 rounds building/dropping a 256-cell list and a depth-8 tree | 961529344 |
| long_lived_heap | A retained 65,535-node tree across 45,000 garbage rounds, then a full traversal | 4288928266 |
| handlers | 2,000,000 handled/resumed operations and 1,000 standard async scopes with two yielding workers | 146198893 |
| closures | 40,000 captured-function compositions, indirect maps and higher-order folds | 2283500800 |
| recursion | 60,000 descents with 768 live non-tail frames, mixing saved values on unwind | 30002934520 |
| equality | 2,000 rounds comparing three separately allocated 8,192-cell lists, including a final-cell mismatch | 222000 |
| text | 1,000 immutable `<>` builds of 4 KiB, consuming every byte; quadratic Buffer baseline | 818089984000 |
| lexer | 60 lexes of the frozen async source, hashing every output byte with a different seed | 29819953033 |

The checksum formulas are described beside the programs. Recursion sums saved
frame contributions modulo 1,000,003. Lexer hashing uses multiplier 257 modulo
1,000,000,007; its pinned output was independently checked against `src/lex.rs`.
`support/lex.elya` links to the current `selfhost/lex.elya` on main, so lexer
changes are measured along with compiler changes. `fixtures/async.elya.txt` is the
unchanged `src/std/async.elya` from main commit
`d7caea39a59684fbbc405e93a3c41319aa8bf425`. Freezing the input keeps later
standard-library edits from silently changing the lexer workload; the handler
benchmark imports the current `elya/async` implementation.

Results contain all samples, median and minimum wall seconds, and the maximum
peak RSS across samples in KiB. GNU time reports process high-water RSS; BSD time
is supported with platform unit conversion. Without those, Linux `/proc/uptime`
provides 10 ms wall resolution and sampled `VmHWM` gives a lower bound on peak RSS.
With neither, wall time uses the UTC clock at 1 s resolution and RSS is null.
The JSON records the timing/RSS method, commit, dirty state, toolchain, host and
selected runtime settings. Recording/replay environment variables are rejected.

Each schema-version-1 JSON has a `benchmarks` array with `native` and `evaluator`
objects. Completed backends contain `samples`, `median_seconds`, `min_seconds`
and `peak_rss_kib`; skipped evaluators contain their reason and deadline instead.
The timestamp/SHA filename gains a suffix on collision, preserving earlier runs.
Comparison validates the documents and summaries, requires the same benchmark
sources, fixed inputs and checksums, then reports new/old median ratios for both
backends. Compiler, standard-library and linked lexer implementation changes are
comparison targets; they are excluded from the workload fingerprint. Ratios
strictly above 1.10 are marked. A skipped backend has no ratio.
