//! The selective-CPS partition (spec §8.1-8.2).
//!
//! Deliberately LLVM-free: this is a pure question about a `Ty`, so it needs no
//! `Context` and its tests are unit tests.

use elya::types::{EffectRow, RowTail, Ty};

/// The one effect label the back end treats as builtin. Sound as a *name* test
/// only because lowering refuses a user-declared `effect IO` (D1, Task 2) --
/// `CoreModule` carries no effect declarations, so the name is all codegen has.
/// Mirrors `src/types.rs`'s `OBSERVABLE_EFFECTS`.
const BUILTIN_EFFECT: &str = "IO";

/// True iff a call to a value of this type may need its continuation captured.
///
/// A non-function type is not a call site and answers `false`.
///
/// No caller outside the tests until Task 7b's call-site check; delete this
/// `allow` there, when the first caller lands.
#[allow(dead_code)]
pub fn needs_cps(ty: &Ty) -> bool {
    match ty {
        Ty::Fn(_, row, _) => row_needs_cps(row),
        _ => false,
    }
}

fn row_needs_cps(row: &EffectRow) -> bool {
    if row.labels.keys().any(|l| l != BUILTIN_EFFECT) {
        return true;
    }
    match row.tail {
        RowTail::Closed => false,
        // A row we cannot see the end of might carry a user-declared effect.
        // Answer conservatively: over-CPS costs speed, under-CPS is a wrong
        // answer -- the asymmetry `closure.rs` states for over- vs
        // under-capture. If the codegen suite starts hitting this arm in
        // practice, that is a signal to investigate why an unresolved row
        // survived inference, NOT a signal to flip the default.
        RowTail::Open(_) | RowTail::ErrorRow => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use elya::span::Span;
    use elya::types::{EffectLabel, EffectRow, RowTail, Ty, TyCon};
    use std::collections::BTreeMap;

    fn row(labels: &[&str], tail: RowTail) -> EffectRow {
        let mut m = BTreeMap::new();
        for l in labels {
            m.insert(
                (*l).to_string(),
                EffectLabel {
                    args: Vec::new(),
                    span: Span::EMPTY,
                },
            );
        }
        EffectRow { labels: m, tail }
    }

    fn func(r: EffectRow) -> Ty {
        Ty::Fn(
            vec![Ty::Base(TyCon::Int)],
            r,
            Box::new(Ty::Base(TyCon::Int)),
        )
    }

    #[test]
    fn a_pure_function_is_a_direct_call() {
        assert!(!needs_cps(&func(EffectRow::pure())));
    }

    #[test]
    fn a_printing_function_is_still_a_direct_call() {
        // The whole point of 8.2: `{IO}` must NOT select CPS, or every
        // function that prints pays for machinery it cannot use.
        assert!(!needs_cps(&func(row(&["IO"], RowTail::Closed))));
    }

    #[test]
    fn a_user_declared_effect_selects_cps() {
        assert!(needs_cps(&func(row(&["State"], RowTail::Closed))));
    }

    #[test]
    fn a_user_effect_alongside_io_still_selects_cps() {
        assert!(needs_cps(&func(row(&["IO", "State"], RowTail::Closed))));
    }

    #[test]
    fn an_unresolved_tail_selects_cps_conservatively() {
        // Over-CPS costs speed. Under-CPS is a wrong answer. Same asymmetry
        // `fv_walk` cites for over- vs under-capture (closure.rs). This one is
        // the poison tail; the open row variable has its own test below, so
        // each unresolved tail fails on its own.
        assert!(needs_cps(&func(row(&[], RowTail::ErrorRow))));
    }

    #[test]
    fn an_open_row_tail_selects_cps_conservatively() {
        // A row-polymorphic function's row stays open after the freeze: its
        // tail is a row variable that may yet carry a user-declared effect.
        assert!(needs_cps(&func(row(&[], RowTail::Open(0)))));
    }

    #[test]
    fn a_non_function_type_is_not_a_call_site_at_all() {
        assert!(!needs_cps(&Ty::Base(TyCon::Int)));
    }
}
