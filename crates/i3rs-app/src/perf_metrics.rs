#[cfg(all(feature = "perf_metrics", not(target_arch = "wasm32")))]
mod enabled {
    use std::collections::{HashMap, VecDeque};
    use std::sync::{LazyLock, Mutex};
    use std::time::{Duration, Instant};

    const MAX_SAMPLES_PER_SPAN: usize = 512;
    const LOG_INTERVAL: Duration = Duration::from_secs(3);

    #[derive(Default)]
    struct SpanSamples {
        values_ms: VecDeque<f64>,
    }

    impl SpanSamples {
        fn push(&mut self, value_ms: f64) {
            if self.values_ms.len() == MAX_SAMPLES_PER_SPAN {
                self.values_ms.pop_front();
            }
            self.values_ms.push_back(value_ms);
        }

        fn percentile(&self, percentile: f64) -> f64 {
            if self.values_ms.is_empty() {
                return 0.0;
            }

            let mut sorted: Vec<f64> = self.values_ms.iter().copied().collect();
            sorted.sort_by(f64::total_cmp);
            let idx = ((sorted.len() - 1) as f64 * percentile).round() as usize;
            sorted[idx.min(sorted.len() - 1)]
        }
    }

    struct Registry {
        spans: HashMap<&'static str, SpanSamples>,
        last_log: Instant,
    }

    impl Default for Registry {
        fn default() -> Self {
            Self {
                spans: HashMap::new(),
                last_log: Instant::now(),
            }
        }
    }

    static REGISTRY: LazyLock<Mutex<Registry>> = LazyLock::new(|| Mutex::new(Registry::default()));

    pub struct ScopeGuard {
        name: &'static str,
        start: Instant,
    }

    impl Drop for ScopeGuard {
        fn drop(&mut self) {
            let elapsed_ms = self.start.elapsed().as_secs_f64() * 1000.0;
            if let Ok(mut registry) = REGISTRY.lock() {
                registry
                    .spans
                    .entry(self.name)
                    .or_default()
                    .push(elapsed_ms);
            }
        }
    }

    pub fn scope(name: &'static str) -> ScopeGuard {
        ScopeGuard {
            name,
            start: Instant::now(),
        }
    }

    pub fn maybe_log_summary() {
        let Ok(mut registry) = REGISTRY.lock() else {
            return;
        };

        if registry.last_log.elapsed() < LOG_INTERVAL || registry.spans.is_empty() {
            return;
        }

        let mut names: Vec<_> = registry.spans.keys().copied().collect();
        names.sort_unstable();

        eprintln!("perf_metrics summary:");
        for name in names {
            if let Some(samples) = registry.spans.get(name) {
                eprintln!(
                    "  {name}: n={} p50={:.2}ms p95={:.2}ms",
                    samples.values_ms.len(),
                    samples.percentile(0.50),
                    samples.percentile(0.95),
                );
            }
        }

        registry.last_log = Instant::now();
    }
}

#[cfg(not(all(feature = "perf_metrics", not(target_arch = "wasm32"))))]
mod enabled {
    pub struct ScopeGuard;

    pub fn scope(_name: &'static str) -> ScopeGuard {
        ScopeGuard
    }

    pub fn maybe_log_summary() {}
}

pub use enabled::{maybe_log_summary, scope};
