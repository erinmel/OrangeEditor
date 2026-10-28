//! Measurement loop, run sessions and the results table.
//!
//! A *pump* is one host step timed with [`Instant`]: `Run::pump(budget)` (plus
//! `advance_to` when the run only waits on a timer) for control plans, or one
//! `poll_export(budget)` for data plans, preceded by `begin_epoch` and `set_input` when it
//! opens an epoch. Allocation counters cover a whole run: start, every pump, stop and
//! drop/dispose. Plans and documents are built before the window opens.

use crate::counter::{Counts, Snapshot};
use orange_compiler::ControlPlan;
use orange_model::id::VariableId;
use orange_model::{GraphDocument, Value};
use orange_runtime::{Run, RunLimits, RunOptions};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy)]
pub struct Config {
    pub budget: usize,
    pub warmup: usize,
    pub runs: usize,
}

pub trait Session {
    type Outcome;
    /// One pump. Returns `false` once the workload has no more work.
    fn step(&mut self) -> bool;
    /// Teardown inside the measurement window; the outcome is checked after it closes.
    fn finish(self) -> Self::Outcome;
}

pub struct Row {
    pub name: String,
    pub units: u64,
    pub unit: &'static str,
    pub pumps: usize,
    pub counts: Counts,
    /// Allocation counts differed between measured runs (the first run is reported).
    pub counts_vary: bool,
    pub pump_p50: Duration,
    pub pump_p95: Duration,
    pub total_p50: Duration,
}

pub fn measure<S: Session>(
    name: String,
    units: u64,
    unit: &'static str,
    config: Config,
    mut start: impl FnMut() -> S,
    mut check: impl FnMut(S::Outcome),
) -> Row {
    let mut pumps = 0;
    for _ in 0..config.warmup.max(1) {
        let mut session = start();
        pumps = 1;
        while session.step() {
            pumps += 1;
        }
        check(session.finish());
    }
    // Reserved up front so recording a pump never allocates inside the window.
    let mut durations = Vec::with_capacity(pumps * config.runs);
    let mut totals = Vec::with_capacity(config.runs);
    let mut first: Option<Counts> = None;
    let mut counts_vary = false;
    for _ in 0..config.runs {
        let capacity = durations.capacity();
        let before = durations.len();
        let window = Snapshot::start();
        let begin = Instant::now();
        let mut session = start();
        loop {
            let pump = Instant::now();
            let more = session.step();
            durations.push(pump.elapsed());
            if !more {
                break;
            }
        }
        let outcome = session.finish();
        let total = begin.elapsed();
        let counts = window.finish();
        assert_eq!(capacity, durations.capacity(), "{name}: pump count changed");
        assert_eq!(
            pumps,
            durations.len() - before,
            "{name}: pump count changed"
        );
        check(outcome);
        totals.push(total);
        match first {
            None => first = Some(counts),
            Some(reported) => counts_vary |= reported != counts,
        }
    }
    durations.sort_unstable();
    totals.sort_unstable();
    Row {
        name,
        units,
        unit,
        pumps,
        counts: first.expect("at least one measured run"),
        counts_vary,
        pump_p50: percentile(&durations, 50),
        pump_p95: percentile(&durations, 95),
        total_p50: percentile(&totals, 50),
    }
}

/// Nearest-rank percentile of a sorted, non-empty sample.
fn percentile(sorted: &[Duration], percent: usize) -> Duration {
    let rank = (sorted.len() * percent).div_ceil(100).max(1);
    sorted[rank - 1]
}

pub fn print_table(rows: &[Row]) {
    println!(
        "| workload | units | pumps/run | allocs/run | allocs/unit | bytes/run | peak live bytes | pump p50 µs | pump p95 µs | run total p50 ms |"
    );
    println!("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
    for row in rows {
        let vary = if row.counts_vary { " (varies)" } else { "" };
        println!(
            "| {} | {} {} | {} | {}{vary} | {:.2} | {} | {} | {:.2} | {:.2} | {:.3} |",
            row.name,
            row.units,
            row.unit,
            row.pumps,
            row.counts.allocations,
            row.counts.allocations as f64 / row.units as f64,
            row.counts.allocated_bytes,
            row.counts.peak_live_bytes,
            row.pump_p50.as_secs_f64() * 1e6,
            row.pump_p95.as_secs_f64() * 1e6,
            row.total_p50.as_secs_f64() * 1e3,
        );
    }
}

/// A control run drained to idleness; the outcome is one run variable and the number of
/// virtual-clock advances the host had to make.
pub struct RunSession {
    run: Run,
    budget: usize,
    probe: VariableId,
    wakeups: usize,
}

impl RunSession {
    pub fn start(
        plan: &Arc<ControlPlan>,
        document: &GraphDocument,
        probe: VariableId,
        budget: usize,
    ) -> Self {
        let options = RunOptions {
            limits: RunLimits {
                max_trace: 0,
                ..Default::default()
            },
            ..Default::default()
        };
        let run = Run::start(plan.clone(), document, plan.data().snapshot(), 42, options)
            .expect("run starts");
        Self {
            run,
            budget,
            probe,
            wakeups: 0,
        }
    }
}

impl Session for RunSession {
    type Outcome = (Option<Value>, usize);

    fn step(&mut self) -> bool {
        let progress = self.run.pump(self.budget).expect("pump");
        if progress.ready {
            return true;
        }
        match progress.next_wakeup {
            Some(at) => {
                self.run.advance_to(at).expect("advance");
                self.wakeups += 1;
                true
            }
            None => false,
        }
    }

    fn finish(mut self) -> Self::Outcome {
        let value = self.run.variable(self.probe).cloned();
        self.run.stop().expect("stop");
        self.run.dispose().expect("dispose");
        (value, self.wakeups)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_rank_percentiles() {
        let sample: Vec<_> = (1..=20).map(Duration::from_nanos).collect();
        assert_eq!(percentile(&sample, 50), Duration::from_nanos(10));
        assert_eq!(percentile(&sample, 95), Duration::from_nanos(19));
        assert_eq!(percentile(&sample[..1], 95), Duration::from_nanos(1));
    }
}
