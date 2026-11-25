//! Deterministic QNX 8.0 diagnostic engines:
//! 1. `slog2info` log triage & root-cause attribution
//! 2. `pidin` thread priority-inversion & mutex-block detector
//! 3. Microsecond scheduling jitter analyzer (P50 / P95 / P99 / Max)

#[derive(Debug, Clone)]
pub struct DiagnosticFinding {
    pub code: &'static str,
    pub severity: &'static str,
    pub component: String,
    pub evidence: String,
    pub recommendation: &'static str,
}

#[derive(Debug, Clone)]
pub struct HealthReport {
    pub score: u32,
    pub status: &'static str,
    pub findings: Vec<DiagnosticFinding>,
}

pub fn analyze_slog2(log_text: &str) -> HealthReport {
    let mut findings = Vec::new();
    let mut penalty: u32 = 0;

    for (idx, raw) in log_text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let lower = line.to_lowercase();
        let line_no = idx + 1;

        if lower.contains("watchdog") && (lower.contains("timeout") || lower.contains("expired")) {
            penalty += 35;
            findings.push(DiagnosticFinding {
                code: "QNX_WDT_TIMEOUT",
                severity: "CRITICAL",
                component: format!("slog2:L{}", line_no),
                evidence: line.chars().take(160).collect(),
                recommendation: "Inspect blocked real-time threads holding mutexes or starved below priority 20.",
            });
        } else if lower.contains("sigsegv") || lower.contains("sigbus") || lower.contains("dumper") {
            penalty += 30;
            findings.push(DiagnosticFinding {
                code: "QNX_PROC_CRASH",
                severity: "CRITICAL",
                component: format!("slog2:L{}", line_no),
                evidence: line.chars().take(160).collect(),
                recommendation: "Collect core dump via dumper and check pointer bounds / shared memory mmap alignment.",
            });
        } else if lower.contains("enomem") || lower.contains("mmap failed") || lower.contains("out of memory") {
            penalty += 25;
            findings.push(DiagnosticFinding {
                code: "QNX_MEM_EXHAUST",
                severity: "HIGH",
                component: format!("slog2:L{}", line_no),
                evidence: line.chars().take(160).collect(),
                recommendation: "Audit typed memory partitions and check for leaked IPC channels or unfreed shm objects.",
            });
        } else if lower.contains("priority inversion") || lower.contains("mutex") && lower.contains("stall") {
            penalty += 20;
            findings.push(DiagnosticFinding {
                code: "QNX_PRIO_INVERSION",
                severity: "HIGH",
                component: format!("slog2:L{}", line_no),
                evidence: line.chars().take(160).collect(),
                recommendation: "Enable PTHREAD_PRIO_INHERIT on mutexes shared across real-time priority bands.",
            });
        } else if lower.contains("pulse") && (lower.contains("drop") || lower.contains("overflow"))
            || lower.contains("eagain")
        {
            penalty += 12;
            findings.push(DiagnosticFinding {
                code: "QNX_IPC_BACKPRESSURE",
                severity: "MEDIUM",
                component: format!("slog2:L{}", line_no),
                evidence: line.chars().take(160).collect(),
                recommendation: "Increase channel pulse queue depth or offload blocking work from MsgReceive() loop.",
            });
        }
    }

    let score = 100u32.saturating_sub(penalty);
    let status = if score >= 85 {
        "HEALTHY"
    } else if score >= 60 {
        "DEGRADED"
    } else {
        "CRITICAL"
    };

    HealthReport {
        score,
        status,
        findings,
    }
}

pub fn analyze_pidin_threads(pidin_text: &str) -> Vec<DiagnosticFinding> {
    let mut findings = Vec::new();
    for (idx, raw) in pidin_text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("pid ") {
            continue;
        }
        // Heuristic for real-time priority (>= 20r / 20f) stuck in MUTEX or REPLY
        let tokens: Vec<&str> = line.split_whitespace().collect();
        let has_rt_prio = tokens.iter().any(|t| {
            let digits: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
            let suffix = t.trim_start_matches(|c: char| c.is_ascii_digit());
            matches!(suffix, "r" | "f" | "o")
                && digits.parse::<u32>().map(|p| p >= 20).unwrap_or(false)
        });
        let blocked_state = tokens
            .iter()
            .find(|t| matches!(**t, "MUTEX" | "SEM" | "DEAD" | "STOPPED"))
            .copied();

        if has_rt_prio {
            if let Some(state) = blocked_state {
                findings.push(DiagnosticFinding {
                    code: "QNX_RT_THREAD_BLOCKED",
                    severity: if state == "DEAD" { "CRITICAL" } else { "HIGH" },
                    component: format!("pidin:L{}", idx + 1),
                    evidence: line.chars().take(160).collect(),
                    recommendation: "High-priority real-time thread is blocked; check lock holder priority and IPC reply latency.",
                });
            }
        }
    }
    findings
}

#[derive(Debug, Clone)]
pub struct LatencySummary {
    pub samples: usize,
    pub p50_us: f64,
    pub p95_us: f64,
    pub p99_us: f64,
    pub max_us: f64,
    pub sla_us: f64,
    pub passed: bool,
}

pub fn summarize_latency_us(samples: &[f64], sla_us: f64) -> LatencySummary {
    if samples.is_empty() {
        return LatencySummary {
            samples: 0,
            p50_us: 0.0,
            p95_us: 0.0,
            p99_us: 0.0,
            max_us: 0.0,
            sla_us,
            passed: false,
        };
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let pct = |p: f64| -> f64 {
        let idx = ((p / 100.0) * ((sorted.len() - 1) as f64)).round() as usize;
        sorted[idx.min(sorted.len() - 1)]
    };
    let p50 = pct(50.0);
    let p95 = pct(95.0);
    let p99 = pct(99.0);
    let max = *sorted.last().unwrap_or(&0.0);
    LatencySummary {
        samples: sorted.len(),
        p50_us: p50,
        p95_us: p95,
        p99_us: p99,
        max_us: max,
        sla_us,
        passed: p99 <= sla_us,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slog2_flags_watchdog_and_prio_inversion() {
        let sample = "\
Nov 25 10:01:02.114  motor_ctrl.104  INFO   loop initialized at 1000Hz
Nov 25 10:01:05.882  motor_ctrl.104  ERROR  mutex stall detected: priority inversion on bus_lock
Nov 25 10:01:06.001  safety_wdt.88   CRIT   watchdog timeout expired on motor_ctrl (delta=14.2ms)";
        let rep = analyze_slog2(sample);
        assert_eq!(rep.status, "CRITICAL");
        assert!(rep.score <= 50);
        assert_eq!(rep.findings.len(), 2);
        assert_eq!(rep.findings[0].code, "QNX_PRIO_INVERSION");
        assert_eq!(rep.findings[1].code, "QNX_WDT_TIMEOUT");
    }

    #[test]
    fn pidin_detects_blocked_rt_thread() {
        let sample = "\
pid tid name prio STATE
104 2 /opt/motor_ctrl 25f MUTEX 0x4801a0
201 1 /usr/sbin/sshd 10r SIGWAITINFO";
        let hits = analyze_pidin_threads(sample);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].code, "QNX_RT_THREAD_BLOCKED");
    }

    #[test]
    fn latency_summary_evaluates_sla() {
        let samples = vec![12.0, 15.0, 18.0, 19.0, 22.0, 25.0, 40.0, 180.0];
        let s = summarize_latency_us(&samples, 100.0);
        assert!(!s.passed);
        assert_eq!(s.max_us, 180.0);
    }
}
