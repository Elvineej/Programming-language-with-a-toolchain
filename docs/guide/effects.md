# Effects and handlers

An effect declares a set of operations that code can request while it runs. A
handler provides the behavior for those requests. The same calculation can ask
for a name, a clock reading, or a choice, and the caller can decide how to answer
each request. A test can supply fixed answers through a handler while another
caller uses a different policy. The function that asks the question stays the
same.

Run the examples below from the repository root with
`cargo run -p elya-cli -- run <file>`, substituting the linked file's path. The
output blocks show what Elya prints, with terminal colors omitted from the
diagnostics.

## Declare, perform, and handle

`effect Ask` declares the effect, and `fn ask(prompt: String) -> String` declares
its operation. An operation signature specifies what information the caller
supplies and what value the operation produces when resumed. Calling
`ask("name")` **performs** the operation. It looks like an ordinary call, but it
passes control to a handler instead of entering a function named `ask`. The
`/ {Ask}` in `greet`'s signature exposes that request to callers.

`handle greet() with { ... }` evaluates `greet()` under the given clauses.
`Ask.ask(prompt)` selects the operation and binds its argument. The clause
calls `resume` with a string, which becomes the value of the original `ask`
call. Elya continues evaluating the concatenation in `greet`. When the body
finishes, `return(text)` receives its value and adds the exclamation mark.
That transformed value comes back from `resume`, and the clause returns it as
the value of the whole `handle` expression.

A named handler covers every operation of one effect, with one clause per
operation and the declared number of arguments. You can omit the effect name
in a clause when using the operation's bare name. You can also omit the
`return` clause to pass the body's normal result through unchanged.

[01_ask.elya](effects/01_ask.elya)

```elya
// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)

effect Ask { fn ask(prompt: String) -> String }

fn greet() / {Ask} -> String {
  "Hello, " <> ask("name")
}

pub fn main() {
  let message = handle greet() with {
    Ask.ask(prompt) -> resume(if prompt == "name" { "Ada" } else { "friend" })
    return(text) -> text <> "!"
  }
  io.println(message)
}
```

```text
Hello, Ada!
```

## Stop on failure

A clause can return directly. The rest of the computation after the operation,
called the **continuation**, is then discarded. In this example, failing while
checking the name skips the welcome message and the handler's `return` clause.
The value of the failure clause becomes the result of the handle.

`Never` is an empty type declared by this program. Since it has no constructors,
there is no normal value to pass to `resume` for `Fail.fail`. An empty `match`
on that result lets `checked_name` use the failing expression where the other
branch returns a string. The `return` clause still describes what to do on normal
completion, so a nonempty name would get a welcome message and an exclamation
mark.

[02_failure.elya](effects/02_failure.elya)

```elya
// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)

type Never { }
effect Fail { fn fail(reason: String) -> Never }

fn checked_name(name: String) / {Fail} -> String {
  if name == "" { match fail("missing name") { } } else { name }
}

pub fn main() {
  let message = handle {
    "Welcome, " <> checked_name("")
  } with {
    Fail.fail(reason) -> "Rejected: " <> reason
    return(text) -> text <> "!"
  }
  io.println(message)
}
```

```text
Rejected: missing name
```

## Resume later with a generator

A clause can put `resume` inside a closure and return that closure as part of a
value. Here, `start` runs until the first `emit` and returns `More` with the number
and a function for continuing. The caller chooses when to call `next()`.
That call supplies `Unit` as the result of the paused `emit` and runs until the
next emission or normal completion. Resuming reinstalls the handler, so the
next emission produces another `More`. The return clause produces `Done` when
the generator finishes.

Each `More` holds its own one-shot continuation. A consumer can advance once
from that step or stop by dropping the closure. The consumer owns printing through its call to `io.println`.

[03_generator.elya](effects/03_generator.elya)

```elya
// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)

effect Emit { fn emit(n: Int) -> Unit }
type Step { Done, More(Int, fn() -> Step) }

fn numbers() / {Emit} -> Unit {
  let _ = emit(10)
  emit(20)
}

fn start() -> Step {
  handle numbers() with {
    Emit.emit(n) -> More(n, fn() { resume(Unit) })
    return(_) -> Done
  }
}

fn print_steps(step: Step) / {IO} -> Unit {
  match step {
    Done -> io.println("done")
    More(n, next) -> {
      io.println(int.to_string(n))
      print_steps(next())
    }
  }
}

pub fn main() { print_steps(start()) }
```

```text
10
20
done
```

## Pass state through a handler

A handler can also return a function that accepts the current state. Here
every clause returns a function of type `fn(Int) -> Int`. The result of the
handle is such a function, and `run(40)` supplies its initial state.

The `get` clause uses the state twice for different purposes. In
`resume(state)(state)`, the first argument is the answer to `get()`. The
second calls the function returned by the resumed computation, carrying the
same state forward. The `put` clause answers its operation with `Unit` and
passes the new state to the returned function. At normal completion the
return clause wraps the computation's final result in a function that ignores
the state. Here that result came from the last `get()`.

[04_state.elya](effects/04_state.elya)

```elya
// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)

effect State {
  fn get() -> Int
  fn put(value: Int) -> Unit
}

fn increment_twice() / {State} -> Int {
  let _ = put(get() + 1)
  let _ = put(get() + 1)
  get()
}

pub fn main() {
  let run = handle increment_twice() with {
    State.get() -> fn(state: Int) { resume(state)(state) }
    State.put(value) -> fn(state: Int) { resume(Unit)(value) }
    return(result) -> fn(state: Int) { result }
  }
  io.println(int.to_string(run(40)))
}
```

```text
42
```

## Nest handlers

Elya looks for the innermost active handler that matches an operation. An
operation can pass through handlers for other effects on the way to a match.
Here `title()` reaches the outer handler. Its resume reinstates the inner
handler, which then answers `name()`. Both handlers use the implicit identity
return clause.

You can nest two handlers for the same effect too. The inner one answers
requests made in its body first. The operation clause itself runs outside
the handler it belongs to. This lets a clause deliberately perform the
operation again to ask an outer handler for an answer.

[05_nested.elya](effects/05_nested.elya)

```elya
// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)

effect Name { fn name() -> String }
effect Title { fn title() -> String }

pub fn main() {
  let message = handle {
    handle { title() <> " " <> name() } with {
      Name.name() -> resume("Ada")
    }
  } with {
    Title.title() -> resume("Dr.")
  }
  io.println(message)
}
```

```text
Dr. Ada
```

## Explore alternatives with `with multi`

To run a continuation more than once, declare the effect with `effect multi`
and use `with multi` at the handler. Each `resume` restarts the same captured
part of the computation with its own answer. The example explores the two
boolean outcomes of `flip()`. Each branch goes through the return clause
before the operation clause joins the two strings.

Work that happened before the operation has already happened once. Work after
the operation can happen once per resume, including observable I/O. The
checker warns about this with E0426. Keep printing after the handle, as in
this example, when you want one final line of output. Multi-shot continuations
also have restrictions on capturing affine values, since a replay could use a
value intended for a single use more than once.

[06_multi.elya](effects/06_multi.elya)

```elya
// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)

effect multi Flip { fn flip() -> Bool }

fn choose() / {Flip} -> String {
  if flip() { "heads" } else { "tails" }
}

pub fn main() {
  let outcomes = handle choose() with multi {
    Flip.flip() -> resume(True) <> " / " <> resume(False)
    return(side) -> "[" <> side <> "]"
  }
  io.println(outcomes)
}
```

```text
[heads] / [tails]
```

## Trace different effects with `with any`

A `with any` handler has a single `op(o)` operation clause. It can intercept
supported user operations from different effects without listing them by name.
The operation value `o` keeps track of the expected answer type.
`op.perform(o)` forwards the request outward and returns an answer of that
type. `op.show(o)` formats the request, and `op.encode(o, answer)` formats the
answer. An operation-generic handler can also use `op.decode` to turn a saved
answer back into a value of the operation's result type.

In this trace the outer handlers still supply the answers. The tracing clause
prints each question and answer, then resumes with the forwarded value.
Generic interception is for operations with data arguments and results, and
the host effect `IO` is outside its scope.

[07_any.elya](effects/07_any.elya)

```elya
// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)

effect Dice { fn roll(sides: Int) -> Int }
effect Clock { fn now() -> Int }

fn trace(body) {
  handle body() with any {
    op(o) -> {
      let answer = op.perform(o)
      io.println(op.show(o) <> " = " <> op.encode(o, answer))
      resume(answer)
    }
  }
}

pub fn main() {
  let total = handle {
    handle trace(fn() { roll(6) + now() }) with {
      Dice.roll(sides) -> resume(sides)
    }
  } with {
    Clock.now() -> resume(10)
  }
  io.println(int.to_string(total))
}
```

```text
Dice.roll(6) = 6
Clock.now() = 10
16
```

## Read effect rows and work with `IO`

An effect row sits after `/` and before the return arrow. A closed row like
`{Ask}` names the permitted effect. `{}` is an explicit empty row. Elya can
infer a function's effects when its signature leaves them unspecified. A call
to an effectful function brings its effects into the caller's row unless a
handler discharges them.

Elya infers a lowercase row variable such as `e` separately for each call.
In `call`, the callback and the enclosing function share `e` because the
function performs whatever the callback performs. `print_result` adds `IO`
with `{IO | e}`. Its first call has an empty callback row, and the second
has `Ask` in that row. Handling `Ask` leaves `main` with just `IO`.

`io.println` performs the built-in `IO` effect, as do operations such as
`io.read_file`. The host supplies `IO` for `main`. Application effects need
handlers before they reach the entry point.

[08_rows.elya](effects/08_rows.elya)

```elya
// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)

effect Ask { fn ask() -> String }

fn call(f: fn() / {e} -> String) / {e} -> String { f() }

fn print_result(f: fn() / {e} -> String) / {IO | e} -> Unit {
  io.println(call(f))
}

pub fn main() / {IO} -> Unit {
  print_result(fn() { "pure callback" })
  handle print_result(fn() { ask() }) with {
    Ask.ask() -> resume("effectful callback")
  }
}
```

```text
pure callback
effectful callback
```

## Read the diagnostics

An effect that reaches `main` without a handler produces E0420 during
checking. The diagnostic names the effect and points to the expression that
performs it. Start at that site and find the caller that can supply the
answer. Add the handler around the computation at that call site.

[09_unhandled.elya](effects/09_unhandled.elya)

```elya
// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)

effect Ask { fn ask() -> String }

pub fn main() {
  io.println(ask())
}
```

```text stderr
[E0420] Error: effect `Ask` is never handled
   ╭─[docs/guide/effects/09_unhandled.elya:7:14]
   │
 7 │   io.println(ask())
   │              ──┬──  
   │                ╰──── `Ask` is performed here
   │ 
   │ Help: `main` may perform only {IO}; handle it with `handle … with { Ask.op(..) -> … }`
───╯
```

E0425 occurs during evaluation when a one-shot continuation is resumed a
second time. The first resume in this example completes normally. The error
points at the second call. Remove the second resume if the computation should
run once. If exploring alternatives is intentional, use a multi-shot effect
and handler together. Using `with multi` on an ordinary effect produces E0427.

[10_one_shot.elya](effects/10_one_shot.elya)

```elya
// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)

effect Pick { fn pick() -> Int }

pub fn main() {
  let _ = handle pick() with {
    Pick.pick() -> {
      let _ = resume(1)
      resume(2)
    }
  }
  io.println("unreached")
}
```

```text stderr
[E0425] Error: a one-shot continuation was resumed twice
    ╭─[docs/guide/effects/10_one_shot.elya:10:7]
    │
 10 │       resume(2)
    │       ────┬────  
    │           ╰────── this handler is one-shot: only a `with multi` handler of an `effect multi` resumes more than once
────╯
```

Other useful codes are E0207 for a missing operation clause, E0421 for a
purity violation, and E0423 for incompatible effect rows. For a missing clause,
compare the handler with the effect declaration. For a row error, read the
effects on both sides of the reported call or annotation and check where
each effect is meant to be handled.
