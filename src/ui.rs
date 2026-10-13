//! 终端输出与交互。
//!
//! 约定：进度与结果走 stdout，警告与错误走 stderr。这样 `fundeploy service x status`
//! 的输出可以直接重定向进文件而不混进警告。

use anyhow::{bail, Result};
use owo_colors::OwoColorize;
use std::io::IsTerminal;
use std::sync::OnceLock;

/// 该不该上色。
fn color() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        if std::env::var_os("NO_COLOR").is_some() {
            return false;
        }
        if std::env::var("TERM").is_ok_and(|t| t == "dumb") {
            return false;
        }
        std::io::stdout().is_terminal()
    })
}

/// 能不能问人。重定向、CI、显式 NONINTERACTIVE 都算不能。
pub fn interactive() -> bool {
    if truthy_env("NONINTERACTIVE") || truthy_env("CI") {
        return false;
    }
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

fn truthy_env(name: &str) -> bool {
    matches!(
        std::env::var(name)
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "y" | "yes" | "true" | "on"
    )
}

/// 一个步骤的开始。
pub fn step(msg: impl AsRef<str>) {
    let msg = msg.as_ref();
    if color() {
        println!("{} {msg}", "==>".cyan().bold());
    } else {
        println!("==> {msg}");
    }
}

/// 分节标题。
pub fn heading(msg: impl AsRef<str>) {
    let msg = msg.as_ref();
    if color() {
        println!("{}", format!("== {msg} ==").bold());
    } else {
        println!("== {msg} ==");
    }
}

pub fn info(msg: impl AsRef<str>) {
    println!("{}", msg.as_ref());
}

/// 跟在上一行后面的补充说明，缩进两格。
pub fn hint(msg: impl AsRef<str>) {
    for line in msg.as_ref().lines() {
        println!("  {line}");
    }
}

pub fn ok(msg: impl AsRef<str>) {
    let msg = msg.as_ref();
    if color() {
        println!("{} {msg}", "✓".green().bold());
    } else {
        println!("✓ {msg}");
    }
}

pub fn warn(msg: impl AsRef<str>) {
    let msg = msg.as_ref();
    if color() {
        eprintln!("{} {msg}", "警告:".yellow().bold());
    } else {
        eprintln!("警告: {msg}");
    }
}

pub fn blank() {
    println!();
}

/// `键: 值` 一行，值为 None 时打「未安装」。
pub fn kv(key: impl AsRef<str>, value: Option<&str>) {
    match value {
        Some(v) => println!("{}: {v}", key.as_ref()),
        None => println!("{}: 未安装", key.as_ref()),
    }
}

/// 危险操作确认。
///
/// `env_yes` 是跳过询问的环境变量名——没有它，卸载这类操作在 CI 和脚本里就
/// 完全没法用；但默认不能靠「检测不到终端就当同意」，那会让一次误连的
/// 重定向把东西删掉。所以无人值守时必须显式给这个变量，否则直接报错退出。
pub fn confirm_destructive(prompt: &str, env_yes: &str) -> Result<()> {
    if truthy_env(env_yes) {
        return Ok(());
    }
    if !interactive() {
        bail!("{prompt}\n当前是无人值守环境，确认请设 {env_yes}=1 后重试。");
    }
    let yes = inquire::Confirm::new(prompt)
        .with_default(false)
        .prompt()
        .unwrap_or(false);
    if !yes {
        bail!("已取消。");
    }
    Ok(())
}

/// 从一组候选里选一个。返回 None 表示用户取消。
pub fn select<'a>(prompt: &str, options: &'a [String]) -> Result<Option<&'a String>> {
    if options.is_empty() {
        bail!("没有可选项");
    }
    if !interactive() {
        bail!("{prompt}\n当前是无人值守环境，请把选项直接写在命令行上。");
    }
    Ok(
        inquire::Select::new(prompt, options.iter().collect::<Vec<_>>())
            .prompt()
            .ok(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 环境变量的真值形式都认() {
        for v in ["1", "y", "yes", "true", "on", "YES", "True"] {
            std::env::set_var("__FD_TEST_TRUTHY", v);
            assert!(truthy_env("__FD_TEST_TRUTHY"), "没认出 {v}");
        }
        for v in ["", "0", "no", "false", "off", "随便"] {
            std::env::set_var("__FD_TEST_TRUTHY", v);
            assert!(!truthy_env("__FD_TEST_TRUTHY"), "误认了 {v}");
        }
        std::env::remove_var("__FD_TEST_TRUTHY");
    }

    #[test]
    fn 未设置的环境变量是假值() {
        std::env::remove_var("__FD_TEST_UNSET");
        assert!(!truthy_env("__FD_TEST_UNSET"));
    }

    #[test]
    fn 无人值守下没给确认变量就报错而不是默认同意() {
        // 守的是「误把确认当同意」：测试环境里没有 tty，若实现把它当同意，
        // 这里就会返回 Ok，而真实场景下对应的是把东西直接卸掉。
        std::env::set_var("NONINTERACTIVE", "1");
        std::env::remove_var("__FD_TEST_YES");
        let err = confirm_destructive("要删吗", "__FD_TEST_YES")
            .unwrap_err()
            .to_string();
        assert!(err.contains("__FD_TEST_YES=1"), "实际: {err}");
        std::env::remove_var("NONINTERACTIVE");
    }

    #[test]
    fn 给了确认变量就不再询问() {
        std::env::set_var("__FD_TEST_YES2", "1");
        confirm_destructive("要删吗", "__FD_TEST_YES2").unwrap();
        std::env::remove_var("__FD_TEST_YES2");
    }
}
