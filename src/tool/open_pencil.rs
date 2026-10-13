//! OpenPencil：CLI 与 MCP 服务器（npm 全局包），外加可选的桌面安装包。
//!
//! 它没有常驻进程，所以不是服务。装完得到的是两个命令行程序，其中
//! `openpencil-mcp` 由 Claude Code / Cursor 这类客户端按需拉起、用完即走——
//! 进程的生死归客户端管，不归这里管。

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::{uninstall_env_var, Extra, Tool};
use crate::common::env;
use crate::common::fs::{guard_removable, listing};
use crate::common::github::{self, Asset, Release};
use crate::common::npm::Npm;
use crate::common::platform::Platform;
use crate::common::{archive, asset, cmd, download};
use crate::ui;

const DEFAULT_REPO: &str = "open-pencil/open-pencil";
const DEFAULT_CLI_PKG: &str = "@open-pencil/cli";
const DEFAULT_MCP_PKG: &str = "@open-pencil/mcp";

/// MCP 服务器装完之后落在 PATH 上的可执行文件名。客户端配置里填的就是它。
const MCP_BIN: &str = "openpencil-mcp";
const CLI_BIN: &str = "openpencil";

const EXTRAS: &[Extra] = &[
    Extra {
        name: "install-desktop",
        about: "下载桌面版安装包（Tauri）",
    },
    Extra {
        name: "mcp-config",
        about: "打印 MCP 客户端的接入配置",
    },
];

pub struct OpenPencil {
    repo: String,
    home: PathBuf,
    cli_pkg: String,
    mcp_pkg: String,
    /// npm 包版本。None 表示跟最新版。
    version: Option<String>,
    /// 指定用哪个 npm/pnpm。
    npm_bin: Option<String>,
    /// 桌面包的版本 tag。None 表示取 release 里的 latest。
    desktop_version: Option<String>,
    /// 点名要哪个附件，用于自动挑选不灵时兜底。
    desktop_asset: Option<String>,
}

impl OpenPencil {
    pub fn from_env() -> Self {
        Self {
            repo: env::str_or("OPEN_PENCIL_GITHUB_REPO", DEFAULT_REPO),
            // 旧名 OPEN_PENCIL_SERVICE_HOME 仍然认：它是 Bash 时代把 open-pencil
            // 当服务管时留下的，机器上已经有的 dist/ 不该因为改了个名就找不到。
            home: env::opt("OPEN_PENCIL_HOME")
                .or_else(|| env::opt("OPEN_PENCIL_SERVICE_HOME"))
                .map(|s| env::expand_tilde(&s))
                .unwrap_or_else(|| env::home().join("opt/open-pencil")),
            cli_pkg: env::str_or("OPEN_PENCIL_CLI_PACKAGE", DEFAULT_CLI_PKG),
            mcp_pkg: env::str_or("OPEN_PENCIL_MCP_PACKAGE", DEFAULT_MCP_PKG),
            // Bash 里这个变量的默认值是字面量 "latest"，显式写 latest 等同于不写。
            version: env::opt("OPEN_PENCIL_VERSION").filter(|v| v != "latest"),
            npm_bin: env::opt("OPEN_PENCIL_NPM_BIN"),
            desktop_version: env::opt("OPEN_PENCIL_DESKTOP_VERSION"),
            desktop_asset: env::opt("OPEN_PENCIL_DESKTOP_ASSET"),
        }
    }

    fn dist_dir(&self) -> PathBuf {
        self.home.join("dist")
    }

    fn packages(&self) -> [&str; 2] {
        [&self.cli_pkg, &self.mcp_pkg]
    }

    /// 装或升两个包。`version` 为 None 时跟最新版。
    fn provision(&self, version: Option<&str>) -> Result<()> {
        for pkg in self.packages() {
            let npm = Npm::for_package(pkg, self.npm_bin.as_deref())?;
            if let Some(w) = npm.shadow_warning(pkg) {
                ui::warn(w);
            }
            let spec = npm.spec(pkg, version);
            ui::step(format!("{} 安装 {spec}", npm.name()));
            npm.install_global(&spec)
                .with_context(|| format!("装 {spec} 失败。权限不够的话用 nvm/volta 这类无需 sudo 的 Node 版本管理器，别 sudo npm -g"))?;
        }
        ui::blank();
        self.print_versions();
        Ok(())
    }

    fn installed_version(&self, pkg: &str) -> Option<String> {
        Npm::for_package(pkg, self.npm_bin.as_deref())
            .ok()?
            .installed_version(pkg)
    }

    fn print_versions(&self) {
        for pkg in self.packages() {
            ui::kv(pkg, self.installed_version(pkg).as_deref());
        }
    }

    // ------------------------------------------------------------ 桌面安装包

    /// 下载桌面版安装包。
    fn install_desktop(&self) -> Result<()> {
        let plat = Platform::current()?;
        let release = match &self.desktop_version {
            Some(v) => {
                let tag = github::tag_of(v)?;
                github::by_tag(&self.repo, &tag)?
            }
            None => github::latest(&self.repo)?,
        };

        let asset = self.choose_asset(&release, plat)?;
        let dest = self.dist_dir().join(&asset.name);

        ui::step(format!("下载 {} {}", release.tag_name, asset.name));
        download::hint_mirror();
        download::to_file(
            &asset.browser_download_url,
            &dest,
            &format!("open-pencil {}", release.tag_name),
        )?;

        post_download(&dest)?;
        Ok(())
    }

    fn choose_asset<'a>(&self, release: &'a Release, plat: Platform) -> Result<&'a Asset> {
        if let Some(name) = &self.desktop_asset {
            return release.asset(name).ok_or_else(|| {
                anyhow::anyhow!(
                    "{} 上没有附件 {name}。这个 release 有: {}",
                    release.tag_name,
                    asset::names(release)
                )
            });
        }
        asset::pick(&release.assets, plat).ok_or_else(|| {
            anyhow::anyhow!(
                "{} 里没有适配本机（{} {}）的桌面安装包。\n\
                 这个 release 有: {}\n\
                 其中有能用的就设 OPEN_PENCIL_DESKTOP_ASSET=<附件名> 点名。",
                release.tag_name,
                plat.os.go(),
                plat.arch.go(),
                asset::names(release)
            )
        })
    }

    // ------------------------------------------------------------ MCP 配置

    fn mcp_config(&self) -> Result<()> {
        ui::heading("OpenPencil MCP 接入");
        self.print_versions();

        let bin = cmd::find(MCP_BIN);
        ui::kv(MCP_BIN, bin.as_deref());
        if bin.is_none() {
            ui::warn(format!(
                "PATH 上没有 {MCP_BIN}。客户端是按这个名字拉起进程的，\
                 配置填了也会在启动时失败——先确认包装好了，再确认 npm 的全局 bin 目录在 PATH 上。"
            ));
        }
        ui::blank();

        ui::heading("Claude Code（用户级 settings.json）");
        println!("{CLAUDE_SNIPPET}");
        ui::blank();
        ui::heading("Cursor / Windsurf 等其它 MCP 客户端");
        println!("{GENERIC_SNIPPET}");
        ui::blank();
        ui::hint(format!(
            "Claude Code 也可以一条命令加好: claude mcp add --scope user open-pencil -- {MCP_BIN}"
        ));
        ui::hint("工具清单: https://openpencil.dev/reference/mcp-tools");
        Ok(())
    }
}

impl Tool for OpenPencil {
    fn name(&self) -> &str {
        "open-pencil"
    }

    fn summary(&self) -> &str {
        "OpenPencil CLI 与 MCP 服务器"
    }

    fn install(&self, version: Option<&str>) -> Result<()> {
        // 命令行给的版本优先于 OPEN_PENCIL_VERSION：前者是这一次的意图，
        // 后者是这台机器的长期设置。
        self.provision(version.or(self.version.as_deref()))
    }

    fn upgrade(&self) -> Result<()> {
        // 升级就是去拿最新版，所以这里不看 OPEN_PENCIL_VERSION——那个变量钉的是
        // 「这台机器要哪一版」，拿它来升级等于原地装一遍。要钉版本用 install。
        self.provision(None)
    }

    fn uninstall(&self) -> Result<()> {
        ui::confirm_destructive(
            &format!(
                "将卸载 {} 与 {}，并删除 {}（含已下载的桌面安装包），确认？",
                self.cli_pkg,
                self.mcp_pkg,
                self.home.display()
            ),
            &uninstall_env_var(self.name()),
        )?;
        guard_removable(&self.home)?;

        // 先 MCP 后 CLI：装的时候 CLI 在前，卸的时候倒过来，免得中途失败留下一个
        // 「CLI 没了、MCP 还在」的状态——客户端拉起 MCP 时才会发现。
        for pkg in [&self.mcp_pkg, &self.cli_pkg] {
            let Ok(npm) = Npm::for_package(pkg, self.npm_bin.as_deref()) else {
                ui::warn(format!("找不到 npm/pnpm，{pkg} 没能卸掉。"));
                continue;
            };
            ui::step(format!("{} 卸载 {pkg}", npm.name()));
            if !npm.uninstall_global(pkg) {
                ui::warn(format!("卸 {pkg} 没成功（多半本来就没装），继续。"));
            }
        }

        if self.home.exists() {
            std::fs::remove_dir_all(&self.home)
                .with_context(|| format!("删除失败: {}", self.home.display()))?;
            ui::ok(format!("已删除 {}。", self.home.display()));
        }
        Ok(())
    }

    fn status(&self) -> Result<()> {
        ui::heading("open-pencil");
        ui::kv("目录", Some(&self.home.display().to_string()));
        ui::kv("上游仓库", Some(&self.repo));
        ui::kv("版本", Some(self.version.as_deref().unwrap_or("最新版")));
        ui::blank();

        ui::heading("npm 全局包");
        self.print_versions();
        ui::blank();

        ui::heading("PATH 上的可执行文件");
        ui::kv(CLI_BIN, cmd::find(CLI_BIN).as_deref());
        ui::kv(MCP_BIN, cmd::find(MCP_BIN).as_deref());
        ui::blank();

        ui::heading("桌面安装包");
        let dist = self.dist_dir();
        match listing(&dist) {
            Some(lines) if !lines.is_empty() => {
                for l in lines {
                    println!("  {l}");
                }
            }
            _ => ui::hint("还没下载。要的话: fundeploy tool open-pencil install-desktop"),
        }
        Ok(())
    }

    fn extras(&self) -> &'static [Extra] {
        EXTRAS
    }

    fn extra(&self, name: &str, args: &[String]) -> Result<()> {
        if !args.is_empty() {
            bail!("{name} 不收参数，多给了: {}", args.join(" "));
        }
        match name {
            "install-desktop" => self.install_desktop(),
            "mcp-config" => self.mcp_config(),
            _ => bail!("open-pencil 声明了动作 {name} 却没有实现它"),
        }
    }
}

/// 下完之后该怎么用它。每种格式不一样，不说的话人还得自己查。
fn post_download(dest: &Path) -> Result<()> {
    let name = dest.file_name().unwrap_or_default().to_string_lossy();
    let lower = name.to_ascii_lowercase();
    let path = dest.display();

    if lower.ends_with(".appimage") {
        // AppImage 下下来是不可执行的，不 chmod 的话双击和命令行都是一句
        // 「Permission denied」。
        archive::make_executable(dest)?;
        ui::ok(format!("已下载，直接执行: {path}"));
    } else if lower.ends_with(".deb") {
        ui::ok(format!("已下载，安装: sudo dpkg -i {path}"));
    } else if lower.ends_with(".rpm") {
        ui::ok(format!("已下载，安装: sudo rpm -ivh {path}"));
    } else if lower.ends_with(".dmg") {
        ui::ok(format!("已下载，在 Finder 里挂载安装: {path}"));
    } else {
        ui::ok(format!("已下载: {path}"));
    }
    Ok(())
}

const CLAUDE_SNIPPET: &str = r#"  {
    "permissions": { "allow": ["mcp__open-pencil__*"] },
    "mcpServers": {
      "open-pencil": { "command": "openpencil-mcp" }
    }
  }"#;

const GENERIC_SNIPPET: &str = r#"  {
    "mcpServers": {
      "open-pencil": { "command": "openpencil-mcp" }
    }
  }"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 额外动作不收参数() {
        let t = OpenPencil::from_env();
        let err = t
            .extra("mcp-config", &["多余的".to_string()])
            .unwrap_err()
            .to_string();
        assert!(err.contains("不收参数"), "实际: {err}");
    }

    #[test]
    fn 旧的_home_变量名仍然认() {
        // Bash 时代把 open-pencil 当服务管，机器上已经有的 dist/ 不该因为改了个
        // 变量名就找不到。
        std::env::remove_var("OPEN_PENCIL_HOME");
        std::env::set_var("OPEN_PENCIL_SERVICE_HOME", "/opt/旧地方");
        assert_eq!(OpenPencil::from_env().home, PathBuf::from("/opt/旧地方"));
        // 两个都设的时候新名字说了算。
        std::env::set_var("OPEN_PENCIL_HOME", "/opt/新地方");
        assert_eq!(OpenPencil::from_env().home, PathBuf::from("/opt/新地方"));
        std::env::remove_var("OPEN_PENCIL_HOME");
        std::env::remove_var("OPEN_PENCIL_SERVICE_HOME");
    }

    #[test]
    fn 字面量_latest_等同于不钉版本() {
        std::env::set_var("OPEN_PENCIL_VERSION", "latest");
        assert_eq!(OpenPencil::from_env().version, None);
        std::env::set_var("OPEN_PENCIL_VERSION", "0.15.1");
        assert_eq!(OpenPencil::from_env().version.as_deref(), Some("0.15.1"));
        std::env::remove_var("OPEN_PENCIL_VERSION");
    }
}
