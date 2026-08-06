//! Placeholder during the Slice-2 eval refactor. The real bounded-depth TCE
//! assertions (against the CEK machine's `peak_kont_depth`) land in Task 10,
//! after Task 8 introduces the machine.

#[test]
fn tce_assertions_arrive_with_the_cek_machine() {
    // Intentionally trivial: keeps the test target compiling between the eval
    // refactor (Task 7) and the CEK machine (Task 8) / bounded assertions (Task 10).
}
