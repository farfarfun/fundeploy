//! 服务配置文件的落地。

use anyhow::{Context, Result};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// 问 CLI 它实际在用哪个配置目录。
///
/// 配置目录的位置会随上游版本变（同一个 fundeploy 要同时伺候新旧两种布局，一条
/// `rollback` 就能把机器退回旧布局）。写死任何一边都会让配置文件生成到 CLI 根本
/// 不看的位置上——症状是 start 报 config file not found，而文件明明「已生成」。
///
/// `server status` 不论服务在不在跑都会打印 `config <路径>`，所以这条路随时可问。
/// 问不出来（没装、换了输出格式）才退回 `fallback`。
pub fn cli_config_dir(cli: &str, fallback: &Path) -> PathBuf {
    let Ok(out) = crate::common::cmd::try_run([cli, "server", "status"]) else {
        return fallback.to_path_buf();
    };
    out.stdout
        .lines()
        .find_map(parse_config_path)
        .map(Path::new)
        .and_then(|p| p.parent())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| fallback.to_path_buf())
}

/// 从一行输出里抠出 `config <路径>)` 里的路径。
fn parse_config_path(line: &str) -> Option<&str> {
    let rest = line.split_once("config ")?.1;
    let path = rest.split_once(')')?.0.trim();
    (!path.is_empty()).then_some(path)
}

/// 建一个只有本用户能进的目录。
pub fn ensure_private_dir(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("建不出目录 {}", dir.display()))?;
    // 0700：这底下会放口令和库地址。
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    Ok(())
}

/// 写一个配置文件，**只在它不存在时写**。返回是否真的写了。
///
/// 「不存在才写」是硬规则，不是优化。这些文件里放的是生产库地址和管理口令，
/// 覆盖一次就等于把一台在跑的机器悄悄指向别的库——而且是在 install/start 这种
/// 看起来人畜无害的命令里发生的。
pub fn write_if_absent(path: &Path, content: &str) -> Result<bool> {
    if path.exists() {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        ensure_private_dir(dir)?;
    }
    std::fs::write(path, content).with_context(|| format!("写不了 {}", path.display()))?;
    // 0600：里面可以放口令。
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 从_status_输出里抠出配置路径() {
        assert_eq!(
            parse_config_path("not running (config /home/u/.farfarfun/funread/api/config.toml)"),
            Some("/home/u/.farfarfun/funread/api/config.toml")
        );
    }

    #[test]
    fn 没有配置信息的行抠不出东西() {
        assert_eq!(parse_config_path("running, pid 1234"), None);
        assert_eq!(parse_config_path(""), None);
        // 有 `config ` 但没有收尾的右括号，说明格式变了，宁可退回兜底值。
        assert_eq!(parse_config_path("config /tmp/a.toml"), None);
    }

    #[test]
    fn 路径为空时不当成有效结果() {
        assert_eq!(parse_config_path("(config )"), None);
    }

    #[test]
    fn 已存在的文件绝不被覆盖() {
        // 这条守的是生产事故：这个文件里是库地址和管理口令，覆盖一次就等于
        // 把一台在跑的机器悄悄指向别的库。
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("config.toml");
        std::fs::write(&cfg, "url = \"生产库\"\n").unwrap();

        assert!(!write_if_absent(&cfg, "url = \"新的\"\n").unwrap());
        assert_eq!(std::fs::read_to_string(&cfg).unwrap(), "url = \"生产库\"\n");
    }

    #[test]
    fn 文件不存在时写出来并且只有本用户可读() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("sub").join("config.toml");

        assert!(write_if_absent(&cfg, "hello\n").unwrap());
        assert_eq!(std::fs::read_to_string(&cfg).unwrap(), "hello\n");

        let mode = std::fs::metadata(&cfg).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "文件权限是 {mode:o}");
        let dmode = std::fs::metadata(cfg.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dmode, 0o700, "目录权限是 {dmode:o}");
    }

    #[test]
    fn 空文件也算存在() {
        // 上游只检查文件在不在，空壳是合法的；把它当「没写过」会覆盖掉
        // 操作者故意清空的那份。
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("config.toml");
        std::fs::write(&cfg, "").unwrap();
        assert!(!write_if_absent(&cfg, "新内容").unwrap());
    }

    #[test]
    fn 查不到配置目录时退回兜底值() {
        let fallback = PathBuf::from("/tmp/__fd_fallback__");
        assert_eq!(
            cli_config_dir("__definitely_not_a_command__", &fallback),
            fallback
        );
    }
}
