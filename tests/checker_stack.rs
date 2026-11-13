//! The parser's, the resolver's and the checker's recursive paths are a few
//! small frames per nesting level: deep nests of calls, lambdas, matches and
//! handlers parse and check on a thread of 1.25 MiB in a debug build. After
//! cf3f028 one level of `op(fn() { .. })` took 33 KiB of a debug stack
//! (`infer_call`'s frame was 16 KiB, every kind of call typed in it), and the
//! 60 levels of `infer_complexity` overflowed a test thread's 2 MiB on Windows
//! (spec 2026-11-13-elya-front-end-frames).

const STACK: usize = 1280 * 1024;

/// Parsed and checked on a thread of `STACK` bytes.
fn checks_on_a_small_stack(name: &str, src: String) {
    let name = name.to_string();
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(move || {
            let (module, diags, _) = elya::front_end_with("deep.elya", &src, &mut |_| None);
            assert!(diags.is_empty(), "{name}: {diags:?}");
            let (diags, _) = elya::types::infer_typed_table(&elya::Session::new(), &module);
            assert!(diags.is_empty(), "{name}: {diags:?}");
        })
        .expect("spawn")
        .join()
        .expect("the check ran");
}

const DEPTH: usize = 60;

#[test]
fn nested_performs_in_lambdas_check_on_a_small_stack() {
    let mut body = "1".to_string();
    for _ in 0..DEPTH {
        body = format!("op(fn() {{ {body} }})");
    }
    checks_on_a_small_stack(
        "performs",
        format!("effect E(t) {{ fn op(k: t) -> Int }}\nfn f() {{ {body} }}\npub fn main() -> Int {{ 0 }}\n"),
    );
}

#[test]
fn nested_calls_matches_and_lambdas_check_on_a_small_stack() {
    let mut body = "x".to_string();
    for i in 0..DEPTH {
        body = format!(
            "id(match Some({i}) {{ Some(y) -> (fn(z) {{ if z == y {{ {body} }} else {{ z }} }})(y)  None -> 0 }})"
        );
    }
    checks_on_a_small_stack(
        "calls",
        format!(
            "type Option(a) {{ None, Some(a) }}\nfn id(v: Int) -> Int {{ v }}\nfn f(x: Int) -> Int {{ {body} }}\npub fn main() -> Int {{ f(1) }}\n"
        ),
    );
}

#[test]
fn nested_handlers_and_resumes_check_on_a_small_stack() {
    let mut body = "ask()".to_string();
    for _ in 0..DEPTH {
        body = format!(
            "handle {{ ask() + {body} }} with {{ Ask.ask() -> resume(1)  return(v) -> v }}"
        );
    }
    checks_on_a_small_stack(
        "handlers",
        format!(
            "effect Ask {{ fn ask() -> Int }}\nfn f() {{ {body} }}\npub fn main() -> Int {{ 0 }}\n"
        ),
    );
}
