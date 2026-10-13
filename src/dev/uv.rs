//! uv（Astral 的 Python 包管理器）。

use std::path::PathBuf;

use anyhow::Result;

use super::{install_root, run_installer, set, Dev};
use crate::common::pm::Pm;
use crate::common::{archive, cmd, env};
use crate::ui;

fn install_url() -> String {
    env::str_or(
        "FUNDEPLOY_UV_INSTALL_URL",
        "https://astral.sh/uv/install.sh",
    )
}

pub struct Uv;

impl Uv {
    /// 装好的 uv 在哪儿。
    ///
    /// 不靠猜：上游安装器在不同版本里把二进制放过 `<dir>` 也放过 `<dir>/bin`，
    /// 猜错了提示出来的 PATH 就是错的，而人照着配完会发现命令仍旧找不到。
    fn located(&self) -> Option<PathBuf> {
        archive::find_file(&self.root(), "uv")
    }
}

impl Dev for Uv {
    fn name(&self) -> &str {
        "uv"
    }

    fn summary(&self) -> &str {
        "uv：Python 包与环境管理"
    }

    fn pkgs(&self, pm: Pm) -> &'static [&'static str] {
        match pm {
            // uv 还很新，Debian/Ubuntu 的稳定源里普遍没有；给了包名也只会
            // 换来一句「软件包未找到」，不如直接让它走 source。
            Pm::Apt | Pm::Yum => &[],
            _ => &["uv"],
        }
    }

    fn root(&self) -> PathBuf {
        install_root("FUNDEPLOY_UV_ROOT", "uv")
    }

    fn bin_dir(&self) -> Option<PathBuf> {
        self.located().and_then(|p| p.parent().map(PathBuf::from))
    }

    fn probe(&self) -> (&'static str, &'static [&'static str]) {
        ("uv", &["--version"])
    }

    fn install_source(&self, version: Option<&str>) -> Result<()> {
        if let Some(v) = version {
            anyhow::bail!("uv 的官方安装脚本只装最新版，钉不到 {v}。");
        }
        let root = self.root();
        std::fs::create_dir_all(&root)?;
        run_installer(
            &install_url(),
            "uv",
            &[],
            &[
                set("UV_INSTALL_DIR", root.to_string_lossy().as_ref()),
                // 跟这一域其余几样保持一致：PATH 由我们打印出来，不替人改 rc 文件。
                set("INSTALLER_NO_MODIFY_PATH", "1"),
            ],
        )?;
        match self.located() {
            Some(p) => ui::ok(format!("uv 装到了 {}", p.display())),
            None => anyhow::bail!("安装脚本跑完了，但 {} 下找不到 uv", root.display()),
        }
        Ok(())
    }

    fn upgrade_source(&self) -> Result<()> {
        let Some(uv) = self.located() else {
            ui::info("没找到装在 ~/opt 下的 uv，改为重新安装。");
            return self.install_source(None);
        };
        cmd::run([uv.to_string_lossy().as_ref(), "self", "update"])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apt_下不给包名() {
        // Debian/Ubuntu 稳定源里没有 uv，给了只会换来「软件包未找到」。
        assert!(Uv.pkgs(Pm::Apt).is_empty());
        assert_eq!(Uv.pkgs(Pm::Brew), &["uv"]);
    }

    #[test]
    fn 安装脚本地址可以改() {
        static M: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _g = M.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(install_url(), "https://astral.sh/uv/install.sh");
        std::env::set_var("FUNDEPLOY_UV_INSTALL_URL", "https://example.com/i.sh");
        assert_eq!(install_url(), "https://example.com/i.sh");
        std::env::remove_var("FUNDEPLOY_UV_INSTALL_URL");
    }

    #[test]
    fn 没装的时候问不出_bin_目录() {
        // 目录不在就该是 None，而不是一个指向空气的路径——报出去人会照着配 PATH。
        static M: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _g = M.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("FUNDEPLOY_UV_ROOT", "/nonexistent/fundeploy/uv");
        assert!(Uv.bin_dir().is_none());
        std::env::remove_var("FUNDEPLOY_UV_ROOT");
    }
}
