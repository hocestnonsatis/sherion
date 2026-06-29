//! Lightweight profiling hooks built on `tracing`.
//!
//! Enable with `RUST_LOG=sherion=trace` or the `profiler` cargo feature for
//! structured span names around hot paths.

use std::time::Instant;

/// RAII span around a hot path; emits a trace event with elapsed time on drop.
pub struct ScopedTimer {
    label: &'static str,
    started: Instant,
}

impl ScopedTimer {
    pub fn new(label: &'static str) -> Self {
        tracing::trace!(target: "sherion::profiler", %label, "begin");
        Self {
            label,
            started: Instant::now(),
        }
    }
}

impl Drop for ScopedTimer {
    fn drop(&mut self) {
        let elapsed_ms = self.started.elapsed().as_secs_f64() * 1000.0;
        tracing::trace!(
            target: "sherion::profiler",
            label = self.label,
            elapsed_ms,
            "end"
        );
    }
}

#[macro_export]
macro_rules! profile_scope {
    ($label:expr) => {
        let _profile_scope = $crate::profiler::ScopedTimer::new($label);
    };
}

#[cfg(test)]
mod tests {
    use super::ScopedTimer;

    #[test]
    fn scoped_timer_constructs() {
        let _timer = ScopedTimer::new("test_scope");
    }
}
