// Elya. Copyright (c) 2026 Elvin Elakkattu (https://github.com/Elvineej).
// SPDX-License-Identifier: PolyForm-Strict-1.0.0 (see LICENSE.md)
//! Evaluator coverage for `elya/math`, shared with the native suite.

#[path = "programs/math.rs"]
mod cases;

#[test]
fn shared_math_cases() {
    for (source, expected) in cases::programs() {
        let outcome = elya::run_source_outcome("math_cases.elya", &source);
        assert!(outcome.error.is_none(), "{:?}", outcome.error);
        assert_eq!(outcome.output, expected);
        assert_eq!(outcome.value, Some(Ok(0)));
    }
}

#[test]
fn shared_math_failures() {
    for &(expression, message) in cases::FAILURES {
        let source = cases::failure_program(expression);
        let outcome = elya::run_source_outcome("math_failure.elya", &source);
        assert!(outcome.output.is_empty(), "{expression}");
        let error = outcome.error.expect("a runtime error");
        assert!(error.contains(message), "{expression}: {error}");
    }
}
