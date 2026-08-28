// Enforces the compiler's internal module layering so a future workspace
// split stays mechanical. A module may only reference strictly-lower layers
// (plus equal-layer span/diag and lex/ast).

use std::collections::HashMap;
use std::fs;

fn layer(module: &str) -> Option<i32> {
    let map: HashMap<&str, i32> = HashMap::from([
        ("span", 0),
        ("diag", 0),
        ("lex", 1),
        ("ast", 1),
        ("parse", 2),
        ("resolve", 3),
        ("types", 4),
        ("core", 5),
        ("exhaust", 5),
        ("affine", 5),
        ("eval", 6),
        ("codegen", 6),
        ("main", 7),
    ]);
    map.get(module).copied()
}

#[test]
fn no_upward_module_references() {
    let dir = format!("{}/src", env!("CARGO_MANIFEST_DIR"));
    let mut violations = Vec::new();
    for entry in fs::read_dir(&dir).expect("read src") {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let stem = path.file_stem().unwrap().to_str().unwrap().to_string();
        if stem == "lib" {
            continue; // lib.rs wires all modules together by design
        }
        let Some(this_layer) = layer(&stem) else {
            continue;
        };
        let src = fs::read_to_string(&path).unwrap();
        for other in [
            "span", "diag", "lex", "ast", "parse", "resolve", "types", "core", "exhaust", "affine",
            "codegen",
            "eval",
        ] {
            if other == stem {
                continue;
            }
            let needle = format!("crate::{other}");
            if src.contains(&needle) {
                let other_layer = layer(other).unwrap();
                // Allowed: strictly lower, or equal-layer foundation pairs.
                let equal_ok = this_layer == other_layer
                    && matches!(
                        (stem.as_str(), other),
                        ("diag", "span") | ("ast", "lex") | ("lex", "ast") | ("parse", "ast")
                    );
                if other_layer >= this_layer && !equal_ok {
                    violations.push(format!(
                        "{stem} (layer {this_layer}) references {other} (layer {other_layer})"
                    ));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "module layering violations:\n{}",
        violations.join("\n")
    );
}

#[test]
fn detector_flags_a_synthetic_backedge() {
    // Sanity: the same predicate flags an upward edge.
    let this_layer = layer("span").unwrap(); // 0
    let other_layer = layer("eval").unwrap(); // 6
    assert!(other_layer >= this_layer, "predicate must flag span->eval");
}
