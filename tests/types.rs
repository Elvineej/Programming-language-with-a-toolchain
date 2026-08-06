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

fn schemes(src: &str) -> std::collections::HashMap<String, String> {
    let (m, d) = elya::parse::parse_module(&Session::new(), src);
    assert!(d.is_empty(), "parse: {d:?}");
    let (ss, diags) = elya::types::infer_schemes(&Session::new(), &m);
    assert!(diags.is_empty(), "type: {diags:?}");
    ss.into_iter().collect()
}

#[test]
fn identity_generalizes() {
    let s = schemes("fn id(x) { x }\n");
    assert_eq!(s["id"], "forall a. fn(a) -> a");
}

#[test]
fn const_generalizes_two_vars() {
    let s = schemes("fn first(x, y) { x }\n");
    assert_eq!(s["first"], "forall a b. fn(a, b) -> a");
}

#[test]
fn identity_used_at_two_types_typechecks() {
    let src = "fn id(x) { x }\npub fn main() { let _ = id(1)\n io.println(id(\"hi\")) }\n";
    let (m, d) = elya::parse::parse_module(&Session::new(), src);
    assert!(d.is_empty());
    let diags = elya::types::infer(&Session::new(), &m);
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn mutual_recursion_typechecks() {
    let src = "fn even(n) { if n == 0 { True } else { odd(n - 1) } }\n\
               fn odd(n) { if n == 0 { False } else { even(n - 1) } }\n";
    let (m, d) = elya::parse::parse_module(&Session::new(), src);
    assert!(d.is_empty());
    assert!(elya::types::infer(&Session::new(), &m).is_empty());
}
