//! Target Transport:
//! - `TargetMode::Demo`: Built-in QNX SDP 8.0 QEMU fixture exhibiting a real-time
//!   priority inversion + watchdog timeout so judges & CI can reproduce the incident immediately.
//! - `TargetMode::DemoFixed`: Post-remediation QNX SDP 8.0 fixture after enabling
//!   `PTHREAD_PRIO_INHERIT` on `can_bus_lock` (100/100 healthy, P99 jitter < 30us).
//! - `TargetMode::Ssh`: Real SSH execution against QNX SDP 8.0 running in QEMU (`127.0.0.1:2222`)
//!   or Raspberry Pi 5, strictly guarded by `crate::gateway::evaluate_qnx_command`.

use std::process::Command;

use crate::gateway::evaluate_qnx_command;

#[derive(Debug, Clone)]
pub enum TargetMode {
    Demo,
    DemoFixed,
    Ssh {
        host: String,
        port: u16,
        user: String,
    },
}

pub fn run_target_command(mode: &TargetMode, cmd: &str) -> Result<String, String> {
    let decision = evaluate_qnx_command(cmd);
    if !decision.allowed {
        return Err(decision.reason);
    }

    match mode {
        TargetMode::Demo => Ok(demo_fixture(cmd, false)),
        TargetMode::DemoFixed => Ok(demo_fixture(cmd, true)),
        TargetMode::Ssh { host, port, user } => {
            let output = Command::new("ssh")
                .args([
                    "-o",
                    "BatchMode=yes",
                    "-o",
                    "ConnectTimeout=5",
                    "-p",
                    &port.to_string(),
                    &format!("{}@{}", user, host),
                    cmd,
                ])
                .output()
                .map_err(|e| format!("Failed to spawn ssh: {}", e))?;
            if output.status.success() {
                Ok(String::from_utf8_lossy(&output.stdout).to_string())
            } else {
                Err(format!(
                    "SSH exited with {}: {}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr)
                ))
            }
        }
    }
}

fn demo_fixture(cmd: &str, fixed: bool) -> String {
    if cmd.starts_with("pidin info") {
        "\
CPU:X86_64 Release:8.0.0 FreeMem:1728Mb/2048Mb BootTime:Nov 25 09:00:00 UTC 2026
Processes: 42, Threads: 118
Processor1: 686 QEMU Virtual CPU 2400MHz FPU
Processor2: 686 QEMU Virtual CPU 2400MHz FPU"
            .to_string()
    } else if cmd.starts_with("pidin threads") {
        if fixed {
            "\
pid tid name                     prio STATE
  1   1 procnto-smp-instr         0r  READY
 88   1 /opt/qnx/safety_wdt      28f  NANOSLEEP
104   1 /opt/qnx/motor_ctrl      25f  NANOSLEEP
104   2 /opt/qnx/telemetry_log   10r  RECEIVE
201   1 /usr/sbin/sshd           10r  SIGWAITINFO"
                .to_string()
        } else {
            "\
pid tid name                     prio STATE
  1   1 procnto-smp-instr         0r  READY
 88   1 /opt/qnx/safety_wdt      28f  NANOSLEEP
104   1 /opt/qnx/motor_ctrl      25f  MUTEX 0x4801a0
104   2 /opt/qnx/telemetry_log   10r  READY
201   1 /usr/sbin/sshd           10r  SIGWAITINFO"
                .to_string()
        }
    } else if cmd.starts_with("slog2info") {
        if fixed {
            "\
Nov 25 10:20:01.002  procnto.1       INFO   QNX Neutrino 8.0.0 microkernel healthy (QEMU x86_64)
Nov 25 10:20:02.110  motor_ctrl.104  INFO   loop initialized at 1000Hz (prio_inherit=enabled)
Nov 25 10:20:15.400  safety_wdt.88   INFO   all real-time heartbeats within 5.0ms SLA"
                .to_string()
        } else {
            "\
Nov 25 10:12:01.004  procnto.1       INFO   QNX Neutrino 8.0.0 microkernel booted (QEMU x86_64)
Nov 25 10:14:22.410  motor_ctrl.104  INFO   control loop 1000Hz active
Nov 25 10:15:03.882  motor_ctrl.104  ERROR  mutex stall detected: priority inversion on can_bus_lock (held by telemetry_log prio 10r)
Nov 25 10:15:04.015  safety_wdt.88   CRIT   watchdog timeout expired on motor_ctrl (jitter=13.4ms > 5.0ms SLA)
Nov 25 10:15:04.019  ipc_router.64   WARN   pulse queue overflow dropped 4 telemetry frames (EAGAIN)"
                .to_string()
        }
    } else if cmd.starts_with("/tmp/qnx-mcp-staging/latency_bench") {
        if fixed {
            "11.4,13.2,12.8,14.5,16.1,15.0,17.2,14.8,19.4,24.6".to_string()
        } else {
            "14.2,16.8,15.1,18.4,21.0,19.7,22.3,17.9,34.5,142.8".to_string()
        }
    } else {
        format!("[demo] executed allowed command: {}", cmd)
    }
}
