use std::time::{Duration, Instant};

/// Non-Unix busy detection contract: a tab is "busy" when PTY output arrived
/// within the configured window. This is a best-effort activity heuristic, not
/// foreground-process detection (see Unix `tcgetpgrp` in `PtySession::is_busy`).
#[cfg_attr(unix, allow(dead_code))]
pub fn recent_output_is_busy(
    last_output_at: Option<Instant>,
    now: Instant,
    heuristic_ms: u64,
) -> bool {
    let window = Duration::from_millis(heuristic_ms.max(1));
    last_output_at.is_some_and(|at| now.duration_since(at) < window)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_output_means_idle() {
        let now = Instant::now();
        assert!(!recent_output_is_busy(None, now, 500));
    }

    #[test]
    fn recent_output_within_window_is_busy() {
        let now = Instant::now();
        let last = now - Duration::from_millis(100);
        assert!(recent_output_is_busy(Some(last), now, 500));
    }

    #[test]
    fn output_outside_window_is_idle() {
        let now = Instant::now();
        let last = now - Duration::from_millis(600);
        assert!(!recent_output_is_busy(Some(last), now, 500));
    }

    #[test]
    fn zero_heuristic_ms_clamps_to_one_millisecond() {
        let now = Instant::now();
        let last = now - Duration::from_micros(500);
        assert!(recent_output_is_busy(Some(last), now, 0));
        let older = now - Duration::from_millis(2);
        assert!(!recent_output_is_busy(Some(older), now, 0));
    }

    #[test]
    fn boundary_at_exact_window_is_idle() {
        let now = Instant::now();
        let last = now - Duration::from_millis(500);
        assert!(!recent_output_is_busy(Some(last), now, 500));
    }
}
