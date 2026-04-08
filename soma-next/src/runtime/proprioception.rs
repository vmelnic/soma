//! Proprioception — runtime self-monitoring and resource tracking.
//!
//! Provides a `SelfModel` snapshot of the SOMA instance's current state:
//! memory usage, CPU estimate, active sessions, loaded capabilities, uptime,
//! and peer connections. Used by the CLI `metrics` command and by the
//! distributed layer when advertising load to peers.

use std::time::{Duration, Instant};

/// A point-in-time snapshot of the SOMA instance's self-knowledge.
///
/// Unlike cumulative metrics, a `SelfModel` captures current resource
/// utilization and capability counts at the moment `snapshot()` is called.
#[derive(Debug, Clone)]
pub struct SelfModel {
    /// Current resident set size in bytes.
    pub rss_bytes: u64,
    /// Estimated CPU usage as a percentage (0.0 - 100.0).
    /// On macOS this uses `mach_task_basic_info`; on Linux it reads
    /// `/proc/self/statm`. Falls back to 0.0 on unsupported platforms.
    pub cpu_percent: f64,
    /// Number of sessions currently in a non-terminal state.
    pub active_sessions: u64,
    /// Number of packs loaded in this runtime.
    pub loaded_packs: u64,
    /// Number of skills registered across all packs.
    pub registered_skills: u64,
    /// Number of ports registered and available.
    pub registered_ports: u64,
    /// Seconds since the runtime started.
    pub uptime_seconds: u64,
    /// Number of known peer connections (from the peer registry).
    pub peer_connections: u64,
}

impl SelfModel {
    /// Compute a normalized load factor in the range 0.0 (idle) to 1.0 (fully loaded).
    ///
    /// The heuristic blends memory pressure, CPU usage, and session activity.
    /// Useful for advertising load to peers for routing decisions.
    pub fn load_factor(&self) -> f64 {
        // Memory pressure: assume 512 MB is "full load" for a SOMA instance.
        const MEM_CEILING: f64 = 512.0 * 1024.0 * 1024.0;
        let mem_load = (self.rss_bytes as f64 / MEM_CEILING).min(1.0);

        // CPU is already 0-100, normalize to 0-1.
        let cpu_load = (self.cpu_percent / 100.0).min(1.0);

        // Session pressure: assume 64 concurrent sessions is "full load".
        const SESSION_CEILING: f64 = 64.0;
        let session_load = (self.active_sessions as f64 / SESSION_CEILING).min(1.0);

        // Weighted blend: CPU and sessions matter more than raw memory.
        let load = 0.25 * mem_load + 0.40 * cpu_load + 0.35 * session_load;
        load.clamp(0.0, 1.0)
    }

    /// Format a human-readable report of the self-model.
    pub fn report(&self) -> String {
        let rss_mb = self.rss_bytes as f64 / (1024.0 * 1024.0);
        format!(
            "Self-Model:\n\
             \x20 rss:               {:.1} MB ({} bytes)\n\
             \x20 cpu:               {:.1}%\n\
             \x20 active_sessions:   {}\n\
             \x20 loaded_packs:      {}\n\
             \x20 registered_skills: {}\n\
             \x20 registered_ports:  {}\n\
             \x20 uptime:            {}s\n\
             \x20 peer_connections:  {}\n\
             \x20 load_factor:       {:.3}",
            rss_mb,
            self.rss_bytes,
            self.cpu_percent,
            self.active_sessions,
            self.loaded_packs,
            self.registered_skills,
            self.registered_ports,
            self.uptime_seconds,
            self.peer_connections,
            self.load_factor(),
        )
    }

    /// Serialize to JSON for the MCP health endpoint or API consumers.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "rss_bytes": self.rss_bytes,
            "rss_mb": (self.rss_bytes as f64 / (1024.0 * 1024.0)),
            "cpu_percent": self.cpu_percent,
            "active_sessions": self.active_sessions,
            "loaded_packs": self.loaded_packs,
            "registered_skills": self.registered_skills,
            "registered_ports": self.registered_ports,
            "uptime_seconds": self.uptime_seconds,
            "peer_connections": self.peer_connections,
            "load_factor": self.load_factor(),
        })
    }
}

// ---------------------------------------------------------------------------
// RSS measurement — platform-specific
// ---------------------------------------------------------------------------

/// Read the current RSS (Resident Set Size) in bytes.
///
/// On macOS, uses `mach_task_basic_info` via the Mach kernel API.
/// On Linux, reads `/proc/self/statm` (second field * page size).
/// Returns 0 on unsupported platforms or on failure.
pub fn current_rss_bytes() -> u64 {
    #[cfg(target_os = "macos")]
    {
        macos_rss_bytes()
    }
    #[cfg(target_os = "linux")]
    {
        linux_rss_bytes()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        0
    }
}

#[cfg(target_os = "macos")]
fn macos_rss_bytes() -> u64 {
    // Use MACH_TASK_BASIC_INFO to get current RSS (not peak).
    // SAFETY: mach_task_self() is always valid for the current task.
    // The info struct is stack-allocated and fully written by the kernel call.
    #[allow(deprecated)] // libc marks mach_task_self as deprecated in favor of mach2 crate
    unsafe {
        let task = libc::mach_task_self();
        let mut info: libc::mach_task_basic_info_data_t = std::mem::zeroed();
        let mut count: libc::mach_msg_type_number_t = libc::MACH_TASK_BASIC_INFO_COUNT;

        let kr = libc::task_info(
            task,
            libc::MACH_TASK_BASIC_INFO,
            (&raw mut info) as libc::task_info_t,
            &raw mut count,
        );

        if kr == libc::KERN_SUCCESS {
            info.resident_size as u64
        } else {
            0
        }
    }
}

#[cfg(target_os = "linux")]
fn linux_rss_bytes() -> u64 {
    // /proc/self/statm fields: size resident shared text lib data dt (in pages).
    // We want the second field (resident) multiplied by page size.
    let Ok(contents) = std::fs::read_to_string("/proc/self/statm") else {
        return 0;
    };
    let Some(resident_pages_str) = contents.split_whitespace().nth(1) else {
        return 0;
    };
    let Ok(resident_pages) = resident_pages_str.parse::<u64>() else {
        return 0;
    };
    // Page size is typically 4096, but query it properly.
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if page_size > 0 {
        resident_pages * page_size as u64
    } else {
        resident_pages * 4096
    }
}

// ---------------------------------------------------------------------------
// CPU measurement — platform-specific
// ---------------------------------------------------------------------------

/// Estimate CPU usage percentage since `reference_time`.
///
/// Computes (user_time + system_time) / wall_time * 100 as a rough
/// approximation. This gives lifetime average, not instantaneous.
/// Returns 0.0 on unsupported platforms or on failure.
pub fn cpu_percent_since(reference_time: Instant) -> f64 {
    let wall_elapsed = reference_time.elapsed();
    if wall_elapsed.as_nanos() == 0 {
        return 0.0;
    }

    let cpu_time = process_cpu_time();
    if cpu_time.as_nanos() == 0 {
        return 0.0;
    }

    let pct = (cpu_time.as_secs_f64() / wall_elapsed.as_secs_f64()) * 100.0;
    pct.clamp(0.0, 100.0 * num_cpus())
}

/// Total user + system CPU time consumed by this process.
fn process_cpu_time() -> Duration {
    // SAFETY: zeroed rusage is valid, RUSAGE_SELF is always a valid argument.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::getrusage(libc::RUSAGE_SELF, &raw mut usage) };
    if rc != 0 {
        return Duration::ZERO;
    }

    let user = Duration::new(
        usage.ru_utime.tv_sec as u64,
        usage.ru_utime.tv_usec as u32 * 1000,
    );
    let system = Duration::new(
        usage.ru_stime.tv_sec as u64,
        usage.ru_stime.tv_usec as u32 * 1000,
    );
    user + system
}

/// Number of logical CPUs (for clamping CPU percentage).
fn num_cpus() -> f64 {
    let n = unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) };
    if n > 0 { n as f64 } else { 1.0 }
}

// ---------------------------------------------------------------------------
// RuntimeCounts — injectable counts from the runtime for snapshot()
// ---------------------------------------------------------------------------

/// Capability and session counts gathered from the Runtime struct.
/// Passed into `snapshot_with_counts()` so the proprioception module
/// stays decoupled from the specific runtime types.
#[derive(Debug, Clone, Default)]
pub struct RuntimeCounts {
    pub active_sessions: u64,
    pub loaded_packs: u64,
    pub registered_skills: u64,
    pub registered_ports: u64,
    pub peer_connections: u64,
}

/// Take a full self-model snapshot using provided runtime counts
/// and the given start time.
pub fn snapshot(start_time: Instant, counts: &RuntimeCounts) -> SelfModel {
    SelfModel {
        rss_bytes: current_rss_bytes(),
        cpu_percent: cpu_percent_since(start_time),
        active_sessions: counts.active_sessions,
        loaded_packs: counts.loaded_packs,
        registered_skills: counts.registered_skills,
        registered_ports: counts.registered_ports,
        uptime_seconds: start_time.elapsed().as_secs(),
        peer_connections: counts.peer_connections,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_returns_nonzero_rss_on_supported_platforms() {
        let rss = current_rss_bytes();
        // On macOS and Linux, any running process has nonzero RSS.
        if cfg!(any(target_os = "macos", target_os = "linux")) {
            assert!(rss > 0, "RSS should be nonzero on supported platforms");
        }
    }

    #[test]
    fn snapshot_produces_valid_self_model() {
        let start = Instant::now();
        let counts = RuntimeCounts {
            active_sessions: 3,
            loaded_packs: 2,
            registered_skills: 10,
            registered_ports: 4,
            peer_connections: 1,
        };
        let model = snapshot(start, &counts);

        assert_eq!(model.active_sessions, 3);
        assert_eq!(model.loaded_packs, 2);
        assert_eq!(model.registered_skills, 10);
        assert_eq!(model.registered_ports, 4);
        assert_eq!(model.peer_connections, 1);
        // Uptime should be 0 or 1 second (test runs fast).
        assert!(model.uptime_seconds <= 1);
    }

    #[test]
    fn load_factor_idle_is_low() {
        let model = SelfModel {
            rss_bytes: 10 * 1024 * 1024, // 10 MB
            cpu_percent: 0.0,
            active_sessions: 0,
            loaded_packs: 1,
            registered_skills: 5,
            registered_ports: 2,
            uptime_seconds: 60,
            peer_connections: 0,
        };
        let load = model.load_factor();
        assert!(load < 0.1, "idle system should have low load factor, got {load}");
    }

    #[test]
    fn load_factor_busy_is_high() {
        let model = SelfModel {
            rss_bytes: 400 * 1024 * 1024, // 400 MB
            cpu_percent: 90.0,
            active_sessions: 50,
            loaded_packs: 5,
            registered_skills: 100,
            registered_ports: 20,
            uptime_seconds: 3600,
            peer_connections: 10,
        };
        let load = model.load_factor();
        assert!(load > 0.7, "busy system should have high load factor, got {load}");
    }

    #[test]
    fn load_factor_clamped_to_unit_range() {
        let model = SelfModel {
            rss_bytes: u64::MAX,
            cpu_percent: 200.0,
            active_sessions: u64::MAX,
            loaded_packs: 0,
            registered_skills: 0,
            registered_ports: 0,
            uptime_seconds: 0,
            peer_connections: 0,
        };
        let load = model.load_factor();
        assert!(
            (0.0..=1.0).contains(&load),
            "load factor should be in [0, 1], got {load}"
        );
    }

    #[test]
    fn report_format_contains_key_fields() {
        let model = SelfModel {
            rss_bytes: 50 * 1024 * 1024,
            cpu_percent: 12.5,
            active_sessions: 2,
            loaded_packs: 1,
            registered_skills: 8,
            registered_ports: 3,
            uptime_seconds: 120,
            peer_connections: 0,
        };
        let report = model.report();
        assert!(report.contains("rss:"), "report should contain rss field");
        assert!(report.contains("cpu:"), "report should contain cpu field");
        assert!(report.contains("active_sessions:"), "report should contain sessions");
        assert!(report.contains("load_factor:"), "report should contain load_factor");
        assert!(report.contains("50.0 MB"), "report should show MB conversion");
    }

    #[test]
    fn to_json_contains_all_fields() {
        let model = SelfModel {
            rss_bytes: 100_000,
            cpu_percent: 5.0,
            active_sessions: 1,
            loaded_packs: 2,
            registered_skills: 3,
            registered_ports: 4,
            uptime_seconds: 10,
            peer_connections: 0,
        };
        let json = model.to_json();
        assert_eq!(json["rss_bytes"], 100_000);
        assert_eq!(json["cpu_percent"], 5.0);
        assert_eq!(json["active_sessions"], 1);
        assert_eq!(json["loaded_packs"], 2);
        assert_eq!(json["registered_skills"], 3);
        assert_eq!(json["registered_ports"], 4);
        assert_eq!(json["uptime_seconds"], 10);
        assert_eq!(json["peer_connections"], 0);
        // load_factor should be present and a number.
        assert!(json["load_factor"].is_number());
    }

    #[test]
    fn cpu_percent_since_returns_finite_value() {
        let start = Instant::now();
        // Do a tiny bit of work so there's measurable CPU time.
        let mut sum: u64 = 0;
        for i in 0..10_000 {
            sum = sum.wrapping_add(i);
        }
        std::hint::black_box(sum);

        let pct = cpu_percent_since(start);
        assert!(pct.is_finite(), "CPU percent should be a finite number");
        assert!(pct >= 0.0, "CPU percent should be non-negative");
    }

    #[test]
    fn default_runtime_counts_is_all_zeros() {
        let counts = RuntimeCounts::default();
        assert_eq!(counts.active_sessions, 0);
        assert_eq!(counts.loaded_packs, 0);
        assert_eq!(counts.registered_skills, 0);
        assert_eq!(counts.registered_ports, 0);
        assert_eq!(counts.peer_connections, 0);
    }
}
