//! Map a run outcome to a process exit code.
//! 0 = all requests sent + all assertions passed.
//! 1 = one or more assertion failures.
//! 2 = execution error (network/parse/script) when not an assertion failure.

use golden_core::result::RunResult;

/// Exit code for a fatal CLI error (parse, discovery, no collections, IO).
pub const FATAL: i32 = 2;

/// Map a completed RunResult to its exit code.
pub fn code_for_result(result: &RunResult) -> i32 {
    let t = &result.totals;
    if t.failed_assertions > 0 {
        1
    } else if t.failed_requests > 0 {
        2
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use golden_core::result::{RunResult, Totals};

    fn totals(failed_assertions: usize, failed_requests: usize) -> RunResult {
        RunResult {
            collections: vec![],
            totals: Totals {
                requests: 1,
                failed_requests,
                assertions: 1,
                failed_assertions,
                total_ms: 0,
            },
        }
    }

    #[test]
    fn all_pass_is_zero() {
        assert_eq!(code_for_result(&totals(0, 0)), 0);
    }

    #[test]
    fn assertion_failure_is_one() {
        assert_eq!(code_for_result(&totals(2, 0)), 1);
    }

    #[test]
    fn execution_error_is_two_even_without_assertion_failures() {
        // a failed request with no failed assertion = transport/exec error
        assert_eq!(code_for_result(&totals(0, 1)), 2);
    }

    #[test]
    fn assertion_failure_takes_precedence_over_request_failure() {
        // when both occur, assertion failure (1) is the headline code
        // per spec: 1 = assertion fail, 2 = exec error "when not otherwise an assertion"
        assert_eq!(code_for_result(&totals(1, 1)), 1);
    }

    #[test]
    fn fatal_error_is_two() {
        assert_eq!(FATAL, 2);
    }
}
