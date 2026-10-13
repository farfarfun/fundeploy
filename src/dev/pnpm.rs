//! pnpm。

use std::path::PathBuf;

use anyhow::{bail, Result};

use super::{install_root, set, Dev};
use crate::common::pm::Pm;
use crate::common::{cmd, env};
use crate::ui;

pub struct Pnpm;

impl Pnpm {
    /// 本域装的 Node 优先。
    ///
    /// `fundeploy dev nodejs install --method source` 之后 node 还不在 PATH 上
    /// （PATH 要人自己配），紧接着装 pnpm 就会报「需要 node」——而 node 明明刚
    /// 装好，就在边上。
    fn node_env(&self) -> Vec<cmd::EnvEdit> {
        let bin = super::nodejs::NodeJs.root().join("bin");
        if !bin.is_dir() {
            return Vec::new();
        }
        let path = match env::opt("PATH") {
            Some(p) => format!("{}:{p}", bin.display()),
            None => bin.display().to_string(),
        };
        vec![set("PATH", path)]
    }

    fn node_bin(&self, name: &str) -> Option<String> {
        let local = super::nodejs::NodeJs.root().join("bin").join(name);
        if local.is_file() {
            return Some(local.to_string_lossy().into_owned());
        }
        cmd::find(name)
    }

    /// 走 npm 全局前缀而不是 corepack。
    fn via_npm(&self) -> Result<()> {
        let npm = self
            .node_bin("npm")
            .ok_or_else(|| anyhow::anyhow!("需要 npm，先 `fundeploy dev nodejs install`。"))?;
        let root = self.root();
        std::fs::create_dir_all(root.join("bin"))?;
        let mut env = self.node_env();
        env.push(set("npm_config_prefix", root.to_string_lossy().as_ref()));
        ui::step(format!("npm install -g pnpm（前缀 {}）", root.display()));
        cmd::run_with([&npm, "install", "-g", "pnpm"], &env)?;
        ui::ok(format!("pnpm 装到了 {}", root.join("bin").display()));
        Ok(())
    }

    fn via_corepack(&self) -> Result<()> {
        let corepack = self
            .node_bin("corepack")
            .ok_or_else(|| anyhow::anyhow!("没有 corepack"))?;
        let env = self.node_env();
        ui::step("corepack enable + prepare pnpm@latest");
        cmd::run_with([&corepack, "enable"], &env)?;
        cmd::run_with([&corepack, "prepare", "pnpm@latest", "--activate"], &env)?;
        ui::ok("pnpm 已由 corepack 接管。");
        Ok(())
    }
}

impl Dev for Pnpm {
    fn name(&self) -> &str {
        "pnpm"
    }

    fn summary(&self) -> &str {
        "pnpm 包管理器"
    }

    fn pkgs(&self, pm: Pm) -> &'static [&'static str] {
        match pm {
            Pm::Brew | Pm::Apk | Pm::Pacman => &["pnpm"],
            // apt / dnf / yum / zypper 的官方源里都没有 pnpm。
            _ => &[],
        }
    }

    fn root(&self) -> PathBuf {
        install_root("FUNDEPLOY_NPM_GLOBAL_ROOT", "npm")
    }

    /// 两条安装路径的落点不一样。
    ///
    /// npm 全局前缀那条装进 `<root>/bin`；corepack 那条把 shim 放在 corepack
    /// 自己边上，也就是 Node 的 bin 里。只认前者的话，走 corepack 装完 status
    /// 回头就报「未安装」——刚装完就说没装，人只会再装一遍。
    fn bin_dir(&self) -> Option<PathBuf> {
        let own = self.root().join("bin");
        if own.join("pnpm").is_file() {
            return Some(own);
        }
        let node = super::nodejs::NodeJs.root().join("bin");
        node.join("pnpm").is_file().then_some(node)
    }

    fn probe(&self) -> (&'static str, &'static [&'static str]) {
        ("pnpm", &["-v"])
    }

    fn install_source(&self, version: Option<&str>) -> Result<()> {
        if let Some(v) = version {
            bail!("pnpm 这里只装最新版，钉不到 {v}。要指定版本用 `corepack prepare pnpm@{v} --activate`。");
        }
        if env::flag("FUNDEPLOY_PNPM_VIA_NPM", false) {
            return self.via_npm();
        }
        // corepack 优先：它是 Node 官方的那条路，也认项目里的 packageManager 字段。
        // 但 Node 25 起 corepack 不再随发行版打包，装不到就自动改走 npm 全局前缀
        // ——这里不自动回退的话，在新版 Node 上得到的是一句 "corepack: not found"。
        match self.via_corepack() {
            Ok(()) => Ok(()),
            Err(e) => {
                ui::info(format!("corepack 这条路没走通（{e}），改用 npm 全局安装。"));
                self.via_npm()
            }
        }
    }

    fn uninstall_source(&self) -> Result<()> {
        if let Some(corepack) = self.node_bin("corepack") {
            let _ = cmd::run_with([&corepack, "disable", "pnpm"], &self.node_env());
        }
        super::wipe(&self.root())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 多数发行版的源里没有_pnpm() {
        for pm in [Pm::Apt, Pm::Dnf, Pm::Yum, Pm::Zypper] {
            assert!(Pnpm.pkgs(pm).is_empty(), "{} 不该报有 pnpm", pm.name());
        }
        assert_eq!(Pnpm.pkgs(Pm::Brew), &["pnpm"]);
    }

    #[test]
    fn 全局前缀目录跟_node_的分开() {
        // 共用一个目录的话，卸 pnpm 会把 Node 一起删掉。
        assert_ne!(Pnpm.root(), super::super::nodejs::NodeJs.root());
    }

    #[test]
    fn corepack_装在_node_的_bin_里也算装了() {
        static M: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _g = M.lock().unwrap_or_else(|e| e.into_inner());
        let d = tempfile::tempdir().unwrap();
        let npm = d.path().join("npm");
        let node = d.path().join("node");
        std::env::set_var("FUNDEPLOY_NPM_GLOBAL_ROOT", &npm);
        std::env::set_var("FUNDEPLOY_NODE_ROOT", &node);

        assert!(Pnpm.bin_dir().is_none(), "两处都没有时该是 None");

        std::fs::create_dir_all(node.join("bin")).unwrap();
        std::fs::write(node.join("bin/pnpm"), "").unwrap();
        assert_eq!(Pnpm.bin_dir(), Some(node.join("bin")));

        // 自己那份在的时候以自己那份为准。
        std::fs::create_dir_all(npm.join("bin")).unwrap();
        std::fs::write(npm.join("bin/pnpm"), "").unwrap();
        assert_eq!(Pnpm.bin_dir(), Some(npm.join("bin")));

        std::env::remove_var("FUNDEPLOY_NPM_GLOBAL_ROOT");
        std::env::remove_var("FUNDEPLOY_NODE_ROOT");
    }

    #[test]
    fn 钉版本时明确拒绝而不是装最新版() {
        let e = Pnpm.install_source(Some("9.0.0")).unwrap_err().to_string();
        assert!(e.contains("9.0.0"), "实际: {e}");
    }
}
