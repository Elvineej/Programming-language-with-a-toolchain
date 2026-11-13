//! Values of more than 64 words. A descriptor row's pointer mask was one word:
//! a constructor's 65th field, a closure's 64th capture or the 63rd value live
//! across a call shifted a bit out of range, here (a panic in a debug build of
//! the compiler) and in the collector (`mask >> field`, undefined in C; on
//! x86-64 both wrapped and agreed, an `Int` in the wrapped place read as a
//! pointer). Found by ChatGPT's review; wide rows now carry their mask words
//! after the table (`push_row`, `gc_field_is_heap`).
//!
//! Each program holds lists only through the wide value while 300 rounds of
//! garbage are made, runs under `ELYA_GC_STRESS=1` (a collection at every
//! allocation; a freed value read stops the program) and plainly, and writes
//! what the evaluator computes.

use std::path::PathBuf;
use std::process::Command;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("elya-wide-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Compiled on a large stack, as the CLI compiles (a sum of 200 terms nests
/// 200 deep).
fn native(tag: &str, src: &str) -> [(i32, String, String); 2] {
    let (tag, src) = (tag.to_string(), src.to_string());
    std::thread::Builder::new()
        .stack_size(512 << 20)
        .spawn(move || native_on_this_stack(&tag, &src))
        .expect("spawn")
        .join()
        .expect("compiled and ran")
}

fn native_on_this_stack(tag: &str, src: &str) -> [(i32, String, String); 2] {
    let (module, diags, _) = elya::front_end_with("wide.elya", src, &mut |_| None);
    assert!(diags.is_empty(), "{diags:?}");
    let (diags, table) = elya::types::infer_typed_table(&elya::Session::new(), &module);
    assert!(diags.is_empty(), "{diags:?}");
    let core = elya::core::lower_module(&module, &table).expect("lowers");
    let dir = temp_dir(tag);
    let obj = dir.join("program.o");
    elya_codegen::compile_module(&core, &obj).expect("compile_module");
    let exe = dir.join(if cfg!(windows) {
        "program.exe"
    } else {
        "program"
    });
    elya_codegen::link(&obj, &exe).expect("link");
    let run = |stress: Option<&str>| {
        let mut cmd = Command::new(&exe);
        if let Some(s) = stress {
            cmd.env("ELYA_GC_STRESS", s);
        }
        let out = cmd.output().expect("run");
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    let both = [run(None), run(Some("1"))];
    std::fs::remove_dir_all(&dir).ok();
    both
}

const PRELUDE: &str = "type L { Nil, Cons(Int, L) }\n\
    fn build(n, acc) { if n == 0 { acc } else { build(n - 1, Cons(n, acc)) } }\n\
    fn sum(xs, acc) { match xs { Nil -> acc  Cons(h, t) -> sum(t, acc + h) } }\n\
    fn churn(n, acc) { if n == 0 { acc } else { churn(n - 1, acc + sum(build(20, Nil), 0)) } }\n";

fn expect(tag: &str, src: &str, want: &str) {
    for (i, got) in native(tag, src).into_iter().enumerate() {
        assert_eq!(
            got,
            (0, want.to_string(), String::new()),
            "{tag} ({})",
            if i == 0 { "plain" } else { "ELYA_GC_STRESS=1" }
        );
    }
}

/// A constructor of `n` fields: Ints but for lists at the places `heap`, each
/// list held only by the value across the garbage; `Int` fields hold odd
/// numbers that are no address. The lists' sums, read back.
fn ctor_program(n: usize, heap: &[usize]) -> (String, i64) {
    let fields: Vec<&str> = (0..n)
        .map(|i| if heap.contains(&i) { "L" } else { "Int" })
        .collect();
    let args: Vec<String> = (0..n)
        .map(|i| {
            if heap.contains(&i) {
                format!("build({}, Nil)", i + 1)
            } else {
                format!("{}", 2 * i + 1)
            }
        })
        .collect();
    let pats: Vec<String> = (0..n).map(|i| format!("a{i}")).collect();
    let sums: Vec<String> = heap.iter().map(|i| format!("sum(a{i}, 0)")).collect();
    let ints: i64 = (0..n)
        .filter(|i| !heap.contains(i))
        .map(|i| 2 * i as i64 + 1)
        .sum();
    let int_sum: Vec<String> = (0..n)
        .filter(|i| !heap.contains(i))
        .map(|i| format!("a{i}"))
        .collect();
    let want: i64 = heap
        .iter()
        .map(|&i| ((i + 1) * (i + 2) / 2) as i64)
        .sum::<i64>()
        + ints;
    let src = format!(
        "{PRELUDE}type W {{ W({}) }}\n\
         fn mk() {{ W({}) }}\n\
         pub fn main() {{ let w = mk()  let c = churn(300, 0)  match w {{ W({}) -> {} + {} + c - c }} }}\n",
        fields.join(", "),
        args.join(", "),
        pats.join(", "),
        sums.join(" + "),
        int_sum.join(" + "),
    );
    (src, want)
}

#[test]
fn a_constructor_of_more_than_64_fields_is_traced_field_by_field() {
    // 64 fields (the last a one-word mask holds), 65, 70, 130 and 200; heap
    // values on both sides of each word boundary of the mask.
    for (n, heap) in [
        (64usize, vec![0usize, 63]),
        (65, vec![0, 63, 64]),
        (70, vec![5, 69]),
        (130, vec![1, 63, 64, 65, 127, 128, 129]),
        (200, vec![0, 64, 128, 192, 199]),
    ] {
        let (src, want) = ctor_program(n, &heap);
        expect(&format!("ctor{n}"), &src, &format!("{want}\n"));
    }
}

#[test]
fn a_closure_of_more_than_63_captures_keeps_them() {
    for n in [63usize, 64, 70, 150] {
        let lets: String = (0..n)
            .map(|i| format!("  let x{i} = build({}, Nil)\n", i + 1))
            .collect();
        let body: Vec<String> = (0..n).map(|i| format!("sum(x{i}, 0)")).collect();
        let want: i64 = (0..n).map(|i| ((i + 1) * (i + 2) / 2) as i64).sum();
        let src = format!(
            "{PRELUDE}fn mk() {{\n{lets}  fn() {{ {} }}\n}}\n\
             pub fn main() {{ let f = mk()  let c = churn(300, 0)  f() + c - c }}\n",
            body.join(" + ")
        );
        expect(&format!("closure{n}"), &src, &format!("{want}\n"));
    }
}

#[test]
fn more_than_62_values_live_across_a_call_are_kept() {
    for n in [62usize, 63, 70, 150] {
        let lets: String = (0..n)
            .map(|i| format!("  let x{i} = build({}, Nil)\n", i + 1))
            .collect();
        let body: Vec<String> = (0..n).map(|i| format!("sum(x{i}, 0)")).collect();
        let want: i64 = (0..n).map(|i| ((i + 1) * (i + 2) / 2) as i64).sum();
        // `churn` is not in tail position: every list is live across it, and
        // across the handler's perform below.
        let src = format!(
            "{PRELUDE}effect Ask {{ fn ask() -> Int }}\n\
             fn f() -> Int {{\n{lets}  let c = churn(300, 0) + ask()\n  {} + c - c\n}}\n\
             pub fn main() -> Int {{ handle {{ f() }} with {{ Ask.ask() -> resume(churn(50, 0))  return(v) -> v }} }}\n",
            body.join(" + ")
        );
        expect(&format!("frame{n}"), &src, &format!("{want}\n"));
    }
}

/// `==` on a wide constructor reads each field as what it is: a list past the
/// 64th field compared by its cells, a Float past it by IEEE (`NaN` is not
/// itself, `0.0` is `-0.0`), an Int by its bits.
#[test]
fn equality_reads_a_wide_constructors_fields_as_what_they_are() {
    let n = 70;
    let fields: Vec<&str> = (0..n)
        .map(|i| match i {
            66 => "Float",
            68 => "L",
            _ => "Int",
        })
        .collect();
    let mk = |f: &str, l: &str| -> String {
        let args: Vec<String> = (0..n)
            .map(|i| match i {
                66 => f.to_string(),
                68 => l.to_string(),
                _ => format!("{i}"),
            })
            .collect();
        format!("W({})", args.join(", "))
    };
    let b = |e: String| format!("(if {e} {{ 1 }} else {{ 0 }})");
    let src = format!(
        "{PRELUDE}type W {{ W({}) }}\n\
         pub fn main() {{\n  {} * 1000 + {} * 100 + {} * 10 + {}\n}}\n",
        fields.join(", "),
        b(format!(
            "{} == {}",
            mk("1.5", "build(3, Nil)"),
            mk("1.5", "build(3, Nil)")
        )),
        b(format!(
            "{} == {}",
            mk("1.5", "build(3, Nil)"),
            mk("1.5", "build(4, Nil)")
        )),
        b(format!("{} == {}", mk("0.0", "Nil"), mk("-.0.0", "Nil"))),
        b(format!(
            "{} == {}",
            mk("(0.0 /. 0.0)", "Nil"),
            mk("(0.0 /. 0.0)", "Nil")
        )),
    );
    expect("eq", &src, "1010\n");
}
