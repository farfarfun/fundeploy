//! Cockpit Tools：AI IDE 的账号管理器（上游是个 Tauri 桌面应用）。
//!
//! 它有窗口但没有常驻进程——点开用、关掉就没了，起停归桌面环境管，所以是工具
//! 不是服务。这里只认 Linux 的 AppImage：它下下来 chmod +x 就能跑，不要 root，
//! 也不碰系统包数据库，装和卸都只是动一个文件。macOS/Windows 的包请直接去上游
//! Releases 拿，那边装完是系统的事，不该由这里半管不管。

use std::path::PathBuf;

use anyhow::{bail, Context, Result};

use super::{uninstall_env_var, Extra, Tool};
use crate::common::fs::{guard_removable, human_size};
use crate::common::github::{self, Release};
use crate::common::platform::{Os, Platform};
use crate::common::{archive, asset, cmd, download, env};
use crate::ui;

const DEFAULT_REPO: &str = "jlcodes99/cockpit-tools";

/// 落地之后的固定文件名。
///
/// 上游的附件名里带版本号，照搬的话每升一次级就多一个文件、桌面入口还得跟着
/// 改。固定成一个名字，升级就是原地换文件。
const APPIMAGE: &str = "Cockpit.Tools.AppImage";

const DESKTOP_FILE: &str = "cockpit-tools.desktop";

const EXTRAS: &[Extra] = &[Extra {
    name: "run",
    about: "前台启动（终端附着，后面的参数原样传给它）",
}];

pub struct CockpitTools {
    repo: String,
    home: PathBuf,
    /// 要哪个版本。None 表示跟 release 里的 latest。
    version: Option<String>,
    /// 点名要哪个附件，用于自动挑选不灵时兜底。
    asset: Option<String>,
}

impl CockpitTools {
    pub fn from_env() -> Self {
        Self {
            repo: env::str_or("COCKPIT_TOOLS_GITHUB_REPO", DEFAULT_REPO),
            home: env::opt("COCKPIT_TOOLS_HOME")
                .map(|s| env::expand_tilde(&s))
                .unwrap_or_else(|| env::home().join("opt/cockpit-tools")),
            version: env::opt("COCKPIT_TOOLS_VERSION"),
            asset: env::opt("COCKPIT_TOOLS_ASSET"),
        }
    }

    fn appimage(&self) -> PathBuf {
        self.home.join("bin").join(APPIMAGE)
    }

    fn version_file(&self) -> PathBuf {
        self.home.join("version")
    }

    fn desktop_file(&self) -> PathBuf {
        env::home()
            .join(".local/share/applications")
            .join(DESKTOP_FILE)
    }

    fn installed_version(&self) -> Option<String> {
        let v = std::fs::read_to_string(self.version_file()).ok()?;
        let v = v.trim().to_string();
        (!v.is_empty()).then_some(v)
    }

    fn provision(&self, version: Option<&str>) -> Result<()> {
        let plat = Platform::current()?;
        if plat.os != Os::Linux {
            bail!(
                "只装 Linux 的 AppImage。macOS 的 dmg 请从上游取: https://github.com/{}/releases",
                self.repo
            );
        }

        let release = match version {
            Some(v) => github::by_tag(&self.repo, &github::tag_of(v)?)?,
            None => github::latest(&self.repo)?,
        };
        let want = self.choose_asset(&release, plat)?;

        ui::step(format!("下载 {} {}", release.tag_name, want.name));
        download::hint_mirror();

        // 临时目录放在 home 下而不是 /tmp：同一个文件系统，最后那步换文件才是
        // 原子的 rename，而不是跨设备拷贝拷到一半断电。
        let work = self.home.join(".tmp");
        let _ = std::fs::remove_dir_all(&work);
        std::fs::create_dir_all(&work)
            .with_context(|| format!("建临时目录失败: {}", work.display()))?;
        let tmp = work.join(&want.name);

        download::to_file(
            &want.browser_download_url,
            &tmp,
            &format!("cockpit-tools {}", release.tag_name),
        )?;

        let sums = release
            .checksums_url()
            .and_then(|u| download::get_text(u).ok());
        download::verify(&tmp, sums.as_deref(), &want.name)?;

        // 先加执行位再搬进去：反过来的话，中间存在一瞬间「文件在位但不可执行」，
        // 这时候点桌面入口得到的是一句 Permission denied。
        archive::make_executable(&tmp)?;
        let dest = self.appimage();
        std::fs::create_dir_all(dest.parent().unwrap())?;
        std::fs::rename(&tmp, &dest).with_context(|| format!("放置失败: {}", dest.display()))?;
        let _ = std::fs::remove_dir_all(&work);

        std::fs::write(self.version_file(), format!("{}\n", release.version()))
            .with_context(|| format!("写版本记录失败: {}", self.version_file().display()))?;
        self.write_desktop_entry()?;

        ui::ok(format!("{} 已装在 {}", release.tag_name, dest.display()));
        ui::hint("启动: fundeploy tool cockpit-tools run（或在应用菜单里找 Cockpit Tools）");
        Ok(())
    }

    fn choose_asset<'a>(&self, release: &'a Release, plat: Platform) -> Result<&'a github::Asset> {
        if let Some(name) = &self.asset {
            return release.asset(name).ok_or_else(|| {
                anyhow::anyhow!(
                    "{} 上没有附件 {name}。这个 release 有: {}",
                    release.tag_name,
                    asset::names(release)
                )
            });
        }
        asset::pick_ext(&release.assets, plat, &[".appimage"]).ok_or_else(|| {
            anyhow::anyhow!(
                "{} 里没有适配本机（{} {}）的 AppImage。\n\
                 这个 release 有: {}\n\
                 其中有能用的就设 COCKPIT_TOOLS_ASSET=<附件名> 点名。",
                release.tag_name,
                plat.os.go(),
                plat.arch.go(),
                asset::names(release)
            )
        })
    }

    /// 写用户级的桌面入口，让它出现在应用菜单里。
    ///
    /// 写 `~/.local/share/applications` 而不是 `/usr/share/applications`：后者要
    /// root，而这东西本来就只装给当前这个人用。
    fn write_desktop_entry(&self) -> Result<()> {
        let path = self.desktop_file();
        let dir = path.parent().unwrap();
        std::fs::create_dir_all(dir).with_context(|| format!("建目录失败: {}", dir.display()))?;

        let entry = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Cockpit Tools\n\
             Comment=AI IDE account manager\n\
             Exec={}\n\
             Terminal=false\n\
             Categories=Development;Utility;\n\
             StartupNotify=true\n",
            self.appimage().display()
        );
        std::fs::write(&path, entry).with_context(|| format!("写入失败: {}", path.display()))?;

        // 有些桌面环境靠这个缓存才能立刻看到新条目；没这个命令也不影响用。
        if cmd::has("update-desktop-database") {
            let _ = cmd::try_run(["update-desktop-database", &dir.display().to_string()]);
        }
        Ok(())
    }

    fn run(&self, args: &[String]) -> Result<()> {
        let app = self.appimage();
        if !app.is_file() {
            bail!(
                "还没装（{} 不在）。先执行: fundeploy tool cockpit-tools install",
                app.display()
            );
        }
        let mut argv = vec![app.display().to_string()];
        argv.extend(args.iter().cloned());
        cmd::run(argv).with_context(|| {
            format!(
                "起不来多半是缺 FUSE：Ubuntu 22.04 起不再预装 libfuse2，而 AppImage 要靠它把自己挂载起来。\n\
                 装上: sudo apt install libfuse2\n\
                 或者免挂载跑: {} --appimage-extract-and-run",
                app.display()
            )
        })
    }
}

impl Tool for CockpitTools {
    fn name(&self) -> &str {
        "cockpit-tools"
    }

    fn summary(&self) -> &str {
        "Cockpit Tools 桌面应用（AI IDE 账号管理）"
    }

    fn install(&self, version: Option<&str>) -> Result<()> {
        // 命令行给的版本优先于 COCKPIT_TOOLS_VERSION：前者是这一次的意图，
        // 后者是这台机器的长期设置。
        self.provision(version.or(self.version.as_deref()))
    }

    fn upgrade(&self) -> Result<()> {
        // 不看 COCKPIT_TOOLS_VERSION：那个变量钉的是「这台机器要哪一版」，
        // 拿它来升级等于原地装一遍。要钉版本用 install --version。
        self.provision(None)
    }

    fn uninstall(&self) -> Result<()> {
        ui::confirm_destructive(
            &format!(
                "将删除 {} 和桌面入口 {}，确认？",
                self.home.display(),
                self.desktop_file().display()
            ),
            &uninstall_env_var(self.name()),
        )?;
        guard_removable(&self.home)?;

        if self.home.exists() {
            std::fs::remove_dir_all(&self.home)
                .with_context(|| format!("删除失败: {}", self.home.display()))?;
            ui::ok(format!("已删除 {}。", self.home.display()));
        }
        let desktop = self.desktop_file();
        if desktop.exists() {
            std::fs::remove_file(&desktop)
                .with_context(|| format!("删除失败: {}", desktop.display()))?;
            ui::ok(format!("已删除 {}。", desktop.display()));
        }
        Ok(())
    }

    fn status(&self) -> Result<()> {
        ui::heading("cockpit-tools");
        ui::kv("目录", Some(&self.home.display().to_string()));
        ui::kv("上游仓库", Some(&self.repo));
        ui::kv("版本", self.installed_version().as_deref());

        let app = self.appimage();
        match std::fs::metadata(&app) {
            Ok(m) => ui::kv(
                "AppImage",
                Some(&format!("{}  {}", app.display(), human_size(m.len()))),
            ),
            Err(_) => {
                ui::kv("AppImage", None);
                ui::hint("安装: fundeploy tool cockpit-tools install");
            }
        }

        let desktop = self.desktop_file();
        ui::kv(
            "桌面入口",
            desktop
                .exists()
                .then(|| desktop.display().to_string())
                .as_deref(),
        );
        if desktop.exists() && !app.is_file() {
            // 入口还在、它指向的文件没了：在应用菜单里点它不会有任何反应，
            // 也没有任何报错，光看菜单根本看不出这东西其实已经不在了。
            ui::warn(
                "桌面入口指向的 AppImage 不在了，点它不会有反应。装回来，或者执行 uninstall 清掉。",
            );
        }
        Ok(())
    }

    fn extras(&self) -> &'static [Extra] {
        EXTRAS
    }

    fn extra(&self, name: &str, args: &[String]) -> Result<()> {
        match name {
            "run" => self.run(args),
            _ => bail!("cockpit-tools 声明了动作 {name} 却没有实现它"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 默认装到家目录下并且能被覆盖() {
        let t = CockpitTools::from_env();
        assert!(t.home.starts_with(env::home()));
        assert_eq!(t.appimage().file_name().unwrap(), APPIMAGE);
        // 装在哪儿都得过得了删除保护那一关，否则 uninstall 到不了删的那一步。
        assert!(guard_removable(&t.home).is_ok());
    }

    #[test]
    fn 桌面入口写在用户目录下() {
        let t = CockpitTools::from_env();
        assert!(t.desktop_file().starts_with(env::home()));
        assert!(t.desktop_file().ends_with(DESKTOP_FILE));
    }

    #[test]
    fn 没装的时候_run_先报没装() {
        // 真跑起来是个 GUI 程序，这里只能验「没装」这条路。
        let t = CockpitTools {
            repo: DEFAULT_REPO.into(),
            home: env::home().join("opt/__不存在的_cockpit_tools__"),
            version: None,
            asset: None,
        };
        let err = t.extra("run", &[]).unwrap_err().to_string();
        assert!(err.contains("还没装"), "实际: {err}");
    }
}
