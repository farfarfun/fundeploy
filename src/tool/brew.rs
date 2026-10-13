//! Homebrew：跑上游的官方安装器。
//!
//! 装 Homebrew 这件事本身不值得自己实现一遍——官方脚本要处理 Xcode 命令行工具、
//! Linux 上的 glibc 版本门槛、sudo 提权、装到哪个 prefix。这里只负责两件它不管
//! 的事：把脚本经由镜像改写层取下来（它在 raw.githubusercontent.com 上，直连常
//! 常是几 KB/s），以及在跑它之前把话问清楚。

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

use super::{uninstall_env_var, Tool};
use crate::common::cmd::{self, EnvEdit};
use crate::common::fs::Scratch;
use crate::common::{download, env};
use crate::source::brew as source_brew;
use crate::ui;

const DEFAULT_INSTALL_URL: &str =
    "https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh";
const DEFAULT_UNINSTALL_URL: &str =
    "https://raw.githubusercontent.com/Homebrew/install/HEAD/uninstall.sh";

/// 官方安装器会把 brew 放在这几个地方之一。
///
/// 装完它并不会动你的 shell 配置，所以当前这个 shell 的 PATH 上多半还是没有
/// brew——只认 PATH 的话，刚装完再查一次会得到「没装」。
const KNOWN_PATHS: &[&str] = &[
    "/opt/homebrew/bin/brew",
    "/usr/local/bin/brew",
    "/home/linuxbrew/.linuxbrew/bin/brew",
];

pub struct Brew {
    install_url: String,
    uninstall_url: String,
}

impl Brew {
    pub fn from_env() -> Self {
        Self {
            install_url: env::str_or("BREW_INSTALL_URL", DEFAULT_INSTALL_URL),
            uninstall_url: env::str_or("BREW_UNINSTALL_URL", DEFAULT_UNINSTALL_URL),
        }
    }

    fn find(&self) -> Option<PathBuf> {
        if let Some(p) = cmd::find("brew") {
            return Some(PathBuf::from(p));
        }
        KNOWN_PATHS
            .iter()
            .map(Path::new)
            .find(|p| p.is_file())
            .map(Path::to_path_buf)
    }

    /// 下载官方脚本并执行它。
    fn run_official(&self, url: &str, args: &[&str], env: &[EnvEdit]) -> Result<()> {
        let scratch = Scratch::new("brew")?;
        let script = scratch.path().join("brew-official.sh");

        ui::step(format!("下载 {url}"));
        download::hint_mirror();
        download::to_file(url, &script, "Homebrew 官方脚本")?;
        // 镜像站出错时常常回一个 200 + 空响应或者错误页。空文件交给 bash 是
        // 「什么都没发生，退出码 0」——报成功而 brew 根本没装。
        if std::fs::metadata(&script).map(|m| m.len()).unwrap_or(0) == 0 {
            bail!("下载到的脚本是空的: {url}");
        }
        ui::hint(format!("要先过目的话: {}", script.display()));

        // 落盘执行文件，而不是把正文插值进 `bash -c "<脚本正文>"`：后者既脆弱
        // （引号、长度），也让「先读一眼再执行」无从谈起。
        let mut argv = vec!["/bin/bash".to_string(), script.display().to_string()];
        argv.extend(args.iter().map(|s| s.to_string()));
        cmd::run_with(argv, env)
    }
}

impl Tool for Brew {
    fn name(&self) -> &str {
        "brew"
    }

    fn summary(&self) -> &str {
        "Homebrew 包管理器"
    }

    fn install(&self, version: Option<&str>) -> Result<()> {
        if let Some(v) = version {
            bail!("Homebrew 的官方安装器不收版本号（装的总是当前 HEAD），--version {v} 没处使。");
        }
        if let Some(brew) = self.find() {
            ui::ok(format!("已安装: {}", brew.display()));
            let _ = cmd::run([brew.as_os_str().to_string_lossy().as_ref(), "--version"]);
            return Ok(());
        }

        // 官方安装器自 2022 年起就认 HOMEBREW_*_GIT_REMOTE / *_DOMAIN，所以配过源
        // 的机器第一次装就能走镜像。不带的话，它会去 GitHub 克隆几百 MB 的 brew
        // 仓库——这正是国内装 brew 卡住的那一步。
        let env = source_brew::install_env();
        if !env.is_empty() {
            ui::info("走已配好的 brew 镜像安装。");
        } else {
            ui::hint("直连 GitHub 可能很慢。先 fundeploy source brew use 换个源再装会快很多。");
        }
        self.run_official(&self.install_url, &[], &env)?;

        let Some(brew) = self.find() else {
            bail!(
                "安装器跑完了，却没在 PATH 和常见位置上找到 brew。按它最后几行的提示配 PATH 再试。"
            );
        };
        ui::ok(format!("已安装: {}", brew.display()));
        ui::hint(format!(
            "当前 shell 里还用不了，加进 shell 配置: eval \"$({} shellenv)\"",
            brew.display()
        ));
        Ok(())
    }

    fn upgrade(&self) -> Result<()> {
        let Some(brew) = self.find() else {
            ui::info("还没装，先装上。");
            return self.install(None);
        };
        // brew update 更的是 Homebrew 自己和配方库，不动已装的包——那是
        // `brew upgrade`，会把一机器的软件都升一遍，不该由这里替人决定。
        ui::step("brew update");
        cmd::run([brew.as_os_str().to_string_lossy().as_ref(), "update"])?;
        ui::hint("要升已装的软件: brew upgrade");
        Ok(())
    }

    fn uninstall(&self) -> Result<()> {
        if self.find().is_none() {
            ui::info("Homebrew 没装，跳过。");
            return Ok(());
        }
        ui::confirm_destructive(
            "将卸载 Homebrew **以及用它装过的全部软件**，确认？",
            &uninstall_env_var(self.name()),
        )?;
        // NONINTERACTIVE=1：确认已经在上面问过了，卸载器再问一遍只会让这条命令
        // 在脚本里卡住。--force 是它跳过自己那句确认的开关。
        self.run_official(
            &self.uninstall_url,
            &["--force"],
            &[("NONINTERACTIVE".to_string(), Some("1".to_string()))],
        )?;
        if let Some(brew) = self.find() {
            ui::warn(format!(
                "卸载器跑完了，{} 还在。它对自己装不下的东西（另一个 prefix、root 装的）无能为力，剩下的得手工清。",
                brew.display()
            ));
        }
        Ok(())
    }

    fn status(&self) -> Result<()> {
        ui::heading("brew");
        let Some(brew) = self.find() else {
            ui::kv("可执行文件", None);
            ui::hint("安装: fundeploy tool brew install");
            return Ok(());
        };
        ui::kv("可执行文件", Some(&brew.display().to_string()));
        if cmd::find("brew").is_none() {
            ui::warn(format!(
                "装了但不在 PATH 上。加进 shell 配置: eval \"$({} shellenv)\"",
                brew.display()
            ));
        }
        let brew = brew.display().to_string();
        if let Ok(prefix) = cmd::capture([&brew, "--prefix"]) {
            ui::kv("prefix", Some(&prefix));
        }
        if let Ok(v) = cmd::capture([&brew, "--version"]) {
            for line in v.lines() {
                ui::hint(line);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 指定版本要报错而不是装个最新版() {
        let err = Brew::from_env().install(Some("4.2.0")).unwrap_err();
        assert!(err.to_string().contains("不收版本号"), "实际: {err}");
    }

    #[test]
    fn 脚本地址默认指向官方且可以覆盖() {
        let b = Brew::from_env();
        assert!(b.install_url.ends_with("/install.sh"));
        assert!(b.uninstall_url.ends_with("/uninstall.sh"));

        std::env::set_var("BREW_INSTALL_URL", "https://example.invalid/x.sh");
        assert_eq!(Brew::from_env().install_url, "https://example.invalid/x.sh");
        std::env::remove_var("BREW_INSTALL_URL");
    }
}
