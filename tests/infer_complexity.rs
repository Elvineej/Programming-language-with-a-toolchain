//! Inference costs in proportion to the program plus the types it writes
//! (2026-10-31, spec `docs/superpowers/specs/2026-10-31-elya-scalability-design.md`).
//!
//! Two timing-free measures, each taken at `n` and at `2n`: the steps inference
//! counts (`elya::types::inference_steps`, D6) and the allocations it makes on
//! this thread (the counting allocator below: copying a type visits nothing a
//! walk would not, but allocates). A program whose types are bounded doubles
//! both when it doubles ([`assert_linear`]). A program whose types grow faster
//! than it does (link `i` of a chain has a type of `i` arrows, so its types
//! total n^2) may cost as its types grow, no faster ([`assert_in_proportion`]):
//! nothing that writes the types out can do better.
//!
//! Negative controls (each reverted; the file compared byte for byte after):
//! - K1, `free_vars` resolving again at every node (D1): the chain and two
//!   towers fail;
//! - K2, the environment's free variables read from every scheme in scope
//!   again (D2): many functions, lets under a parameter and the chain fail;
//! - K3, `unify` resolving both sides whole at every level (D3): two towers
//!   fail;
//! - K4, a flush taking every inclusion (D4): closures calling a value fail;
//! - K5, no compression where duplicates are settled (D5): closures calling a
//!   value fail;
//! - K6, `resolve_row` cloning a label and resolving the clone (D1): nested
//!   performs fail, on allocations alone;
//! - K7, `open_covariant` resolving at every arrow (D1): the chain and two
//!   towers fail;
//! - K10, no `compress_rows` before the zonk (D5): closures calling a value
//!   fail (ratio 3.35);
//! - K26, a row variable bound to the younger one it meets (D15): aliases of
//!   a function parameter fail (2.87).

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

/// Counts this thread's allocations: a count read before and after one call
/// on one thread is that call's alone, whatever other threads allocate.
struct Counting;

thread_local! {
    static ALLOCS: Cell<u64> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

fn allocs() -> u64 {
    ALLOCS.with(|c| c.get())
}

/// What inferring one program cost, and the size of the types it wrote.
#[derive(Debug, Clone, Copy)]
struct Cost {
    steps: u64,
    allocs: u64,
    /// Every node of every node's zonked type (labels' arguments included).
    types: u64,
}

fn ty_size(t: &elya::types::Ty) -> u64 {
    use elya::types::Ty;
    match t {
        Ty::Var(_) | Ty::Base(_) | Ty::Error => 1,
        Ty::Fn(ps, row, r) => {
            1 + ps.iter().map(ty_size).sum::<u64>()
                + row
                    .labels
                    .values()
                    .flat_map(|l| l.args.iter())
                    .map(ty_size)
                    .sum::<u64>()
                + ty_size(r)
        }
        Ty::Tuple(xs) | Ty::Con(_, xs) => 1 + xs.iter().map(ty_size).sum::<u64>(),
    }
}

fn cost(src: &str) -> Cost {
    let (module, diags, _) = elya::front_end_with("t.elya", src, &mut |_| None);
    assert!(diags.is_empty(), "{diags:?}\n{src}");
    let before = allocs();
    let steps = elya::types::inference_steps(&module);
    let allocs = allocs() - before;
    let (diags, table) = elya::types::infer_typed_table(&elya::Session::new(), &module);
    assert!(diags.is_empty(), "{diags:?}");
    let types = table.values().map(ty_size).sum();
    Cost {
        steps,
        allocs,
        types,
    }
}

/// `shape(n)` and `shape(2n)` each cost at most 2.1 times the other's half:
/// linear, with a tenth of a doubling for what amortizes (a map's growth).
fn assert_linear(name: &str, shape: impl Fn(usize) -> String, n: usize) {
    let (a, b) = (cost(&shape(n)), cost(&shape(2 * n)));
    for (what, x, y) in [("steps", a.steps, b.steps), ("allocs", a.allocs, b.allocs)] {
        assert!(
            10 * y <= 21 * x,
            "{name}: {what}({n}) = {x}, {what}({}) = {y}, ratio {:.2} (linear is 2.00)",
            2 * n,
            y as f64 / x as f64
        );
    }
}

/// `shape(2n)` costs at most 1.1 times what `shape(n)` costs per node of the
/// types it writes: inference grows as its types do, no faster.
fn assert_in_proportion(name: &str, shape: impl Fn(usize) -> String, n: usize) {
    let (a, b) = (cost(&shape(n)), cost(&shape(2 * n)));
    for (what, x, y) in [("steps", a.steps, b.steps), ("allocs", a.allocs, b.allocs)] {
        assert!(
            10 * y * a.types <= 11 * x * b.types,
            "{name}: {what} per type node {:.2} at {n}, {:.2} at {} (types {} and {})",
            x as f64 / a.types as f64,
            y as f64 / b.types as f64,
            2 * n,
            a.types,
            b.types
        );
    }
}

fn lines(head: &[String], body: Vec<String>, tail: &[&str]) -> String {
    let mut out: Vec<String> = head.to_vec();
    out.extend(body);
    out.extend(tail.iter().map(|s| s.to_string()));
    out.join("\n") + "\n"
}

/// `n` top-level functions, each calling the one before: every group was
/// generalized against every scheme so far.
fn many_functions(n: usize) -> String {
    let body = (1..n)
        .map(|i| format!("fn f{i}(x: Int) -> Int {{ f{}(x) + 1 }}", i - 1))
        .collect();
    lines(
        &["fn f0(x: Int) -> Int { x + 1 }".to_string()],
        body,
        &[&format!("pub fn main() -> Int {{ f{}(0) }}", n - 1)],
    )
}

#[test]
fn many_functions_infer_linearly() {
    assert_linear("many functions", many_functions, 200);
}

/// A parameter in scope of `n` generalized `let`s: each asked what the
/// environment leaves free.
fn lets_under_a_parameter(n: usize) -> String {
    let body = (0..n)
        .map(|i| format!("  let g{i} = fn(y) {{ y + x }}"))
        .collect();
    lines(
        &["fn f(x: Int) -> Int {".to_string()],
        body,
        &[
            &format!("  g{}(1)", n - 1),
            "}",
            "pub fn main() -> Int { f(1) }",
        ],
    )
}

#[test]
fn lets_under_a_parameter_infer_linearly() {
    assert_linear("lets under a parameter", lets_under_a_parameter, 200);
}

/// A function value called inside `n` closures: each call is an inclusion,
/// and relays its callee's tail into the ambient.
fn closures_calling_a_value(n: usize) -> String {
    let body = (1..n)
        .map(|i| {
            format!(
                "  let a{i} = (a{} + apply(fn() {{ f() }})) % 1000003",
                i - 1
            )
        })
        .collect();
    lines(
        &[
            "fn apply(k: fn() -> Int) -> Int { k() }".to_string(),
            "pub fn main() -> Int {".to_string(),
            "  let f = fn() { 1 }".to_string(),
            "  let a0 = 0".to_string(),
        ],
        body,
        &[&format!("  a{}", n - 1), "}"],
    )
}

#[test]
fn closures_calling_a_value_infer_linearly() {
    assert_linear("closures calling a value", closures_calling_a_value, 200);
}

/// A block of `n` statements, each using the one before.
fn long_block(n: usize) -> String {
    let body = (1..n)
        .map(|i| format!("  let a{i} = (a{} * 3 + {i}) % 1000003", i - 1))
        .collect();
    lines(
        &[
            "pub fn main() -> Int {".to_string(),
            "  let a0 = 1".to_string(),
        ],
        body,
        &[&format!("  a{}", n - 1), "}"],
    )
}

#[test]
fn a_long_block_infers_linearly() {
    assert_linear("a long block", long_block, 1000);
}

/// `let a{i+1} = fn() { a{i} }`: link `i`'s type is a tower of `i` arrows.
fn chain(n: usize) -> String {
    let body = (1..=n)
        .map(|i| format!("  let a{i} = fn() {{ a{} }}", i - 1))
        .collect();
    lines(
        &[
            "pub fn main() -> Int {".to_string(),
            "  let a0 = fn() { 1 }".to_string(),
        ],
        body,
        &["  0", "}"],
    )
}

#[test]
fn a_chain_of_closures_costs_in_proportion_to_its_types() {
    assert_in_proportion("a chain of closures", chain, 40);
}

/// `op(fn() { op(fn() { .. }) })`, `n` deep, through an effect whose argument
/// is a type parameter: each closure's row holds the one inside it.
fn nested_performs(n: usize) -> String {
    let mut body = "1".to_string();
    for _ in 0..n {
        body = format!("op(fn() {{ {body} }})");
    }
    format!(
        "effect E(t) {{ fn op(k: t) -> Int }}\nfn f() {{ {body} }}\npub fn main() -> Int {{ 0 }}\n"
    )
}

#[test]
fn nested_performs_cost_in_proportion_to_their_types() {
    assert_in_proportion("nested performs", nested_performs, 30);
}

/// Each link unifies two towers, an `if`'s arms.
fn two_towers(n: usize) -> String {
    let body = (1..=n)
        .map(|i| {
            format!(
                "  let a{i} = if c {{ fn() {{ a{} }} }} else {{ fn() {{ a{} }} }}",
                i - 1,
                i - 1
            )
        })
        .collect();
    lines(
        &[
            "fn f(c: Bool) -> Int {".to_string(),
            "  let a0 = fn() { 1 }".to_string(),
        ],
        body,
        &["  0", "}", "pub fn main() -> Int { f(True) }"],
    )
}

#[test]
fn two_towers_unified_cost_in_proportion_to_their_types() {
    assert_in_proportion("two towers", two_towers, 30);
}

/// `n` closures over a function parameter: each closure's type mentions the
/// parameter's, so each stays open in the environment, and every `let` after
/// it asked what the environment leaves free (D14).
fn closures_over_a_function_parameter(n: usize) -> String {
    let body = (1..n)
        .map(|i| format!("  let g{i} = fn() {{ k() + {i} }}"))
        .collect();
    lines(
        &["fn f(k) {".to_string()],
        body,
        &[
            &format!("  g{}()", n - 1),
            "}",
            "pub fn main() -> Int { f(fn() { 1 }) }",
        ],
    )
}

#[test]
fn closures_over_a_function_parameter_infer_linearly() {
    assert_linear(
        "closures over a function parameter",
        closures_over_a_function_parameter,
        200,
    );
}

/// `let k{i} = k{i-1}`, each alias called: a value of the parameter's type,
/// open, under every later `let` (D14).
fn aliases_of_a_function_parameter(n: usize) -> String {
    let body = (1..n)
        .flat_map(|i| {
            [
                format!("  let k{i} = k{}", i - 1),
                format!("  let a{i} = (k{i}() + a{}) % 1000003", i - 1),
            ]
        })
        .collect();
    lines(
        &[
            "fn f(k) {".to_string(),
            "  let k0 = k".to_string(),
            "  let a0 = 0".to_string(),
        ],
        body,
        &[
            &format!("  a{}", n - 1),
            "}",
            "pub fn main() -> Int { f(fn() { 1 }) }",
        ],
    )
}

#[test]
fn aliases_of_a_function_parameter_infer_linearly() {
    assert_linear(
        "aliases of a function parameter",
        aliases_of_a_function_parameter,
        200,
    );
}

/// Non-value `let`s of function type, each beside a generalized one: each
/// non-value binding stays open (its phantom rows until the group ends), and
/// every generalized `let` after it asked about it (D14; D2's limit, the part 1
/// review's finding 2).
fn open_lets_beside_generalized_ones(n: usize) -> String {
    let body = (0..n)
        .flat_map(|i| {
            [
                format!("  let r{i} = id(fn(y) {{ y }})"),
                format!("  let g{i} = fn(z) {{ z }}"),
            ]
        })
        .collect();
    lines(
        &[
            "fn id(x) { x }".to_string(),
            "pub fn main() -> Int {".to_string(),
        ],
        body,
        &[&format!("  r{}(g{}(1))", n - 1, n - 1), "}"],
    )
}

#[test]
fn open_lets_beside_generalized_ones_infer_linearly() {
    assert_linear(
        "open lets beside generalized ones",
        open_lets_beside_generalized_ones,
        200,
    );
}

/// One recursive group of `n` functions, each binding a local `let`: every
/// member is open while the group is inferred (D14; D2's limit).
fn a_recursive_group_with_local_lets(n: usize) -> String {
    let body = (0..n)
        .map(|i| {
            format!(
                "fn f{i}(x: Int) -> Int {{ let t = fn(z) {{ z }}  if x == 0 {{ t(0) }} else {{ f{}(x - 1) }} }}",
                (i + 1) % n
            )
        })
        .collect();
    lines(&[], body, &["pub fn main() -> Int { f0(3) }"])
}

#[test]
fn a_recursive_group_with_local_lets_infers_linearly() {
    assert_linear(
        "a recursive group with local lets",
        a_recursive_group_with_local_lets,
        200,
    );
}

/// Affine bindings down a block, each consumed by a closure called once, in a
/// program declaring a `multi` effect: the affine pass's counts and held sets
/// (spec 2026-11-06-elya-affine-across-functions D2, D3) stay in proportion.
fn affine_bindings_down_a_block(n: usize) -> String {
    let body = (0..n)
        .flat_map(|i| {
            [
                format!("  let f{i} = File({i})"),
                format!("  let g{i} = fn() {{ read(f{i}) }}"),
                format!("  let n{} = n{i} + g{i}()", i + 1),
            ]
        })
        .collect();
    lines(
        &[
            "linear type File { File(Int) }".to_string(),
            "fn read(f: File) -> Int { match f { File(n) -> n } }".to_string(),
            "effect multi Flip { fn flip() -> Bool }".to_string(),
            "pub fn main() -> Int {".to_string(),
            "  let n0 = 0".to_string(),
        ],
        body,
        &[&format!("  n{n}"), "}"],
    )
}

#[test]
fn affine_bindings_down_a_block_infer_linearly() {
    assert_linear(
        "affine bindings down a block",
        affine_bindings_down_a_block,
        200,
    );
}
