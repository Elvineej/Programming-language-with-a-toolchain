//! A block's length is not a stack bound, natively (2026-10-30, spec
//! `docs/superpowers/specs/2026-10-30-elya-long-blocks-design.md` D1, D4). Each
//! program is compiled on a 512 KiB thread ([`STACK`]), linked and RUN, and its
//! value compared with the evaluator's.
//!
//! Negative controls (each reverted; the file compared byte for byte after), all
//! measured on 512 KiB:
//! - K2, `mono.rs` back to its recursion down a block (as at fa24c18): both
//!   tests overflow in `mono` (`map_vars`, under `map_expr`);
//! - K3, the long `main`'s expected value computed with `+ i + 1`: the test fails
//!   on the evaluator's value (667003, not 738781), so it reads values, not only
//!   whether the build survives;
//! - K4, `reflect::targets_in` recursive again (a small frame: one call per
//!   node through `push_children`): both tests overflow in `targets_in`;
//! - K5, `anf::rename`'s `Let` arm recursive again: the places test overflows in
//!   `rename` (rule 4 is reached: the alias of an open-row parameter); the long
//!   `main`, which has no alias, passes;
//! - K6, `any::map_vars_named`'s `Let` arm recursive again: the places test
//!   overflows in `map_vars_named` (the `with any` clause's block is reached).

use std::process::Command;

fn no_files(_: &str) -> Option<(String, String)> {
    None
}

/// The thread every program here is compiled and evaluated on: 512 KiB. A pass
/// that still recursed once per statement would need more than 26 bytes a frame
/// for the long `main`, 105 for the places (the review's F2: on 8 MiB a
/// small-frame recursion could pass the 3 000-statement places).
const STACK: usize = 512 << 10;

/// The evaluator's value line, and the native binary's, for `src`; both
/// compiled and evaluated on a [`STACK`]-sized thread.
fn both(src: String, tag: &'static str) -> (String, String) {
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(move || {
            let o = elya::run_source_outcome_with("main.elya", &src, &mut no_files);
            assert!(o.error.is_none(), "{tag}: {:?}", o.error);
            let want = format!("{}{}\n", o.output, o.line.expect("a value").expect("data"));
            let (module, diags, _) = elya::front_end_with("main.elya", &src, &mut no_files);
            assert!(diags.is_empty(), "{tag}: {diags:?}");
            let session = elya::Session::new();
            let (diags, table) = elya::types::infer_typed_table(&session, &module);
            assert!(diags.is_empty(), "{tag}: {diags:?}");
            let core = elya::core::lower_module(&module, &table).expect("lowers");
            let dir = std::env::temp_dir().join(format!("elya-long-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("temp dir");
            let obj = dir.join(format!("{tag}.o"));
            let exe = dir.join(format!("{tag}{}", std::env::consts::EXE_SUFFIX));
            elya_codegen::compile_module(&core, &obj).unwrap_or_else(|e| panic!("{tag}: {e}"));
            elya_codegen::link(&obj, &exe).expect("link");
            let out = Command::new(&exe).output().expect("run the binary");
            std::fs::remove_dir_all(&dir).ok();
            assert!(out.status.success(), "{tag}: {out:?}");
            (want, String::from_utf8(out.stdout).expect("UTF-8"))
        })
        .expect("a thread")
        .join()
        .expect("no panic")
}

/// A block of `n` statements binding `{p}1` .. `{p}n`, each from the one before.
struct Block<'a> {
    p: &'a str,
    /// What `{p}0` is bound to.
    from: &'a str,
    n: usize,
    /// Every `ask_every`-th statement performs `ask` instead (0: none).
    ask_every: usize,
    indent: &'a str,
    /// Statements before `{p}0`, on the same spine.
    pre: &'a [&'a str],
    /// A binding of this name halfway down (a rebinding, for `anf`'s rule 4).
    rebind: Option<&'a str>,
    /// Float arithmetic instead of Int.
    float: bool,
    /// The block's last expression, from its last binding's name.
    end: fn(&str) -> String,
}

impl Block<'_> {
    fn new<'a>(p: &'a str, from: &'a str, n: usize) -> Block<'a> {
        Block {
            p,
            from,
            n,
            ask_every: 0,
            indent: "  ",
            pre: &[],
            rebind: None,
            float: false,
            end: |last| last.to_string(),
        }
    }

    fn text(&self) -> String {
        let (p, ind) = (self.p, self.indent);
        let mut s = String::from("{\n");
        for x in self.pre {
            s.push_str(&format!("{ind}  {x}\n"));
        }
        s.push_str(&format!("{ind}  let {p}0 = {}\n", self.from));
        for i in 1..=self.n {
            let prev = format!("{p}{}", i - 1);
            if let Some(r) = self.rebind {
                if i == self.n / 2 {
                    s.push_str(&format!("{ind}  let {r} = {prev}\n"));
                }
            }
            let value = if self.ask_every > 0 && i % self.ask_every == 0 {
                format!("ask({prev})")
            } else if self.float {
                format!("{prev} *. 1.0001 +. 0.5")
            } else {
                format!("({prev} * 3 + {i}) % 1000003")
            };
            s.push_str(&format!("{ind}  let {p}{i} = {value}\n"));
        }
        s.push_str(&format!(
            "{ind}  {}\n{ind}}}",
            (self.end)(&format!("{p}{}", self.n))
        ));
        s
    }
}

#[test]
fn a_long_main_builds_and_runs_on_a_small_thread() {
    let n = 20_000;
    let b = Block {
        indent: "",
        ..Block::new("a", "1", n)
    };
    let src = format!("pub fn main() -> Int {}\n", b.text());
    let mut a: i64 = 1;
    for i in 1..=n as i64 {
        a = (a * 3 + i) % 1_000_003;
    }
    let (want, got) = both(src, "main");
    assert_eq!(want, format!("{a}\n"), "the evaluator");
    assert_eq!(got, want, "native");
}

/// A long block in each place a block can be, and through each pass that
/// treats a `let` specially (the review's F2):
/// - a lambda's body, a match arm, an `if`'s branch;
/// - a function that performs (the CPS emitter's), a handle's body that
///   performs, a clause that resumes, a `return` clause, an effectful lambda,
///   a `with any` clause;
/// - `anf`'s rule 4: an alias of an open-row parameter, the aliased name bound
///   again halfway down (`rename`, `binds`, `fresh_binders`);
/// - a generic local used at two types (`mono`'s clones) and a row-polymorphic
///   local used pure and effectful (`specialize`'s clones), each at the start
///   of the long spine and used at its end;
/// - blocks of other types: a `Float`, and a function of four parameters
///   (`spill`'s argument records: every `let` on the spine has that type).
#[test]
fn long_blocks_build_in_every_place_a_block_can_be() {
    let n = 5_000;
    let b = |p, from| Block::new(p, from, n);
    let parts = [
        "effect Ask { fn ask(n: Int) -> Int }".to_string(),
        "type Opt { No, Yes(Int) }".to_string(),
        format!("fn in_lambda() -> Int {{ let f = fn(x: Int) {}  f(2) }}", b("l", "x").text()),
        format!(
            "fn in_arm(o: Opt) -> Int {{ match o {{ No -> 0  Yes(x) -> {} }} }}",
            b("m", "x").text()
        ),
        format!("fn in_if(c: Bool) -> Int {{ if c {} else {{ 0 }} }}", b("i", "4").text()),
        format!(
            "fn performs(x: Int) / {{Ask}} -> Int {}",
            Block {
                ask_every: 1_000,
                indent: "",
                ..b("p", "x")
            }
            .text()
        ),
        format!(
            "fn in_handle() -> Int {{ handle {} with {{ Ask.ask(n) -> {} }} }}",
            Block {
                ask_every: 1_250,
                ..b("h", "5")
            }
            .text(),
            Block {
                indent: "    ",
                end: |last| format!("resume({last})"),
                ..b("c", "n")
            }
            .text()
        ),
        "fn in_function() -> Int { handle { performs(7) } with { Ask.ask(n) -> resume(n + 1) } }"
            .to_string(),
        format!(
            "fn alias(f) {}",
            Block {
                indent: "",
                pre: &["let g = f"],
                rebind: Some("f"),
                ..b("a", "g()")
            }
            .text()
        ),
        "fn in_alias() -> Int { alias(fn() { 6 }) + handle { alias(fn() { ask(2) }) } with { Ask.ask(n) -> resume(n * 5) } }"
            .to_string(),
        format!(
            "fn generic_local() -> Int {}",
            Block {
                indent: "",
                pre: &["let id = fn(y) { y }"],
                end: |last| format!("if id(True) {{ {last} }} else {{ 0 }}"),
                ..b("g", "id(3)")
            }
            .text()
        ),
        format!(
            "fn row_local() / {{Ask}} -> Int {}",
            Block {
                indent: "",
                pre: &["let ap = fn(h) { h() + 1 }"],
                end: |last| format!("{last} + ap(fn() {{ ask({last}) }})"),
                ..b("r", "ap(fn() { 1 })")
            }
            .text()
        ),
        "fn in_row() -> Int { handle { row_local() } with { Ask.ask(n) -> resume(n + 9) } }"
            .to_string(),
        format!(
            "fn in_any() -> Int {{ handle {{ handle {{ ask(3) }} with any {{ op(o) -> {} }} }} with {{ Ask.ask(n) -> resume(n + 1) }} }}",
            Block {
                pre: &["let v = op.perform(o)"],
                end: |last| format!("io.println(int.to_string({last}))\n  resume(v)"),
                ..b("w", "7")
            }
            .text()
        ),
        format!(
            "fn in_return() -> Int {{ handle {{ ask(2) }} with {{ Ask.ask(n) -> resume(n)  return(r) -> {} }} }}",
            b("z", "r").text()
        ),
        format!(
            "fn in_effectful_lambda() -> Int {{ handle {{\n  let f = fn(x: Int) {}\n  f(3)\n}} with {{ Ask.ask(n) -> resume(n + 2) }} }}",
            Block {
                ask_every: 1_500,
                ..b("e", "x")
            }
            .text()
        ),
        format!(
            "fn floats() -> Float {}",
            Block {
                indent: "",
                float: true,
                ..b("q", "1.5")
            }
            .text()
        ),
        format!(
            "fn quad() -> fn(Int, Int, Int, Int) -> Int {}",
            Block {
                indent: "",
                end: |last| format!("fn(a, b, c, d) {{ a + b + c + d + {last} }}"),
                ..b("u", "1")
            }
            .text()
        ),
        "pub fn main() -> Int {\n  io.println(float.to_string(floats()))\n  (in_lambda() + in_arm(Yes(3)) + in_if(True) + in_handle() + in_function() + in_alias() + generic_local() + in_row() + in_any() + in_return() + in_effectful_lambda() + quad()(1, 2, 3, 4)) % 1000003\n}"
            .to_string(),
    ];
    let (want, got) = both(parts.join("\n") + "\n", "places");
    assert_eq!(got, want, "native");
}
