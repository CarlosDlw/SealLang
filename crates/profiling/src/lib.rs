//! Phase timing.
//!
//! Performance work needs evidence. Every compiler phase is wrapped in
//! [`Profiler::time`], and `--time-passes` prints the report. When profiling is
//! disabled the wrapper only costs a relaxed atomic load.

#![warn(missing_docs)]

use std::fmt::Write as _;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// One measured phase invocation.
#[derive(Clone, Debug)]
pub struct PhaseRecord {
    /// Phase name.
    pub name: &'static str,
    /// Wall-clock duration.
    pub duration: Duration,
}

/// Collects phase timings.
#[derive(Debug)]
pub struct Profiler {
    enabled: AtomicBool,
    records: Mutex<Vec<PhaseRecord>>,
}

impl Profiler {
    /// Creates a profiler; when disabled, `time` does not record anything.
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled: AtomicBool::new(enabled),
            records: Mutex::new(Vec::new()),
        }
    }

    /// Creates a disabled profiler.
    pub fn disabled() -> Self {
        Self::new(false)
    }

    /// Returns `true` if timing is enabled.
    #[inline]
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Runs `body`, recording its duration under `name`.
    pub fn time<T>(&self, name: &'static str, body: impl FnOnce() -> T) -> T {
        if !self.is_enabled() {
            return body();
        }
        let start = Instant::now();
        let result = body();
        let duration = start.elapsed();
        self.records
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(PhaseRecord { name, duration });
        result
    }

    /// Records an externally measured duration.
    pub fn record(&self, name: &'static str, duration: Duration) {
        self.records
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(PhaseRecord { name, duration });
    }

    /// Returns a snapshot of the records, slowest first.
    pub fn snapshot(&self) -> Vec<PhaseRecord> {
        let mut records = self
            .records
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        records.sort_by(|left, right| {
            right
                .duration
                .cmp(&left.duration)
                .then_with(|| left.name.cmp(right.name))
        });
        records
    }

    /// Removes all records.
    pub fn clear(&self) {
        self.records
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
    }

    /// Renders a human-readable report.
    pub fn report(&self) -> String {
        let records = self.snapshot();
        if records.is_empty() {
            return "no phase timings recorded\n".to_owned();
        }
        let mut output = String::new();
        let _ = writeln!(output, "{:<28} {:>12}", "phase", "time");
        for record in &records {
            let _ = writeln!(
                output,
                "{:<28} {:>10.3} ms",
                record.name,
                record.duration.as_secs_f64() * 1_000.0
            );
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::Profiler;

    #[test]
    fn disabled_profiler_does_not_record() {
        let profiler = Profiler::disabled();
        let value = profiler.time("phase", || 41 + 1);
        assert_eq!(value, 42);
        assert!(profiler.snapshot().is_empty());
        assert!(profiler.report().contains("no phase timings"));
    }

    #[test]
    fn enabled_profiler_records_the_phase() {
        let profiler = Profiler::new(true);
        profiler.time("parse", || ());
        let snapshot = profiler.snapshot();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].name, "parse");
        assert!(profiler.report().contains("parse"));
    }

    #[test]
    fn records_are_sorted_slowest_first() {
        let profiler = Profiler::new(true);
        profiler.record("fast", Duration::from_micros(1));
        profiler.record("slow", Duration::from_millis(10));
        profiler.record("medium", Duration::from_millis(5));
        let names: Vec<_> = profiler
            .snapshot()
            .into_iter()
            .map(|record| record.name)
            .collect();
        assert_eq!(names, vec!["slow", "medium", "fast"]);
    }

    #[test]
    fn clear_removes_records() {
        let profiler = Profiler::new(true);
        profiler.record("phase", Duration::from_millis(1));
        profiler.clear();
        assert!(profiler.snapshot().is_empty());
    }
}
