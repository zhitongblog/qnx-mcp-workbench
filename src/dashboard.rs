//! Zero-dependency local HTTP telemetry & Action Gateway dashboard (`qnx-mcp-workbench serve`).
//! Lets judges inspect QNX 8.0 slog2 triage, blocked RT threads, microsecond jitter SLA charts,
//! and test fail-closed Action Gateway policies live in any browser.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

use crate::diagnostics::{analyze_pidin_threads, analyze_slog2, summarize_latency_us};
use crate::gateway::evaluate_qnx_command;
use crate::mcp::{escape_json, extract_json_str, get_audit_entries, record_audit};
use crate::transport::{run_target_command, TargetMode};

fn build_telemetry_json(mode: &TargetMode) -> String {
    let info = run_target_command(mode, "pidin info").unwrap_or_default();
    let slog = run_target_command(mode, "slog2info").unwrap_or_default();
    let threads = run_target_command(mode, "pidin threads").unwrap_or_default();
    let raw_lat = run_target_command(mode, "/tmp/qnx-mcp-staging/latency_bench").unwrap_or_default();

    let report = analyze_slog2(&slog);
    let rt_hits = analyze_pidin_threads(&threads);
    let samples: Vec<f64> = raw_lat
        .split(',')
        .filter_map(|s| s.trim().parse::<f64>().ok())
        .collect();
    let lat = summarize_latency_us(&samples, 100.0);

    let findings_json: Vec<String> = report
        .findings
        .iter()
        .chain(rt_hits.iter())
        .map(|f| {
            format!(
                r#"{{"code":"{}","severity":"{}","component":"{}","evidence":"{}","recommendation":"{}"}}"#,
                escape_json(f.code),
                escape_json(f.severity),
                escape_json(&f.component),
                escape_json(&f.evidence),
                escape_json(f.recommendation)
            )
        })
        .collect();

    let samples_json: Vec<String> = samples.iter().map(|v| format!("{:.1}", v)).collect();
    let audit_json: Vec<String> = get_audit_entries();

    format!(
        r#"{{"score":{},"status":"{}","pidinInfo":"{}","threads":"{}","latency":{{"p50":{:.1},"p95":{:.1},"p99":{:.1},"max":{:.1},"sla":{:.1},"passed":{},"samples":[{}]}},"findings":[{}],"audit":[{}]}}"#,
        report.score,
        escape_json(report.status),
        escape_json(&info),
        escape_json(&threads),
        lat.p50_us,
        lat.p95_us,
        lat.p99_us,
        lat.max_us,
        lat.sla_us,
        lat.passed,
        samples_json.join(","),
        findings_json.join(","),
        audit_json.join(",")
    )
}

fn send_http(mut stream: TcpStream, status: &str, content_type: &str, body: &str) {
    let resp = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status,
        content_type,
        body.len(),
        body
    );
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.flush();
}

fn handle_connection(mut stream: TcpStream, base_mode: &TargetMode) {
    let mut buf = [0u8; 8192];
    let n = match stream.read(&mut buf) {
        Ok(v) if v > 0 => v,
        _ => return,
    };
    let req = String::from_utf8_lossy(&buf[..n]);
    let first_line = req.lines().next().unwrap_or("");
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    let method = parts.first().copied().unwrap_or("GET");
    let path = parts.get(1).copied().unwrap_or("/");

    if method == "GET" && (path == "/" || path == "/index.html") {
        send_http(stream, "200 OK", "text/html; charset=utf-8", DASHBOARD_HTML);
        return;
    }

    if method == "GET" && path.starts_with("/api/telemetry") {
        let active_mode = if matches!(base_mode, TargetMode::Ssh { .. }) {
            base_mode.clone()
        } else if path.contains("scenario=fixed") {
            TargetMode::DemoFixed
        } else {
            TargetMode::Demo
        };
        let json = build_telemetry_json(&active_mode);
        send_http(stream, "200 OK", "application/json", &json);
        return;
    }

    if method == "POST" && path == "/api/exec" {
        let body = req.split("\r\n\r\n").nth(1).unwrap_or("");
        let cmd = extract_json_str(body, "command").unwrap_or_default();
        let scenario = extract_json_str(body, "scenario").unwrap_or_default();
        let active_mode = if matches!(base_mode, TargetMode::Ssh { .. }) {
            base_mode.clone()
        } else if scenario == "fixed" {
            TargetMode::DemoFixed
        } else {
            TargetMode::Demo
        };
        let decision = evaluate_qnx_command(&cmd);
        let (allowed, output) = if !decision.allowed {
            record_audit("web_gateway", "blocked", &decision.reason);
            (false, decision.reason)
        } else {
            match run_target_command(&active_mode, &cmd) {
                Ok(out) => {
                    record_audit("web_gateway", "ok", &cmd);
                    (true, out)
                }
                Err(e) => {
                    record_audit("web_gateway", "error", &e);
                    (false, e)
                }
            }
        };
        let resp = format!(
            r#"{{"allowed":{},"command":"{}","output":"{}"}}"#,
            allowed,
            escape_json(&cmd),
            escape_json(&output)
        );
        send_http(stream, "200 OK", "application/json", &resp);
        return;
    }

    send_http(stream, "404 Not Found", "text/plain", "Not Found");
}

pub fn serve_http(mode: TargetMode, port: u16) {
    let addr = format!("127.0.0.1:{}", port);
    let listener = TcpListener::bind(&addr).expect("failed to bind HTTP dashboard port");
    println!("QNX 8.0 MCP Workbench Live Dashboard running at http://{}", addr);
    println!("Press Ctrl+C to stop.");
    for stream in listener.incoming().flatten() {
        handle_connection(stream, &mode);
    }
}

const DASHBOARD_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8" />
<title>QNX 8.0 MCP Diagnostic Workbench</title>
<style>
  :root { --bg:#0b1120; --card:#111c33; --border:#24324d; --text:#e2e8f0; --muted:#94a3b8; --green:#22c55e; --amber:#f59e0b; --red:#ef4444; --cyan:#38bdf8; }
  * { box-sizing:border-box; }
  body { margin:0; font-family:system-ui,-apple-system,sans-serif; background:var(--bg); color:var(--text); padding:18px; }
  header { display:flex; justify-content:space-between; align-items:center; flex-wrap:wrap; gap:10px; margin-bottom:16px; border-bottom:1px solid var(--border); padding-bottom:12px; }
  .grid { display:grid; grid-template-columns:1.1fr 1fr; gap:14px; }
  .card { background:var(--card); border:1px solid var(--border); border-radius:10px; padding:14px; }
  h2 { margin:0 0 10px 0; font-size:13px; text-transform:uppercase; letter-spacing:.06em; color:var(--muted); }
  .score { font-size:32px; font-weight:800; }
  .HEALTHY { color:var(--green); } .DEGRADED { color:var(--amber); } .CRITICAL { color:var(--red); }
  .btn { background:#1e293b; color:var(--text); border:1px solid var(--border); border-radius:6px; padding:6px 11px; cursor:pointer; font-size:12px; }
  .btn.active { background:#0284c7; border-color:#0284c7; color:#fff; font-weight:600; }
  .finding { padding:8px 10px; border-left:3px solid var(--red); background:#0b1224; border-radius:6px; margin-bottom:8px; font-size:12.5px; }
  .finding.HIGH { border-left-color:var(--amber); } .finding.MEDIUM { border-left-color:var(--cyan); }
  pre { background:#091021; border:1px solid var(--border); border-radius:6px; padding:10px; font-size:11.5px; overflow:auto; max-height:190px; white-space:pre-wrap; }
  input { background:#091021; color:var(--text); border:1px solid var(--border); border-radius:6px; padding:7px 10px; width:72%; font-family:monospace; }
  canvas { width:100%; background:#091021; border-radius:6px; border:1px solid var(--border); }
</style>
</head>
<body>
<header>
  <div><strong style="font-size:18px;">QNX 8.0 AI-Native MCP Diagnostic Workbench</strong> <span style="color:var(--muted);font-size:12px;">Track 4 Host Tooling + Action Gateway</span></div>
  <div>
    <button id="btn-buggy" class="btn active" onclick="setScenario('buggy')">1. Priority Inversion Incident</button>
    <button id="btn-fixed" class="btn" onclick="setScenario('fixed')">2. After PTHREAD_PRIO_INHERIT Fix</button>
  </div>
</header>
<div class="grid">
  <div class="card">
    <h2>Kernel Health &amp; Deterministic slog2 / pidin Triage</h2>
    <div style="display:flex;justify-content:space-between;align-items:center;margin-bottom:10px;">
      <div><span id="score" class="score">—</span> <span id="status" style="font-weight:700;margin-left:8px;"></span></div>
      <div id="lat-badge" style="font-size:12px;padding:4px 10px;border-radius:6px;background:#17233b;"></div>
    </div>
    <div id="findings"></div>
    <h2>Live pidin threads</h2>
    <pre id="threads"></pre>
  </div>
  <div class="card">
    <h2>Real-Time Scheduling Jitter (us vs 100us SLA)</h2>
    <canvas id="chart" width="520" height="190"></canvas>
    <h2 style="margin-top:14px;">Fail-Closed Action Gateway Live Tester</h2>
    <div style="display:flex;gap:6px;margin-bottom:8px;">
      <input id="cmd-input" value="slay -f procnto-smp-instr" />
      <button class="btn active" onclick="runCmd()">Execute via Gateway</button>
    </div>
    <div style="display:flex;gap:6px;margin-bottom:8px;flex-wrap:wrap;">
      <button class="btn" onclick="tryCmd('pidin info')">Allow: pidin info</button>
      <button class="btn" onclick="tryCmd('slay -f procnto-smp-instr')">Block: slay procnto</button>
      <button class="btn" onclick="tryCmd('pidin info; rm -rf /')">Block: shell chaining</button>
    </div>
    <pre id="cmd-out">Click any test command to see the Action Gateway decision + audit entry.</pre>
    <h2>Tamper-Evident JSONL Audit Stream</h2>
    <pre id="audit"></pre>
  </div>
</div>
<script>
let scenario = 'buggy';
function setScenario(s) {
  scenario = s;
  document.getElementById('btn-buggy').className = 'btn' + (s==='buggy'?' active':'');
  document.getElementById('btn-fixed').className = 'btn' + (s==='fixed'?' active':'');
  refresh();
}
function tryCmd(c) { document.getElementById('cmd-input').value = c; runCmd(); }
async function runCmd() {
  const cmd = document.getElementById('cmd-input').value;
  const r = await fetch('/api/exec', { method:'POST', headers:{'Content-Type':'application/json'}, body: JSON.stringify({ command: cmd, scenario }) });
  const d = await r.json();
  document.getElementById('cmd-out').textContent = (d.allowed ? '[ALLOWED]\n' : '[BLOCKED BY ACTION GATEWAY]\n') + d.output;
  refresh();
}
function drawChart(lat) {
  const c = document.getElementById('chart'), ctx = c.getContext('2d');
  ctx.clearRect(0,0,c.width,c.height);
  const samples = lat.samples || [];
  if (!samples.length) return;
  const maxVal = Math.max(lat.sla * 1.25, ...samples, 50);
  const pad = 24, w = c.width - pad*2, h = c.height - pad*2;
  const slaY = pad + h - (lat.sla / maxVal) * h;
  ctx.strokeStyle = '#ef4444'; ctx.setLineDash([5,4]); ctx.beginPath(); ctx.moveTo(pad, slaY); ctx.lineTo(c.width-pad, slaY); ctx.stroke(); ctx.setLineDash([]);
  ctx.fillStyle = '#fca5a5'; ctx.font = '11px sans-serif'; ctx.fillText('SLA ' + lat.sla + 'us', c.width - 82, slaY - 4);
  const bw = (w / samples.length) * 0.72;
  samples.forEach((v, i) => {
    const x = pad + i * (w / samples.length) + 4;
    const bh = (v / maxVal) * h;
    ctx.fillStyle = v > lat.sla ? '#ef4444' : '#38bdf8';
    ctx.fillRect(x, pad + h - bh, bw, bh);
    ctx.fillStyle = '#cbd5e1'; ctx.font = '10px monospace'; ctx.fillText(v.toFixed(0), x + 2, pad + h - bh - 4);
  });
}
async function refresh() {
  const r = await fetch('/api/telemetry?scenario=' + scenario);
  const d = await r.json();
  const sc = document.getElementById('score'); sc.textContent = d.score + '/100'; sc.className = 'score ' + d.status;
  const st = document.getElementById('status'); st.textContent = d.status; st.className = d.status;
  document.getElementById('lat-badge').textContent = `P50=${d.latency.p50}us | P99=${d.latency.p99}us | ${d.latency.passed ? 'SLA PASS' : 'SLA BREACH'}`;
  document.getElementById('threads').textContent = d.threads;
  document.getElementById('findings').innerHTML = d.findings.length
    ? d.findings.map(f => `<div class="finding ${f.severity}"><strong>[${f.severity}] ${f.code}</strong> (${f.component})<div>${f.evidence}</div><div style="color:#7dd3fc;margin-top:3px;">Fix: ${f.recommendation}</div></div>`).join('')
    : '<div class="finding" style="border-left-color:#22c55e;">All deterministic slog2 &amp; RT thread checks passed (0 anomalies).</div>';
  document.getElementById('audit').textContent = d.audit.length ? d.audit.join('\n') : '(no commands executed yet)';
  drawChart(d.latency);
}
refresh();
</script>
</body>
</html>"#;
