use elya::Session;

fn type_diags(src: &str) -> Vec<String> {
    let (m, d) = elya::parse::parse_module(&Session::new(), src);
    assert!(d.is_empty(), "parse: {d:?}");
    elya::types::infer(&Session::new(), &m)
        .into_iter()
        .map(|x| x.code)
        .collect()
}

#[test]
fn well_typed_module_has_no_type_errors() {
    let src = "fn double(x) { x + x }\npub fn main() { let _ = double(21)\n io.println(\"ok\") }\n";
    assert!(type_diags(src).is_empty(), "{:?}", type_diags(src));
}

#[test]
fn ill_typed_operator_is_e0400() {
    let src = "pub fn main() { let _ = 1 + \"a\"\n io.println(\"x\") }\n";
    assert_eq!(type_diags(src), vec!["E0400".to_string()]);
}
