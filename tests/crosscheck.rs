//! CEK output must equal the tree-walker oracle on every example program.

use elya::parse::parse_module;
use elya::Session;

fn both(src: &str) -> (String, String) {
    let (m, d) = parse_module(&Session::new(), src);
    assert!(d.is_empty(), "parse: {d:?}");
    let cek = elya::eval::run_module(&m).unwrap().output().to_string();
    let tree = elya::eval::run_module_tree(&m).unwrap().output().to_string();
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
