//! 外部命令执行。
//!
//! 整个程序的本质是编排外部命令，所以这一层只有一个目标：让一次调用是一行。
//!
//! ```ignore
//! run(["uv", "tool", "install", "--upgrade", pkg])?;          // 要求成功
//! let v = capture(["npm", "view", pkg, "version"])?;           // 要输出
//! let out = try_run(["systemctl", "is-active", unit])?;        // 容许失败
//! ```

use anyhow::{anyhow, bail, Context, Result};
use std::ffi::OsStr;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// 一次外部命令的结果。
#[derive(Debug, Clone)]
pub struct Output {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

/// 把 argv 拼成可以直接粘回终端复跑的字符串，用于报错。
///
/// 操作者看到「某条命令失败」之后第一件事就是自己跑一遍看完整输出，所以报错里
/// 那条命令必须是可直接粘贴执行的——带空格的参数要正确引起来。
fn render(argv: &[String]) -> String {
    argv.iter()
        .map(|a| {
            if a.is_empty() || a.contains([' ', '"', '\'', '$', '*', '?', '&', '|', ';', '\n']) {
                format!("'{}'", a.replace('\'', r"'\''"))
            } else {
                a.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn to_argv<I, S>(args: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    args.into_iter()
        .map(|a| a.as_ref().to_string_lossy().into_owned())
        .collect()
}

/// 跑一个命令，stdout/stderr 直通终端，额外导出 `envs`。退出码原样返回
/// （`None` 表示被信号终止）。
///
/// 这是所有「要让操作者实时看到输出」的调用的底座：npm/uv 的进度条、上游
/// `status` 的报告、前台运行的服务，都不能被捕获到缓冲区里。
pub fn spawn_status<I, S>(args: I, envs: &[(String, String)]) -> Result<Option<i32>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let edits: Vec<EnvEdit> = envs
        .iter()
        .map(|(k, v)| (k.clone(), Some(v.clone())))
        .collect();
    spawn_status_in(args, &edits, None)
}

/// 对子进程环境的一处改动。`None` 表示把这个变量从继承来的环境里**删掉**。
///
/// 删掉是个真需求：Go 那边用 godotenv 读 `.env` 的服务默认不覆盖已存在的环境
/// 变量，操作者 shell 里 export 过一个同名的 `PORT`，`.env` 里那行就彻底不生效，
/// 服务起在另一个端口上而且谁也没报错。
pub type EnvEdit = (String, Option<String>);

/// 跑一个命令，可以指定工作目录和环境改动。
///
/// 工作目录不是可有可无的：有些上游会从当前目录读东西（new-api 启动时
/// `godotenv.Load(".env")`），在哪儿跑决定了它加载哪份配置。
pub fn spawn_status_in<I, S>(
    args: I,
    env: &[EnvEdit],
    cwd: Option<&std::path::Path>,
) -> Result<Option<i32>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let argv = to_argv(args);
    let (prog, rest) = argv
        .split_first()
        .ok_or_else(|| anyhow!("spawn_status() 收到空命令"))?;

    let mut c = Command::new(prog);
    c.args(rest);
    apply_env(&mut c, env);
    if let Some(dir) = cwd {
        c.current_dir(dir);
    }
    let status = c
        .status()
        .with_context(|| format!("起不了进程: {}", render(&argv)))?;
    Ok(status.code())
}

/// 把环境改动套到一条待执行的命令上。
pub fn apply_env(c: &mut Command, env: &[EnvEdit]) {
    for (k, v) in env {
        match v {
            Some(v) => c.env(k, v),
            None => c.env_remove(k),
        };
    }
}

/// 跑一个命令，stdout/stderr 直通终端。退出码非 0 即报错。
pub fn run<I, S>(args: I) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    run_with(args, &[])
}

/// 同 [`run`]，但带环境改动。
pub fn run_with<I, S>(args: I, env: &[EnvEdit]) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let argv = to_argv(args);
    match spawn_status_in(argv.clone(), env, None)? {
        Some(0) => Ok(()),
        other => bail!(
            "命令失败（退出码 {}）:\n  {}",
            other.map_or("信号终止".to_string(), |c| c.to_string()),
            render(&argv)
        ),
    }
}

/// 同 [`run`]，但允许失败：返回 Err 时调用方自行决定是否忽略。
/// 用在「探测」语义的地方，比如 `command -v xxx`、`systemctl is-active`。
pub fn try_run<I, S>(args: I) -> Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    try_run_with(args, &[])
}

/// 同 [`try_run`]，但带环境改动。
///
/// 探测本身常常依赖环境：同一个 `airflow jobs check`，带不带 `AIRFLOW_HOME`
/// 问的是两个库。
pub fn try_run_with<I, S>(args: I, env: &[EnvEdit]) -> Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let argv = to_argv(args);
    let (prog, rest) = argv
        .split_first()
        .ok_or_else(|| anyhow!("try_run() 收到空命令"))?;

    let mut c = Command::new(prog);
    c.args(rest).stdin(Stdio::null());
    apply_env(&mut c, env);
    let out = c
        .output()
        .with_context(|| format!("起不了进程: {}", render(&argv)))?;

    Ok(Output {
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

/// 同 [`try_run`]，但最多等 `timeout`。超时返回 `Ok(None)`，并把进程杀掉。
///
/// 探测类的调用必须有这个上限。`git ls-remote` 没有连接超时可配，碰上把包默默
/// 丢掉而不是拒绝的防火墙（屏蔽端口最常见的做法）就只能耗在内核的 TCP 重传上，
/// 两分钟起步——而一条诊断命令卡两分钟，表现跟它要诊断的毛病一模一样。
pub fn try_run_timeout<I, S>(args: I, timeout: Duration) -> Result<Option<Output>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    try_run_timeout_with(args, &[], timeout)
}

/// 同 [`try_run_timeout`]，但带环境改动。
pub fn try_run_timeout_with<I, S>(
    args: I,
    env: &[EnvEdit],
    timeout: Duration,
) -> Result<Option<Output>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let argv = to_argv(args);
    let (prog, rest) = argv
        .split_first()
        .ok_or_else(|| anyhow!("try_run_timeout() 收到空命令"))?;

    let mut c = Command::new(prog);
    c.args(rest)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    apply_env(&mut c, env);
    let mut child = c
        .spawn()
        .with_context(|| format!("起不了进程: {}", render(&argv)))?;

    // 两根管子各派一个线程抽干。留在主线程边轮询边读的话，输出一超过管道缓冲区
    // （64 KB）子进程就卡在 write 上不退出，而我们正等着它退出。
    let out = drain(child.stdout.take());
    let err = drain(child.stderr.take());

    let deadline = Instant::now() + timeout;
    loop {
        if let Some(st) = child
            .try_wait()
            .with_context(|| format!("等不到结果: {}", render(&argv)))?
        {
            return Ok(Some(Output {
                code: st.code(),
                stdout: out.join().unwrap_or_default(),
                stderr: err.join().unwrap_or_default(),
            }));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            // 不去 join 那两个线程：被杀的进程若留下了还攥着管道写端的孙子进程，
            // 读会一直阻塞，而超时这条路本来就不要输出。
            return Ok(None);
        }
        std::thread::sleep(POLL);
    }
}

/// 轮询子进程是否退出的间隔。探测命令都是秒级的，这个精度足够，开销可以忽略。
const POLL: Duration = Duration::from_millis(20);

/// 在后台把一根管子读干，返回取内容的句柄。
fn drain(pipe: Option<impl Read + Send + 'static>) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut p) = pipe {
            let _ = p.read_to_end(&mut buf);
        }
        String::from_utf8_lossy(&buf).into_owned()
    })
}

/// 跑一个命令并取回 stdout（已 trim）。退出码非 0 即报错，错误里带上 stderr。
pub fn capture<I, S>(args: I) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    capture_with(args, &[])
}

/// 同 [`capture`]，但带环境改动。
pub fn capture_with<I, S>(args: I, env: &[EnvEdit]) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let argv = to_argv(args);
    let out = try_run_with(argv.clone(), env)?;
    if !out.success() {
        let tail: String = out
            .stderr
            .lines()
            .rev()
            .take(5)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n  ");
        bail!(
            "命令失败（退出码 {}）:\n  {}{}",
            out.code.map_or("信号终止".to_string(), |c| c.to_string()),
            render(&argv),
            if tail.is_empty() {
                String::new()
            } else {
                format!("\n  {tail}")
            }
        );
    }
    Ok(out.stdout.trim().to_string())
}

/// 命令在 PATH 上吗。等价于 `command -v <name> >/dev/null`。
pub fn has(name: &str) -> bool {
    which::which(name).is_ok()
}

/// 解析命令的绝对路径。
pub fn find(name: &str) -> Option<String> {
    which::which(name)
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

/// 要求某个命令存在，否则报一条带安装指引的错。
pub fn require(name: &str, hint: &str) -> Result<String> {
    find(name).ok_or_else(|| {
        if hint.is_empty() {
            anyhow!("需要 {name}，但 PATH 上找不到")
        } else {
            anyhow!("需要 {name}，但 PATH 上找不到。{hint}")
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 成功的命令返回_ok() {
        run(["true"]).unwrap();
    }

    #[test]
    fn 失败的命令报错且错误里带可复跑的命令() {
        let err = run(["false"]).unwrap_err().to_string();
        assert!(err.contains("退出码 1"), "实际: {err}");
        assert!(err.contains("false"), "实际: {err}");
    }

    #[test]
    fn capture_取回_stdout_并_trim() {
        assert_eq!(capture(["echo", "hello"]).unwrap(), "hello");
    }

    #[test]
    fn capture_失败时把_stderr_带进错误() {
        // 报错里必须能看出到底哪儿错了，否则操作者只知道「失败」。
        let err = capture(["sh", "-c", "echo boom >&2; exit 3"])
            .unwrap_err()
            .to_string();
        assert!(err.contains("退出码 3"), "实际: {err}");
        assert!(err.contains("boom"), "stderr 没被带进错误: {err}");
    }

    #[test]
    fn try_run_不把失败当错误() {
        let out = try_run(["sh", "-c", "exit 7"]).unwrap();
        assert_eq!(out.code, Some(7));
        assert!(!out.success());
    }

    #[test]
    fn 起不了的进程报的是_起不了_而不是退出码() {
        let err = run(["__definitely_not_a_command__"])
            .unwrap_err()
            .to_string();
        assert!(err.contains("起不了进程"), "实际: {err}");
    }

    #[test]
    fn 带空格的参数在报错里被正确引起来() {
        // 这条守的是报错的可复跑性：参数里有空格时直接 join 会拼出一条
        // 粘回终端跑不起来的命令，操作者会以为是别的问题。
        let err = run(["false", "a b", "c'd"]).unwrap_err().to_string();
        assert!(err.contains(r"'a b'"), "实际: {err}");
        assert!(err.contains(r"'c'\''d'"), "实际: {err}");
    }

    #[test]
    fn has_与_find_对一定存在的命令一致() {
        assert!(has("sh"));
        assert!(find("sh").is_some());
        assert!(!has("__definitely_not_a_command__"));
        assert!(find("__definitely_not_a_command__").is_none());
    }

    #[test]
    fn require_的错误里带安装指引() {
        let err = require("__nope__", "装它：apt install nope")
            .unwrap_err()
            .to_string();
        assert!(err.contains("apt install nope"), "实际: {err}");
    }

    #[test]
    fn 空命令不会_panic() {
        assert!(run(Vec::<String>::new()).is_err());
        assert!(try_run(Vec::<String>::new()).is_err());
        assert!(try_run_timeout(Vec::<String>::new(), Duration::from_secs(1)).is_err());
    }

    #[test]
    fn 超时的命令被杀掉并返回_none() {
        let t = Instant::now();
        let out = try_run_timeout(["sleep", "30"], Duration::from_millis(300)).unwrap();
        assert!(out.is_none());
        // 必须是超时那一刻就回来，而不是等 sleep 自己跑完。
        assert!(
            t.elapsed() < Duration::from_secs(5),
            "实际等了 {:?}",
            t.elapsed()
        );
    }

    #[test]
    fn 没超时的命令照常拿到输出和退出码() {
        let out = try_run_timeout(
            ["sh", "-c", "echo hi; echo bad >&2; exit 4"],
            Duration::from_secs(10),
        )
        .unwrap()
        .unwrap();
        assert_eq!(out.code, Some(4));
        assert_eq!(out.stdout.trim(), "hi");
        assert_eq!(out.stderr.trim(), "bad");
    }

    #[test]
    fn 输出撑爆管道缓冲区也不会卡死() {
        // 这条守的是「另开线程抽管子」那个决定：输出超过 64 KB 时，若主线程
        // 只轮询不读，子进程会阻塞在 write 上，双方互等，超时之前谁也回不来。
        let out = try_run_timeout(
            ["sh", "-c", "yes 0123456789abcdef | head -n 40000"],
            Duration::from_secs(20),
        )
        .unwrap()
        .unwrap();
        assert!(out.success());
        assert_eq!(out.stdout.len(), 40000 * 17);
    }
}
