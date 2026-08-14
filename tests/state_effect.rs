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
