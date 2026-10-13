//! Go。

use std::path::PathBuf;

use anyhow::{Context, Result};

use super::{install_root, Dev};
use crate::common::platform::Platform;
use crate::common::pm::Pm;
use crate::common::{archive, download, fs};
use crate::ui;

/// 官方发布的版本清单入口。纯文本，第一行就是最新稳定版（形如 `go1.27.2`）。
const VERSION_URL: &str = "https://go.dev/VERSION?m=text";
const DL_BASE: &str = "https://go.dev/dl";

pub struct Go;

impl Dev for Go {
    fn name(&self) -> &str {
        "go"
    }

    fn summary(&self) -> &str {
        "Go 工具链"
    }

    fn pkgs(&self, pm: Pm) -> &'static [&'static str] {
        match pm {
            Pm::Apt => &["golang-go"],
            Pm::Dnf | Pm::Yum => &["golang"],
            _ => &["go"],
        }
    }

    fn root(&self) -> PathBuf {
        install_root("FUNDEPLOY_GO_ROOT", "go")
    }

    fn probe(&self) -> (&'static str, &'static [&'static str]) {
        ("go", &["version"])
    }

    fn install_source(&self, version: Option<&str>) -> Result<()> {
        let plat = Platform::current()?;
        let suffix = format!("{}-{}", plat.os.go(), plat.arch.go());
        let ver = match version {
            Some(v) => normalize(v),
            None => latest()?,
        };
        let name = format!("{ver}.{suffix}.tar.gz");
        let url = format!("{DL_BASE}/{name}");

        let scratch = fs::Scratch::new("go")?;
        let pkg = scratch.path().join(&name);
        download::to_file(&url, &pkg, &format!("Go {ver}"))?;

        // 官方包自带一层 go/ 顶层目录，剥掉它，这样安装路径不随版本变。
        //
        // 先清空再解，不然换版本时上一版留下的文件会跟新版混在一起——Go 的
        // src/ 和 pkg/ 跨版本增删频繁，混起来后 `go build` 的报错指向的是
        // 根本不存在的旧标准库文件。
        let root = self.root();
        if root.exists() {
            fs::guard_removable(&root)?;
            std::fs::remove_dir_all(&root)
                .with_context(|| format!("清不掉旧的 {}", root.display()))?;
        }
        archive::untar_gz(&pkg, &root, 1)?;
        ui::ok(format!("Go {ver} 装到了 {}", root.display()));
        Ok(())
    }
}

/// 补上 `go` 前缀：人填 `1.22.4` 和 `go1.22.4` 都该能用。
fn normalize(v: &str) -> String {
    let v = v.trim();
    match v.strip_prefix("go") {
        Some(rest) => format!("go{rest}"),
        None => format!("go{v}"),
    }
}

fn latest() -> Result<String> {
    let body = download::get_text(VERSION_URL).context("问不到 go.dev 上的最新版本")?;
    let line = body
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("go"))
        .ok_or_else(|| anyhow::anyhow!("go.dev 返回的内容里没有版本号:\n{body}"))?;
    Ok(line.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 版本号带不带_go_前缀都行() {
        assert_eq!(normalize("1.22.4"), "go1.22.4");
        assert_eq!(normalize("go1.22.4"), "go1.22.4");
        assert_eq!(normalize("  go1.22.4 "), "go1.22.4");
    }

    #[test]
    fn 各家包管理器下的包名不一样() {
        // apt 叫 golang-go，dnf 叫 golang，brew 叫 go——照一个名字装过去就是
        // 「软件包未找到」。
        assert_eq!(Go.pkgs(Pm::Apt), &["golang-go"]);
        assert_eq!(Go.pkgs(Pm::Dnf), &["golang"]);
        assert_eq!(Go.pkgs(Pm::Brew), &["go"]);
    }

    #[test]
    fn 安装根目录默认在_opt_下() {
        assert!(Go.root().ends_with("opt/go"));
    }
}
