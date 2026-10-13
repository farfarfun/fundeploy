//! OpenPencil：CLI 与 MCP 服务器（npm 全局包），外加可选的桌面安装包。
//!
//! 它没有常驻进程，所以不是服务。装完得到的是两个命令行程序，其中
//! `openpencil-mcp` 由 Claude Code / Cursor 这类客户端按需拉起、用完即走——
//! 进程的生死归客户端管，不归这里管。

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::{uninstall_env_var, Extra, Tool};
use crate::common::env;
use crate::common::fs::guard_removable;
use crate::common::github::{self, Asset, Release};
use crate::common::npm::Npm;
use crate::common::platform::{Arch, Os, Platform};
use crate::common::{cmd, download};
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
                    asset_names(release)
                )
            });
        }
        pick_asset(&release.assets, plat).ok_or_else(|| {
            anyhow::anyhow!(
                "{} 里没有适配本机（{} {}）的桌面安装包。\n\
                 这个 release 有: {}\n\
                 其中有能用的就设 OPEN_PENCIL_DESKTOP_ASSET=<附件名> 点名。",
                release.tag_name,
                plat.os.go(),
                plat.arch.go(),
                asset_names(release)
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

// -------------------------------------------------------------------- 附件挑选

/// 本平台能用的安装包后缀，按偏好排序。
///
/// Linux 上 AppImage 排第一：下下来 chmod +x 就能跑，不需要 root，也不碰系统包
/// 数据库。deb/rpm 要 sudo，装进去之后再卸是另一件事。
fn preferred_exts(os: Os) -> &'static [&'static str] {
    match os {
        Os::Linux => &[".appimage", ".deb", ".rpm"],
        Os::MacOs => &[".dmg"],
    }
}

/// 这个架构在附件名里可能的几种写法。
///
/// 同一个 Tauri 项目在不同版本里换过写法（`amd64` / `x86_64` / `x64` 都见过），
/// 所以全都认。
fn arch_tokens(arch: Arch) -> &'static [&'static str] {
    match arch {
        Arch::X86_64 => &["amd64", "x86_64", "x64"],
        Arch::Aarch64 => &["arm64", "aarch64"],
    }
}

/// 所有架构写法的并集，用来判断一个附件名里到底提没提架构。
const ALL_ARCH_TOKENS: &[&str] = &["amd64", "x86_64", "x64", "arm64", "aarch64"];

/// 从 release 的附件里挑一个本机能用的。
///
/// 按名字匹配而不是照着模板拼：上游改过附件命名（版本号的位置、架构写法），
/// 拼出来的名字一旦对不上就是 404，而实际上 release 里躺着一个能用的包。
/// 顺带也就支持了「某个版本只发了部分平台」——挑不到就明说，并把有哪些列出来。
fn pick_asset(assets: &[Asset], plat: Platform) -> Option<&Asset> {
    for ext in preferred_exts(plat.os) {
        let candidates: Vec<&Asset> = assets
            .iter()
            .filter(|a| a.name.to_ascii_lowercase().ends_with(ext))
            .collect();

        if let Some(a) = candidates.iter().find(|a| {
            let n = a.name.to_ascii_lowercase();
            arch_tokens(plat.arch).iter().any(|t| n.contains(t))
        }) {
            return Some(a);
        }
        // 名字里压根没提架构的，当通用包收下（macOS 的 universal dmg 就是这样）。
        // 但提了**别的**架构的不行——那是给另一台机器的。
        if let Some(a) = candidates.iter().find(|a| {
            let n = a.name.to_ascii_lowercase();
            !ALL_ARCH_TOKENS.iter().any(|t| n.contains(t))
        }) {
            return Some(a);
        }
    }
    None
}

fn asset_names(release: &Release) -> String {
    if release.assets.is_empty() {
        return "（一个附件都没有，多半是 CI 还没传完）".into();
    }
    release
        .assets
        .iter()
        .map(|a| a.name.as_str())
        .collect::<Vec<_>>()
        .join("、")
}

/// 下完之后该怎么用它。每种格式不一样，不说的话人还得自己查。
fn post_download(dest: &Path) -> Result<()> {
    let name = dest.file_name().unwrap_or_default().to_string_lossy();
    let lower = name.to_ascii_lowercase();
    let path = dest.display();

    if lower.ends_with(".appimage") {
        // AppImage 下下来是不可执行的，不 chmod 的话双击和命令行都是一句
        // 「Permission denied」。
        make_executable(dest)?;
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

#[cfg(unix)]
fn make_executable(p: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perm = std::fs::metadata(p)?.permissions();
    perm.set_mode(perm.mode() | 0o111);
    std::fs::set_permissions(p, perm).with_context(|| format!("改权限失败: {}", p.display()))
}

#[cfg(not(unix))]
fn make_executable(_p: &Path) -> Result<()> {
    Ok(())
}

/// 目录里有什么，按名字排序。目录不存在返回 None。
fn listing(dir: &Path) -> Option<Vec<String>> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| {
            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
            format!("{}  {}", e.file_name().to_string_lossy(), human_size(size))
        })
        .collect();
    out.sort();
    Some(out)
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
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

    fn assets(names: &[&str]) -> Vec<Asset> {
        names
            .iter()
            .map(|n| Asset {
                name: (*n).into(),
                browser_download_url: format!("https://example.invalid/{n}"),
            })
            .collect()
    }

    fn plat(os: Os, arch: Arch) -> Platform {
        Platform { os, arch }
    }

    #[test]
    fn linux_优先挑_appimage() {
        let a = assets(&[
            "OpenPencil_0.13.2_amd64.deb",
            "OpenPencil_0.13.2_amd64.AppImage",
            "OpenPencil_0.13.2_x64.dmg",
        ]);
        let got = pick_asset(&a, plat(Os::Linux, Arch::X86_64)).unwrap();
        assert_eq!(got.name, "OpenPencil_0.13.2_amd64.AppImage");
    }

    #[test]
    fn 没有_appimage_时退到_deb() {
        let a = assets(&["OpenPencil_0.13.2_amd64.deb"]);
        let got = pick_asset(&a, plat(Os::Linux, Arch::X86_64)).unwrap();
        assert_eq!(got.name, "OpenPencil_0.13.2_amd64.deb");
    }

    #[test]
    fn 架构不对的附件不会被当成本机的() {
        // 这条最要紧：拿一个 arm64 的包在 x86 机器上跑，报错是「格式不对」，
        // 跟「上游没发这个平台」看着完全不同。
        let a = assets(&["OpenPencil_0.13.2_arm64.AppImage"]);
        assert!(pick_asset(&a, plat(Os::Linux, Arch::X86_64)).is_none());
    }

    #[test]
    fn 架构的几种写法都认() {
        for name in [
            "OpenPencil_1.0.0_x86_64.AppImage",
            "OpenPencil_1.0.0_amd64.AppImage",
            "OpenPencil_1.0.0_x64.AppImage",
        ] {
            let a = assets(&[name]);
            assert!(
                pick_asset(&a, plat(Os::Linux, Arch::X86_64)).is_some(),
                "没认出 {name}"
            );
        }
        for name in [
            "OpenPencil_1.0.0_aarch64.AppImage",
            "OpenPencil_1.0.0_arm64.AppImage",
        ] {
            let a = assets(&[name]);
            assert!(
                pick_asset(&a, plat(Os::Linux, Arch::Aarch64)).is_some(),
                "没认出 {name}"
            );
        }
    }

    #[test]
    fn 名字里不提架构的当通用包() {
        // macOS 的 universal dmg 不带架构后缀。
        let a = assets(&["OpenPencil_1.0.0_universal.dmg"]);
        assert!(pick_asset(&a, plat(Os::MacOs, Arch::Aarch64)).is_some());
    }

    #[test]
    fn macos_按架构区分两份_dmg() {
        let a = assets(&["OpenPencil_1.0.0_x64.dmg", "OpenPencil_1.0.0_aarch64.dmg"]);
        assert_eq!(
            pick_asset(&a, plat(Os::MacOs, Arch::Aarch64)).unwrap().name,
            "OpenPencil_1.0.0_aarch64.dmg"
        );
        assert_eq!(
            pick_asset(&a, plat(Os::MacOs, Arch::X86_64)).unwrap().name,
            "OpenPencil_1.0.0_x64.dmg"
        );
    }

    #[test]
    fn 大小写不影响匹配() {
        let a = assets(&["OpenPencil_1.0.0_AMD64.appimage"]);
        assert!(pick_asset(&a, plat(Os::Linux, Arch::X86_64)).is_some());
    }

    /// 上游 v0.15.1 的真实附件清单。
    ///
    /// 挑附件这件事唯一会出错的地方就是上游改了命名，所以拿一份真的来钉住：
    /// 每个 `.sig`、`latest.json`、Windows 的 exe/msi 都在里面，一条规则漏了
    /// 就会挑中它们中的某一个。
    const V0_15_1: &[&str] = &[
        "latest.json",
        "OpenPencil-0.15.1-1.x86_64.rpm",
        "OpenPencil-0.15.1-1.x86_64.rpm.sig",
        "OpenPencil_0.15.1_aarch64.dmg",
        "OpenPencil_0.15.1_amd64.AppImage",
        "OpenPencil_0.15.1_amd64.AppImage.sig",
        "OpenPencil_0.15.1_amd64.deb",
        "OpenPencil_0.15.1_amd64.deb.sig",
        "OpenPencil_0.15.1_arm64-setup.exe",
        "OpenPencil_0.15.1_arm64_en-US.msi",
        "OpenPencil_0.15.1_x64-setup.exe",
        "OpenPencil_0.15.1_x64.dmg",
        "OpenPencil_0.15.1_x64_en-US.msi",
        "OpenPencil_aarch64.app.tar.gz",
    ];

    #[test]
    fn 从真实的_release_里挑对了() {
        let a = assets(V0_15_1);
        assert_eq!(
            pick_asset(&a, plat(Os::Linux, Arch::X86_64)).unwrap().name,
            "OpenPencil_0.15.1_amd64.AppImage"
        );
        assert_eq!(
            pick_asset(&a, plat(Os::MacOs, Arch::Aarch64)).unwrap().name,
            "OpenPencil_0.15.1_aarch64.dmg"
        );
        assert_eq!(
            pick_asset(&a, plat(Os::MacOs, Arch::X86_64)).unwrap().name,
            "OpenPencil_0.15.1_x64.dmg"
        );
        // 上游这一版没给 linux/arm64 发任何包。挑不到要明说，不能拿 x86 的凑数。
        assert!(pick_asset(&a, plat(Os::Linux, Arch::Aarch64)).is_none());
    }

    #[test]
    fn 不相干的附件不会被挑中() {
        // release 里常常还挂着 latest.json、签名文件这些。
        let a = assets(&["latest.json", "OpenPencil_1.0.0_amd64.AppImage.sig"]);
        assert!(pick_asset(&a, plat(Os::Linux, Arch::X86_64)).is_none());
    }

    #[test]
    fn 没有附件时挑不出东西() {
        assert!(pick_asset(&[], plat(Os::Linux, Arch::X86_64)).is_none());
    }

    #[test]
    fn 字节数转成人看的单位() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1024), "1.0 KiB");
        assert_eq!(human_size(1024 * 1024 * 3 / 2), "1.5 MiB");
    }

    #[test]
    fn 额外动作不收参数() {
        let t = OpenPencil::from_env();
        let err = t
            .extra("mcp-config", &["多余的".to_string()])
            .unwrap_err()
            .to_string();
        assert!(err.contains("不收参数"), "实际: {err}");
    }
}
