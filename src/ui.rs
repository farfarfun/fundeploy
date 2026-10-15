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

/// 按终端显示宽度右补空格，用来排表格的列。
///
/// `format!("{:<12}")` 补的是**字符数**，而终端里一个汉字占两列。中英混排的表
/// 格用它排出来是歪的——「清华大学」和 `aliyun` 都算 4 和 6 个字符，实际却是
/// 8 列和 6 列。
pub fn pad(s: &str, width: usize) -> String {
    let used = s.chars().map(char_cols).sum::<usize>();
    format!("{s}{}", " ".repeat(width.saturating_sub(used)))
}

/// 一个字符在终端里占几列。
///
/// 只认「东亚宽字符」这几段——中日韩文字、全角标点、常见符号。这里不引
/// unicode-width：要排的只有中文标签，为它多一个依赖不划算。
fn char_cols(c: char) -> usize {
    match c as u32 {
        0x1100..=0x115F        // 朝鲜语字母
        | 0x2E80..=0x303E      // 部首、中日韩符号
        | 0x3041..=0x33FF      // 假名、注音、兼容字符
        | 0x3400..=0x4DBF      // 扩展 A 区汉字
        | 0x4E00..=0x9FFF      // 基本汉字
        | 0xA000..=0xA4CF      // 彝文
        | 0xAC00..=0xD7A3      // 朝鲜语音节
        | 0xF900..=0xFAFF      // 兼容汉字
        | 0xFE30..=0xFE6F      // 竖排标点
        | 0xFF00..=0xFF60      // 全角字母数字标点
        | 0xFFE0..=0xFFE6 => 2,
        _ => 1,
    }
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

/// 问一句是/否，默认「是」，答「不」不算错。
///
/// 和 [`confirm_destructive`] 三处不同，都是为「本来就该发生」的那类动作准备的
/// ——典型是 `stop` 停不下来时按端口强停：
///
/// - **回答「不」返回 false 而不是报错**，调用方自己决定怎么往下走。
/// - **默认选项是「是」**，常规一次 restart 只需一个回车。
/// - **无人值守时按「是」办**，不报错退出。`stop` 跑在 cron 和 CI 里是常态，
///   在那里停下来问等于让 restart 断在半路——而这恰恰是加这道确认要避免的事。
///
/// `env_yes` 给「在终端里也别问我」用。
pub fn confirm_default_yes(prompt: &str, env_yes: &str) -> bool {
    if truthy_env(env_yes) || !interactive() {
        return true;
    }
    // 读不到回答（Ctrl+C、Esc）当成「不」：那是人主动中断，不是默认路径。
    inquire::Confirm::new(prompt)
        .with_default(true)
        .prompt()
        .unwrap_or(false)
}

/// 问一个值。`default` 非空时直接回车就取它。
pub fn input(prompt: &str, default: &str) -> Result<String> {
    if !interactive() {
        bail!("{prompt}\n当前是无人值守环境，请把这个值直接写在命令行上。");
    }
    let mut q = inquire::Text::new(prompt);
    if !default.is_empty() {
        q = q.with_default(default);
    }
    Ok(q.prompt()?.trim().to_string())
}

/// 问一个口令，输入不回显。
///
/// 比起让操作者写在命令行上，这里少了一条泄漏路径：shell 历史。
pub fn password(prompt: &str) -> Result<String> {
    if !interactive() {
        bail!("{prompt}\n当前是无人值守环境，请把这个值直接写在命令行上。");
    }
    Ok(inquire::Password::new(prompt)
        .with_display_mode(inquire::PasswordDisplayMode::Masked)
        .without_confirmation()
        .prompt()?)
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
    fn 补齐按显示宽度算而不是字符数() {
        // 「清华大学」占 8 列，补到 12 列只该加 4 个空格；按字符数算会加 8 个，
        // 表格就歪了。
        assert_eq!(pad("清华大学", 12).len() - "清华大学".len(), 4);
        assert_eq!(pad("aliyun", 12), "aliyun      ");
        assert_eq!(pad("官方", 4), "官方");
        // 超出宽度时不截断也不 panic。
        assert_eq!(pad("非常非常长的名字", 4), "非常非常长的名字");
    }

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

    #[test]
    fn 默认同意的那种确认在无人值守下就是同意() {
        // 跟上面那条正好相反，也必须相反：stop 收尾的强停走的是这条，在 cron 和 CI
        // 里停下来问（或者报错）等于让 restart 断在半路。
        std::env::set_var("NONINTERACTIVE", "1");
        std::env::remove_var("__FD_TEST_YES3");
        assert!(confirm_default_yes("要停吗", "__FD_TEST_YES3"));
        std::env::remove_var("NONINTERACTIVE");
    }

    #[test]
    fn 默认同意的那种确认也认跳过变量() {
        std::env::set_var("__FD_TEST_YES4", "1");
        assert!(confirm_default_yes("要停吗", "__FD_TEST_YES4"));
        std::env::remove_var("__FD_TEST_YES4");
    }
}
