//! farfarfun 已发布到组织 apt 源和 Homebrew tap 的工具。

use anyhow::{bail, Context, Result};

use super::{uninstall_env_var, Tool};
use crate::common::cmd;
use crate::common::pm::Pm;
use crate::ui;

pub struct FarfarfunTool {
    pub name: &'static str,
    pub summary: &'static str,
    pub desktop: bool,
}

impl FarfarfunTool {
    fn package_manager(&self) -> Result<Pm> {
        if cfg!(target_os = "macos") {
            return cmd::has("brew")
                .then_some(Pm::Brew)
                .ok_or_else(|| anyhow::anyhow!("先安装 Homebrew: fundeploy tool brew install"));
        }
        if cmd::has("apt-get") {
            return Ok(Pm::Apt);
        }
        if cmd::has("brew") && !self.desktop {
            return Ok(Pm::Brew);
        }
        bail!(
            "{} 目前支持 Debian/Ubuntu 的 apt{}",
            self.name,
            if self.desktop {
                " 和 macOS 的 Homebrew cask"
            } else {
                " 或 Homebrew"
            }
        )
    }

    fn apt_version(&self) -> Option<String> {
        let out = cmd::try_run(["dpkg-query", "-W", "-f=${Status}\t${Version}", self.name]).ok()?;
        if !out.success() {
            return None;
        }
        parse_dpkg_status(&out.stdout)
    }

    fn brew_version(&self) -> Option<String> {
        if !cmd::has("brew") {
            return None;
        }
        let mut args = vec!["brew", "list", "--versions"];
        if self.desktop {
            args.push("--cask");
        }
        args.push(self.name);
        let out = cmd::try_run(args).ok()?;
        if !out.success() {
            return None;
        }
        out.stdout.split_whitespace().nth(1).map(str::to_string)
    }

    fn provision(&self, pm: Pm) -> Result<()> {
        ui::step(format!("用 {} 安装 {}", pm.name(), self.name));
        pm.install(&[self.name]).with_context(|| match pm {
            Pm::Apt => "请先按 README 添加 farfarfun 的 apt 源".to_string(),
            Pm::Brew => "请先运行 brew tap farfarfun/packages https://github.com/farfarfun/packages，再运行 brew trust farfarfun/packages".to_string(),
            _ => unreachable!(),
        })?;
        ui::ok(format!("{} 已安装", self.name));
        Ok(())
    }
}

impl Tool for FarfarfunTool {
    fn name(&self) -> &str {
        self.name
    }

    fn summary(&self) -> &str {
        self.summary
    }

    fn install(&self, version: Option<&str>) -> Result<()> {
        if let Some(version) = version {
            bail!("{} 的包管理器安装不支持指定版本 {version}", self.name);
        }
        self.provision(self.package_manager()?)
    }

    fn upgrade(&self) -> Result<()> {
        let apt = self.apt_version().is_some();
        let brew = self.brew_version().is_some();
        if !apt && !brew && !self.desktop && cmd::has(self.name) {
            bail!(
                "{} 已在 PATH 上，但不是 apt/Homebrew 安装；请用原安装渠道升级",
                self.name
            );
        }
        if apt {
            self.provision(Pm::Apt)?;
        }
        if brew {
            self.provision(Pm::Brew)?;
        }
        if !apt && !brew {
            self.install(None)?;
        }
        Ok(())
    }

    fn uninstall(&self) -> Result<()> {
        let apt = self.apt_version().is_some();
        let brew = self.brew_version().is_some();
        if !apt && !brew {
            ui::info(format!("{} 未由 apt 或 Homebrew 安装", self.name));
            return Ok(());
        }
        ui::confirm_destructive(
            &format!("要卸载 {} 吗", self.name),
            &uninstall_env_var(self.name),
        )?;
        if apt {
            Pm::Apt.uninstall(&[self.name])?;
        }
        if brew {
            Pm::Brew.uninstall(&[self.name])?;
        }
        ui::ok(format!("{} 已卸载", self.name));
        Ok(())
    }

    fn status(&self) -> Result<()> {
        let apt = self.apt_version();
        let brew = self.brew_version();
        let cli = (!self.desktop).then(|| cmd::find(self.name)).flatten();
        let installed = apt.is_some() || brew.is_some() || cli.is_some();
        let mut card = ui::Card::new(self.name).badge(
            if installed {
                ui::State::Up
            } else {
                ui::State::Idle
            },
            if installed { "已安装" } else { "未安装" },
        );
        if let Some(version) = apt {
            card = card.row("apt 版本", version);
        }
        if let Some(version) = brew {
            card = card.row("Homebrew 版本", version);
        }
        if !self.desktop {
            card = card.row_state(
                "PATH 上的命令",
                if cli.is_some() {
                    ui::State::Up
                } else {
                    ui::State::Idle
                },
                cli.unwrap_or_else(|| "没找到".into()),
            );
        }
        card.print();
        Ok(())
    }
}

fn parse_dpkg_status(output: &str) -> Option<String> {
    output
        .trim()
        .strip_prefix("install ok installed\t")
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 安装状态只认完整安装而非残留配置() {
        assert_eq!(
            parse_dpkg_status("install ok installed\t1.2.3"),
            Some("1.2.3".into())
        );
        assert_eq!(parse_dpkg_status("deinstall ok config-files\t1.2.3"), None);
    }
}
