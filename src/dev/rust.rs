//! Rust（经 rustup）。

use std::path::PathBuf;

use anyhow::Result;

use super::{install_root, run_installer, set, wipe, Dev};
use crate::common::cmd;
use crate::common::pm::Pm;
use crate::ui;

const RUSTUP_SH: &str = "https://sh.rustup.rs";

pub struct Rust;

impl Rust {
    fn rustup_home(&self) -> PathBuf {
        self.root().join("rustup")
    }

    fn cargo_home(&self) -> PathBuf {
        self.root().join("cargo")
    }

    fn envs(&self) -> Vec<cmd::EnvEdit> {
        vec![
            set("RUSTUP_HOME", self.rustup_home().to_string_lossy().as_ref()),
            set("CARGO_HOME", self.cargo_home().to_string_lossy().as_ref()),
        ]
    }

    fn rustup_bin(&self) -> PathBuf {
        self.cargo_home().join("bin").join("rustup")
    }
}

impl Dev for Rust {
    fn name(&self) -> &str {
        "rust"
    }

    fn summary(&self) -> &str {
        "Rust 工具链（rustc / cargo）"
    }

    fn pkgs(&self, pm: Pm) -> &'static [&'static str] {
        match pm {
            Pm::Brew => &["rust"],
            _ => &["rustc", "cargo"],
        }
    }

    fn root(&self) -> PathBuf {
        install_root("FUNDEPLOY_RUST_ROOT", "rust")
    }

    fn bin_dir(&self) -> Option<PathBuf> {
        // rustup 把 shim 放在 $CARGO_HOME/bin，不是 <root>/bin。
        let d = self.cargo_home().join("bin");
        d.is_dir().then_some(d)
    }

    fn probe(&self) -> (&'static str, &'static [&'static str]) {
        ("rustc", &["--version"])
    }

    fn install_source(&self, version: Option<&str>) -> Result<()> {
        let toolchain = version.unwrap_or("stable");
        std::fs::create_dir_all(self.rustup_home())?;
        std::fs::create_dir_all(self.cargo_home())?;
        // --no-modify-path：rustup 默认会往 ~/.profile、~/.bashrc、~/.zshenv 里
        // 追加一行 source $CARGO_HOME/env。装到 ~/opt 下这种非默认位置时，那行
        // 在卸载之后会变成每开一个新 shell 都报一次的 "No such file"。这一域里
        // 其余几样（go/node/uv/pnpm）也都是自己打印 PATH，保持一致。
        run_installer(
            RUSTUP_SH,
            "rustup",
            &["-y", "--no-modify-path", "--default-toolchain", toolchain],
            &self.envs(),
        )?;
        ui::ok(format!(
            "Rust（{toolchain}）装到了 {}",
            self.root().display()
        ));
        Ok(())
    }

    fn upgrade_source(&self) -> Result<()> {
        if !self.rustup_bin().is_file() {
            ui::info("没找到 rustup，改为重新安装。");
            return self.install_source(None);
        }
        let rustup = self.rustup_bin().to_string_lossy().into_owned();
        // self update 在包管理器装的 rustup 上会被拒（它由包管理器负责），
        // 所以失败只提一句，真正要紧的是下面那条。
        if cmd::run_with([&rustup, "self", "update"], &self.envs()).is_err() {
            ui::info("rustup 自身没能更新，继续更新工具链。");
        }
        cmd::run_with([&rustup, "update", "stable"], &self.envs())
    }

    fn uninstall_source(&self) -> Result<()> {
        if self.rustup_bin().is_file() {
            let rustup = self.rustup_bin().to_string_lossy().into_owned();
            // 先让 rustup 自己收尾：它知道自己往 shell 配置里写过什么。
            let _ = cmd::run_with([&rustup, "self", "uninstall", "-y"], &self.envs());
        }
        wipe(&self.root())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 工具链目录挂在安装根下面() {
        let r = Rust.root();
        assert!(Rust.rustup_home().starts_with(&r));
        assert!(Rust.cargo_home().starts_with(&r));
        assert!(Rust.rustup_bin().ends_with("cargo/bin/rustup"));
    }

    #[test]
    fn 两个_home_都传给了子进程() {
        // 不传的话 rustup 装的是 ~/.rustup / ~/.cargo，而我们随后删的是 ~/opt/rust。
        let keys: Vec<String> = Rust.envs().into_iter().map(|(k, _)| k).collect();
        assert!(keys.contains(&"RUSTUP_HOME".to_string()));
        assert!(keys.contains(&"CARGO_HOME".to_string()));
    }

    #[test]
    fn brew_一个包别家两个() {
        assert_eq!(Rust.pkgs(Pm::Brew), &["rust"]);
        assert_eq!(Rust.pkgs(Pm::Apt), &["rustc", "cargo"]);
    }
}
