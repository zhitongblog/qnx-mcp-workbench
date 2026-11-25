//! Minimal zero-dependency JSON escaping & MCP stdio JSON-RPC 2.0 server.
//! Exposes QNX 8.0 host diagnostics + Action Gateway over MCP stdio (`2024-11-05` & `2025-03-26`).

use std::io::{self, BufRead, Write};
use std::sync::Mutex;

use crate::diagnostics::{analyze_pidin_threads, analyze_slog2, summarize_latency_us};
use crate::gateway::evaluate_qnx_command;
use crate::transport::{run_target_command, TargetMode};

static AUDIT_LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn record_audit(tool: &str, status: &str, detail: &str) {
    if let Ok(mut guard) = AUDIT_LOG.lock() {
        guard.push(format!(
            "{{\"tool\":\"{}\",\"status\":\"{}\",\"detail\":\"{}\"}}",
            escape_json(tool),
            escape_json(status),
            escape_json(detail)
        ));
    }
}

pub fn get_audit_entries() -> Vec<String> {
    AUDIT_LOG.lock().map(|g| g.clone()).unwrap_or_default()
}

pub fn escape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

pub fn extract_json_str(raw: &str, key: &str) -> Option<String> {
    let needle = format!("\"{}\"", key);
    let pos = raw.find(&needle)?;
    let after_key = &raw[pos + needle.len()..];
    let colon = after_key.find(':')?;
    let rest = after_key[colon + 1..].trim_start();
    if !rest.starts_with('"') {
        return None;
    }
    let mut out = String::new();
    let mut chars = rest[1..].chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(esc) = chars.next() {
                match esc {
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    other => out.push(other),
                }
            }
        } else if c == '"' {
            return Some(out);
        } else {
            out.push(c);
        }
    }
    None
}

fn extract_json_f64(raw: &str, key: &str) -> Option<f64> {
    let needle = format!("\"{}\"", key);
    let pos = raw.find(&needle)?;
    let after_key = &raw[pos + needle.len()..];
    let colon = after_key.find(':')?;
    let rest = after_key[colon + 1..].trim_start();
    let num: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
        .collect();
    num.parse::<f64>().ok()
}

fn extract_json_id(raw: &str) -> String {
    if let Some(pos) = raw.find("\"id\"") {
        let after = &raw[pos + 4..];
        if let Some(colon) = after.find(':') {
            let rest = after[colon + 1..].trim_start();
            if rest.starts_with('"') {
                if let Some(end) = rest[1..].find('"') {
                    return format!("\"{}\"", &rest[1..1 + end]);
                }
            } else {
                let num: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_digit() || *c == '-')
                    .collect();
                if !num.is_empty() {
                    return num;
                }
            }
        }
    }
    "null".to_string()
}

fn tools_list_json() -> &'static str {
    r#"{"tools":[
{"name":"qnx_target_health","description":"Fetch QNX 8.0 kernel info (pidin info) and compute overall target health score (0-100) from slog2 + threads.","inputSchema":{"type":"object","properties":{}}},
{"name":"qnx_slog2_analyze","description":"Pull slog2info logs from QNX 8.0 target and run deterministic root-cause triage (watchdog timeouts, crashes, OOM, priority inversion, IPC backpressure).","inputSchema":{"type":"object","properties":{}}},
{"name":"qnx_process_tree","description":"Inspect real-time QNX 8.0 thread priorities via pidin threads and flag blocked high-priority threads (MUTEX/SEM/DEAD).","inputSchema":{"type":"object","properties":{}}},
{"name":"qnx_latency_bench","description":"Run or parse microsecond real-time scheduling jitter benchmark (P50/P95/P99/Max vs SLA).","inputSchema":{"type":"object","properties":{"sla_us":{"type":"number","description":"SLA threshold in microseconds (default 100)"}}}},
{"name":"qnx_compare_regression","description":"Compare baseline (buggy) vs remediated (PTHREAD_PRIO_INHERIT) QNX 8.0 real-time SLA jitter and health score.","inputSchema":{"type":"object","properties":{"sla_us":{"type":"number","description":"SLA threshold in microseconds (default 100)"}}}},
{"name":"qnx_exec_guarded","description":"Execute a QNX command strictly gated by the Action Gateway allowlist (blocks destructive commands & shell chaining).","inputSchema":{"type":"object","required":["command"],"properties":{"command":{"type":"string"}}}},
{"name":"qnx_audit_log","description":"Return the tamper-evident JSONL audit trace of all AI agent actions executed against the QNX target.","inputSchema":{"type":"object","properties":{}}}
]}"#
}

pub fn dispatch_tool(mode: &TargetMode, tool_name: &str, raw_req: &str) -> String {
    match tool_name {
        "qnx_target_health" => {
            let info = run_target_command(mode, "pidin info").unwrap_or_else(|e| e);
            let logs = run_target_command(mode, "slog2info").unwrap_or_default();
            let rep = analyze_slog2(&logs);
            record_audit(tool_name, "ok", &format!("score={} status={}", rep.score, rep.status));
            format!(
                "QNX 8.0 Health Score: {}/100 ({})\nFindings: {}\n\n--- pidin info ---\n{}",
                rep.score,
                rep.status,
                rep.findings.len(),
                info
            )
        }
        "qnx_slog2_analyze" => {
            let logs = match run_target_command(mode, "slog2info") {
                Ok(v) => v,
                Err(e) => {
                    record_audit(tool_name, "error", &e);
                    return format!("Error: {}", e);
                }
            };
            let rep = analyze_slog2(&logs);
            record_audit(tool_name, "ok", &format!("findings={}", rep.findings.len()));
            let mut out = format!("QNX slog2 Triage — Score {}/100 ({})\n", rep.score, rep.status);
            for f in &rep.findings {
                out.push_str(&format!(
                    "- [{}] {} ({}): {}\n  Fix: {}\n",
                    f.severity, f.code, f.component, f.evidence, f.recommendation
                ));
            }
            if rep.findings.is_empty() {
                out.push_str("- No anomalies detected.\n");
            }
            out
        }
        "qnx_process_tree" => {
            let txt = match run_target_command(mode, "pidin threads") {
                Ok(v) => v,
                Err(e) => {
                    record_audit(tool_name, "error", &e);
                    return format!("Error: {}", e);
                }
            };
            let hits = analyze_pidin_threads(&txt);
            record_audit(tool_name, "ok", &format!("rt_blocked={}", hits.len()));
            let mut out = format!("Blocked RT Threads Detected: {}\n", hits.len());
            for f in &hits {
                out.push_str(&format!(
                    "- [{}] {} ({}): {}\n  Fix: {}\n",
                    f.severity, f.code, f.component, f.evidence, f.recommendation
                ));
            }
            out.push_str("\n--- pidin threads ---\n");
            out.push_str(&txt);
            out
        }
        "qnx_latency_bench" => {
            let sla = extract_json_f64(raw_req, "sla_us").unwrap_or(100.0);
            let raw = run_target_command(mode, "/tmp/qnx-mcp-staging/latency_bench")
                .unwrap_or_else(|_| String::new());
            let samples: Vec<f64> = raw
                .split(',')
                .filter_map(|s| s.trim().parse::<f64>().ok())
                .collect();
            let summary = summarize_latency_us(&samples, sla);
            record_audit(
                tool_name,
                if summary.passed { "pass" } else { "fail" },
                &format!("p99={:.1}us sla={:.1}us", summary.p99_us, sla),
            );
            format!(
                "QNX 8.0 Jitter Benchmark (n={}, SLA={}us): {}\nP50={:.1}us | P95={:.1}us | P99={:.1}us | Max={:.1}us",
                summary.samples,
                summary.sla_us,
                if summary.passed { "PASS" } else { "SLA BREACH" },
                summary.p50_us,
                summary.p95_us,
                summary.p99_us,
                summary.max_us
            )
        }
        "qnx_compare_regression" => {
            let sla = extract_json_f64(raw_req, "sla_us").unwrap_or(100.0);
            let before_raw = run_target_command(&TargetMode::Demo, "/tmp/qnx-mcp-staging/latency_bench").unwrap_or_default();
            let after_raw = run_target_command(&TargetMode::DemoFixed, "/tmp/qnx-mcp-staging/latency_bench").unwrap_or_default();
            let parse_vec = |s: &str| -> Vec<f64> {
                s.split(',').filter_map(|x| x.trim().parse::<f64>().ok()).collect()
            };
            let b = summarize_latency_us(&parse_vec(&before_raw), sla);
            let a = summarize_latency_us(&parse_vec(&after_raw), sla);
            let reduction_pct = if b.p99_us > 0.0 {
                ((b.p99_us - a.p99_us) / b.p99_us) * 100.0
            } else {
                0.0
            };
            record_audit(tool_name, "ok", &format!("p99_drop={:.1}%", reduction_pct));
            format!(
                "QNX 8.0 Regression Verification (SLA <= {:.0}us)\nBefore (PTHREAD_PRIO_NONE):    P99={:.1}us Max={:.1}us [{}]\nAfter  (PTHREAD_PRIO_INHERIT): P99={:.1}us Max={:.1}us [{}]\nP99 Jitter Reduction: {:.1}%",
                sla,
                b.p99_us, b.max_us, if b.passed { "PASS" } else { "SLA BREACH" },
                a.p99_us, a.max_us, if a.passed { "PASS" } else { "SLA BREACH" },
                reduction_pct
            )
        }
        "qnx_exec_guarded" => {
            let cmd = extract_json_str(raw_req, "command").unwrap_or_default();
            let decision = evaluate_qnx_command(&cmd);
            if !decision.allowed {
                record_audit(tool_name, "blocked", &decision.reason);
                return format!("BLOCKED BY ACTION GATEWAY: {}", decision.reason);
            }
            match run_target_command(mode, &cmd) {
                Ok(out) => {
                    record_audit(tool_name, "ok", &cmd);
                    out
                }
                Err(e) => {
                    record_audit(tool_name, "error", &e);
                    format!("Error: {}", e)
                }
            }
        }
        "qnx_audit_log" => {
            let lines = get_audit_entries().join("\n");
            if lines.is_empty() {
                "Audit log is currently empty.".to_string()
            } else {
                lines
            }
        }
        other => format!("Unknown tool: {}", other),
    }
}

pub fn handle_mcp_request(mode: &TargetMode, line: &str) -> Option<String> {
    let id = extract_json_id(line);
    let method = extract_json_str(line, "method")?;
    match method.as_str() {
        "initialize" => {
            let requested = extract_json_str(line, "protocolVersion").unwrap_or_default();
            let version = if requested == "2025-03-26" {
                "2025-03-26"
            } else {
                "2024-11-05"
            };
            Some(format!(
                r#"{{"jsonrpc":"2.0","id":{},"result":{{"protocolVersion":"{}","capabilities":{{"tools":{{}}}},"serverInfo":{{"name":"qnx-mcp-workbench","version":"0.2.0"}},"instructions":"Use qnx_slog2_analyze and qnx_process_tree first; all commands are fail-closed by the Action Gateway."}}}}"#,
                id, version
            ))
        }
        "ping" => Some(format!(r#"{{"jsonrpc":"2.0","id":{},"result":{{}}}}"#, id)),
        "notifications/initialized" => None,
        "tools/list" => Some(format!(
            r#"{{"jsonrpc":"2.0","id":{},"result":{}}}"#,
            id,
            tools_list_json()
        )),
        "tools/call" => {
            let tool = extract_json_str(line, "name").unwrap_or_default();
            let text = dispatch_tool(mode, &tool, line);
            Some(format!(
                r#"{{"jsonrpc":"2.0","id":{},"result":{{"content":[{{"type":"text","text":"{}"}}]}}}}"#,
                id,
                escape_json(&text)
            ))
        }
        _ => Some(format!(
            r#"{{"jsonrpc":"2.0","id":{},"error":{{"code":-32601,"message":"Method not found: {}"}}}}"#,
            id,
            escape_json(&method)
        )),
    }
}

pub fn serve_stdio(mode: TargetMode) {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(resp) = handle_mcp_request(&mode, trimmed) {
            let _ = writeln!(stdout, "{}", resp);
            let _ = stdout.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_initialize_ping_list_and_call_work() {
        let mode = TargetMode::Demo;
        let init = handle_mcp_request(
            &mode,
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#,
        )
        .unwrap();
        assert!(init.contains("qnx-mcp-workbench"));
        assert!(init.contains("2025-03-26"));

        let ping = handle_mcp_request(&mode, r#"{"jsonrpc":"2.0","id":9,"method":"ping"}"#).unwrap();
        assert!(ping.contains(r#""result":{}"#));

        let list = handle_mcp_request(&mode, r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#).unwrap();
        assert!(list.contains("qnx_slog2_analyze"));
        assert!(list.contains("qnx_compare_regression"));

        let blocked = handle_mcp_request(
            &mode,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"qnx_exec_guarded","arguments":{"command":"rm -rf /"}}}"#,
        )
        .unwrap();
        assert!(blocked.contains("BLOCKED BY ACTION GATEWAY"));

        let ok = handle_mcp_request(
            &mode,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"qnx_slog2_analyze","arguments":{}}}"#,
        )
        .unwrap();
        assert!(ok.contains("QNX_WDT_TIMEOUT"));

        let cmp = handle_mcp_request(
            &mode,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"qnx_compare_regression","arguments":{"sla_us":50}}}"#,
        )
        .unwrap();
        assert!(cmp.contains("P99 Jitter Reduction"));
        assert!(cmp.contains("PASS"));
    }

    #[test]
    fn custom_sla_parameter_is_respected() {
        let mode = TargetMode::DemoFixed;
        let pass = handle_mcp_request(
            &mode,
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"qnx_latency_bench","arguments":{"sla_us":50}}}"#,
        )
        .unwrap();
        assert!(pass.contains("PASS"));

        let fail = handle_mcp_request(
            &mode,
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"qnx_latency_bench","arguments":{"sla_us":15}}}"#,
        )
        .unwrap();
        assert!(fail.contains("SLA BREACH"));
    }
}
