//! Rooting is complete under a collection at every allocation (2026-11-05,
//! spec `docs/superpowers/specs/2026-10-31-elya-scalability-design.md` D13).
//! Each program runs natively with `ELYA_GC_STRESS=1`, which collects at every
//! allocation, poisons what it frees and keeps it, and stops the program if
//! anything reachable names a freed block: a value the code still reads but no
//! longer roots is freed at once and reads as poison, and its output and value
//! must be the evaluator's. The programs are the places where a block's
//! binding stops being a root (D13): after its last reader, and not before.

use std::process::Command;

fn no_files(_: &str) -> Option<(String, String)> {
    None
}

/// The evaluator's text and value line, and the native binary's under stress.
fn both(src: &str, tag: &str) -> (String, String) {
    let o = elya::run_source_outcome_with("main.elya", src, &mut no_files);
    assert!(o.error.is_none(), "{tag}: {:?}", o.error);
    let want = format!("{}{}\n", o.output, o.line.expect("a value").expect("data"));
    let (module, diags, _) = elya::front_end_with("main.elya", src, &mut no_files);
    assert!(diags.is_empty(), "{tag}: {diags:?}");
    let session = elya::Session::new();
    let (diags, table) = elya::types::infer_typed_table(&session, &module);
    assert!(diags.is_empty(), "{tag}: {diags:?}");
    let core = elya::core::lower_module(&module, &table).expect("lowers");
    let dir = std::env::temp_dir().join(format!("elya-stress-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let obj = dir.join(format!("{tag}.o"));
    let exe = dir.join(format!("{tag}{}", std::env::consts::EXE_SUFFIX));
    elya_codegen::compile_module(&core, &obj).unwrap_or_else(|e| panic!("{tag}: {e}"));
    elya_codegen::link(&obj, &exe).expect("link");
    let out = Command::new(&exe)
        .env("ELYA_GC_STRESS", "1")
        .output()
        .expect("run the binary");
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.status.success(), "{tag}: {out:?}");
    (want, String::from_utf8(out.stdout).expect("UTF-8"))
}

const PROGRAMS: &[(&str, &str)] = &[
    (
        // A list read after the strings below it allocate, and by the end.
        "read-after-allocations",
        "type L { Nil, Cons(Int, L) }\n\
         fn build(n) { if n == 0 { Nil } else { Cons(n, build(n - 1)) } }\n\
         fn sum(xs) { match xs { Nil -> 0  Cons(h, t) -> h + sum(t) } }\n\
         pub fn main() -> Int {\n\
         \x20 let xs = build(50)\n\
         \x20 let s0 = \"a\" <> \"z\"\n\
         \x20 let s1 = s0 <> int.to_string(sum(xs))\n\
         \x20 let s2 = s1 <> \"c\"\n\
         \x20 let ys = build(20)\n\
         \x20 sum(xs) * 1000 + sum(ys) * 10 + string.length(s2)\n\
         }\n",
    ),
    (
        // A string only the block's end reads, past later allocations.
        "the-end-reads",
        "pub fn main() -> Int {\n\
         \x20 let keep = \"kept\" <> int.to_string(7)\n\
         \x20 let a = \"x\" <> \"y\"\n\
         \x20 let b = a <> \"z\"\n\
         \x20 let c = b <> \"w\"\n\
         \x20 io.println(keep)\n\
         \x20 string.length(c) * 100 + string.length(keep)\n\
         }\n",
    ),
    (
        // A closure holds what it captured after the capture's last reader.
        "a-closure-holds-its-capture",
        "fn apply(k: fn() -> Int) -> Int { k() }\n\
         pub fn main() -> Int {\n\
         \x20 let s = \"abc\" <> \"def\"\n\
         \x20 let f = fn() { string.length(s) }\n\
         \x20 let t = \"1\" <> \"2\"\n\
         \x20 let u = t <> \"3\"\n\
         \x20 apply(f) * 10 + string.length(u)\n\
         }\n",
    ),
    (
        // A name rebound down a block, and an outer one, dead and live,
        // shadowed by an inner block.
        "shadowing",
        "pub fn main() -> Int {\n\
         \x20 let s = \"a\" <> \"b\"\n\
         \x20 let s = s <> \"c\"\n\
         \x20 let dead = \"d\" <> \"e\"\n\
         \x20 let t = { let s = \"x\" <> \"y\"  let dead = s <> \"q\"  dead <> \"z\" }\n\
         \x20 let u = { let dead = t <> \"!\"  dead <> s }\n\
         \x20 io.println(u)\n\
         \x20 string.length(s) * 100 + string.length(u)\n\
         }\n",
    ),
    (
        // A string read after a perform (saved by the site), and one dead
        // before it.
        "across-a-perform",
        "effect Ask { fn ask() -> Int }\n\
         fn f() / {Ask, IO} -> Int {\n\
         \x20 let s = \"a\" <> int.to_string(ask())\n\
         \x20 let dead = \"d\" <> \"e\"\n\
         \x20 let t = s <> \"b\"\n\
         \x20 let n = ask()\n\
         \x20 let u = t <> int.to_string(n)\n\
         \x20 io.println(u)\n\
         \x20 string.length(u) * 100 + string.length(s)\n\
         }\n\
         pub fn main() -> Int { handle { f() } with { Ask.ask() -> resume(12) } }\n",
    ),
    (
        // Strings across a multi-shot resume: the block runs twice from the
        // site, each run allocating.
        "multi-shot",
        "effect multi Flip { fn flip() -> Bool }\n\
         fn choose() {\n\
         \x20 let s = \"p\" <> \"q\"\n\
         \x20 let a = if flip() { s <> \"1\" } else { s <> \"22\" }\n\
         \x20 let b = a <> \"r\"\n\
         \x20 string.length(b) * 10 + string.length(s)\n\
         }\n\
         pub fn main() -> Int { handle choose() with multi { Flip.flip() -> resume(True) * 1000 + resume(False) } }\n",
    ),
    (
        // A block in a match arm reads what the function bound above it.
        "a-block-in-an-arm",
        "type L { Nil, Cons(Int, L) }\n\
         fn f(xs) {\n\
         \x20 let tag = \"t\" <> \"u\"\n\
         \x20 match xs {\n\
         \x20   Nil -> string.length(tag)\n\
         \x20   Cons(h, t) -> { let s = int.to_string(h) <> tag  let r = s <> \"!\"  string.length(r) }\n\
         \x20 }\n\
         }\n\
         pub fn main() -> Int { f(Cons(12345, Nil)) * 10 + f(Nil) }\n",
    ),
    (
        // Many collections: a string grown through a tail call.
        "growing",
        "fn grow(n, s) { if n == 0 { string.length(s) } else { let t = s <> \"x\"  let u = t <> int.to_string(n)  grow(n - 1, u) } }\n\
         pub fn main() -> Int { grow(200, \"\") }\n",
    ),
    (
        // A large string (a block past the free lists, handed back to the
        // allocator when freed) that nothing reads, freed by the next
        // allocation, then shadowed by an inner block that allocates: rooting
        // the parked dead value again would mark a block already freed.
        "a-dead-large-string-shadowed",
        "pub fn main() -> Int {\n\
         \x20 let b0 = \"0123456789abcdef\"\n\
         \x20 let b1 = b0 <> b0\n\
         \x20 let b2 = b1 <> b1\n\
         \x20 let b3 = b2 <> b2\n\
         \x20 let s = b3 <> b3\n\
         \x20 let t = \"x\" <> \"y\"\n\
         \x20 let r = { let s = t <> \"z\"  let u = s <> \"w\"  let v = b3 <> b3  let w = b3 <> b3  string.length(u) * 1000 + string.length(v) + string.length(w) }\n\
         \x20 r\n\
         }\n",
    ),
    (
        // The same before a site, in an effectful function: the site roots
        // the bindings its region holds, and the dead one is not among them.
        "a-dead-large-string-before-a-site",
        "effect Ask { fn ask(n: Int) -> Int }\n\
         fn f() / {Ask} -> Int {\n\
         \x20 let b0 = \"0123456789abcdef\"\n\
         \x20 let b1 = b0 <> b0\n\
         \x20 let b2 = b1 <> b1\n\
         \x20 let b3 = b2 <> b2\n\
         \x20 let b4 = b3 <> b3\n\
         \x20 let n = string.length(b4)\n\
         \x20 let t = \"x\" <> \"y\"\n\
         \x20 let x = ask(n)\n\
         \x20 let c = b3 <> b3\n\
         \x20 let e = b3 <> b3\n\
         \x20 let d = c <> e <> t\n\
         \x20 x * 1000 + string.length(d)\n\
         }\n\
         pub fn main() -> Int { handle { f() } with { Ask.ask(n) -> resume(n + 1) } }\n",
    ),
    (
        // A block in value position in an effectful function: the CPS
        // emitter's value-position loop retires `dead` before the site (the
        // review of part 3b: only its tail-position loop was tested).
        "a-value-block-in-an-effectful-function",
        "effect Ask { fn ask() -> Int }\n\
         fn f() {\n\
         \x20 let r = {\n\
         \x20   let dead = \"d\" <> \"e\"\n\
         \x20   let n = string.length(dead)\n\
         \x20   let t = \"x\" <> \"y\"\n\
         \x20   let k = ask()\n\
         \x20   t <> int.to_string(n + k)\n\
         \x20 }\n\
         \x20 string.length(r)\n\
         }\n\
         pub fn main() -> Int { handle { f() } with { Ask.ask() -> resume(1) } }\n",
    ),
    (
        // The same through a resumption: after each site, the rest of the
        // value block is lowered again in the resumption, retiring there too
        // (the review's program).
        "a-value-block-resumed",
        "effect Ask { fn ask() -> Int }\n\
         fn f() {\n\
         \x20 let r = {\n\
         \x20   let a = ask()\n\
         \x20   let dead = \"d\" <> int.to_string(a)\n\
         \x20   let n = string.length(dead)\n\
         \x20   let t = \"x\" <> \"y\"\n\
         \x20   let k = ask()\n\
         \x20   let keep = t <> \"z\"\n\
         \x20   let dead = keep <> keep\n\
         \x20   let m = string.length(dead)\n\
         \x20   let w = \"w\" <> \"w\"\n\
         \x20   let j = ask()\n\
         \x20   keep <> int.to_string(n + k + m + j) <> w\n\
         \x20 }\n\
         \x20 io.println(r)\n\
         \x20 string.length(r)\n\
         }\n\
         pub fn main() -> Int { handle { f() } with { Ask.ask() -> resume(1) } }\n",
    ),
];

#[test]
fn every_program_answers_as_the_evaluator_does_under_a_collection_at_every_allocation() {
    for (tag, src) in PROGRAMS {
        let (want, got) = both(src, tag);
        assert_eq!(got, want, "{tag}");
    }
}
