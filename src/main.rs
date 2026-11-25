mod dashboard;
mod diagnostics;
mod gateway;
mod mcp;
mod transport;

use std::env;

use diagnostics::{analyze_pidin_threads, analyze_slog2, summarize_latency_us};
use transport::{run_target_command, TargetMode};

fn parse_args(args: &[String]) -> (TargetMode, u16) {
    let mut use_ssh = false;
    let mut fixed = false;
    let mut host = "127.0.0.1".to_string();
    let mut ssh_port: u16 = 2222;
    let mut http_port: u16 = 8088;
    let mut user = "qnxuser".to_string();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--ssh" => use_ssh = true,
            "--fixed" => fixed = true,
            "--host" if i + 1 < args.len() => {
                host = args[i + 1].clone();
                i += 1;
            }
            "--port" if i + 1 < args.len() => {
                let p = args[i + 1].parse().unwrap_or(2222);
                if args.first().map(|s| s.as_str()) == Some("serve") && !use_ssh {
                    http_port = p;
                } else {
                    ssh_port = p;
                }
                i += 1;
            }
            "--http-port" if i + 1 < args.len() => {
                http_port = args[i + 1].parse().unwrap_or(8088);
                i += 1;
            }
            "--user" if i + 1 < args.len() => {
                user = args[i + 1].clone();
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }

    let mode = if use_ssh {
        TargetMode::Ssh {
            host,
            port: ssh_port,
            user,
        }
    } else if fixed {
        TargetMode::DemoFixed
    } else {
        TargetMode::Demo
    };
    (mode, http_port)
}

fn run_doctor(mode: &TargetMode) {
    println!("=== QNX 8.0 AI-Native Diagnostic Doctor ===");
    let info = run_target_command(mode, "pidin info").unwrap_or_else(|e| e);
    println!("\n[1/4] Kernel & Memory (pidin info):\n{}", info);

    let slog = run_target_command(mode, "slog2info").unwrap_or_default();
    let report = analyze_slog2(&slog);
    println!(
        "\n[2/4] Deterministic slog2 Triage — Health Score: {}/100 ({})",
        report.score, report.status
    );
    for f in &report.findings {
        println!(
            "  - [{}] {} @ {}: {}\n    -> Fix: {}",
            f.severity, f.code, f.component, f.evidence, f.recommendation
        );
    }

    let threads = run_target_command(mode, "pidin threads").unwrap_or_default();
    let rt_hits = analyze_pidin_threads(&threads);
    println!(
        "\n[3/4] Real-Time Thread Audit — {} blocked RT thread(s)",
        rt_hits.len()
    );
    for f in &rt_hits {
        println!("  - [{}] {}: {}", f.severity, f.code, f.evidence);
    }

    let raw_lat = run_target_command(mode, "/tmp/qnx-mcp-staging/latency_bench").unwrap_or_default();
    let samples: Vec<f64> = raw_lat
        .split(',')
        .filter_map(|s| s.trim().parse::<f64>().ok())
        .collect();
    let lat = summarize_latency_us(&samples, 100.0);
    println!(
        "\n[4/4] Jitter SLA Check (SLA <= {:.0}us): {} (P50={:.1}us P95={:.1}us P99={:.1}us Max={:.1}us)",
        lat.sla_us,
        if lat.passed { "PASS" } else { "SLA BREACH" },
        lat.p50_us,
        lat.p95_us,
        lat.p99_us,
        lat.max_us
    );
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("doctor");
    let (mode, http_port) = parse_args(&args[1..]);

    match cmd {
        "mcp" => mcp::serve_stdio(mode),
        "doctor" => run_doctor(&mode),
        "serve" => dashboard::serve_http(mode, http_port),
        _ => {
            eprintln!(
                "Usage:\n  qnx-mcp-workbench doctor [--fixed | --ssh --host 127.0.0.1 --port 2222 --user qnxuser]\n  qnx-mcp-workbench serve [--port 8088] [--ssh --host 127.0.0.1 --port 2222 --user qnxuser]\n  qnx-mcp-workbench mcp [--fixed | --ssh --host 127.0.0.1 --port 2222 --user qnxuser]"
            );
            std::process::exit(1);
        }
    }
}
