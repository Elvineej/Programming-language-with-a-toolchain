# Elya — Language & Toolchain Design Specification

- **Codename:** Elya
- **Status:** Design approved (brainstorming complete); ready for implementation planning
- **Date:** 2026-08-05
- **Scope of this document:** the full language design (identity, syntax, type system, effect system, semantics) plus the compiler architecture and testing strategy for the **first implementation cycle — Vertical Slices 1–3** (front end + tree-walking/CEK interpreter). The broader roadmap (native codegen, runtime/GC, standard library, toolchain, self-hosting) is recorded here as the north-star arc but each later slice gets its own spec → plan → build cycle.

---

## 0. How to read this document

Elya is a large, multi-year ambition. To avoid the classic "boil the ocean" failure, this spec **fully specifies the language** but **scopes implementation to Slices 1–3**. Everything downstream is intentionally left as roadmap (§13) so that the first implementation plan is bounded and always keeps a running language.

The single sentence every later ambiguity appeals to is the **manifesto** (§1). When in doubt: prefer purity, prefer inference, prefer expressing a capability as an effect rather than a keyword.

---

## 1. Manifesto & Identity

> **Elya is a statically-typed, natively-compiled language where side effects are part of a function's type and are handled by ordinary code.** It has ML's data modeling (algebraic types, exhaustive pattern matching, global inference), Gleam's calm readability, and Rust's ambition for a real toolchain — but its one radical bet is *effects as first-class, inferred, handler-resolved values*. Purity is the default; anything that touches the world announces itself in its type; and `try`/`throw`, `async`/`await`, generators, and dependency injection are not language features but **library handlers** over one uniform mechanism. Elya refuses ambient authority, refuses exceptions, and refuses to make you write types the compiler can infer.

**Elya refuses:**
- **Ambient authority** — a function that performs I/O says so in its type (`/ {IO}`).
- **Exceptions / stack unwinding** — recoverable errors are `Result(T, E)` values; unrecoverable control flow is an `Exn` *effect*, handled explicitly.
- **Ambient mutable loops** — no `while`/`for`/`let mut` in the core; iteration is recursion with **guaranteed tail-call elimination**, and mutable state is the `State` effect.
- **Boilerplate the compiler can infer** — global Hindley–Milner inference with row-polymorphic effect inference; most code carries no type annotations.

---

## 2. The Four Design Axes (decided) + the Signature Feature

These are the "on paper" decisions the master roadmap requires before any lexer code. They are **decided**, not open.

| Axis | Decision (v1) | Rationale |
|---|---|---|
| **Type system** | Static, global inference via **Hindley–Milner (Algorithm J)**. Algebraic data types (sum + product), parametric generics, **traits/typeclasses** for ad-hoc polymorphism. **No subtyping.** Deferred: higher-kinded types, dependent types, GADTs. | Inference-first, few annotations; no-subtyping keeps inference tractable and errors legible. |
| **Memory model** | **Tracing garbage collector.** v1 interpreter uses the host (Rust) heap + `Rc`/arena; native slices add stop-the-world mark-sweep → generational. **Not** ownership/borrowing in v1. | Simplest for the user; the borrow checker is a year-long sink and is explicitly *not* the point of this language. |
| **Error handling** | `Result(T, E)` and `Option(T)` with a `?` propagation operator for recoverable errors; unrecoverable control flow is the `Exn` **effect**. **No exceptions.** | Honest APIs, trivial to type-check, no unwinding machinery. |
| **Concurrency** | **Designed now, built later.** Target model: `async`/generators/structured concurrency expressed as **effects + handlers** (an `Async` effect handled by an event-loop). No concurrency runtime in Slices 1–3. | The effect machinery already gives us the shape; committing the model now avoids codegen rework. |

**Signature (experimental) feature — carried from day one:** **Algebraic effects & handlers** with **row-polymorphic effect inference**. Side effects are declared like interfaces, tracked in function types as an inferred *effect row*, and interpreted by user-defined handlers with access to the delimited continuation (`resume`). This one mechanism subsumes exceptions, state, generators, async, and dependency injection. It lands in **Slice 3**.

---

## 3. Concrete Syntax (ML/Gleam-flavored, brace-delimited)

### 3.1 Aesthetic decisions (locked)

- **Brace-delimited** (not indentation-sensitive) — far simpler to lex/parse; Gleam-like.
- **Generics look like Gleam:** type application uses parentheses and **lowercase type variables** — `Tree(a)`, `List(Int)`, `Map(k, v)`. Parens dodge the `<>`/comparison parse ambiguity and are coherent with the flavor. (`[a]` was considered and rejected as inconsistent.)
- **Purity + recursion + guaranteed TCE.** No `while`/`for`/`let mut` in the core. Iteration is recursion (with the TCE *guarantee* of §7.3) plus higher-order iterators (`map`/`fold`/`filter`). Mutable state is the `State` effect.
- **Numeric operators are monomorphic per type (Gleam route), not trait-dispatched.** Int: `+ - * / %`; Float: `+. -. *. /.`. Structural equality `==`/`!=` is built-in for all types. Ordering: `< <= > >=` on Int, `<. <=. >. >=.` on Float. This keeps HM inference free of numeric-literal defaulting and keeps error messages crisp. (Trait-based numeric overloading is deferred; `Show`, custom `Eq`, etc. still use traits.)
- **Effect row in a function type** is written after the parameter list: `fn f(args) / {E1, E2} -> Ret`. A pure function omits the row (empty `<>`, inferred).
- **String concatenation** is `<>`. **Pipeline** is `|>`. **Error propagation** is `?`.
- **Lists:** `[]`, `[x, ..xs]` (cons), literals `[1, 2, 3]`.

### 3.2 Canonical snippets

```elya
import elya/io

pub fn main() / {IO} {
  io.println("Hello, Elya!")
}
```

```elya
pub type Tree(a) {
  Leaf
  Node(left: Tree(a), value: a, right: Tree(a))
}

pub fn size(t: Tree(a)) -> Int {
  case t {
    Leaf -> 0
    Node(l, _, r) -> 1 + size(l) + size(r)
  }
}
```

```elya
pub trait Show(a) {
  fn show(x: a) -> String
}

pub fn describe(xs: List(a)) -> String {   // pure: no effect row
  "list of " <> int.show(list.length(xs))
}
```

---

## 4. Formal Grammar (EBNF)

This is the reference grammar. Expression precedence is handled by a Pratt parser (§10.4); the EBNF below gives structure, not precedence.

```ebnf
program        = { import } , { decl } ;

import         = "import" , module_path , [ "as" , ident ] ;
module_path    = ident , { "/" , ident } ;

decl           = [ "pub" ] , ( fn_decl | type_decl | effect_decl
                             | trait_decl | impl_decl | const_decl ) ;

fn_decl        = "fn" , ident , "(" , [ params ] , ")" ,
                 [ "/" , effect_row ] , [ "->" , type ] , block ;
params         = param , { "," , param } ;
param          = ident , [ ":" , type ] ;
(* Type variables are implicit and ML-style: any lowercase name appearing in an
   annotation (e.g. `a` in `List(a)`) is universally quantified. Functions carry
   no explicit generic parameter list. *)

type_decl      = "type" , type_ctor , "{" , { variant } , "}" ;
type_ctor      = uident , [ "(" , type_var , { "," , type_var } , ")" ] ;
variant        = uident , [ "(" , ctor_fields , ")" ] ;
ctor_fields    = ctor_field , { "," , ctor_field } ;
ctor_field     = [ ident , ":" ] , type ;

effect_decl    = "effect" , type_ctor , "{" , { op_sig } , "}" ;
op_sig         = "fn" , ident , "(" , [ params ] , ")" , "->" , type ;

trait_decl     = "trait" , uident , "(" , type_var , ")" , "{" , { op_sig } , "}" ;
impl_decl      = "impl" , uident , "(" , type , ")" , "{" , { fn_decl } , "}" ;

const_decl     = "const" , ident , [ ":" , type ] , "=" , expr ;

block          = "{" , { stmt } , [ expr ] , "}" ;
stmt           = let_stmt | expr ;
let_stmt       = "let" , pattern , [ ":" , type ] , "=" , expr ;

expr           = literal | ident | qualified | call
               | field_access | lambda | if_expr | case_expr
               | block | handle_expr | list_expr | tuple_expr | paren_expr
               | unary_expr | binary_expr | postfix_expr ;

qualified      = ident , "." , ident ;                  (* module.member; module-vs-field
                                                           resolved during name resolution *)
call           = expr , "(" , [ args ] , ")" ;
args           = expr , { "," , expr } ;
field_access   = expr , "." , ident ;
lambda         = "fn" , "(" , [ params ] , ")" , block ;

(* Operator precedence and associativity are resolved by a Pratt
   (precedence-climbing) parser, not encoded in this grammar. *)
unary_expr     = ( "-" | "!" ) , expr ;
binary_expr    = expr , binop , expr ;
postfix_expr   = expr , "?" ;                           (* Result/Option propagation *)
binop          = "+"  | "-"  | "*"  | "/"  | "%"
               | "+." | "-." | "*." | "/."
               | "==" | "!=" | "<"  | "<=" | ">"  | ">="
               | "<." | "<=." | ">." | ">=."
               | "<>" | "|>" | "&&" | "||" ;

list_expr      = "[" , [ expr , { "," , expr } , [ "," , ".." , expr ] ] , "]" ;
tuple_expr     = "(" , expr , "," , expr , { "," , expr } , ")" ;
paren_expr     = "(" , expr , ")" ;

if_expr        = "if" , expr , block , "else" , block ;
case_expr      = "case" , expr , "{" , { case_arm } , "}" ;
case_arm       = pattern , [ "if" , expr ] , "->" , expr ;

handle_expr    = "handle" , expr , "with" , [ "multi" ] ,
                 "{" , { handler_arm } , "}" ;
handler_arm    = ( qualified , "(" , [ params ] , ")" , "->" , expr )   (* Op clause *)
               | ( "return" , "(" , ident , ")" , "->" , expr ) ;       (* return clause *)

effect_row     = "{" , [ effect_ref , { "," , effect_ref } ] , "}" ;
effect_ref     = uident , [ "(" , type , { "," , type } , ")" ] ;

type           = type_var | named_type | fn_type | tuple_type ;
type_var       = lident ;                               (* lowercase => variable *)
named_type     = uident , [ "(" , type , { "," , type } , ")" ] ;
fn_type        = "fn" , "(" , [ type , { "," , type } ] , ")" ,
                 [ "/" , effect_row ] , "->" , type ;
tuple_type     = "(" , type , { "," , type } , ")" ;

pattern        = "_" | literal | lident | ctor_pattern | tuple_pattern | list_pattern ;
ctor_pattern   = uident , [ "(" , [ pattern , { "," , pattern } ] , ")" ] ;
list_pattern   = "[" , [ pattern , { "," , pattern } , [ "," , ".." , lident ] ] , "]" ;

literal        = int | float | string | "True" | "False" | "Unit" ;
```

Lexical notes: `uident` starts uppercase (types, constructors, effects, traits); `lident`/`ident` start lowercase (values, type variables, fields). Comments: `//` line, `/* */` block (nestable). Numeric literals allow `_` separators and `0x`/`0b`/`0o` prefixes. Strings support escapes and `<>`-based concatenation (no interpolation in v1).

---

## 5. The Ten Example Programs (syntax spec + first golden tests)

These are simultaneously the syntax specification, the first end-to-end regression tests, and the reality check on the type/effect system. All are in final Elya syntax.

**01 — Hello world**
```elya
import elya/io

pub fn main() / {IO} {
  io.println("Hello, Elya!")
}
```

**02 — Recursion & guaranteed TCE (factorial + tail-recursive fib)**
```elya
pub fn factorial(n) {
  case n {
    0 -> 1
    _ -> n * factorial(n - 1)
  }
}

pub fn fib(n) {
  fn go(i, a, b) {
    case i {
      0 -> a
      _ -> go(i - 1, b, a + b)     // tail call — must run in bounded stack
    }
  }
  go(n, 0, 1)
}
```

**03 — Record type + functions**
```elya
pub type Point { Point(x: Float, y: Float) }

pub fn origin() -> Point { Point(0.0, 0.0) }

pub fn distance(a: Point, b: Point) -> Float {
  let dx = a.x -. b.x
  let dy = a.y -. b.y
  math.sqrt(dx *. dx +. dy *. dy)
}
```

**04 — Enum + exhaustive match**
```elya
pub type Shape {
  Circle(radius: Float)
  Rect(width: Float, height: Float)
}

pub fn area(s: Shape) -> Float {
  case s {
    Circle(r) -> 3.14159 *. r *. r
    Rect(w, h) -> w *. h
  }
}
```

**05 — Generic function + generic data structure**
```elya
pub type Stack(a) { Stack(items: List(a)) }

pub fn push(s: Stack(a), x: a) -> Stack(a) {
  Stack([x, ..s.items])
}

pub fn pop(s: Stack(a)) -> Option((a, Stack(a))) {
  case s.items {
    [] -> None
    [top, ..rest] -> Some((top, Stack(rest)))
  }
}
```

**06 — Trait definition + two implementations**
```elya
pub trait Show(a) {
  fn show(x: a) -> String
}

pub type Color { Red  Green  Blue }

impl Show(Color) {
  fn show(c) {
    case c { Red -> "red"  Green -> "green"  Blue -> "blue" }
  }
}

impl Show(Bool) {
  fn show(b) { case b { True -> "true"  False -> "false" } }
}
```

**07 — Result-based error handling with `?`**
```elya
import elya/int

pub type ParseError { NotANumber(input: String) }

pub fn add_strings(a: String, b: String) -> Result(Int, ParseError) {
  let x = int.parse(a)?
  let y = int.parse(b)?
  Ok(x + y)
}
```

**08 — Closures / higher-order functions**
```elya
import elya/list

pub fn sum_of_squares(xs: List(Int)) -> Int {
  xs
  |> list.map(fn(x) { x * x })
  |> list.fold(0, fn(acc, x) { acc + x })
}
```

**09 — A module importing another module**
```elya
// geometry/circle.elya
pub fn area(r: Float) -> Float { 3.14159 *. r *. r }

// main.elya
import elya/io
import geometry/circle

pub fn main() / {IO} {
  io.println(float.show(circle.area(2.0)))
}
```

**10 — End-to-end word count (stdlib + I/O effect + `?`)**
```elya
import elya/io
import elya/string
import elya/list
import elya/map
import elya/int

pub fn main() / {IO} -> Result(Unit, io.Error) {
  let text = io.read_file("poem.txt")?
  let counts =
    text
    |> string.split(" ")
    |> list.fold(map.new(), fn(m, w) {
         map.update(m, w, 0, fn(n) { n + 1 })
       })
  map.each(counts, fn(word, n) {
    io.println(word <> ": " <> int.show(n))
  })
  Ok(Unit)
}
```

---

## 6. Type System

### 6.1 Inference

- **Algorithm J** (union-find substitution over type variables — the practical HM variant). Each expression is assigned a fresh type variable; the AST walk generates unification constraints; `let`-generalization quantifies free type **and effect-row** variables; each use instantiates fresh variables.
- **No subtyping.** Ad-hoc polymorphism is via traits, not subtyping. This is what keeps inference and error messages tractable, and it is preserved by the effect system (see §8.2).
- **Traits/typeclasses** are resolved to explicit **dictionary values** during lowering to Core IR (§10.6). Interpreter-friendly; a later native slice may switch to monomorphization.
- **Exhaustiveness** of `case` is checked by **Maranget's algorithm** over Core; non-exhaustive matches and useless arms are diagnostics.

### 6.2 Representation

```rust
enum Type {
    Var(TyVar),
    Con(SymbolId, Vec<Type>),          // e.g. List(Int), Tree(a)
    Fn(Vec<Type>, EffectRow, Box<Type>),
    Tuple(Vec<Type>),
}
struct EffectRow { labels: BTreeMap<EffectLabel, Provenance>, tail: Option<RowVar> }
```

Effect labels carry **provenance** (the operation call-site that introduced the effect and the boundary it flowed through). This is load-bearing for diagnostics (§9).

---

## 7. Semantics — the CEK Machine

### 7.1 Machine

Evaluation is a **CEK abstract machine**: state `⟨ control , env , k ⟩` where

- `control` is the expression/Core node under evaluation,
- `env` is a **persistent, immutable** map from variables to values,
- `k` is the continuation: a **persistent, immutable linked list of frames** (`AppK`, `CaseK`, `LetK`, `HandleK{handler}`, `OpK`, …).

Capturing a continuation is just holding a pointer into the persistent `k` list. This immutability is what makes handlers and multi-shot `resume` sound (§8.4) and TCE trivial (§7.3).

**Sequencing:** Slice 1 ships a dead-simple **recursive tree-walker** (fastest path to hello-world). Slice 2 **refactors to the CEK machine** (load-bearing work, signed up for). Slice 3 adds effects/handlers on top of the CEK machine. The tree-walker is retained as a test oracle (§11).

### 7.2 Values

Runtime values: integers, floats, bool, `Unit`, strings, tuples, lists, constructors (tagged), closures (code + captured `env`), trait dictionaries, and **`Resume(k_cap, env_at_capture)`** continuation values (§8.4).

### 7.3 Tail-Call Elimination — a tested correctness guarantee

TCE is a **compiler promise with regression tests**, not an optimization that "usually happens." All iteration rests on it.

1. A **tail-position analysis** precisely marks tail calls: the last expression of a block; both arms of a tail `case`; the body under a handler's `return` clause; and **`resume` in tail position** (see §8.5). The precise definition is part of the spec and is unit-tested.
2. The **CEK step for a marked tail call replaces `control` without pushing a frame** — provably O(1) continuation growth. Tail `resume` **splices `k_cap` into the current slot** rather than pushing.
3. **Regression suite** (§11.4) asserts a **K-depth ceiling** on deep tail-recursive programs (equality to a pinned small constant where composed), paired with non-tail controls asserted to *grow* — so a broken analysis fails loudly.
4. When native codegen arrives, the same programs assert LLVM `musttail`. For Slices 1–3 the bounded-K-depth test *is* the guarantee.

---

## 8. The Effect System (Elya's soul)

### 8.1 Declarations, rows, handlers

```elya
effect Log { fn log(msg: String) -> Unit }

effect State(s) {
  fn get() -> s
  fn put(v: s) -> Unit
}
```

A function's type is `(args) / <row> -> ret`. Pure functions have the empty row `<>` and need no annotation. `throw`/`try` is an `Exn` effect whose handler never calls `resume`; `async` is an `Async` effect handled by an event-loop; a generator is a `Yield` effect. **One mechanism, many features.**

```elya
handle prog() with {
  State.get()  -> resume(0)
  State.put(n) -> resume(Unit)
  return(x)    -> x
}
```

### 8.2 Inference flavor — row-polymorphic (keeps "no subtyping")

Function types carry an **effect-row variable** unified alongside ordinary type variables — a conservative extension of Algorithm J. Effects are **idempotent simple rows** (each label appears at most once — simpler than record rows). Row unification matches common labels and unifies residual tails via a fresh row variable.

Crucially, **row polymorphism provides effect composition without subtyping**, preserving the §2 "no subtyping" decision. Sub-effecting via subset constraints was considered and **rejected** — it is a form of subtyping that would contaminate the entire inferencer.

### 8.3 One-shot by default; `multi` opt-in

Handlers are **one-shot by default**: each `resume` may be called at most once. To resume more than once (nondeterminism, backtracking), a handler must be written `handle … with multi { … }`. This default is **enforced**, not assumed (§9.5, §11.5), and it is the enforcement point for the cleanup design axis (§8.6).

### 8.4 Handler capture & multi-shot resume (representation proof)

At an operation call `perform Op(v)`, the machine walks `k` to the nearest `HandleK{H}` handling `Op` and splits `k`:

- `k_cap` = frames *above* the handler (from the perform point up to it),
- `k_rest` = the handler frame and below.

The clause runs with `resume` bound to a first-class value `Resume(k_cap, env_at_capture)`. Calling `resume(w)` reconstructs `⟨ w , env_at_capture , k_cap ++ k_now ⟩`.

**Multi-shot is free because `k_cap` and `env` are persistent immutable data:** invoking `Resume(k_cap)` twice re-enters the *same* captured continuation independently, each producing its own run, because nothing was mutated or popped. One-shot is not a different representation — it is the same `Resume` value called ≤ once (and dropped afterward). No redesign separates them.

**Worked multi-shot example — nondeterminism with state and cleanup across a second resume:**
```elya
effect Choice { fn choose(xs: List(a)) -> a }

pub fn all(body) {
  handle body() with multi {
    Choice.choose(xs) -> list.flat_map(xs, fn(x) { resume(x) })
    return(v)         -> [v]
  }
}

pub fn search() / {Choice, IO} -> Int {
  ensure(fn() { io.println("cleanup") }, fn() {   // cleanup frame lives INSIDE k_cap
    let picked = choose([10, 20])                 // capture k_cap here; env0 = env before `picked`
    picked * 2
  })
}
// all(search) => prints "cleanup" twice, returns [20, 40]
```

Trace: capture at `choose` yields `k_cap = [ensure-cleanup · bind picked · picked*2 · return]` over `env0`.
- `resume(10)`: binds `picked=10` in a fresh extension of `env0` → `20` → ensure fires ("cleanup") → `return(20)` → `[20]`.
- `resume(20)`: re-enters the **same** `k_cap` → binds `picked=20` in **another** fresh extension of the **same** `env0` → `40` → ensure fires again → `[40]`.
- `flat_map` → `[20, 40]`.

**State does not leak** (branch 2 extends `env0`, never `env0{picked:10}`), and **cleanup is per-branch** (the ensure frame in `k_cap` re-runs each resume). Both are consequences of immutable capture.

### 8.5 `resume` in tail position

When a handler clause ends in `resume(x)` in tail position, the machine **splices `k_cap` into the current slot without pushing** — this is what makes the "tail-through-resume" TCE case (§11.4) run in bounded stack. It is a specific CEK rule, committed now, not later.

### 8.6 Cleanup as an explicit design axis (per-branch vs fire-once)

Cleanup timing is a **semantic choice made by frame placement**, not an implementation accident:

- Cleanup fires **per-branch** iff its frame lies *inside* the nearest multi-shot handler's captured region (`k_cap`).
- Cleanup fires **once** iff it lies *below* the handler (`k_rest`).

**Double-free hazard:** a resource acquired **once outside** the handler but released by an `ensure` placed **inside** a `multi` handler runs its release per branch — a double-free:
```elya
let conn = db.open()                     // acquired once, outside
handle
  ensure(fn(){ db.close(conn) }, fn(){   // WRONG: inside the multi handler
    let row = choose(rows)               // multi-shot
    process(conn, row)
  })
with multi { Choice.choose(xs) -> list.flat_map(xs, resume) ; return(v) -> [v] }
```

**Rule:** a resource's release must sit at the same scope *relative to the handler* as its acquisition. Acquired outside ⇒ release outside (wrap the whole `handle`; lands in `k_rest`; fires once). Acquired inside the body ⇒ `ensure` inside (per-branch). Corrected:
```elya
let conn = db.open()
ensure(fn(){ db.close(conn) }, fn(){     // fire-once: wraps the handle, in k_rest
  handle
    let row = choose(rows)
    process(conn, row)
  with multi { ... }
})
```

**Selection & enforcement (no linear types in v1):** `ensure(cleanup, body)` = per-branch; wrapping the `handle` = fire-once. A **lint (E0426)** flags a resource acquired outside a `with multi` handler and released inside it, catching the double-free class at compile time. The full static guarantee awaits linear/affine types (deferred; §13, §14).

---

## 9. Diagnostics (error quality is a tested feature)

Rendering uses `ariadne`/`codespan`-style multi-span output. The governing discipline: **the common failure — "you forgot to handle an effect" — is a purpose-built diagnostic, never fallout from row unification.** Three rules make raw tail-variable dumps impossible:

- **Rows carry provenance** (§6.2): every label records its operation call-site and the boundary it crossed.
- **Discharge is a separate pass from unification:** "is this effect handled?" is checked at `handle` nodes and declared function boundaries, producing its own diagnostic class — it never reaches the unifier.
- **Zonk before report:** on any failure the row is solved to *named labels + an opaque "other effects"*; a tail variable is **never** printed as `%r7`.

### 9.1 E0420 — unhandled effect
```
error[E0420]: effect `Log` is never handled
  ┌─ app.elya:8:3
4 │   log("hi " <> name)
  │   ─── `Log` is performed here (inside `greet`)
8 │   greet("ada")
  │   ^^^^^^^^^^^^ calling `greet` performs `Log`, which escapes here
  │
  = help: handle it:  handle greet("ada") with { Log.log(m) -> ... }
  = help: or widen the row:  fn main() / {IO, Log}
```

### 9.2 E0421 — purity violation (closed empty row)
```
error[E0421]: this function must be pure here, but it performs `IO`
  ┌─ app.elya:8:28
2 │ pub fn shout(s) / {IO} -> String { io.println(s) ... }
  │                   ──── `shout` performs `IO`
8 │ let loud = list.map(names, shout)
  │            ────────────────^^^^^─ `list.map` requires `fn(a) -> b` with no effects
  │
  = help: use the effectful sibling:  list.for_each(names, shout)   // -> Unit / {IO}
```

### 9.3 E0423 — effect-row mismatch (both the simple and the mid-unification cases)

**Simple case (declared vs actual):**
```
error[E0423]: effect row mismatch: `sync` performs more than it declares
  ┌─ app.elya:6:14
5 │ pub fn sync(url) / {Log} -> Unit {
  │                    ───── declared to perform only {Log}
6 │   let body = fetch(url)
  │              ^^^^^^^^^^ this performs `Net`, not in the declared row
  │
  = the rows differ by exactly:  {Net}
  = help: add it → fn sync(url) / {Log, Net}   ·   or handle Net here
```

**Mid-unification case (shared-label payload conflict, type vars still open).** In a simple-row system two *fully open* rows never fail (they resolve by extension); the tail-variable gibberish in Koka/Links actually comes from **payload-type conflicts on a shared label** or a **closed tail** discovered late. Handled thus:
```
error[E0423]: conflicting uses of effect `State`
  ┌─ app.elya:9:6
6 │ fn use_int(x) / {State(Int)} ...
  │                  ─────────── requires `State(Int)`
7 │ fn use_str(x) / {State(String)} ...
  │                  ────────────── requires `State(String)`
9 │ both(fn(){ use_int(0) }, fn(){ use_str("hi") })
  │      ─────────────────    ─────────────────── `both` runs both under one `State` row
  │
  = effect `State` is used at two types:  State(Int)  vs  State(String)
  = help: they cannot share one handler; give `both` two effect params, or split
```
Renderer rule: **zonk both payloads**, print the effect name once with its two types; an unsolved payload renders through provenance as a source-anchored name ("the element type of `xs`"), never `%s1`.

### 9.4 E0424 — cyclic effect row (occurs-check)
Reported as the offending label + its origin ("effect row would be infinite"), never as a raw substitution.

### 9.5 E0425 / E0426 — one-shot & cleanup enforcement
- **E0425** — statically detectable double-resume in a non-`multi` handler (e.g., `resume` inside `flat_map`/a loop): rejected at compile time.
- **E0426** — a resource acquired outside a `with multi` handler and released inside it (double-free lint, §8.6).

**UI-test invariant:** every effect diagnostic fixture asserts the message contains the effect name(s) and (for E0423) the set/type difference, and asserts the message contains **no** `%r`/`%e`/`%s` token.

---

## 10. Compiler Architecture (Slices 1–3)

### 10.1 Minimum crate layout (not the maximal one)

**One crate, `elya` (lib + bin); modules are the pass boundaries.** The pure-function pass discipline (§10.3) is a *signature* rule, not a crate-boundary rule, so it survives collapse into one crate intact. Split into a workspace later only if compile times force it — kept mechanical by the layering test (§10.7, §11.6).

```
elya/
  Cargo.toml                # single crate: lib + bin
  src/
    main.rs                 # CLI: `elya run` / `elya check`
    lib.rs                  # Session (interners, source map), pipeline wiring
    span.rs   diag.rs       # spans + ariadne diagnostics (foundation)
    lex.rs    ast.rs  parse.rs
    resolve.rs  types.rs  core.rs  eval.rs
  tests/
    ui/                     # //~ ERROR fixtures  — LIVE from Slice 1
    tce/                    # bounded-K-depth      — harness Slice 1, cases Slice 2+
    arch/                   # module-layering DAG test — from Slice 1
    examples/               # the 10 programs, golden output
```

Dependencies kept minimal: `logos` (lexer), `ariadne` (diagnostics), `insta` (snapshots); dev-only `proptest`, `cargo-fuzz`.

### 10.2 Pipeline / data flow

```
source ──lex──▶ tokens ──parse──▶ AST ──resolve──▶ (AST + SymbolTable)
   ──infer──▶ typed AST (types + effect rows) ──lower──▶ Core IR ──eval──▶ value / output
```

### 10.3 Pure-pass discipline (salsa-ready insurance)

No globals, no `thread_local`. All shared state (string interner, symbol interner, source map) lives in a `Session` passed by reference. Each pass is `fn(&Session, In) -> (Out, Vec<Diagnostic>)`. That signature is exactly what a query engine (salsa) wraps later for the LSP — incrementality without a rewrite.

### 10.4 Front end

- **Lexer:** `logos`-derived; every token carries a **span** (byte offset + line/col). Errors are diagnostics, never panics.
- **Parser:** **recursive descent** for declarations/statements + **Pratt (precedence-climbing)** for expressions. **Error recovery** by synchronizing at `pub`/`fn`/`type`/`effect`/`}` so one file reports many errors.
- **AST:** enums/structs, every node spanned; arena/`Box` for cache-friendliness.

### 10.5 Semantic analysis

- **Name resolution:** symbol tables + scopes; `pub`/private visibility; imports; forward references. Unresolved names get an `error` `SymbolId` sentinel so inference still runs.
- **Type + effect inference:** §6, §8. An `error` type/row **poisons silently** to suppress cascade errors.
- **Exhaustiveness:** Maranget over Core.

### 10.6 Core IR & desugaring

Lower the typed AST to a small typed **Core**: `case` → decision trees; pipelines `|>` → calls; `?` → `Result` match; method calls → dictionary/def references; traits → explicit dictionary values. The interpreter evaluates Core directly.

### 10.7 Module-layering enforcement (compiler-internal)

The compiler's own Rust modules are assigned a layer: `span/diag → lex → ast → parse → resolve → types → core → eval → cli`. A test (§11.6) fails CI on any **back-edge** (e.g., `types` referencing `eval`), keeping a future workspace split mechanical. This is distinct from **Elya program** modules, which **may** be mutually recursive within a package (needed by the cross-module TCE case, §11.4); Elya **packages** become an enforced DAG when the package-manager slice lands (§13).

---

## 11. Testing Strategy (from commit #1)

### 11.1 Snapshot tests (`insta`)
Per pass: tokens, pretty-AST, resolved symbols, inferred types + effect rows, Core IR, eval output.

### 11.2 The ten examples as golden end-to-end tests
`elya run examples/NN.elya` output is snapshotted. They double as the syntax spec.

### 11.3 Oracle discipline (wired now)
Eval output is the reference. When the CEK machine lands (Slice 2) it must match the tree-walker on every program (cross-check test). Later VM/native slices must match the CEK machine.

### 11.4 TCE regression suite (bounded K-depth, with grow/bounded controls)
Each case asserts a **K-depth ceiling** and pairs with a non-tail control asserted to *grow*:

- **Deep self-tail-recursion:** `count_down(10_000_000)` at bounded K-depth.
- **Mutual recursion across Elya modules:** `parity/even.is_even` ↔ `parity/odd.is_odd`, `is_even(10_000_000)` bounded — proves tail calls survive symbol/module resolution.
- **Tail-through-resume:** a handler whose clause ends in tail `resume`, driving a `State`-threaded countdown 10M steps, bounded.
- **Tail calls in handlers:** a handler clause body ending in an ordinary tail call is O(1).
- **Composed case (tight ceiling):** a tail call **into** a handler that **tail-resumes into** a function that **tail-calls back across a module boundary**, 10M steps, asserting `assert_eq!(peak_k_depth, K_MAX)` where `K_MAX` is a single-digit constant pinned at first measurement. A per-iteration off-by-one drives depth toward 10M (fails); a constant off-by-one makes it `K_MAX+1` (fails the equality). This composition is where splice-into-current-slot meets symbol resolution.

### 11.5 One-shot enforcement (negative tests)
- **E0425 static:** `resume` inside `flat_map`/a loop in a non-`multi` handler is rejected at compile time — asserted by a UI test.
- **Runtime:** a one-shot `Resume` is single-use; a second call that escaped static detection raises a **structured diagnostic** ("continuation resumed twice; handler is one-shot — use `with multi`") with both spans — asserted by a runtime test.

### 11.6 Module-layering test (`tests/arch/layering.rs`)
Scans `src/*.rs` for `use crate::…` edges and fails on any back-edge against the pinned layer order (§10.7).

### 11.7 Diagnostic UI tests (`//~ ERROR[Ennnn]`)
A corpus of bad programs asserting error code + message substring + span. The §9 diagnostics are the first fixtures. Effect diagnostics (E0420/E0421/E0423/E0424/E0425/E0426) join when effects land in Slice 3; parse/resolve/type diagnostics are live from Slice 1. **No test infrastructure is deferred — only fixtures that require unbuilt features.**

### 11.8 Fuzzing & property tests
- `cargo-fuzz` on lexer + parser: must never panic; always produce diagnostics.
- `proptest`: parse∘pretty round-trip (up to spans); `unify` soundness (unify then apply substitution ⇒ equal).

### 11.9 CI (every commit)
`cargo test`, `cargo fmt --check`, `cargo clippy -D warnings`, the UI suite, the layering test, and a fuzz smoke run.

---

## 12. Build Order — Slices 1–3 (this cycle)

Thin vertical slices; always keep a running language.

- **Slice 1 — hello-world end to end (recursive tree-walker).** Minimal spanned lexer; recursive-descent + Pratt parser with recovery for a handful of constructs; minimal name resolution; direct AST evaluation. Test infrastructure stood up: `insta`, `tests/ui`, `tests/arch/layering.rs`, TCE harness scaffold, CI. *You now have a language.*
- **Slice 2 — grow the front end + CEK refactor.** Arithmetic, variables, functions, `if`, `case`, real name resolution, first HM type checker. **Refactor the evaluator into the CEK machine.** First TCE assertions come online.
- **Slice 3 — data, polymorphism, and effects.** Records, enums, pattern matching, generics, traits (dictionary-passing), exhaustiveness. **Algebraic effects & handlers** with row-polymorphic inference, one-shot default + `multi`, `resume`, the §9 effect diagnostics, and the full TCE suite (§11.4) green. All ten examples run.

**Exit criterion for this cycle:** `elya run examples/*.elya` produces the golden output for all ten programs; the CEK machine matches the tree-walker oracle; the TCE suite (including the composed tight-ceiling case) is green; the §9 diagnostics have passing UI fixtures; one-shot enforcement negative tests pass; the layering test passes.

---

## 13. Roadmap Beyond This Cycle (north-star; each its own spec)

- **Slice 4 — bytecode VM** (register-based), retarget front end at it; keep CEK as oracle.
- **Slice 5 — native codegen** via **Cranelift (debug backend)** + **LLVM (release backend)**; mark-sweep → generational **GC**; stack maps/safepoints; FFI (C ABI); `musttail` TCE tests.
- **Slice 6 — standard library** written in Elya (dogfooding).
- **Slice 7 — toolchain:** package manager + build system (enforced **package DAG**), then LSP (salsa-backed incremental front end), formatter, linter, DWARF debug info, test runner, docs generator, REPL, tree-sitter grammar, WASM playground.
- **Slice 8 — self-hosting:** rewrite the compiler in Elya; bootstrap to a stage-2 == stage-3 bit-identical fixed point.

**Deferred language features (future research branches):** linear/affine types (for static resource safety, §8.6), ownership/borrowing, higher-kinded types, dependent/refinement types, concurrency runtime for the `Async` effect.

**Cost profile of adding linear/affine types later — additive, not invasive.** Two viable paths exist and neither redesigns Algorithm J: **(a)** a *fully additive* usage/multiplicity checker run as a separate pass over the typed **Core IR** (every binder and use site is already explicit and CFG-shaped there), touching the unifier not at all; or **(b)** a *bounded extension* in the Linear-Haskell style — a `Multiplicity` component on the `Fn` arrow that unifies alongside types, structurally the **same kind of extension already validated for effect rows** (new field on `Fn`, new variable kind, generalize/instantiate), not a new algorithm. The v1 inferencer keeps this additive on purpose by preserving two properties (both already true): the arrow type stays extensible (it already carries the effect row), and Core keeps all binders and uses explicit. **The one genuine future cost, recorded so it is not a surprise:** the *interaction rules* between linearity and multi-shot handlers — a linear resource must be forbidden from capture into a continuation that may resume more than once — require dedicated design (studied in the Effekt / Frank / linear-handlers literature). That is design effort on the *rules*, not a change to the core inference algorithm, and doing it after the effect system is concrete is an advantage rather than a liability.

### Future major-version direction — AI / HPC compute (roadmap only; does not alter v1)

**Status:** north-star for a future major version. **This does not change Slices 1–3 or the implementation bound**, and it does not touch the manifesto (§1) or the four axes (§2). The manifesto remains Elya's single soul — effects-first, purity by default, one radical bet. The features below are a direction the *existing* bet can grow toward, not a second identity competing with the first. **If any item here ever conflicts with the manifesto, the manifesto wins and the conflict is flagged, not silently reconciled.**

Candidate feature set (all deferred): an Intelligent Hardware Scheduler; automatic multithreading; SIMD / auto-vectorization; tensors and GPU/NPU execution; Python interop; an HTTP framework; database drivers; async networking.

**The decisive question — does this fit the effect mechanism, or fight it?** The honest answer is a clean split.

**Fits as a flagship *extension* of effects — the compute surface is handlers over a `Compute` / `Tensor` effect.** Running a kernel on a GPU or NPU is, semantically, an effect: an operation is *performed* (`Tensor.matmul(a, b)`, `Compute.run(kernel)`) and a *handler* decides where and how it executes, then resumes with the result.
- The **hardware scheduler is a handler** (or a stack of them) for the `Compute` effect: it reads device availability and a cost model, honors `@prefer(GPU)` / `@prefer(NPU)` as hints carried on the operation, chooses a backend, dispatches, and resumes. `@prefer(...)` is handler selection, not a separate subsystem.
- It **layers on effects already planned**: device dispatch is asynchronous, so `Compute` sits on the `Async` effect; device buffers are linear resources, so it uses the linear/affine discipline deferred above and the one-shot handler default (§8.3) — a multi-shot handler over device memory would double-free a buffer, exactly the §8.6 hazard, so `Compute` handlers are one-shot.
- To be viable it must be a **coarse-grained, graph-capturing effect**, not an eager per-scalar one: performing a continuation per numeric op would be ruinous. The `Tensor` handler accumulates a compute graph and dispatches it fused (effects-as-staging), rather than suspending on every element. This is a real design commitment, but an *extension* of the mechanism, not a departure from it.

So the scheduler, device placement, `@prefer`, and tensor/kernel operations are a **natural flagship extension of the effects bet** — the same shape as every other Elya effect, and they *strengthen* the manifesto rather than compete with it.

**Fits as libraries *over* effects (no core changes):** async networking, the HTTP framework, and database drivers are ordinary libraries written against the `Async` / `IO` / capability effects. They *use* the mechanism; they add no new core.

**Genuinely separate subsystems (honest future cost — these are *not* handlers):**
- **Automatic parallelization** — auto-multithreading and auto-SIMD/vectorization of *ordinary* code. This is implicit whole-program optimization in the middle-end/codegen, not something the user *performs*, so it cannot be a handler; it is a separate compiler subsystem. It does not threaten purity-by-default because it is semantics-preserving (like any optimizer), so it does not compete with the manifesto — but it is a bolt-on, and its cost is counted as one.
- **GPU/NPU code-generation backends** (SPIR-V / PTX / LLVM-GPU targets) and **device-memory management** — real codegen/runtime subsystems the `Compute` handlers dispatch *into*. The effect gives a clean front-end; the backend is separate engineering.
- **The Python interop bridge** — an FFI + runtime-embedding subsystem. Individual calls *into* Python could be tracked as a `PyFFI` effect (for capability/auditing), but the interop machinery itself is a subsystem, not a handler.

**Summary:** the compute *interface* (scheduler-as-handler, `@prefer`, tensor ops) is a flagship *extension of the effect model* that reinforces the bet; the compute *implementation* (auto-parallelization, GPU/NPU backends, device memory, Python bridge) is separate subsystem work whose cost is acknowledged honestly here. None of it is in scope before the language self-hosts (Slice 8) and the effect system, `Async`, and linear resources are all real.

---

## 14. Risks & Mitigations

- **Boiling the ocean** → vertical slices; this cycle is bounded to Slices 1–3.
- **CEK refactor underestimated** → sequenced deliberately (tree-walker first in Slice 1; CEK in Slice 2 *before* effects in Slice 3); acknowledged as load-bearing.
- **Effect-system error debt** → diagnostics designed in (§9), tested as fixtures, with the no-`%row` invariant.
- **Multi-shot turning into a redesign** → representation proven against the persistent K-stack (§8.4); one-shot and multi-shot share the representation.
- **Resource double-free under multi-shot** → cleanup design axis + E0426 lint now; full guarantee via linear types later (documented).
- **TCE regressions hiding** → equality-to-pinned-constant on the composed case, plus grow/bounded controls.
- **Compiler tangle blocking workspace split** → enforced module-layering DAG test from Slice 1.
- **Momentum loss** → every slice is demoable; there is always something that runs.

---

## 15. Milestone Checklist (Slices 1–3)

- [ ] Spec, EBNF grammar, and 10 example programs (this document).
- [ ] Four design axes + signature feature decided (this document).
- [ ] `Session` + pure-pass pipeline scaffold; CI; `insta`; `tests/ui`; `tests/arch/layering.rs`; TCE harness.
- [ ] Spanned lexer; recursive-descent + Pratt parser with error recovery → AST.
- [ ] Name resolution + symbol tables.
- [ ] HM (Algorithm J) inference + row-polymorphic effect inference + trait resolution + Maranget exhaustiveness.
- [ ] Typed Core IR with desugaring (dictionary-passing traits).
- [ ] **Slice 1:** recursive tree-walker runs hello-world end to end.
- [ ] **Slice 2:** CEK machine matches the tree-walker oracle; first TCE assertions.
- [ ] **Slice 3:** effects & handlers (row-poly inference, one-shot default + `multi`, `resume`, tail-resume); §9 diagnostics with UI fixtures; full TCE suite incl. composed tight-ceiling case; one-shot negative tests; all ten examples run.

---

*Codename Elya. Build thin slices, keep it running, test error messages and TCE as guarantees, and don't start with the borrow checker.*
