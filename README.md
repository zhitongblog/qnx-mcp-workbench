# qnx-mcp-workbench

> **QNX Global Hackathon 2026 — Track 4 (Developer Tooling & Host Tools) + Best Codelab / Tutorial Candidate**
> **1080p Live Demo Video**: https://youtu.be/LPhqSmhgacU

An **AI-native, zero-dependency Rust host workbench, MCP server (`2024-11-05` & `2025-03-26`), and live telemetry dashboard** that connects AI coding agents (Claude Code, Cursor, Codex, Gemini CLI) safely to **QNX SDP 8.0** targets running in **QEMU** or on physical hardware (Raspberry Pi 5).

## Why This Exists
Connecting AI agents directly to real-time QNX Neutrino 8.0 systems fails in two ways today:
1. **Unbounded shell risk**: Giving an LLM raw SSH access to a safety-critical RTOS risks destructive commands (`slay`, `shutdown`, shell injection).
2. **Hallucinated root-cause analysis**: Dumping raw `slog2info` and `pidin` logs into an LLM misses subtle priority inversion, mutex stalls, and watchdog patterns.

`qnx-mcp-workbench` solves both with **zero external crates** (pure Rust `std`):
- **Fail-Closed Action Gateway** (`src/gateway.rs`): Enforces a strict read-only QNX diagnostic allowlist (`pidin`, `slog2info`, `hogs`, `uname`, etc.), blocks destructive commands and shell chaining (`; && || | $(`), and records every call into a tamper-evident JSONL audit log.
- **Deterministic QNX 8.0 Triage Engine** (`src/diagnostics.rs`): Scores target health (`0–100`) from `slog2info`, detects blocked high-priority real-time threads (`>=20r/20f` stuck in `MUTEX`/`SEM`/`DEAD`), and evaluates microsecond scheduling jitter (`P50/P95/P99/Max` vs SLA).
- **Live Zero-Dep Web Telemetry & Gateway Tester** (`src/dashboard.rs`): Launch `cargo run -- serve --port 8088` for an instant browser dashboard with before/after priority-inversion comparison, live jitter chart, and interactive Action Gateway testing.
- **Real QNX 8.0 C Sample Workload** (`examples/qnx-rt-demo/`): Compilable `qcc` motor-control priority-inversion demo (`motor_ctrl.c`) + `clock_nanosleep` jitter benchmark (`latency_bench.c`) + `Makefile`.

## Quick Start

```bash
# 1. Run full unit & MCP conformance tests
cargo test

# 2. Run CLI diagnostic triage (Buggy incident vs PTHREAD_PRIO_INHERIT fix)
cargo run -- doctor
cargo run -- doctor --fixed

# 3. Open the live browser Telemetry & Action Gateway Dashboard (http://127.0.0.1:8088)
cargo run -- serve --port 8088

# 4. Run against a live QNX SDP 8.0 QEMU target (SSH forwarded to localhost:2222)
cargo run -- doctor --ssh --host 127.0.0.1 --port 2222 --user qnxuser

# 5. Run as an MCP stdio server for Claude Code / Cursor
cargo run -- mcp --ssh --host 127.0.0.1 --port 2222 --user qnxuser
```

## Step-by-Step Codelab
Read **[CODELAB.md](./CODELAB.md)** for the full 15-minute hands-on tutorial covering QNX Everywhere SDP 8.0 + QEMU setup, `qcc` cross-compilation of `motor_ctrl.c`, MCP integration, and before/after P99 jitter verification.

## License
MIT
