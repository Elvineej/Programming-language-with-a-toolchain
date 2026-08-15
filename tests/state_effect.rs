//! Slice 4c-1: parameter-passing State. `State` is an ordinary user handler over
//! the effect mechanism (the manifesto's "state is the State effect", no language
//! primitive), interpreted state-passing style — each clause returns a function of
//! the state, `resume` returns a function, and the whole handle is a fn(State) ->
//! Result applied to the initial state. CEK-only + output-verified: no cek==tree
//! oracle stands behind effect evaluation.

use elya::parse::parse_module;
use elya::Session;

/// Type-check (effects checked since 3b), then evaluate on the CEK; return the
/// program output and the peak continuation depth.
fn run_peak(src: &str) -> (String, usize) {
    assert!(
        elya::check_source("t.elya", src).is_ok(),
        "must type-check: {:?}",
        elya::check_source("t.elya", src)
    );
    let (m, d) = parse_module(&Session::new(), src);
    assert!(d.is_empty(), "parse: {d:?}");
    let interp = elya::eval::run_module(&m).unwrap();
    (interp.output().to_string(), interp.peak_kont_depth())
}

#[test]
fn parameter_passing_state_threads_through_resume() {
    // set("a"); x = get(); set(x <> "b"); get()  under a state-passing handler
    // threads "a" -> read into x -> "ab" -> read out. `resume` returns fn(s).
    let src = "effect State {\n\
               fn get() -> String\n\
               fn set(v: String) -> Unit\n\
               }\n\
               fn run() {\n\
               let _ = set(\"a\")\n\
               let x = get()\n\
               let _ = set(x <> \"b\")\n\
               get()\n\
               }\n\
               pub fn main() {\n\
               let program = handle { run() } with {\n\
               State.get() -> fn(s) { (resume(s))(s) }\n\
               State.set(v) -> fn(s) { (resume(Unit))(v) }\n\
               return(x) -> fn(s) { x }\n\
               }\n\
               io.println(program(\"init\"))\n\
               }\n";
    let (out, _peak) = run_peak(src);
    assert_eq!(out, "ab\n");
}

// Pinned from measurement: a State tail-loop peaks at 5 — pure TCE's K_MAX = 3
// plus the effect frame plus the state-passing function-application frame. It MUST
// be constant across N; a peak that grows is a per-operation splice leak (fix the
// machine, never raise the constant). The non-tail grow control below is the teeth.
const K_MAX_STATE: usize = 5;

fn state_tail_loop(n: i64) -> String {
    // Tail-recursive driver: `set` then a tail call to `loop`. `n` performs, then
    // a final `get`; the state-passing handler threads it all.
    format!(
        "effect State {{ fn get() -> String  fn set(v: String) -> Unit }}\n\
         fn loop(n) {{ if n == 0 {{ get() }} else {{ let _ = set(\"x\")  loop(n - 1) }} }}\n\
         pub fn main() {{\n\
           let program = handle {{ loop({n}) }} with {{\n\
             State.get() -> fn(s) {{ (resume(s))(s) }}\n\
             State.set(v) -> fn(s) {{ (resume(Unit))(v) }}\n\
             return(x) -> fn(s) {{ x }}\n\
           }}\n\
           io.println(program(\"init\"))\n\
         }}\n"
    )
}

#[test]
fn state_tail_loop_is_bounded() {
    let (out_small, small) = run_peak(&state_tail_loop(100_000));
    let (out_large, large) = run_peak(&state_tail_loop(1_000_000));
    assert_eq!(out_small, "x\n", "state tail-loop must run to completion");
    assert_eq!(
        out_large, "x\n",
        "state tail-loop must run to completion at large N"
    );
    assert!(
        small <= K_MAX_STATE,
        "state tail-loop peak={small} exceeds K_MAX_STATE={K_MAX_STATE}"
    );
    assert_eq!(
        small, large,
        "state tail-loop peak must be CONSTANT in N (a growing peak is a splice leak): {small} vs {large}"
    );
}

#[test]
fn non_tail_state_loop_grows_with_length() {
    // The recursive `loop` call is under `<>` (non-tail), so each level leaves a
    // frame; peak grows with N. Proves the bounded loop's flatness is a real
    // property of tail position, not an accident of the state-passing machine.
    let prog = |n: i64| {
        format!(
            "effect State {{ fn get() -> String  fn set(v: String) -> Unit }}\n\
             fn loop(n) {{ if n == 0 {{ get() }} else {{ let _ = set(\"x\")  loop(n - 1) <> \"y\" }} }}\n\
             pub fn main() {{\n\
               let program = handle {{ loop({n}) }} with {{\n\
                 State.get() -> fn(s) {{ (resume(s))(s) }}\n\
                 State.set(v) -> fn(s) {{ (resume(Unit))(v) }}\n\
                 return(x) -> fn(s) {{ x }}\n\
               }}\n\
               io.println(program(\"init\"))\n\
             }}\n"
        )
    };
    let (_s, shallow) = run_peak(&prog(5));
    let (_d, deep) = run_peak(&prog(50));
    assert!(
        deep > shallow,
        "non-tail state loop must grow: {shallow} vs {deep}"
    );
    assert!(
        deep >= 45,
        "expected depth ~proportional to n=50, got {deep}"
    );
}
