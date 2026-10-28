//! Measurement loop, run sessions and the results table.
//!
//! A *pump* is one host step timed with [`Instant`]: `Run::pump(budget)` (plus
//! `advance_to` when the run only waits on a timer) for control plans, or one
//! `poll_export(budget)` for data plans, preceded by `begin_epoch` and `set_input` when it
//! opens an epoch. Allocation counters cover a whole run: start, every pump, stop and
//! drop/dispose. Plans and documents are built before the window opens.

use crate::counter::{Counts, Snapshot};
use orange_compiler::{ControlPlan, DataPlan};
use orange_model::id::VariableId;
use orange_model::{GraphDocument, Value};
use orange_runtime::{
    DataActivation, DataLimits, DataOptions, DataPoll, Run, RunLimits, RunOptions,
};
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
    pub pump_p99: Duration,
    /// The slowest single pump observed.
    pub pump_max: Duration,
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
        pump_p99: percentile(&durations, 99),
        pump_max: durations.last().copied().unwrap_or_default(),
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
        "| workload | units | pumps/run | allocs/run | allocs/unit | bytes/run | peak live bytes | pump p50 µs | pump p95 µs | pump p99 µs | pump max µs | run total p50 ms |"
    );
    println!("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
    for row in rows {
        let vary = if row.counts_vary { " (varies)" } else { "" };
        println!(
            "| {} | {} {} | {} | {}{vary} | {:.2} | {} | {} | {:.2} | {:.2} | {:.2} | {:.2} | {:.3} |",
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
            row.pump_p99.as_secs_f64() * 1e6,
            row.pump_max.as_secs_f64() * 1e6,
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

impl RunSession {
    /// The run, for a caller that inspects it after the session ends.
    pub fn into_run(self) -> Run {
        self.run
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

/// A data activation that publishes one host input per epoch and pulls export 0.
pub struct DataSession {
    activation: DataActivation,
    input: usize,
    value: Value,
    budget: usize,
    epochs_left: u64,
    open: bool,
    result: Option<Value>,
}

pub struct DataInput<'a> {
    /// Index into the plan's host inputs.
    pub index: usize,
    pub value: &'a Value,
    pub epochs: u64,
}

impl DataSession {
    pub fn start(
        plan: &Arc<DataPlan>,
        document: &GraphDocument,
        input: &DataInput<'_>,
        budget: usize,
        limits: DataLimits,
    ) -> Self {
        let options = DataOptions {
            limits,
            cancel: None,
        };
        let activation =
            DataActivation::start(plan.clone(), document, plan.snapshot(), 17, 1, options)
                .expect("activation starts");
        Self {
            activation,
            input: input.index,
            value: input.value.clone(),
            budget,
            epochs_left: input.epochs,
            open: false,
            result: None,
        }
    }
}

impl Session for DataSession {
    type Outcome = Option<Value>;

    fn step(&mut self) -> bool {
        if !self.open {
            self.activation.begin_epoch().expect("epoch");
            self.activation
                .set_input(self.input, self.value.clone())
                .expect("host input");
            self.open = true;
        }
        match self
            .activation
            .poll_export(0, &mut { self.budget })
            .expect("poll")
        {
            DataPoll::Pending => true,
            DataPoll::Ready(Some(export)) => {
                self.open = false;
                self.epochs_left -= 1;
                if self.epochs_left > 0 {
                    return true;
                }
                self.result = Some(export.outcome.expect("export value").clone());
                false
            }
            DataPoll::Ready(None) => panic!("missing export"),
        }
    }

    fn finish(self) -> Self::Outcome {
        self.result
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
