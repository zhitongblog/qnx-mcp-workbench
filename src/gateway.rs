//! Action Gateway: Risk-graded safety boundary preventing AI agents from issuing
//! destructive or out-of-policy commands to mission-critical QNX 8.0 targets.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskTier {
    ReadOnly,
    SafeWrite,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateDecision {
    pub allowed: bool,
    pub tier: RiskTier,
    pub reason: String,
}

const READ_ONLY_ALLOWLIST: &[&str] = &[
    "pidin",
    "slog2info",
    "uname",
    "uptime",
    "df",
    "ls",
    "cat",
    "ps",
    "ifconfig",
    "netstat",
    "hogs",
];

const BLOCKED_PATTERNS: &[&str] = &[
    "rm ",
    "rm\t",
    "shutdown",
    "reboot",
    "halt",
    "mkfs",
    "dd ",
    "slay ",
    "kill ",
    "chmod 777",
    "/dev/mem",
    "> /",
    "|",
    ";",
    "&&",
    "||",
    "$(",
    "`",
];

pub fn evaluate_qnx_command(cmd: &str) -> GateDecision {
    let trimmed = cmd.trim();
    if trimmed.is_empty() {
        return GateDecision {
            allowed: false,
            tier: RiskTier::Blocked,
            reason: "Empty command rejected by Action Gateway".to_string(),
        };
    }

    let lower = trimmed.to_lowercase();
    for pat in BLOCKED_PATTERNS {
        if lower.contains(pat) {
            return GateDecision {
                allowed: false,
                tier: RiskTier::Blocked,
                reason: format!("Blocked by Action Gateway: forbidden token '{}'", pat.trim()),
            };
        }
    }

    let first_token = trimmed.split_whitespace().next().unwrap_or("");
    if READ_ONLY_ALLOWLIST.contains(&first_token) {
        return GateDecision {
            allowed: true,
            tier: RiskTier::ReadOnly,
            reason: format!("Allowed read-only QNX diagnostic command '{}'", first_token),
        };
    }

    if first_token == "/tmp/qnx-mcp-staging/latency_bench" {
        return GateDecision {
            allowed: true,
            tier: RiskTier::SafeWrite,
            reason: "Allowed staged benchmark binary under /tmp/qnx-mcp-staging/".to_string(),
        };
    }

    GateDecision {
        allowed: false,
        tier: RiskTier::Blocked,
        reason: format!(
            "Fail-closed: '{}' is not in the QNX Action Gateway allowlist",
            first_token
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_safe_qnx_diagnostics() {
        let d = evaluate_qnx_command("pidin info");
        assert!(d.allowed);
        assert_eq!(d.tier, RiskTier::ReadOnly);

        let d2 = evaluate_qnx_command("slog2info -w");
        assert!(d2.allowed);
    }

    #[test]
    fn blocks_destructive_and_chained_commands() {
        assert!(!evaluate_qnx_command("rm -rf /tmp/foo").allowed);
        assert!(!evaluate_qnx_command("pidin info; shutdown").allowed);
        assert!(!evaluate_qnx_command("slay -f procnto-smp-instr").allowed);
        assert!(!evaluate_qnx_command("pidin info | sh").allowed);
        assert!(!evaluate_qnx_command("unknown_binary --flag").allowed);
    }
}
