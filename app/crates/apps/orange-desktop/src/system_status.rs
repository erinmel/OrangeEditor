//! Low-frequency sampling, off the UI thread and only while requested.
use orange_editor_ui::SystemStatus;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};
use sysinfo::{Networks, System};

pub(super) struct Sampler {
    _stop: mpsc::Sender<()>,
    sample: Arc<Mutex<SystemStatus>>,
}

impl Sampler {
    pub(super) fn start(ctx: eframe::egui::Context) -> Option<Self> {
        let (stop, stopped) = mpsc::channel();
        let sample = Arc::new(Mutex::new(SystemStatus::default()));
        let shared = Arc::clone(&sample);
        std::thread::Builder::new()
            .name("orange-system-status".into())
            .spawn(move || {
                let mut system = System::new();
                system.refresh_cpu_usage();
                let mut networks = Networks::new_with_refreshed_list();
                let mut last = Instant::now();
                while let Err(mpsc::RecvTimeoutError::Timeout) =
                    stopped.recv_timeout(Duration::from_secs(1))
                {
                    networks.refresh(true);
                    let now = Instant::now();
                    let elapsed = now.duration_since(last);
                    last = now;
                    system.refresh_cpu_usage();
                    let (received, sent) =
                        networks.values().fold((0u64, 0u64), |(rx, tx), data| {
                            (
                                rx.saturating_add(data.received()),
                                tx.saturating_add(data.transmitted()),
                            )
                        });
                    let Ok(mut latest) = shared.lock() else {
                        break;
                    };
                    *latest = SystemStatus {
                        cpu_percent: (!system.cpus().is_empty()).then(|| system.global_cpu_usage()),
                        network_kib_per_second: (!networks.is_empty())
                            .then(|| [rate(received, elapsed), rate(sent, elapsed)]),
                    };
                    drop(latest);
                    ctx.request_repaint();
                }
            })
            .ok()?;
        Some(Self {
            _stop: stop,
            sample,
        })
    }

    pub(super) fn latest(&self) -> Option<SystemStatus> {
        self.sample.try_lock().ok().map(|status| *status)
    }
}

fn rate(bytes: u64, elapsed: Duration) -> u64 {
    let kib_per_second = u128::from(bytes) * 1000 / (1024 * elapsed.as_millis().max(1));
    u64::try_from(kib_per_second).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_rates_account_for_elapsed_time() {
        assert_eq!(rate(3072, Duration::from_millis(1500)), 2);
        assert_eq!(rate(0, Duration::from_millis(1000)), 0);
    }
}
