//! 文件系统上那几件需要先想一下再做的事。

use std::path::Path;

use anyhow::{bail, Result};

use super::env;

/// 拦住明显不该被递归删除的路径。
///
/// 要删的目录是从环境变量拼出来的，变量空掉时拼出来的可能就是 `/` 或者家目录本身。
/// `rm -rf $HOME` 这种事只会发生一次，所以在真正动手之前先问一句这像不像一个
/// 安装目录。
pub fn guard_removable(p: &Path) -> Result<()> {
    let home = env::home();
    if p == Path::new("/") || p == home {
        bail!("拒绝删除 {}：这不像是安装目录。", p.display());
    }
    if !p.starts_with(&home) && !p.starts_with("/opt") {
        bail!(
            "{} 既不在 {} 下也不在 /opt 下，拒绝自动删除。确认无误的话手工处理。",
            p.display(),
            home.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 拒绝删除根目录和家目录() {
        assert!(guard_removable(Path::new("/")).is_err());
        assert!(guard_removable(&env::home()).is_err());
    }

    #[test]
    fn 拒绝删除家目录和_opt_之外的路径() {
        assert!(guard_removable(Path::new("/etc")).is_err());
        assert!(guard_removable(Path::new("/var/lib/x")).is_err());
    }

    #[test]
    fn 放行家目录和_opt_下的安装目录() {
        assert!(guard_removable(&env::home().join("opt/airflow")).is_ok());
        assert!(guard_removable(Path::new("/opt/airflow")).is_ok());
    }
}
