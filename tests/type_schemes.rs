// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)
use elya::Session;

fn schemes_text(src: &str) -> String {
    let (m, d) = elya::parse::parse_module(&Session::new(), src);
    assert!(d.is_empty(), "parse: {d:?}");
    let (ss, diags) = elya::types::infer_schemes(&Session::new(), &m);
    assert!(diags.is_empty(), "type: {diags:?}");
    let mut lines: Vec<String> = ss.into_iter().map(|(n, s)| format!("{n} : {s}")).collect();
    lines.sort();
    lines.join("\n")
}

#[test]
fn polymorphic_schemes() {
    insta::assert_snapshot!(
        "poly_schemes",
        schemes_text("fn id(x) { x }\nfn first(x, y) { x }\nfn twice(n) { n + n }\n")
    );
}
