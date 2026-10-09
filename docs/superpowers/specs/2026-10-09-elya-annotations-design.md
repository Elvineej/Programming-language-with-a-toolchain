# Type annotations are checked

**Status:** done (2026-10-09). HANDOFF step 1. The maintainer decided "check them"
(2026-10-08); the design is Claude's under the delegation (rule 2).

## 0. Measured first

The parser's `skip_type_annotation` ("Slice 1 has no type checker") consumed and DISCARDED
every parameter, return and `let` type. So `let s: String = 1` and
`fn f(x: Int) -> String { x }` checked clean, and a function-type annotation
(`fn(Int) -> Int`) was a parse error (E0100), so higher-order signatures could not be
written at all. Only effect rows (`/ {..}`) were checked.

Parsing the annotations (step 1) changed no test. Checking them (step 2) failed exactly two
test programs in the whole workspace, both corpus bugs: `fn g() -> Str` in two
`crates/codegen/src/cps.rs` unit tests (the type is `String`), fixed. Over the 757 probe
programs of the earlier reviews, the only changes are three programs with wrong `let`
annotations (now E0400) and three that used a function type (a parse error before; one now
checks and runs, two now reach the real error they were written to show).

## 1. Design

- **AST.** `Param.ann`, `FnDecl.ret_ann`, `Stmt::Let.ann`, all `Option<Spanned<TypeAnn>>`;
  `TypeAnn` gains `row` for function types, written `fn(A, B) / {E} -> R` (name `"fn"`,
  args `[A, B, R]`).
- **Meaning.** An annotation is unified with the inferred type, and a mismatch is reported
  at the annotation's span.
- **Type variables are flexible.** A lowercase name is ONE unification variable per
  top-level function, shared by its signature, `let`s and lambdas: "the same type here and
  there", not a rigid "for every type". Rigid variables need skolems and would reject
  nothing the evaluator runs; flexible ones still catch `first(x: a, y: a)` called at Int
  and Bool, and generalization still makes `fn id(x: a) -> a` polymorphic. Rigid
  variables can come later behind explicit `forall` syntax.
- **Rows in function types.** No row written means OPEN (any effects): an annotation that
  does not mention effects does not restrict them, which is what sub-effecting expects.
  A written row is exactly that row (`/ {}` is pure). Unknown type or effect names are
  E0432.
- **Where.** Top-level parameters and results are checked against the group's monotypes
  before any body is inferred, so a call earlier in the group already sees them; lambda
  parameters at the lambda; `let` after its value is inferred and before generalization.
- Nothing downstream changes: Core, the evaluator and native code see the same types,
  only more of them are pinned.

## 2. The independent review

Fixed, each red first (`tests/annotations.rs`, "the independent review"):
- **F1 (high).** An inner `let` generalized the function-wide annotation variable; a later
  annotation bound it, and the final zonk rewrote every node typed with it -- natively a
  captured ADT became an untraced Int and was collected (3395 for the evaluator's 42).
  Annotation variables now count as part of the environment when a `let` generalizes:
  one type for the whole function, as §1 says. (So `let id = fn(y: a) { y }` is
  monomorphic; write no annotation to get a polymorphic local.)
- **F2.** An unwritten row was a meaningful row variable, which stopped sub-effecting
  (a pure result could no longer fit a pure parameter) and made results effect-polymorphic
  natively (D16's refusal). Now polarity decides: in a covariant position (a result, a
  `let`) it is an upcast tail, exactly an unannotated lambda's; in a parameter it is a real
  variable.
- **F4.** `Int(String, Bool)` checked clean and `Unit(Int)` said "unknown type": base types
  given arguments are E0400 "takes no type arguments".
- **F5.** `(Int)` and `()` (accepted by the old skipper) parse again, as Int and Unit; a
  tuple type is a parse error that says tuples are not supported.
- **F6 (part).** A return mismatch read "expected `Int`, found `String`" (the body was the
  expected side); now "expected `String`, found `Int`".
- **F7.** A function type in a type or effect declaration said "unknown type `fn`"; it now
  says function-typed fields and operation parameters are not supported yet.

Parked: effect arguments in written rows (`/ {State(Bool)}`) are still discarded, as they
always were for top-level rows (F3, not unsound: the argument is inferred consistently);
parameter mismatches are reported at the uses, not at the annotation (F6, rest).

## 3. Evidence

`tests/annotations.rs`: sixteen tests; controls C1 (checking disabled: the seven original
rejections pass wrongly), C2 (a fresh variable per occurrence: the shared-variable test
passes wrongly), C3 (the message order reverted: the F6 test fails). Over the review's
probes, no accepted program stops on an unhandled effect and every one that builds natively
prints the evaluator's value. Gate: 783 passed, 77 suites.
