#![cfg(unix)]

use std::fs;
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use tempfile::tempdir;

fn write_executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// 用 python3 写一个可控的假 `funmill`：把每次调用记录到日志，并在 `start` / `stop` /
/// `status` / `install` 时维护状态文件与真实监听端口，完整验证 `fundeploy service funmill`
/// 的编排后端升级、单组件重启、server 子命令与透传链路。
fn install_fake_funmill(dir: &Path) {
    let script = r##"#!/usr/bin/env python3
import os, signal, socket, subprocess, sys, time
from pathlib import Path

state_dir = Path(os.environ["FAKE_FUNMILL_STATE"])
log_file = state_dir / "calls.log"
with log_file.open("a", encoding="utf-8") as f:
    f.write(" ".join(sys.argv[1:]) + "\n")

args = sys.argv[1:]
if len(args) >= 2 and args[0] == "--config":
    args = args[2:]

if not args:
    sys.exit(0)

cmd = args[0]
rest = args[1:]

def pid_file(name):
    return state_dir / f"{name}.pid"

def port_of(name):
    return int(os.environ["FUNMILL_API_PORT" if name == "api" else "FUNMILL_BACKEND_PORT"])

def is_alive(pid):
    try:
        os.kill(pid, 0)
        return True
    except OSError:
        return False

def start_one(name):
    pf = pid_file(name)
    port = port_of(name)
    code = (
        "import socket, time; "
        f"s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); "
        f"s.bind(('127.0.0.1', {port})); s.listen(5); "
        "time.sleep(30)"
    )
    proc = subprocess.Popen(
        [sys.executable, "-c", code],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
    )
    pf.write_text(str(proc.pid), encoding="utf-8")
    for _ in range(50):
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.1):
                break
        except OSError:
            time.sleep(0.05)

def stop_one(name):
    pf = pid_file(name)
    if not pf.exists():
        return
    pid = int(pf.read_text().strip())
    try:
        os.kill(pid, signal.SIGTERM)
    except OSError:
        pass
    for _ in range(30):
        if not is_alive(pid):
            break
        time.sleep(0.05)
    pf.unlink(missing_ok=True)

def status_one(name):
    pf = pid_file(name)
    if pf.exists():
        pid = int(pf.read_text().strip())
        if is_alive(pid):
            print(f"{name} is running: pid={pid}")
            sys.exit(0)
    print(f"{name} is stopped")
    sys.exit(1)

if cmd == "services":
    print("dagu\nwindmill")
elif cmd == "install":
    target = rest[0]
    force = "--force" in rest
    bin_path = Path(os.environ["FUNMILL_HOME"]) / "services" / target / target
    bin_path.parent.mkdir(parents=True, exist_ok=True)
    ver = "2.18.0" if force else "2.17.2"
    bin_path.write_text(f"#!/bin/sh\necho {ver}\n", encoding="utf-8")
    bin_path.chmod(0o755)
    print(f"installed {target}: {bin_path}")
elif cmd == "start":
    start_one(rest[0] if rest else "api")
elif cmd == "stop":
    stop_one(rest[0] if rest else "api")
elif cmd == "status":
    status_one(rest[0] if rest else "api")
elif cmd == "restart":
    name = rest[0] if rest else "api"
    stop_one(name)
    start_one(name)
elif cmd == "run":
    print("running api foreground")
else:
    print(f"passthrough: {' '.join(args)}")
"##;
    write_executable(&dir.join("funmill"), script);
}

struct Env {
    _tmp: tempfile::TempDir,
    bin_dir: std::path::PathBuf,
    state_dir: std::path::PathBuf,
    funmill_home: std::path::PathBuf,
    api_port: u16,
    backend_port: u16,
}

impl Env {
    fn new() -> Self {
        let tmp = tempdir().unwrap();
        let bin_dir = tmp.path().join("bin");
        let state_dir = tmp.path().join("state");
        let funmill_home = tmp.path().join("funmill-home");
        fs::create_dir_all(&bin_dir).unwrap();
        fs::create_dir_all(&state_dir).unwrap();
        fs::create_dir_all(&funmill_home).unwrap();
        install_fake_funmill(&bin_dir);
        Self {
            _tmp: tmp,
            bin_dir,
            state_dir,
            funmill_home,
            api_port: free_port(),
            backend_port: free_port(),
        }
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        let path = format!(
            "{}:{}",
            self.bin_dir.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        Command::new(env!("CARGO_BIN_EXE_fundeploy"))
            .args(args)
            .env("PATH", path)
            .env("NONINTERACTIVE", "1")
            .env("NO_COLOR", "1")
            .env("FAKE_FUNMILL_STATE", &self.state_dir)
            .env("FUNMILL_HOME", &self.funmill_home)
            .env("FUNMILL_API_PORT", self.api_port.to_string())
            .env("FUNMILL_BACKEND_PORT", self.backend_port.to_string())
            .env_remove("FUNMILL_API_KEY")
            .env_remove("FUNMILL_BACKEND")
            .env_remove("FUNMILL_CONFIG")
            .output()
            .unwrap()
    }

    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.state_dir.join("calls.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn clear_calls(&self) {
        let _ = fs::remove_file(self.state_dir.join("calls.log"));
    }
}

#[test]
fn funmill_dagu_upgrade_stops_and_restarts_when_running() {
    let env = Env::new();

    // 1. 列出可安装后端
    let out = env.run(&["service", "funmill", "services"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("dagu") && stdout.contains("windmill"),
        "stdout: {stdout}"
    );

    // 2. 先装一版固定 dagu，再启动它
    let out = env.run(&["service", "funmill", "dagu", "install"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = env.run(&["service", "funmill", "dagu", "start"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    env.clear_calls();

    // 3. 运行中升级 dagu：必须先 stop -> install dagu --force -> start
    let out = env.run(&["service", "funmill", "dagu", "upgrade"]);
    assert!(
        out.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("2.17.2 -> 2.18.0"), "stdout: {stdout}");
    let calls = env.calls();
    let stop_i = calls
        .iter()
        .position(|c| c == "stop dagu")
        .expect("应先停 dagu");
    let inst_i = calls
        .iter()
        .position(|c| c == "install dagu --force")
        .expect("应调用 install dagu --force");
    let start_i = calls
        .iter()
        .position(|c| c == "start dagu")
        .expect("升级后应重启 dagu");
    assert!(
        stop_i < inst_i && inst_i < start_i,
        "调用顺序不对: {calls:?}"
    );

    // 4. 单独重启 dagu（dagu restart 与 server restart dagu 两条入口）
    env.clear_calls();
    let out = env.run(&["service", "funmill", "server", "restart", "dagu"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let calls = env.calls();
    assert!(calls.iter().any(|c| c == "stop dagu"), "calls: {calls:?}");
    assert!(calls.iter().any(|c| c == "start dagu"), "calls: {calls:?}");

    // 5. 停掉 dagu 后再 upgrade-backend（省略参数默认 dagu），不应自动启动
    let out = env.run(&["service", "funmill", "dagu", "stop"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    env.clear_calls();
    let out = env.run(&["service", "funmill", "upgrade-backend"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let calls = env.calls();
    assert!(
        calls.iter().any(|c| c == "install dagu --force"),
        "calls: {calls:?}"
    );
    assert!(
        !calls.iter().any(|c| c == "start dagu"),
        "停着的时候不该自动拉起: {calls:?}"
    );
}

#[test]
fn funmill_reads_api_env_and_supports_cli_passthrough() {
    let env = Env::new();

    // api.env 里配了 FUNMILL_API_KEY，不设环境变量也应放行 start-one api
    let api_env = env.funmill_home.join("api/api.env");
    fs::create_dir_all(api_env.parent().unwrap()).unwrap();
    fs::write(&api_env, "FUNMILL_API_KEY=secret-from-file\n").unwrap();

    let out = env.run(&["service", "funmill", "start-one", "api"]);
    assert!(
        out.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let out = env.run(&["service", "funmill", "stop-one", "api"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // cli 原样透传 --config 等参数
    env.clear_calls();
    let out = env.run(&[
        "service",
        "funmill",
        "cli",
        "--config",
        api_env.to_str().unwrap(),
        "services",
    ]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let calls = env.calls();
    assert_eq!(calls, [format!("--config {} services", api_env.display())]);
}
