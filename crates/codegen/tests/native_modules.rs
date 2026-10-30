//! Modules part 1, natively (2026-10-23, spec
//! `docs/superpowers/specs/2026-10-23-elya-modules-part-1-design.md`). Each
//! multi-file program is merged, compiled, linked and RUN, and its outcome
//! compared with the evaluator's.

use std::path::PathBuf;
use std::process::Command;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("elya-modules-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// The evaluator's output and value line, then the native binary's.
fn agree(root: &str, files: &[(&str, &str)], tag: &str) {
    let mut read = |path: &str| {
        files
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(p, t)| (format!("{p}.elya"), t.to_string()))
    };
    let o = elya::run_source_outcome_with("main.elya", root, &mut read);
    assert!(o.error.is_none(), "{tag}: {:?}", o.error);
    let want = format!("{}{}\n", o.output, o.line.unwrap().unwrap());
    let (module, diags, _) = elya::front_end_with("main.elya", root, &mut read);
    assert!(diags.is_empty(), "{tag}: {diags:?}");
    let session = elya::Session::new();
    let (diags, table) = elya::types::infer_typed_table(&session, &module);
    assert!(diags.is_empty(), "{tag}: {diags:?}");
    let core = elya::core::lower_module(&module, &table).expect("lowers");
    let dir = temp_dir(tag);
    let obj = dir.join(format!("{tag}.o"));
    let exe = dir.join(format!("{tag}{}", std::env::consts::EXE_SUFFIX));
    elya_codegen::compile_module(&core, &obj).unwrap_or_else(|e| panic!("{tag}: {e}"));
    elya_codegen::link(&obj, &exe).expect("link");
    let out = Command::new(&exe).output().expect("run the binary");
    std::fs::remove_dir_all(&dir).ok();
    assert!(out.status.success(), "{tag}: {out:?}");
    assert_eq!(String::from_utf8_lossy(&out.stdout), want, "{tag}");
}

#[test]
fn modules_agree() {
    agree(
        "import util as u\npub fn main() -> Int { let g = u.twice  u.twice(20) + g(1) }\n",
        &[("util", "pub fn twice(x: Int) -> Int { x * 2 }\n")],
        "qualified",
    );
    agree(
        "import a/b\nimport util\nimport lib\n\
         pub fn main() -> Int { b.three() * 100 + lib.quad(util.twice(1)) }\n",
        &[
            ("a/b", "pub fn three() -> Int { 3 }\n"),
            ("util", "pub fn twice(x: Int) -> Int { x * 2 }\n"),
            (
                "lib",
                "import util\npub fn quad(x: Int) -> Int { util.twice(util.twice(x)) }\n",
            ),
        ],
        "paths",
    );
    agree(
        "import util\nfn helper() -> Int { 1 }\n\
         pub fn main() -> Int { helper() * 10 + util.f() + util.g(5) * 1000 }\n",
        &[(
            "util",
            "fn helper() -> Int { 2 }\n\
             pub fn f() -> Int { helper() }\n\
             pub fn g(helper: Int) -> Int { helper + 100 }\n",
        )],
        "names",
    );
    // `/ {Log}`: `find` performs `Log` (modules part 2 D2, 2026-10-24; a CHANGED
    // PROGRAM, the expectation unchanged).
    agree(
        "import util / {Log}\n\
         fn get(o: Opt(Int)) -> Int { match o { Yes(n) -> n  No -> 0 } }\n\
         pub fn main() -> Int {\n\
           handle { get(util.find(7)) } with { Log.say(line) -> { io.println(line)  resume(Unit) } }\n\
         }\n",
        &[(
            "util",
            "type Opt(a) { No, Yes(a) }\n\
             effect Log { fn say(line: String) -> Unit }\n\
             pub fn find(n: Int) / {Log} -> Opt(Int) {\n  say(\"looking\")\n  if n > 0 { Yes(n) } else { No }\n}\n",
        )],
        "effects",
    );
}
