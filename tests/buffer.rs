//! Self-hosting, part 2a (2026-11-14, spec
//! `docs/superpowers/specs/2026-11-14-elya-buffer-design.md`): a linear
//! `Buffer` builds text in linear time, in place, because the affine guarantee
//! says no one else holds it.
//!
//! Negative controls: see the plan,
//! `docs/superpowers/plans/2026-11-14-elya-buffer.md`.

#[allow(dead_code)] // each suite uses some of the programs
#[path = "programs/buffer.rs"]
mod programs;
use programs::*;

fn run(src: &str) -> elya::RunOutcome {
    elya::run_source_outcome_with("main.elya", src, &mut |_| None)
}

fn errors(src: &str) -> String {
    match elya::check_source_with("main.elya", src, &mut |_| None) {
        Ok(()) => String::new(),
        Err(e) => e,
    }
}

#[test]
fn a_buffer_builds_the_text_pushed() {
    let o = run(BUILDS);
    assert!(o.error.is_none(), "{:?}", o.error);
    assert_eq!(o.output, builds_out());
    assert_eq!(o.line, Some(Ok("130".to_string())));
}

#[test]
fn a_buffer_is_used_once_and_is_not_data() {
    for (src, code, words) in [
        (
            "pub fn main() -> String {\n  let b = buffer.push(buffer.new(), \"a\")\n  let c = buffer.push(b, \"b\")\n  buffer.finish(b) <> buffer.finish(c)\n}\n",
            "E0428",
            "used more than once",
        ),
        (
            "pub fn main() -> String {\n  let b = buffer.new()\n  let f = fn(s: String) { buffer.finish(buffer.push(b, s)) }\n  f(\"x\") <> f(\"y\")\n}\n",
            "E0428",
            "used more than once",
        ),
        (
            "effect multi Flip { fn flip() -> Bool }\n\
             pub fn main() {\n  let b = buffer.new()\n  io.println(handle {\n    let _ = flip()\n    buffer.finish(b)\n  } with multi { Flip.flip() -> resume(True) })\n}\n",
            "E0429",
            "",
        ),
        (
            "pub fn main() -> Bool { buffer.new() == buffer.new() }\n",
            "E0436",
            "this compares buffers",
        ),
        (
            "type Holder { Holder(Buffer) }\npub fn main() -> Bool { Holder(buffer.new()) == Holder(buffer.new()) }\n",
            "E0436",
            "a `Buffer`",
        ),
        (
            "pub fn main() -> Buffer(Int) { buffer.new() }\n",
            "E0400",
            "takes no type arguments",
        ),
        (
            "pub fn main() -> String { buffer.finish(\"text\") }\n",
            "E0",
            "Buffer",
        ),
    ] {
        let e = errors(src);
        assert!(e.contains(code), "{src}: wanted {code} in {e}");
        assert!(e.contains(words), "{src}: wanted `{words}` in {e}");
    }
    // A buffer threaded through, each binding used once, checks.
    assert_eq!(errors(&appends(3)), "");
}

#[test]
fn building_text_with_a_buffer_is_linear_in_the_evaluator() {
    // Eight times the appends: linear is eight times the time, a push that
    // copies the text (control K2) about thirty (spec D0, D5); the bound
    // leaves room for noise either way.
    let time = |n| {
        let src = appends(n);
        let t = std::time::Instant::now();
        let o = run(&src);
        assert_eq!(o.line, Some(Ok((10 * n).to_string())), "{:?}", o.error);
        t.elapsed().as_secs_f64()
    };
    let small = time(25_000).max(1e-3);
    let large = time(200_000);
    assert!(
        large / small < 14.0,
        "200,000 appends took {large:.3} s, 25,000 took {small:.3} s"
    );
}
