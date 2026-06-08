//! Reporter trait: formats a golden_core RunResult to a writer. One impl per
//! output format. The factory maps a ReporterKind to a boxed Reporter.

pub mod json;
pub mod junit;
pub mod pretty;
pub mod tap;

use std::io::{self, Write};

use golden_core::result::RunResult;

use crate::cli::ReporterKind;

/// A reporter turns a RunResult into formatted bytes on a writer.
pub trait Reporter {
    /// Which kind this reporter is (for the factory round-trip + tests).
    #[allow(dead_code)] // wired in Task 12 (list) for format disambiguation
    fn kind(&self) -> ReporterKind;
    /// Write the formatted report. `color` is a hint the pretty reporter honors.
    fn report(&self, result: &RunResult, out: &mut dyn Write, color: bool) -> io::Result<()>;
}

/// Build a reporter for the requested kind.
pub fn reporter_for(kind: ReporterKind) -> Box<dyn Reporter> {
    match kind {
        ReporterKind::Pretty => Box::new(pretty::PrettyReporter),
        ReporterKind::Junit => Box::new(junit::JunitReporter),
        ReporterKind::Json => Box::new(json::JsonReporter),
        ReporterKind::Tap => Box::new(tap::TapReporter),
    }
}

/// Apply the default reporter (pretty) when none were requested.
pub fn default_if_empty(requested: &[ReporterKind]) -> Vec<ReporterKind> {
    if requested.is_empty() {
        vec![ReporterKind::Pretty]
    } else {
        requested.to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::ReporterKind;

    #[test]
    fn factory_returns_a_reporter_for_each_kind() {
        for kind in [
            ReporterKind::Pretty,
            ReporterKind::Junit,
            ReporterKind::Json,
            ReporterKind::Tap,
        ] {
            let r = reporter_for(kind);
            assert_eq!(r.kind(), kind);
        }
    }

    #[test]
    fn default_kinds_is_pretty_when_empty() {
        assert_eq!(default_if_empty(&[]), vec![ReporterKind::Pretty]);
        assert_eq!(
            default_if_empty(&[ReporterKind::Json]),
            vec![ReporterKind::Json]
        );
    }
}
