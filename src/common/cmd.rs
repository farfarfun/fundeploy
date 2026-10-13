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
use std::process::{Command, Stdio};

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
    let argv = to_argv(args);
    let (prog, rest) = argv
        .split_first()
        .ok_or_else(|| anyhow!("spawn_status() 收到空命令"))?;

    let mut c = Command::new(prog);
    c.args(rest);
    for (k, v) in envs {
        c.env(k, v);
    }
    let status = c
        .status()
        .with_context(|| format!("起不了进程: {}", render(&argv)))?;
    Ok(status.code())
}

/// 跑一个命令，stdout/stderr 直通终端。退出码非 0 即报错。
pub fn run<I, S>(args: I) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let argv = to_argv(args);
    match spawn_status(argv.clone(), &[])? {
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
    let argv = to_argv(args);
    let (prog, rest) = argv
        .split_first()
        .ok_or_else(|| anyhow!("try_run() 收到空命令"))?;

    let out = Command::new(prog)
        .args(rest)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("起不了进程: {}", render(&argv)))?;

    Ok(Output {
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

/// 跑一个命令并取回 stdout（已 trim）。退出码非 0 即报错，错误里带上 stderr。
pub fn capture<I, S>(args: I) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let argv = to_argv(args);
    let out = try_run(argv.clone())?;
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
    }
}
