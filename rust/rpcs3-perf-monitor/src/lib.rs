//! `rpcs3-perf-monitor` — Rust port of `rpcs3/Emu/perf_monitor.cpp`.
//!
//! RPCS3's background thread that samples CPU+RAM every 500ms and logs
//! a summary. The actual sleep loop depends on `thread_ctrl` (RPCS3's
//! own primitive); what's portable is the decision policy (*when* to
//! log) and the message formatter.
//!
//! Frozen from cpp:14..17:
//!
//! - `UPDATE_INTERVAL_US = 500_000` — poll cadence.
//! - `LOG_INTERVAL_US_MAX = 10_000_000` — at most every 10s normally.
//! - `LOG_INTERVAL_US_MIN = 500_000` — as frequent as 500ms when RAM
//!   is growing fast.
//! - `LOG_MEM_INCREASE = 50 * 1024 * 1024` — threshold that tightens
//!   the log cadence to min.
//! - `LOGGED_PAUSE_LIMIT = 2` — how many paused-state logs before going
//!   quiet (cpp:62..67 — "two times so paused state can still be
//!   debugged").
//!
//! Also provides `format_log_message(...)` that matches cpp:73..88 byte
//! for byte.

use core::fmt::Write as _;

pub const UPDATE_INTERVAL_US: u64 = 500_000;
pub const LOG_INTERVAL_US_MAX: u64 = 10_000_000;
pub const LOG_INTERVAL_US_MIN: u64 = 500_000;
pub const LOG_MEM_INCREASE: u64 = 50 * 1024 * 1024;

/// Cpp:69..70 allows exactly 2 paused-state log emissions before going quiet.
pub const LOGGED_PAUSE_LIMIT: u32 = 2;

/// Cpp:43 — choose log interval based on whether RAM is growing fast.
#[must_use]
pub const fn choose_log_interval(mem_increase: u64) -> u64 {
    if mem_increase >= LOG_MEM_INCREASE {
        LOG_INTERVAL_US_MIN
    } else {
        LOG_INTERVAL_US_MAX
    }
}

/// Cpp:40..41 — saturating subtract for tracking memory growth.
#[must_use]
pub const fn memory_increase(current: u64, prior_max: u64) -> u64 {
    if current >= prior_max { current - prior_max } else { 0 }
}

/// Cpp:45 — whether the current elapsed window warrants a log emission.
#[must_use]
pub const fn should_log(elapsed_us: u64, log_interval_us: u64, aborting: bool) -> bool {
    elapsed_us >= log_interval_us || aborting
}

/// Whether to skip a paused-state log (cpp:60..70). After logging
/// `LOGGED_PAUSE_LIMIT` times, further paused samples are skipped until
/// emulation resumes.
#[must_use]
pub const fn should_skip_paused_log(is_paused: bool, logged_pause: u32) -> bool {
    is_paused && logged_pause >= LOGGED_PAUSE_LIMIT
}

/// Cpp:53..58 — if unpaused OR pause_time changed, reset the
/// `logged_pause` counter and stash the new `last_pause_time`.
#[must_use]
pub fn reset_pause_tracking(
    is_paused: bool,
    pause_time: u64,
    last_pause_time: u64,
) -> Option<(u32, u64)> {
    if !is_paused || last_pause_time != pause_time {
        Some((0, pause_time))
    } else {
        None
    }
}

/// Format the CPU/RAM message exactly like cpp:73..88.
///
/// Produces strings like:
///   `"CPU Usage: Total: 42.3%"` (no cores, no RAM),
///   `"CPU Usage: Total: 42.3%, Cores: 50.0%, 34.6%"` (cores only),
///   `"CPU Usage: Total: 42.3%, RAM Usage: 512MB (Peak: 800MB)"` (RAM only),
///   `"CPU Usage: Total: 42.3%, Cores: 50.0%, 34.6%, RAM Usage: ..."` (both).
///
/// `max_memory_usage == 0` suppresses the RAM clause (matches cpp:85).
#[must_use]
pub fn format_log_message(
    total_usage: f64,
    per_core_usage: &[f64],
    current_mem_bytes: u64,
    max_memory_bytes: u64,
) -> String {
    let mut msg = String::new();
    write!(&mut msg, "CPU Usage: Total: {:.1}%", total_usage).unwrap();

    if !per_core_usage.is_empty() {
        msg.push_str(", Cores:");
    }

    for (i, core) in per_core_usage.iter().enumerate() {
        // cpp uses "%s %.1f%%" with " " if i>0 else "," — but reading
        // closer (cpp:82): `"%s %.1f%%", i > 0 ? "," : ""`. That emits a
        // leading ", " only between cores, and the very first core has
        // no prefix. After the ", Cores:" header, each core is `" X%"`
        // with a "," only before non-first cores.
        let prefix = if i > 0 { "," } else { "" };
        write!(&mut msg, "{prefix} {:.1}%", core).unwrap();
    }

    if max_memory_bytes > 0 {
        write!(
            &mut msg,
            ", RAM Usage: {}MB (Peak: {}MB)",
            current_mem_bytes / (1024 * 1024),
            max_memory_bytes / (1024 * 1024),
        )
        .unwrap();
    }

    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_match_cpp() {
        assert_eq!(UPDATE_INTERVAL_US, 500_000);
        assert_eq!(LOG_INTERVAL_US_MAX, 10_000_000);
        assert_eq!(LOG_INTERVAL_US_MIN, 500_000);
        assert_eq!(LOG_MEM_INCREASE, 50 * 1024 * 1024);
        assert_eq!(LOGGED_PAUSE_LIMIT, 2);
    }

    #[test]
    fn choose_log_interval_threshold_flip() {
        // Below threshold → max interval.
        assert_eq!(choose_log_interval(0), LOG_INTERVAL_US_MAX);
        assert_eq!(choose_log_interval(LOG_MEM_INCREASE - 1), LOG_INTERVAL_US_MAX);
        // At/above → min interval.
        assert_eq!(choose_log_interval(LOG_MEM_INCREASE), LOG_INTERVAL_US_MIN);
        assert_eq!(choose_log_interval(LOG_MEM_INCREASE + 1), LOG_INTERVAL_US_MIN);
    }

    #[test]
    fn memory_increase_saturating() {
        assert_eq!(memory_increase(1000, 500), 500);
        assert_eq!(memory_increase(500, 1000), 0);
        assert_eq!(memory_increase(1000, 1000), 0);
    }

    #[test]
    fn should_log_elapsed_or_aborting() {
        assert!(!should_log(0, LOG_INTERVAL_US_MAX, false));
        assert!(should_log(LOG_INTERVAL_US_MAX, LOG_INTERVAL_US_MAX, false));
        assert!(should_log(LOG_INTERVAL_US_MAX + 1, LOG_INTERVAL_US_MAX, false));
        // Aborting fires regardless.
        assert!(should_log(0, LOG_INTERVAL_US_MAX, true));
    }

    #[test]
    fn should_skip_paused_log_after_two_emissions() {
        assert!(!should_skip_paused_log(false, 0));
        assert!(!should_skip_paused_log(false, LOGGED_PAUSE_LIMIT));
        assert!(!should_skip_paused_log(true, 0));
        assert!(!should_skip_paused_log(true, 1));
        assert!(should_skip_paused_log(true, 2));
        assert!(should_skip_paused_log(true, 99));
    }

    #[test]
    fn reset_pause_tracking_paths() {
        // Unpaused → reset.
        assert_eq!(reset_pause_tracking(false, 100, 99), Some((0, 100)));
        // Paused + same pause_time → no reset.
        assert_eq!(reset_pause_tracking(true, 100, 100), None);
        // Paused + different pause_time → reset (resumed+repaused).
        assert_eq!(reset_pause_tracking(true, 200, 100), Some((0, 200)));
    }

    #[test]
    fn format_log_total_only() {
        let msg = format_log_message(42.3, &[], 0, 0);
        assert_eq!(msg, "CPU Usage: Total: 42.3%");
    }

    #[test]
    fn format_log_with_cores() {
        let msg = format_log_message(42.3, &[50.0, 34.6], 0, 0);
        assert_eq!(msg, "CPU Usage: Total: 42.3%, Cores: 50.0%, 34.6%");
    }

    #[test]
    fn format_log_with_cores_and_ram() {
        let msg = format_log_message(50.5, &[25.0, 75.0], 512 * 1024 * 1024, 800 * 1024 * 1024);
        assert_eq!(msg, "CPU Usage: Total: 50.5%, Cores: 25.0%, 75.0%, RAM Usage: 512MB (Peak: 800MB)");
    }

    #[test]
    fn format_log_ram_only_skipped_when_max_zero() {
        let msg = format_log_message(10.0, &[], 999_999_999, 0);
        assert_eq!(msg, "CPU Usage: Total: 10.0%");
    }

    #[test]
    fn format_log_floating_point_precision_one_decimal() {
        let msg = format_log_message(0.0001, &[], 0, 0);
        assert_eq!(msg, "CPU Usage: Total: 0.0%");
        let msg = format_log_message(99.99, &[], 0, 0);
        assert_eq!(msg, "CPU Usage: Total: 100.0%");
    }
}
