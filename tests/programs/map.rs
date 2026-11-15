//! The programs of `tests/std_map.rs`, shared with
//! `crates/codegen/tests/native_std_map.rs` (no tests here). ChatGPT wrote
//! them with the module (spec 2026-11-14-elya-arrays-and-borrowing D7).

/// Every public function, three keys that collide at capacity 8 (FNV-1a of
/// `h`, `p` and `x` is 7 modulo 8: the second and third wrap from the last
/// slot to the first), a replace, 256 keys across several doublings, a map of
/// strings with the empty key and multi-byte keys.
pub const MAP_PROGRAM: &str = r#"import elya/map

fn get_int(borrow m: Array(MapSlot(Int)), key: String) -> Int {
  match map.get(m, key) {
    MapFound(value) -> value
    MapMissing -> 0 - 1
  }
}

fn add_numbers(m: Array(MapSlot(Int)), i: Int, n: Int) -> Array(MapSlot(Int)) {
  if i == n { m } else { add_numbers(map.put(m, "k" <> int.to_string(i), i), i + 1, n) }
}

fn sum_numbers(borrow m: Array(MapSlot(Int)), i: Int, n: Int, total: Int) -> Int {
  if i == n {
    total
  } else {
    match map.get(m, "k" <> int.to_string(i)) {
      MapFound(value) -> if value == i { sum_numbers(m, i + 1, n, total + value) } else { 0 - 999999 }
      MapMissing -> 0 - 999999
    }
  }
}

pub fn main() -> Int {
  let m: Array(MapSlot(Int)) = map.new()
  io.println(int.to_string(map.size(m)))
  let m = map.put(m, "h", 10)
  let m = map.put(m, "p", 20)
  let m = map.put(m, "x", 30)
  let m = map.put(m, "h", 40)
  io.println(int.to_string(map.size(m)))
  let m = add_numbers(m, 0, 256)
  io.println(int.to_string(map.size(m)))
  let a = get_int(m, "h")
  let b = get_int(m, "p")
  let c = get_int(m, "x")
  io.println(int.to_string(a) <> " " <> int.to_string(b) <> " " <> int.to_string(c))
  let total = sum_numbers(m, 0, 256, 0)
  io.println(int.to_string(total))
  io.println((if map.has(m, "h") { "yes" } else { "no" }) <> " " <> (if map.has(m, "absent") { "yes" } else { "no" }))
  io.println(int.to_string(get_int(m, "absent")) <> " " <> int.to_string(map.get_or(m, "absent", 0 - 7)))
  let t: Array(MapSlot(String)) = map.new()
  let t = map.put(t, "", "empty")
  let t = map.put(t, "é", "accent")
  let t = map.put(t, "日本", "kanji")
  io.println(map.get_or(t, "", "-") <> " " <> map.get_or(t, "é", "-") <> " " <> map.get_or(t, "日本", "-"))
  total + a + b + c + map.size(m)
}
"#;

pub const MAP_OUTPUT: &str = "0\n3\n259\n40 20 30\n32640\nyes no\n-1 -7\nempty accent kanji\n";
pub const MAP_VALUE: &str = "32989";

/// `n` insertions, then `n` lookups whose values are summed.
pub fn inserts_and_lookups(n: u32) -> String {
    format!(
        "import elya/map\n\
         fn insert_all(m: Array(MapSlot(Int)), i: Int, n: Int) -> Array(MapSlot(Int)) {{\n  if i == n {{ m }} else {{ insert_all(map.put(m, int.to_string(i), i), i + 1, n) }}\n}}\n\
         fn check_all(borrow m: Array(MapSlot(Int)), i: Int, n: Int, total: Int) -> Int {{\n  if i == n {{ total }} else {{ match map.get(m, int.to_string(i)) {{ MapFound(v) -> if v == i {{ check_all(m, i + 1, n, total + v) }} else {{ 0 - 1 }}  MapMissing -> 0 - 1 }} }}\n}}\n\
         pub fn main() -> Int {{\n  let m: Array(MapSlot(Int)) = insert_all(map.new(), 0, {n})\n  if map.size(m) == {n} {{ check_all(m, 0, {n}, 0) }} else {{ 0 - 1 }}\n}}\n"
    )
}

pub fn sum_below(n: u64) -> String {
    (n * (n - 1) / 2).to_string()
}
