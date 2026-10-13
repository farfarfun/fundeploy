//! Homebrew 换源。
//!
//! Homebrew 在国内有三条独立的慢路径，少换一条就还是慢：
//!
//! | 变量 | 管什么 | 不换的症状 |
//! |---|---|---|
//! | `HOMEBREW_API_DOMAIN` | 配方元数据（4.0 起走 JSON API） | `brew install` 开头就卡住 |
//! | `HOMEBREW_BOTTLE_DOMAIN` | 预编译二进制包 | 元数据秒出，下载几 KB/s |
//! | `HOMEBREW_BREW_GIT_REMOTE` | brew 自己的仓库 | `brew update` 卡住 |
//!
//! 这就是为什么「我换了源还是慢」是个常见问题——绝大多数教程只给了
//! `HOMEBREW_BOTTLE_DOMAIN`。
//!
//! # 跟 HomebrewCN 的几处不同
//!
//! 那个脚本成文于 Homebrew 3.x 年代，现在有几处已经过时或者不必要：
//!
//! - **不再改官方安装脚本。** 它把 install.sh 克隆下来用 sed 把 GitHub 地址换掉；
//!   而官方脚本自 2022 年起就认 `HOMEBREW_BREW_GIT_REMOTE` 了，设环境变量即可
//!   （见 [`install_env`]）。sed 那条路每次上游改一行就断一次。
//! - **不再强制配 homebrew-core 的 git 源。** Homebrew 4.0 起默认不克隆
//!   homebrew-core，改走 API；中科大干脆把这个仓库下线了。只有手动 tap 过的人
//!   才用得上，所以它成了可选项。
//! - **不按关键字 sed 删行。** 见 [`crate::common::block`]。
//! - **只写用户自己的 shell 配置。** 它在 Linux 上写 `/etc/profile`，那要 root，
//!   而且会把一台机器上所有人的 brew 都指到这个源。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::{Mirror, Source};
use crate::common::block::Marker;
use crate::common::cmd::{self, EnvEdit};
use crate::common::env;
use crate::ui;

const OFFICIAL_BREW_GIT: &str = "https://github.com/Homebrew/brew";
const OFFICIAL_CORE_GIT: &str = "https://github.com/Homebrew/homebrew-core";
const PIP_INDEX: &str = "export HOMEBREW_PIP_INDEX_URL=";

pub struct Brew;

/// 内置镜像表。`url` 是 bottle 域名，其余放 extras。
pub fn mirrors() -> Vec<Mirror> {
    vec![
        Mirror::new(
            "ustc",
            "中国科大",
            "https://mirrors.ustc.edu.cn/homebrew-bottles",
        )
        .with("api", "https://mirrors.ustc.edu.cn/homebrew-bottles/api")
        .with("brew_git", "https://mirrors.ustc.edu.cn/brew.git"),
        Mirror::new(
            "tuna",
            "清华大学",
            "https://mirrors.tuna.tsinghua.edu.cn/homebrew-bottles",
        )
        .with(
            "api",
            "https://mirrors.tuna.tsinghua.edu.cn/homebrew-bottles/api",
        )
        .with(
            "brew_git",
            "https://mirrors.tuna.tsinghua.edu.cn/git/homebrew/brew.git",
        )
        .with(
            "core_git",
            "https://mirrors.tuna.tsinghua.edu.cn/git/homebrew/homebrew-core.git",
        ),
        Mirror::new(
            "aliyun",
            "阿里云",
            "https://mirrors.aliyun.com/homebrew/homebrew-bottles",
        )
        .with(
            "api",
            "https://mirrors.aliyun.com/homebrew/homebrew-bottles/api",
        )
        .with("brew_git", "https://mirrors.aliyun.com/homebrew/brew.git")
        .with(
            "core_git",
            "https://mirrors.aliyun.com/homebrew/homebrew-core.git",
        ),
        Mirror::new(
            "tencent",
            "腾讯云",
            "https://mirrors.cloud.tencent.com/homebrew-bottles",
        )
        .with(
            "api",
            "https://mirrors.cloud.tencent.com/homebrew-bottles/api",
        )
        .with(
            "brew_git",
            "https://mirrors.cloud.tencent.com/homebrew/brew.git",
        )
        .with(
            "core_git",
            "https://mirrors.cloud.tencent.com/homebrew/homebrew-core.git",
        ),
        Mirror::new(
            "bfsu",
            "北京外国语",
            "https://mirrors.bfsu.edu.cn/homebrew-bottles",
        )
        .with("api", "https://mirrors.bfsu.edu.cn/homebrew-bottles/api")
        .with(
            "brew_git",
            "https://mirrors.bfsu.edu.cn/git/homebrew/brew.git",
        )
        .with(
            "core_git",
            "https://mirrors.bfsu.edu.cn/git/homebrew/homebrew-core.git",
        ),
        Mirror::new("official", "官方", "https://ghcr.io/v2/homebrew/core")
            .with("api", "https://formulae.brew.sh/api")
            .with("brew_git", OFFICIAL_BREW_GIT)
            .with("core_git", OFFICIAL_CORE_GIT)
            .official(),
    ]
}

/// 当前这台机器配好的 brew 源，拆成要导出的环境变量。
///
/// 给装 brew 的时候用：官方安装脚本会读这几个变量去决定从哪儿克隆，所以第一次
/// 装就能走镜像——这正是「国内装 brew 卡住」的那一步。
pub fn install_env() -> Vec<EnvEdit> {
    let Some(url) = Brew.current() else {
        return Vec::new();
    };
    let Some(m) = mirrors().into_iter().find(|m| m.url == url) else {
        return Vec::new();
    };
    exports(&m).into_iter().map(|(k, v)| (k, Some(v))).collect()
}

/// pip 源变了之后，把 brew 区块里那一行跟着改掉。
///
/// 不同步的话，brew 装 Python 类配方时还在用上一个 pip 源——而人刚刚才明确换掉
/// 它。只动这一行，区块里别的内容原样留着。没配过 brew 源就什么也不做。
pub fn sync_pip_index() {
    let b = Brew;
    let (marker, path) = (b.marker(), b.profile());
    let Some(old) = marker.read(&path) else {
        return;
    };
    let mut body: Vec<String> = old
        .iter()
        .filter(|l| !l.trim().starts_with(PIP_INDEX))
        .cloned()
        .collect();
    if let Some(index) = super::pip::Pip.current() {
        body.push(format!("{PIP_INDEX}\"{index}\""));
    }
    if body != old && marker.write(&path, &body).is_ok() {
        ui::info(format!(
            "顺带同步了 {} 里 brew 的 HOMEBREW_PIP_INDEX_URL。",
            path.display()
        ));
    }
}

/// 一个镜像对应的全部环境变量。
fn exports(m: &Mirror) -> Vec<(String, String)> {
    let mut out = vec![("HOMEBREW_BOTTLE_DOMAIN".to_string(), m.url.clone())];
    if let Some(api) = m.extra("api") {
        out.push(("HOMEBREW_API_DOMAIN".to_string(), api.to_string()));
    }
    if let Some(g) = m.extra("brew_git") {
        out.push(("HOMEBREW_BREW_GIT_REMOTE".to_string(), g.to_string()));
    }
    if let Some(g) = m.extra("core_git") {
        out.push(("HOMEBREW_CORE_GIT_REMOTE".to_string(), g.to_string()));
    }
    out
}

impl Brew {
    /// 往哪个 shell 配置里写。
    ///
    /// 按 `$SHELL` 挑登录时会读到的那一份。挑错的症状是「配好了，新开终端没生效」。
    fn profile(&self) -> PathBuf {
        if let Some(p) = env::opt("FUNDEPLOY_SHELL_PROFILE") {
            return env::expand_tilde(&p);
        }
        let home = env::home();
        let shell = env::str_or("SHELL", "");
        if shell.contains("zsh") {
            return home.join(".zprofile");
        }
        if shell.contains("bash") {
            // bash 存在 .bash_profile 就只读它、完全不读 .profile，
            // 所以有它的时候必须写它。
            let bp = home.join(".bash_profile");
            if bp.exists() {
                return bp;
            }
            return home.join(".profile");
        }
        home.join(".profile")
    }

    fn marker(&self) -> Marker {
        Marker::new("brew mirror")
    }

    /// brew 自己的 git 仓库在哪。没装 brew 时返回 None。
    fn repository(&self) -> Option<PathBuf> {
        let brew = cmd::find("brew")?;
        cmd::capture([&brew, "--repository"])
            .ok()
            .map(|s| PathBuf::from(s.trim()))
    }

    /// 把一个已经克隆下来的仓库的 origin 指到别处。
    ///
    /// 已装的 brew 不会因为环境变量变了就改 origin——那是克隆时写死在
    /// `.git/config` 里的。不改的话 `brew update` 还是去连 GitHub。
    fn retarget(&self, repo: &Path, url: &str) {
        if !repo.join(".git").exists() {
            return;
        }
        let r = cmd::try_run([
            "git",
            "-C",
            &repo.display().to_string(),
            "remote",
            "set-url",
            "origin",
            url,
        ]);
        match r {
            Ok(o) if o.success() => ui::ok(format!("{} -> {url}", repo.display())),
            _ => ui::warn(format!(
                "改不了 {} 的 origin，`brew update` 仍会走原来的地址。",
                repo.display()
            )),
        }
    }

    /// 已装的 brew 全部指到这个镜像。
    fn retarget_all(&self, m: &Mirror) {
        let Some(repo) = self.repository() else {
            return;
        };
        if let Some(g) = m.extra("brew_git") {
            self.retarget(&repo, g);
        }
        // homebrew-core 只有手动 tap 过才在本地，没 tap 过就跳过。
        let core = repo.join("Library/Taps/homebrew/homebrew-core");
        if core.exists() {
            match m.extra("core_git") {
                Some(g) => self.retarget(&core, g),
                None => ui::warn(format!(
                    "{} 上没有 homebrew-core 的 git 镜像，本地这份 tap 更新时仍走原地址。\n\
                     用不到就 brew untap homebrew/core。",
                    m.name
                )),
            }
        }
    }
}

impl Source for Brew {
    fn name(&self) -> &str {
        "brew"
    }

    fn summary(&self) -> &str {
        "Homebrew（配方 API、bottle 二进制、brew 仓库三处一起换）"
    }

    fn config_path(&self) -> PathBuf {
        self.profile()
    }

    fn builtin(&self) -> Vec<Mirror> {
        mirrors()
    }

    /// 量配方元数据的入口文件。它是 `brew install` 真正会先打的那个地址。
    fn probe_url(&self, m: &Mirror) -> String {
        match m.extra("api") {
            Some(api) => format!("{}/formula.jws.json", api.trim_end_matches('/')),
            None => m.url.clone(),
        }
    }

    fn current(&self) -> Option<String> {
        let body = self.marker().read(&self.profile())?;
        body.iter().find_map(|l| {
            l.trim()
                .strip_prefix("export HOMEBREW_BOTTLE_DOMAIN=")
                .map(|v| v.trim_matches('"').to_string())
        })
    }

    fn apply(&self, m: &Mirror) -> Result<()> {
        let mut body: Vec<String> = exports(m)
            .into_iter()
            .map(|(k, v)| format!("export {k}=\"{v}\""))
            .collect();

        // brew 装 Python 类配方时自己开一个隔离 venv，读不到 pip.conf，只认这个
        // 变量。跟着已经配好的 pip 源走，而不是另给一份可能对不上的值。
        if let Some(index) = super::pip::Pip.current() {
            body.push(format!("{PIP_INDEX}\"{index}\""));
        }

        let profile = self.profile();
        self.marker()
            .write(&profile, &body)
            .with_context(|| format!("写不了 {}", profile.display()))?;
        self.retarget_all(m);
        Ok(())
    }

    fn reset(&self) -> Result<()> {
        let profile = self.profile();
        if self.marker().remove(&profile)? {
            ui::ok(format!("已从 {} 去掉镜像设置。", profile.display()));
        } else {
            ui::info("shell 配置里本来就没有镜像设置。");
        }
        // 环境变量清掉了，但已克隆仓库的 origin 还指着镜像，得一并改回去。
        if let Some(repo) = self.repository() {
            self.retarget(&repo, OFFICIAL_BREW_GIT);
            let core = repo.join("Library/Taps/homebrew/homebrew-core");
            if core.exists() {
                self.retarget(&core, OFFICIAL_CORE_GIT);
            }
        }
        Ok(())
    }

    fn after_apply(&self, _m: &Mirror) {
        ui::hint(format!(
            "当前 shell 还是旧值，新开一个终端，或者先 source {}",
            self.profile().display()
        ));
        if cmd::has("brew") {
            ui::hint("然后 brew update 验证一下。");
        } else {
            ui::hint("还没装 brew 的话，现在执行 fundeploy tool brew install 就会走这个源。");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::排队;

    fn 临时配置() -> (tempfile::TempDir, std::sync::MutexGuard<'static, ()>) {
        let g = 排队();
        let d = tempfile::tempdir().unwrap();
        std::env::set_var("FUNDEPLOY_SHELL_PROFILE", d.path().join("profile"));
        (d, g)
    }

    #[test]
    fn 换源把三个变量一起写出去() {
        // 只写 BOTTLE_DOMAIN 是网上最常见的做法，而它只管二进制下载那一段。
        let (_d, _g) = 临时配置();
        let b = Brew;
        b.apply(&b.lookup("tuna").unwrap()).unwrap();
        let text = std::fs::read_to_string(b.config_path()).unwrap();
        for k in [
            "HOMEBREW_BOTTLE_DOMAIN",
            "HOMEBREW_API_DOMAIN",
            "HOMEBREW_BREW_GIT_REMOTE",
            "HOMEBREW_CORE_GIT_REMOTE",
        ] {
            assert!(text.contains(k), "漏了 {k}:\n{text}");
        }
    }

    #[test]
    fn 没有_core_镜像的源就不写那个变量() {
        // 中科大下线了 homebrew-core.git。写一个 404 的地址进去，
        // `brew update` 会每次都报错。
        let (_d, _g) = 临时配置();
        let b = Brew;
        b.apply(&b.lookup("ustc").unwrap()).unwrap();
        let text = std::fs::read_to_string(b.config_path()).unwrap();
        assert!(!text.contains("HOMEBREW_CORE_GIT_REMOTE"), "实际:\n{text}");
        assert!(text.contains("HOMEBREW_BREW_GIT_REMOTE"));
    }

    #[test]
    fn 换第二次是替换不是再追加一份() {
        let (_d, _g) = 临时配置();
        let b = Brew;
        b.apply(&b.lookup("tuna").unwrap()).unwrap();
        b.apply(&b.lookup("aliyun").unwrap()).unwrap();
        let text = std::fs::read_to_string(b.config_path()).unwrap();
        assert_eq!(text.matches("HOMEBREW_BOTTLE_DOMAIN").count(), 1);
        assert!(text.contains("mirrors.aliyun.com"));
        assert!(!text.contains("tsinghua"));
    }

    #[test]
    fn 读得回当前源() {
        let (_d, _g) = 临时配置();
        let b = Brew;
        assert_eq!(b.current(), None);
        b.apply(&b.lookup("bfsu").unwrap()).unwrap();
        assert_eq!(
            b.current().as_deref(),
            Some("https://mirrors.bfsu.edu.cn/homebrew-bottles")
        );
    }

    #[test]
    fn reset_之后_shell_配置里不留痕迹() {
        let (_d, _g) = 临时配置();
        let b = Brew;
        std::fs::write(b.config_path(), "export PATH=/我的:$PATH\n").unwrap();
        b.apply(&b.lookup("tuna").unwrap()).unwrap();
        b.reset().unwrap();
        let text = std::fs::read_to_string(b.config_path()).unwrap();
        assert_eq!(text, "export PATH=/我的:$PATH\n");
        assert_eq!(b.current(), None);
    }

    #[test]
    fn 装_brew_时带上已配好的镜像变量() {
        // 这是「国内装 brew 卡在克隆那一步」的解法：官方脚本认这几个变量。
        let (_d, _g) = 临时配置();
        let b = Brew;
        assert!(install_env().is_empty(), "没配过就不该硬塞变量");

        b.apply(&b.lookup("aliyun").unwrap()).unwrap();
        let e = install_env();
        let get = |k: &str| {
            e.iter()
                .find(|(n, _)| n == k)
                .and_then(|(_, v)| v.clone())
                .unwrap_or_default()
        };
        assert_eq!(
            get("HOMEBREW_BREW_GIT_REMOTE"),
            "https://mirrors.aliyun.com/homebrew/brew.git"
        );
        assert!(get("HOMEBREW_API_DOMAIN").contains("aliyun"));
    }

    #[test]
    fn 探测的是配方_api_而不是_bottle_根() {
        let b = Brew;
        assert_eq!(
            b.probe_url(&b.lookup("official").unwrap()),
            "https://formulae.brew.sh/api/formula.jws.json"
        );
        assert_eq!(
            b.probe_url(&b.lookup("ustc").unwrap()),
            "https://mirrors.ustc.edu.cn/homebrew-bottles/api/formula.jws.json"
        );
    }

    #[test]
    fn 每个源都给了_api_和_brew_仓库地址() {
        // 缺 api 的源换过去等于只换了一半，症状是「还是卡在开头」。
        for m in mirrors() {
            assert!(m.extra("api").is_some(), "{} 缺 api", m.name);
            assert!(m.extra("brew_git").is_some(), "{} 缺 brew_git", m.name);
        }
    }

    #[test]
    fn 换了_pip_源之后_brew_区块里那行跟着变() {
        // 不跟着变的话，brew 装 python 类配方时还在用上一个 pip 源。
        let (d, _g) = 临时配置();
        std::env::set_var("FUNDEPLOY_PIP_CONFIG", d.path().join("pip.conf"));
        let (b, p) = (Brew, super::super::pip::Pip);

        p.apply(&p.lookup("aliyun").unwrap()).unwrap();
        b.apply(&b.lookup("tuna").unwrap()).unwrap();
        assert!(std::fs::read_to_string(b.config_path())
            .unwrap()
            .contains("HOMEBREW_PIP_INDEX_URL=\"https://mirrors.aliyun.com/pypi/simple/\""));

        p.apply(&p.lookup("ustc").unwrap()).unwrap();
        sync_pip_index();
        let text = std::fs::read_to_string(b.config_path()).unwrap();
        assert_eq!(text.matches("HOMEBREW_PIP_INDEX_URL").count(), 1);
        assert!(text.contains("pypi.mirrors.ustc.edu.cn"), "实际:\n{text}");
        // 别的变量不能被这次同步弄丢。
        assert!(text.contains("HOMEBREW_BOTTLE_DOMAIN"));

        p.reset().unwrap();
        let text = std::fs::read_to_string(b.config_path()).unwrap();
        assert!(!text.contains("HOMEBREW_PIP_INDEX_URL"), "实际:\n{text}");
        std::env::remove_var("FUNDEPLOY_PIP_CONFIG");
    }

    #[test]
    fn 没配过_brew_源时同步什么也不做() {
        let (_d, _g) = 临时配置();
        sync_pip_index();
        assert!(!Brew.config_path().exists());
    }

    #[test]
    fn zsh_写_zprofile_而不是_zshrc() {
        let _g = 排队();
        std::env::remove_var("FUNDEPLOY_SHELL_PROFILE");
        std::env::set_var("SHELL", "/bin/zsh");
        assert!(Brew.profile().ends_with(".zprofile"));
        std::env::set_var("SHELL", "/usr/bin/fish");
        assert!(Brew.profile().ends_with(".profile"));
        std::env::remove_var("SHELL");
    }
}
