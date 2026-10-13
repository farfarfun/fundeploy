//! AI 命令行工具（Claude Code、Codex、Cursor）。
//!
//! 三样东西的差别只有几项：官方安装器在哪儿、npm 包叫什么、brew cask 叫什么、
//! 装完会在哪些路径下留东西。所以这里没有 trait——每样工具就是 [`registry`] 里
//! 的一条 [`Ai`] 声明，装、升、卸、查四个动作的流程只写一遍。
//!
//! 同一样工具常常不止一条装法，而且人很容易忘了当初走的是哪条。所以不点名装法
//! 时，卸载是**每条路都走一遍**：只清一条的后果是 `uninstall` 报了成功、命令却
//! 还在 PATH 上，而看不出是为什么。

pub mod registry;

use std::path::PathBuf;

use anyhow::{bail, Result};
use clap::ValueEnum;

use crate::common::platform::{Os, Platform};
use crate::common::{cmd, download, env, fs};
use crate::dev::Dev;
use crate::ui;

/// 装法。
#[derive(ValueEnum, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Method {
    Official,
    Brew,
    Npm,
    Pnpm,
}

impl Method {
    pub const ALL: [Method; 4] = [Method::Official, Method::Brew, Method::Npm, Method::Pnpm];

    pub fn label(self) -> &'static str {
        match self {
            Method::Official => "官方安装器",
            Method::Brew => "Homebrew cask",
            Method::Npm => "npm 全局包",
            Method::Pnpm => "pnpm 全局包",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Method::Official => "official",
            Method::Brew => "brew",
            Method::Npm => "npm",
            Method::Pnpm => "pnpm",
        }
    }
}

/// 官方安装器装完会留下的一处东西。
///
/// `base_env` 是**上游自己**读的那个环境变量（codex 的 `CODEX_INSTALL_DIR`、
/// `CODEX_HOME`），所以不加 `FUNDEPLOY_` 前缀——这里不是在定义一个开关，而是在
/// 用跟安装器完全相同的规则推算它究竟把文件放到了哪儿。推错的后果是卸载时
/// 静默漏掉东西，而命令还在。
pub struct Leftover {
    /// 决定基准目录的环境变量名；空串＝基准目录固定挂在 `$HOME` 下。
    pub base_env: &'static str,
    /// 基准目录的默认值，相对 `$HOME`。
    pub base: &'static str,
    /// 基准目录下的路径。
    pub rel: &'static str,
}

impl Leftover {
    fn path(&self) -> PathBuf {
        let overridden = if self.base_env.is_empty() {
            None
        } else {
            env::opt(self.base_env)
        };
        let base = match overridden {
            Some(d) => env::expand_tilde(&d),
            None => env::home().join(self.base),
        };
        base.join(self.rel)
    }
}

pub struct Official {
    /// 覆盖安装器地址的环境变量名。这个是我们自己的开关，带 `FUNDEPLOY_` 前缀。
    pub url_env: &'static str,
    pub url: &'static str,
    /// 自更新子命令，跟在已装好的那个二进制后面。空＝没有，升级就是重跑安装器。
    pub self_update: &'static [&'static str],
    pub leftovers: &'static [Leftover],
}

impl Official {
    fn url(&self) -> String {
        env::str_or(self.url_env, self.url)
    }
}

/// 一样 AI 命令行工具。全是声明，没有行为。
pub struct Ai {
    pub name: &'static str,
    pub summary: &'static str,
    /// 装完在 PATH 上叫什么，按优先级排。
    ///
    /// 不止一个是因为上游改过名字：cursor 先后用过 `cursor-agent` 和 `agent`，
    /// 机器上留着哪个取决于当初装的是哪一版。
    pub bins: &'static [&'static str],
    /// 问版本用的参数。
    pub version_args: &'static [&'static str],
    pub official: Option<Official>,
    /// npm / pnpm 上的包名。None＝上游没发 npm 包。
    pub package: Option<&'static str>,
    /// Homebrew cask 名。None＝没上 brew。
    pub cask: Option<&'static str>,
}

impl Ai {
    pub fn supports(&self, m: Method) -> bool {
        match m {
            Method::Official => self.official.is_some(),
            Method::Brew => self.cask.is_some(),
            Method::Npm | Method::Pnpm => self.package.is_some(),
        }
    }

    pub fn methods(&self) -> Vec<Method> {
        Method::ALL
            .into_iter()
            .filter(|m| self.supports(*m))
            .collect()
    }

    /// 没点名时走哪条。官方那条优先——它是上游自己维护的，版本也最新。
    fn default_method(&self) -> Result<Method> {
        self.methods()
            .first()
            .copied()
            .ok_or_else(|| anyhow::anyhow!("{} 没有可用的装法", self.name))
    }

    /// 命令行点了名就按点的来，但先核一遍这工具有没有这条路。
    ///
    /// Bash 版把认不出来的装法当成动作名继续往下走，于是
    /// `fundeploy ai cursor npm install` 报的是「未知动作: npm」——真正的问题是
    /// cursor 压根没有 npm 这条路，照着那句话去查动作名会一直查不出所以然。
    fn pick(&self, m: Option<Method>) -> Result<Method> {
        let Some(m) = m else {
            return self.default_method();
        };
        if !self.supports(m) {
            let have: Vec<&str> = self.methods().iter().map(|x| x.key()).collect();
            bail!(
                "{} 没有 {} 这条路，它支持的是: {}",
                self.name,
                m.key(),
                have.join(" / ")
            );
        }
        Ok(m)
    }

    /// 在 PATH 上找得到的那个，返回（命令名，完整路径）。
    fn located(&self) -> Option<(&'static str, String)> {
        self.bins.iter().find_map(|b| cmd::find(b).map(|p| (*b, p)))
    }

    pub fn status(&self) -> Result<()> {
        ui::heading(self.name);
        match self.located() {
            Some((bin, path)) => {
                let mut argv: Vec<String> = vec![path.clone()];
                argv.extend(self.version_args.iter().map(|s| s.to_string()));
                let ver = match cmd::try_run(&argv) {
                    Ok(o) if o.success() => {
                        let out = if o.stdout.trim().is_empty() {
                            o.stderr
                        } else {
                            o.stdout
                        };
                        out.lines().next().unwrap_or("").trim().to_string()
                    }
                    _ => "问不出版本".to_string(),
                };
                ui::kv(format!("{bin}  {path}"), Some(&ver));
            }
            None => ui::kv(self.bins.join(" / "), Some("未安装")),
        }
        let have: Vec<&str> = self.methods().iter().map(|m| m.key()).collect();
        ui::kv("可用装法", Some(&have.join(" / ")));
        Ok(())
    }

    /// `args` 原样传给官方安装器（`fundeploy ai claude install -- stable`）。
    pub fn install(&self, method: Option<Method>, args: &[String]) -> Result<()> {
        let m = self.pick(method)?;
        ui::heading(format!("安装 {}（{}）", self.name, m.label()));
        if m != Method::Official && !args.is_empty() {
            // 多出来的参数只有官方安装器认。悄悄丢掉会让人以为传进去了。
            bail!("额外参数只有 official 装法会用到，{} 用不上它们", m.key());
        }
        match m {
            Method::Official => self.official_run(args),
            Method::Brew => self.brew(&["install", "--cask"]),
            Method::Npm => self.node_pm("npm", &["install", "-g"], true),
            Method::Pnpm => self.node_pm("pnpm", &["add", "-g"], true),
        }?;
        self.report();
        Ok(())
    }

    pub fn upgrade(&self, method: Option<Method>) -> Result<()> {
        let m = self.pick(method)?;
        ui::heading(format!("升级 {}（{}）", self.name, m.label()));
        match m {
            Method::Official => {
                let o = self.official.as_ref().expect("pick 核过了");
                // 有自更新命令就用它：重跑安装器会把整个包重下一遍，而自更新多半
                // 只取差量，而且上游知道自己升级时该保留哪些本地状态。
                if !o.self_update.is_empty() {
                    if let Some((_, path)) = self.located() {
                        let mut argv = vec![path];
                        argv.extend(o.self_update.iter().map(|s| s.to_string()));
                        return cmd::run(&argv);
                    }
                    ui::info("还没装上，改为跑一遍安装器。");
                }
                self.official_run(&[])
            }
            Method::Brew => self.brew(&["upgrade", "--cask"]),
            Method::Npm => self.node_pm("npm", &["install", "-g"], true),
            Method::Pnpm => self.node_pm("pnpm", &["add", "-g"], true),
        }
    }

    /// 不点名装法时每条路都走一遍。
    pub fn uninstall(&self, method: Option<Method>, yes: bool) -> Result<()> {
        let all = method.is_none();
        if let Some(m) = method {
            self.pick(Some(m))?;
        }
        ui::heading(format!(
            "卸载 {}（{}）",
            self.name,
            method.map(Method::label).unwrap_or("所有装法")
        ));
        if !yes {
            // 一次问清，不是每条路径问一遍。Bash 版对每个要删的路径各弹一次，
            // 卸一个 codex 要连按三次 y，于是人养成了闭眼按 y 的习惯。
            ui::confirm_destructive(
                &format!("要卸掉 {} 吗", self.name),
                &format!("FUNDEPLOY_{}_UNINSTALL_YES", self.env_name()),
            )?;
        }

        let mut hit = false;
        for m in self.methods() {
            if !all && Some(m) != method {
                continue;
            }
            let r = match m {
                Method::Official => self.official_clean(),
                Method::Brew => self.brew(&["uninstall", "--cask"]).map(|_| true),
                Method::Npm => self
                    .node_pm("npm", &["uninstall", "-g"], false)
                    .map(|_| true),
                Method::Pnpm => self.node_pm("pnpm", &["remove", "-g"], false).map(|_| true),
            };
            match r {
                Ok(true) => hit = true,
                Ok(false) => {}
                // 本来就不是这么装的很正常，点名时才当成错误。
                Err(e) if all => ui::info(format!("{} 那条路没卸掉：{e}", m.key())),
                Err(e) => return Err(e),
            }
        }
        if !hit {
            ui::info(format!("{} 本来就没装。", self.name));
        }
        if let Some((bin, path)) = self.located() {
            ui::warn(format!(
                "PATH 上还留着 {bin}: {path}（不是 fundeploy 装的）"
            ));
        }
        Ok(())
    }

    fn env_name(&self) -> String {
        self.name.to_ascii_uppercase().replace(['-', '.'], "_")
    }

    fn official_run(&self, args: &[String]) -> Result<()> {
        let o = self
            .official
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("{} 没有官方安装器", self.name))?;
        let scratch = fs::Scratch::new(self.name)?;
        let script = scratch.path().join("install.sh");
        // 先下到文件再执行，不用 `curl … | bash`：管道执行是边下边跑的，连接在
        // 中途断掉时脚本已经执行了前半截，留下的是一堆既不算装上也不算没装的
        // 半成品。另外 ureq 自己跟重定向，不必像 Bash 版那样惦记漏写 `-L`
        // 会让 bash 执行一段空内容、然后静默「安装成功」。
        download::to_file(&o.url(), &script, &format!("{} 安装脚本", self.name))?;
        // 统一用 bash 跑。上游这几个脚本有的写了 bash 专有语法，用 sh 跑会在
        // 中途语法错误退出；反过来拿 bash 跑一个纯 POSIX 脚本没有任何问题。
        let sh = cmd::find("bash").unwrap_or_else(|| "sh".into());
        let mut argv: Vec<String> = vec![sh, script.to_string_lossy().into_owned()];
        argv.extend(args.iter().cloned());
        ui::step(format!("执行 {} 官方安装器", self.name));
        cmd::run(&argv)
    }

    /// 删掉官方安装器留下的东西。返回「有没有真删掉什么」。
    fn official_clean(&self) -> Result<bool> {
        let Some(o) = self.official.as_ref() else {
            return Ok(false);
        };
        let mut hit = false;
        for l in o.leftovers {
            let p = l.path();
            // symlink_metadata：指向别处的符号链接也要删，但只删链接本身。
            if p.symlink_metadata().is_err() {
                continue;
            }
            fs::guard_removable(&p)?;
            if p.is_dir() && !p.is_symlink() {
                std::fs::remove_dir_all(&p)?;
            } else {
                std::fs::remove_file(&p)?;
            }
            ui::ok(format!("已删除 {}", p.display()));
            hit = true;
        }
        Ok(hit)
    }

    fn brew(&self, args: &[&str]) -> Result<()> {
        let cask = self
            .cask
            .ok_or_else(|| anyhow::anyhow!("{} 没上 brew", self.name))?;
        // cask 是 macOS 独有的机制，Linuxbrew 上这条命令只会换来一句
        // "Installing casks is supported only on macOS"。先说清楚，省得人以为
        // 是 brew 装坏了。
        if Platform::current()?.os != Os::MacOs {
            bail!("brew 的 cask 只在 macOS 上有，这台机器请改用 official 或 npm。");
        }
        let brew = cmd::require("brew", "先跑 `fundeploy tool brew install`")?;
        let mut argv = vec![brew];
        argv.extend(args.iter().map(|s| s.to_string()));
        argv.push(cask.to_string());
        cmd::run(&argv)
    }

    /// 走 npm / pnpm。`latest` 为真时给包名钉上 `@latest`。
    fn node_pm(&self, pm: &str, args: &[&str], latest: bool) -> Result<()> {
        let pkg = self
            .package
            .ok_or_else(|| anyhow::anyhow!("{} 没发 npm 包", self.name))?;
        // 本域装的 Node 优先：`fundeploy dev nodejs install --method source` 之后
        // npm 还不在 PATH 上（PATH 要人自己配），此时报「未找到 npm」是误导。
        let local = crate::dev::nodejs::NodeJs
            .root()
            .join("bin")
            .join(pm)
            .to_string_lossy()
            .into_owned();
        let bin = if std::path::Path::new(&local).is_file() {
            local
        } else {
            cmd::require(pm, &format!("先跑 `fundeploy dev {pm} install`"))?
        };
        let mut argv = vec![bin];
        argv.extend(args.iter().map(|s| s.to_string()));
        argv.push(if latest {
            format!("{pkg}@latest")
        } else {
            pkg.to_string()
        });
        cmd::run(&argv)
    }

    fn report(&self) {
        match self.located() {
            Some((bin, path)) => ui::ok(format!("{bin} 已就绪: {path}")),
            None => ui::hint(format!(
                "装好了，但 PATH 上还看不到 {}。多半要把 ~/.local/bin 加进 PATH，\n  或者重开一个终端。",
                self.bins.join(" / ")
            )),
        }
    }
}

/// 对每样工具都做一遍同一件事。
///
/// 不支持点名那条路的跳过、接着往下走。Bash 版的 `ai all brew install` 走到
/// cursor 就整个停了（cursor 只有 official），前面装好的那些也不会告诉你，
/// 得自己回头翻输出。
pub fn each(method: Option<Method>, f: impl Fn(&Ai) -> Result<()>) -> Result<()> {
    let mut failed: Vec<&str> = Vec::new();
    for t in registry::all() {
        if let Some(m) = method {
            if !t.supports(m) {
                ui::info(format!("{} 没有 {} 这条路，跳过。", t.name, m.key()));
                continue;
            }
        }
        if let Err(e) = f(&t) {
            ui::warn(format!("{} 没弄成: {e}", t.name));
            failed.push(t.name);
        }
        ui::blank();
    }
    if !failed.is_empty() {
        bail!("这几样没弄成: {}", failed.join("、"));
    }
    Ok(())
}

/// 列出所有工具的状态。
pub fn list() -> Result<()> {
    for t in registry::all() {
        t.status()?;
        ui::blank();
    }
    ui::hint("装: fundeploy ai <claude|codex|cursor> install [--method official|brew|npm|pnpm]");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn 排队() -> std::sync::MutexGuard<'static, ()> {
        static M: std::sync::Mutex<()> = std::sync::Mutex::new(());
        M.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn 每样工具至少有一条装法() {
        for t in registry::all() {
            assert!(!t.methods().is_empty(), "{} 一条路都没有", t.name);
        }
    }

    #[test]
    fn 官方那条排在最前() {
        // 它是上游自己维护的，版本最新，所以不点名时走它。
        for t in registry::all() {
            if t.official.is_some() {
                assert_eq!(t.default_method().unwrap(), Method::Official, "{}", t.name);
            }
        }
    }

    #[test]
    fn 不支持的装法报的是装法不对而不是动作不对() {
        // Bash 版把认不出来的装法当动作名，于是 `ai cursor npm install` 报
        // 「未知动作: npm」，照着那句话查动作名永远查不出所以然。
        let cursor = registry::find("cursor").unwrap();
        let e = cursor.pick(Some(Method::Npm)).unwrap_err().to_string();
        assert!(e.contains("npm"), "实际: {e}");
        assert!(e.contains("official"), "该把支持的列出来，实际: {e}");
    }

    #[test]
    fn 能装的那几条都认() {
        let claude = registry::find("claude").unwrap();
        for m in [Method::Official, Method::Brew, Method::Npm, Method::Pnpm] {
            assert!(claude.pick(Some(m)).is_ok(), "{m:?}");
        }
    }

    #[test]
    fn 残留路径跟着上游自己的环境变量走() {
        // codex 的安装器读 CODEX_INSTALL_DIR / CODEX_HOME，我们得按同一套规则
        // 算，不然卸载会漏。
        let _g = 排队();
        let codex = registry::find("codex").unwrap();
        let o = codex.official.as_ref().unwrap();
        std::env::set_var("CODEX_INSTALL_DIR", "/tmp/cx/bin");
        std::env::set_var("CODEX_HOME", "/tmp/cx/home");
        let paths: Vec<String> = o
            .leftovers
            .iter()
            .map(|l| l.path().display().to_string())
            .collect();
        assert!(
            paths.contains(&"/tmp/cx/bin/codex".to_string()),
            "{paths:?}"
        );
        assert!(
            paths.contains(&"/tmp/cx/home/packages/standalone".to_string()),
            "{paths:?}"
        );
        std::env::remove_var("CODEX_INSTALL_DIR");
        std::env::remove_var("CODEX_HOME");

        // 不设时落回 $HOME 下的默认值。
        let home = env::home();
        let paths: Vec<PathBuf> = o.leftovers.iter().map(|l| l.path()).collect();
        assert!(paths.iter().all(|p| p.starts_with(&home)), "{paths:?}");
    }

    #[test]
    fn 安装器地址可以改() {
        let _g = 排队();
        let claude = registry::find("claude").unwrap();
        let o = claude.official.as_ref().unwrap();
        assert_eq!(o.url(), o.url);
        std::env::set_var(o.url_env, "https://example.com/i.sh");
        assert_eq!(o.url(), "https://example.com/i.sh");
        std::env::remove_var(o.url_env);
    }

    #[test]
    fn 我们自己的开关带前缀_上游的不带() {
        for t in registry::all() {
            let Some(o) = t.official.as_ref() else {
                continue;
            };
            assert!(
                o.url_env.starts_with("FUNDEPLOY_"),
                "{} 的 {} 是我们自己的开关，该带前缀",
                t.name,
                o.url_env
            );
            for l in o.leftovers {
                assert!(
                    !l.base_env.starts_with("FUNDEPLOY_"),
                    "{} 的 {} 是上游读的，改了名就算不出它把文件放哪儿了",
                    t.name,
                    l.base_env
                );
            }
        }
    }

    #[test]
    fn 卸载开关的名字由工具名推出() {
        assert_eq!(
            registry::find("claude").unwrap().env_name(),
            "CLAUDE".to_string()
        );
    }

    #[test]
    fn 工具名不跟别的域撞车() {
        let services = crate::service::registry::names();
        let tools = crate::tool::registry::names();
        for t in registry::all() {
            assert!(!services.contains(&t.name.to_string()), "{} 撞服务", t.name);
            assert!(!tools.contains(&t.name.to_string()), "{} 撞工具", t.name);
        }
    }

    #[test]
    fn 注册表里没有重名() {
        let names: Vec<&str> = registry::all().iter().map(|t| t.name).collect();
        let mut s = names.clone();
        s.sort_unstable();
        s.dedup();
        assert_eq!(s.len(), names.len(), "有重名: {names:?}");
    }

    #[test]
    fn 按名字和别名都找得到() {
        for n in ["claude", "codex", "cursor", "claude-code", "cursor-agent"] {
            assert!(registry::find(n).is_some(), "找不到 {n}");
        }
        assert!(registry::find("__不存在__").is_none());
    }
}
