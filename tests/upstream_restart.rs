#![cfg(unix)]

use std::fs;
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
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

fn is_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    unsafe { libc::kill(pid, 0) == 0 }
}

fn kill_now(pid: i32) {
    if pid > 1 {
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
    }
}

/// 构造一组假的 `funflix-api` 和 `funflix-web`：
/// - `funflix-api` 在 `FUNFLIX_API_STUCK_ON_TERM=1` 时模拟真实故障：收到 SIGTERM 后
///   立即关闭监听端口，但进程本身拒绝退出；`server stop` 报「在 10s 内未退出（pid ...）」
///   且保留 PID 文件；`server start` 看到 PID 还活着则报「已在运行（pid ...）」并退出 1。
fn install_fake_funflix(bin_dir: &Path) {
    let api_script = r##"#!/usr/bin/env python3
import os, signal, socket, subprocess, sys, time
from pathlib import Path

state = Path(os.environ["FAKE_FUNFLIX_STATE"])
pid_file = state / "api.pid"
port = int(os.environ["FUNFLIX_WEB_BACKEND_PORT"])
args = sys.argv[1:]

def alive(pid):
    try:
        os.kill(pid, 0)
        return True
    except OSError:
        return False

def read_pid():
    if not pid_file.exists():
        return None
    try:
        pid = int(pid_file.read_text().strip())
        return pid if alive(pid) else None
    except ValueError:
        return None

if args[:2] == ["server", "run-daemon"]:
    # 模拟真实的 Web 进程：收到 SIGTERM 立刻关掉监听 socket，但进程继续活着。
    stuck = os.environ.get("FUNFLIX_API_STUCK_ON_TERM") == "1"
    s = socket.socket()
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    s.bind(("127.0.0.1", port))
    s.listen(5)

    def on_term(signum, frame):
        try:
            s.close()
        except OSError:
            pass
        if not stuck:
            sys.exit(0)

    signal.signal(signal.SIGTERM, on_term)
    while True:
        time.sleep(0.2)

elif args[:2] == ["server", "start"]:
    pid = read_pid()
    if pid is not None:
        print(f"funflix-api 已在运行（pid {pid}）")
        sys.exit(1)
    # 通过调用自身脚本启动，让 /proc/<pid>/cmdline 里真实包含 funflix-api。
    proc = subprocess.Popen(
        [sys.argv[0], "server", "run-daemon"],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
    )
    pid_file.write_text(str(proc.pid), encoding="utf-8")
    for _ in range(50):
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.1):
                break
        except OSError:
            time.sleep(0.05)
    sys.exit(0)

elif args[:2] == ["server", "stop"]:
    if os.environ.get("FAKE_STOP_NO_PID_STILL_RUNNING") == "1":
        print("stop timed out")
        sys.exit(1)
    pid = read_pid()
    if pid is None:
        print("未在运行")
        pid_file.unlink(missing_ok=True)
        sys.exit(0)
    try:
        os.kill(pid, signal.SIGTERM)
    except OSError:
        pass
    for _ in range(10):
        if not alive(pid):
            pid_file.unlink(missing_ok=True)
            sys.exit(0)
        time.sleep(0.05)
    print(f"funflix-api 在 10s 内未退出（pid {pid}）")
    sys.exit(1)

elif args[:2] == ["server", "status"]:
    if os.environ.get("FAKE_STOP_NO_PID_STILL_RUNNING") == "1":
        print("running")
        sys.exit(0)
    pid = read_pid()
    if pid is not None:
        print(f"运行中（pid {pid}）")
        sys.exit(0)
    print("未在运行")
    sys.exit(1)
"##;

    let web_script = r##"#!/usr/bin/env python3
import os, signal, socket, subprocess, sys, time
from pathlib import Path

state = Path(os.environ["FAKE_FUNFLIX_STATE"])
pid_file = state / "web.pid"
port = int(os.environ["FUNFLIX_WEB_FRONTEND_PORT"])
args = sys.argv[1:]

def alive(pid):
    try:
        os.kill(pid, 0)
        return True
    except OSError:
        return False

def read_pid():
    if not pid_file.exists():
        return None
    try:
        pid = int(pid_file.read_text().strip())
        return pid if alive(pid) else None
    except ValueError:
        return None

if args[:2] == ["server", "run-daemon"]:
    s = socket.socket()
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    s.bind(("127.0.0.1", port))
    s.listen(5)
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
    while True:
        time.sleep(0.2)
elif args[:2] == ["server", "start"]:
    if read_pid() is not None:
        sys.exit(1)
    proc = subprocess.Popen(
        [sys.argv[0], "server", "run-daemon"],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
    )
    pid_file.write_text(str(proc.pid), encoding="utf-8")
    for _ in range(50):
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.1):
                break
        except OSError:
            time.sleep(0.05)
    sys.exit(0)
elif args[:2] == ["server", "stop"]:
    pid = read_pid()
    if pid is None:
        print("未在运行")
        pid_file.unlink(missing_ok=True)
        sys.exit(0)
    try:
        os.kill(pid, signal.SIGTERM)
    except OSError:
        pass
    for _ in range(20):
        if not alive(pid):
            break
        time.sleep(0.05)
    pid_file.unlink(missing_ok=True)
    sys.exit(0)
elif args[:2] == ["server", "status"]:
    sys.exit(0 if read_pid() is not None else 1)
"##;

    write_executable(&bin_dir.join("funflix-api"), api_script);
    write_executable(&bin_dir.join("funflix-web"), web_script);
}

struct Env {
    _tmp: tempfile::TempDir,
    bin_dir: PathBuf,
    state_dir: PathBuf,
    backend_port: u16,
    frontend_port: u16,
}

impl Env {
    fn new() -> Self {
        let tmp = tempdir().unwrap();
        let bin_dir = tmp.path().join("bin");
        let state_dir = tmp.path().join("state");
        fs::create_dir_all(&bin_dir).unwrap();
        fs::create_dir_all(&state_dir).unwrap();
        install_fake_funflix(&bin_dir);
        Self {
            _tmp: tmp,
            bin_dir,
            state_dir,
            backend_port: free_port(),
            frontend_port: free_port(),
        }
    }

    fn run(&self, args: &[&str], extra_env: &[(&str, &str)]) -> std::process::Output {
        let path = format!(
            "{}:{}",
            self.bin_dir.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_fundeploy"));
        cmd.args(args)
            .env("PATH", path)
            .env("NONINTERACTIVE", "1")
            .env("NO_COLOR", "1")
            .env("FAKE_FUNFLIX_STATE", &self.state_dir)
            .env("FUNFLIX_WEB_BACKEND_PORT", self.backend_port.to_string())
            .env("FUNFLIX_WEB_FRONTEND_PORT", self.frontend_port.to_string())
            .env_remove("FUNFLIX_WEB_BACKEND_CONFIG")
            .env_remove("FUNFLIX_WEB_FRONTEND_CONFIG");
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        cmd.output().unwrap()
    }

    fn read_pid(&self, name: &str) -> Option<i32> {
        fs::read_to_string(self.state_dir.join(format!("{name}.pid")))
            .ok()
            .and_then(|s| s.trim().parse().ok())
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        for name in ["api", "web"] {
            if let Some(pid) = self.read_pid(name) {
                kill_now(pid);
            }
        }
    }
}

#[test]
fn restart_force_stops_lingering_process_that_released_port_on_sigterm() {
    let env = Env::new();

    // 1. 先起一个「收到 SIGTERM 会立刻关端口但死活不退出」的后端。
    let out = env.run(
        &["service", "funflix", "start"],
        &[("FUNFLIX_API_STUCK_ON_TERM", "1")],
    );
    assert!(
        out.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let old_api_pid = env.read_pid("api").expect("后端应已写出 PID");
    assert!(is_alive(old_api_pid));

    // 2. 重启：上游 stop 报错且端口已释放，fundeploy 必须发现并强停残留进程，再干净拉起新进程。
    let out = env.run(&["service", "funflix", "restart"], &[]);
    assert!(
        out.status.success(),
        "restart 不应因为残留进程而失败\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let new_api_pid = env.read_pid("api").expect("重启后后端应有新 PID");
    assert_ne!(old_api_pid, new_api_pid, "后端必须是新拉起的进程");
    assert!(!is_alive(old_api_pid), "旧的残留进程必须已经被强停");
    assert!(is_alive(new_api_pid), "新后端进程应在运行");
}

fn spawn_ignoring_sigterm(ready: &Path, extra_arg: Option<&str>) -> std::process::Child {
    let mut cmd = Command::new("python3");
    cmd.args([
        "-c",
        "import pathlib, signal, sys, time; \
         signal.signal(signal.SIGTERM, signal.SIG_IGN); \
         pathlib.Path(sys.argv[1]).write_text('ok'); \
         time.sleep(60)",
        ready.to_str().unwrap(),
    ]);
    if let Some(arg) = extra_arg {
        cmd.arg(arg);
    }
    let child = cmd.spawn().unwrap();
    for _ in 0..100 {
        if ready.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(ready.exists(), "测试子进程未就绪");
    child
}

#[test]
fn stop_recovers_when_port_already_closed_before_stop_is_called() {
    let env = Env::new();

    // 模拟半死不活的现场：进程还活着、命令行带 funflix-api、PID 文件指向它，但端口早已关闭。
    let mut stuck = spawn_ignoring_sigterm(&env.state_dir.join("stuck.ready"), Some("funflix-api"));
    let stuck_pid = stuck.id() as i32;
    fs::write(env.state_dir.join("api.pid"), format!("{stuck_pid}\n")).unwrap();

    let out = env.run(&["service", "funflix", "stop"], &[]);
    let _ = stuck.wait();
    assert!(
        out.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!is_alive(stuck_pid), "卡住的残留进程应已被清理");
}

#[test]
fn stop_refuses_to_kill_unrelated_pid_reported_by_upstream_in_noninteractive_mode() {
    let env = Env::new();

    // 模拟陈旧 PID 已被系统复用到无关进程（命令行不含 funflix-api，也没占端口，且忽略 SIGTERM）。
    let mut unrelated = spawn_ignoring_sigterm(&env.state_dir.join("unrelated.ready"), None);
    let unrelated_pid = unrelated.id() as i32;
    fs::write(env.state_dir.join("api.pid"), format!("{unrelated_pid}\n")).unwrap();

    let out = env.run(&["service", "funflix", "stop"], &[]);
    let still_alive = is_alive(unrelated_pid);
    let _ = unrelated.kill();
    let _ = unrelated.wait();

    assert!(
        !out.status.success(),
        "无法确认归属的 PID 在无人值守下必须中止停止"
    );
    assert!(still_alive, "无关进程绝不能被误杀");
}

#[test]
fn stop_fails_if_upstream_status_still_reports_running_without_pid() {
    let env = Env::new();

    let out = env.run(
        &["service", "funflix", "stop"],
        &[("FAKE_STOP_NO_PID_STILL_RUNNING", "1")],
    );
    assert!(
        !out.status.success(),
        "上游 status 仍报告在运行时不能假装停止成功"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("上游 status 仍报告在运行"),
        "stderr: {stderr}"
    );
}
