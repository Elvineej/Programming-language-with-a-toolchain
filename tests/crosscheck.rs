//! CEK output must equal the tree-walker oracle on every example program.

use elya::parse::parse_module;
use elya::Session;

fn both(src: &str) -> (String, String) {
    let (m, d) = parse_module(&Session::new(), src);
    assert!(d.is_empty(), "parse: {d:?}");
    let cek = elya::eval::run_module(&m).unwrap().output().to_string();
    let tree = elya::eval::run_module_tree(&m)
        .unwrap()
        .output()
        .to_string();
    (cek, tree)
}

#[test]
fn cek_matches_tree_on_examples() {
    let dir = format!("{}/examples", env!("CARGO_MANIFEST_DIR"));
    for entry in std::fs::read_dir(dir).unwrap() {
        let p = entry.unwrap().path();
        if p.extension().and_then(|e| e.to_str()) == Some("elya") {
            let src = std::fs::read_to_string(&p).unwrap();
            let (cek, tree) = both(&src);
            assert_eq!(cek, tree, "CEK vs tree divergence on {p:?}");
        }
    }
}

fn hand_written() -> Vec<&'static str> {
    vec![
        "fn f(x){ x + 1 }\npub fn main(){ let a = f(f(1))\n io.println(\"ok\") }\n",
        "pub fn main(){ if 1 < 2 { io.println(\"a\") } else { io.println(\"b\") } }\n",
        "fn ev(n){ if n == 0 { True } else { od(n - 1) } }\n\
         fn od(n){ if n == 0 { False } else { ev(n - 1) } }\n\
         pub fn main(){ if ev(10) { io.println(\"even\") } else { io.println(\"odd\") } }\n",
        "fn add(a, b){ a + b }\nfn twice(n){ add(n, n) }\n\
         pub fn main(){ if twice(21) == 42 { io.println(\"yes\") } else { io.println(\"no\") } }\n",
        "pub fn main(){ let s = \"a\" <> \"b\" <> \"c\"\n io.println(s) }\n",
        "fn pick(c, x, y){ if c { x } else { y } }\n\
         pub fn main(){ io.println(pick(1 < 2, \"L\", \"R\")) }\n",
        // Effect-free ADT program: the differential oracle extends to ADTs (spec §3.4).
        "type List(a) { Nil, Cons(a, List(a)) }\n\
         fn sum(acc, xs){ match xs { Nil -> acc  Cons(h, t) -> sum(acc + h, t) } }\n\
         pub fn main(){ if sum(0, Cons(1, Cons(2, Cons(3, Nil)))) == 6 { io.println(\"six\") } else { io.println(\"no\") } }\n",
        // Effect-free higher-order program: the cek==tree oracle extends to closures (spec §3.4).
        "type List(a) { Nil, Cons(a, List(a)) }\n\
         fn map(xs, f){ match xs { Nil -> Nil  Cons(h, t) -> Cons(f(h), map(t, f)) } }\n\
         fn sum(acc, xs){ match xs { Nil -> acc  Cons(h, t) -> sum(acc + h, t) } }\n\
         pub fn main(){\n\
           let ys = map(Cons(1, Cons(2, Cons(3, Nil))), fn(n){ n + 100 })\n\
           if sum(0, ys) == 306 { io.println(\"ho\") } else { io.println(\"no\") }\n\
         }\n",
    ]
}

#[test]
fn cek_matches_tree_on_hand_written_corpus() {
    for src in hand_written() {
        let (cek, tree) = both(src);
        assert_eq!(cek, tree, "CEK vs tree divergence on:\n{src}");
    }
}
