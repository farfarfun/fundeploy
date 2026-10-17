//! 环境变量读取。
//!
//! 端口、地址、包名、超时都能用环境变量盖掉。读法集中在这里，是为了让两条规则
//! 只写一遍：空白等于没设；值不合法时**报出来**再用默认值。

use std::path::PathBuf;

use crate::ui;

/// 取一个字符串，没设或只有空白时用默认值。
///
/// 空白当成没设：`FOO= fundeploy ...` 在 shell 里是顺手就会写出来的东西，当成
/// 「设成了空」会让包名变成空串、地址变成空串，报出来的错离真正的原因很远。
pub fn str_or(key: &str, default: &str) -> String {
    match std::env::var(key) {
        Ok(v) if !v.trim().is_empty() => v,
        _ => default.to_string(),
    }
}

pub fn opt(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

/// 读端口。值不合法时**用默认值并明确报出来**——静默回退的话，操作者会盯着
/// 一个他以为改了的端口，而服务起在另一个端口上。
pub fn port(key: &str, default: u16) -> u16 {
    let Some(raw) = opt(key) else {
        return default;
    };
    match raw.trim().parse::<u16>() {
        Ok(p) if p > 0 => p,
        _ => {
            ui::warn(format!(
                "{key}={raw} 不是合法端口，按默认值 {default} 处理。"
            ));
            default
        }
    }
}

/// 读一个数值（秒数、行数这类）。规则同 [`port`]。
pub fn num<T>(key: &str, default: T) -> T
where
    T: std::str::FromStr + std::fmt::Display,
{
    let Some(raw) = opt(key) else {
        return default;
    };
    match raw.trim().parse::<T>() {
        Ok(v) => v,
        Err(_) => {
            ui::warn(format!(
                "{key}={raw} 不是合法数值，按默认值 {default} 处理。"
            ));
            default
        }
    }
}

/// 开关量。没设时用 `default`，设了就只认那几个真值写法。
pub fn flag(key: &str, default: bool) -> bool {
    let Some(raw) = opt(key) else {
        return default;
    };
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "1" | "y" | "yes" | "true" | "on"
    )
}

/// 家目录。取不到时退回当前目录——那是个不至于让后续路径拼接变成 `/xxx` 的落点。
pub fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// 把开头的 `~` 换成家目录。只认 `~` 和 `~/`：`~user` 要查 passwd，上游也不会写。
pub fn expand_tilde(path: &str) -> PathBuf {
    match path.strip_prefix('~') {
        Some("") => home(),
        Some(rest) if rest.starts_with('/') => home().join(rest.trim_start_matches('/')),
        _ => PathBuf::from(path),
    }
}

/// 某样东西「卸载时跳过确认」的环境变量名。
///
/// service 和 tool 两域各有一份一模一样的实现，连测试都抄了两遍。规则只有一条
/// （服务名大写、`-` 和 `.` 换成 `_`），但它是**对外承诺**的一部分——无人值守的
/// 脚本里写着这些变量名，两份实现哪天漂开一点，那些脚本就会静默地卡在确认上。
pub fn uninstall_yes_var(name: &str) -> String {
    format!(
        "FUNDEPLOY_{}_UNINSTALL_YES",
        name.to_ascii_uppercase().replace(['-', '.'], "_")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 卸载确认的环境变量名由名字推出() {
        // service 和 tool 两域此前各有一份实现、各有一份这样的测试。合成一份
        // 之后两域的例子都留着——变量名是对外承诺，service 那边的服务名带
        // `-`，tool 那边也带，两种都得钉住。
        assert_eq!(
            uninstall_yes_var("funread-web"),
            "FUNDEPLOY_FUNREAD_WEB_UNINSTALL_YES"
        );
        assert_eq!(
            uninstall_yes_var("airflow"),
            "FUNDEPLOY_AIRFLOW_UNINSTALL_YES"
        );
        assert_eq!(
            uninstall_yes_var("open-pencil"),
            "FUNDEPLOY_OPEN_PENCIL_UNINSTALL_YES"
        );
        // `.` 也要换掉：环境变量名里不能有点。
        assert_eq!(uninstall_yes_var("a.b-c"), "FUNDEPLOY_A_B_C_UNINSTALL_YES");
    }

    #[test]
    fn 非法端口退回默认值而不是崩掉() {
        std::env::set_var("__FD_TEST_PORT", "八千");
        assert_eq!(port("__FD_TEST_PORT", 1234), 1234);
        std::env::set_var("__FD_TEST_PORT", "0");
        assert_eq!(port("__FD_TEST_PORT", 1234), 1234);
        std::env::set_var("__FD_TEST_PORT", "70000");
        assert_eq!(port("__FD_TEST_PORT", 1234), 1234);
        std::env::remove_var("__FD_TEST_PORT");
    }

    #[test]
    fn 空环境变量当作没设置() {
        std::env::set_var("__FD_TEST_STR", "   ");
        assert_eq!(str_or("__FD_TEST_STR", "默认"), "默认");
        assert_eq!(opt("__FD_TEST_STR"), None);
        std::env::remove_var("__FD_TEST_STR");
    }

    #[test]
    fn 数值非法时退回默认值() {
        std::env::set_var("__FD_TEST_NUM", "一百");
        assert_eq!(num::<u64>("__FD_TEST_NUM", 60), 60);
        std::env::set_var("__FD_TEST_NUM", "180");
        assert_eq!(num::<u64>("__FD_TEST_NUM", 60), 180);
        std::env::remove_var("__FD_TEST_NUM");
    }

    #[test]
    fn 开关量没设时用默认值() {
        std::env::remove_var("__FD_TEST_FLAG");
        assert!(flag("__FD_TEST_FLAG", true));
        assert!(!flag("__FD_TEST_FLAG", false));
        // 设了就只认真值写法，其他一律为假——包括默认值为 true 的情况，
        // 否则 `FOO=0` 关不掉一个默认开着的开关。
        std::env::set_var("__FD_TEST_FLAG", "0");
        assert!(!flag("__FD_TEST_FLAG", true));
        std::env::set_var("__FD_TEST_FLAG", "YES");
        assert!(flag("__FD_TEST_FLAG", false));
        std::env::remove_var("__FD_TEST_FLAG");
    }

    #[test]
    fn 波浪号展开成家目录() {
        assert_eq!(expand_tilde("~"), home());
        assert_eq!(expand_tilde("~/a/b"), home().join("a/b"));
        // 绝对路径和相对路径原样留下，`~` 不在开头也不展开。
        assert_eq!(expand_tilde("/tmp/x"), PathBuf::from("/tmp/x"));
        assert_eq!(expand_tilde("a/~/b"), PathBuf::from("a/~/b"));
    }
}
