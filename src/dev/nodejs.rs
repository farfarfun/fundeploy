//! Node.js。

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Deserialize;

use super::{install_root, Dev};
use crate::common::platform::{Arch, Platform};
use crate::common::pm::Pm;
use crate::common::{archive, download, fs};
use crate::ui;

const DIST: &str = "https://nodejs.org/dist";

pub struct NodeJs;

impl Dev for NodeJs {
    fn name(&self) -> &str {
        "nodejs"
    }

    fn summary(&self) -> &str {
        "Node.js 运行时（含 npm）"
    }

    fn pkgs(&self, pm: Pm) -> &'static [&'static str] {
        match pm {
            // brew 的 node 一个包就带上 npm；别家是分开的两个。
            Pm::Brew => &["node"],
            _ => &["nodejs", "npm"],
        }
    }

    fn root(&self) -> PathBuf {
        install_root("FUNDEPLOY_NODE_ROOT", "node")
    }

    fn probe(&self) -> (&'static str, &'static [&'static str]) {
        ("node", &["-v"])
    }

    fn install_source(&self, version: Option<&str>) -> Result<()> {
        let plat = Platform::current()?;
        let suffix = format!("{}-{}", plat.os.go(), node_arch(plat.arch));
        let ver = match version {
            Some(v) => normalize(v),
            None => latest_lts()?,
        };
        // 用 .tar.gz 而不是官方更推荐的 .tar.xz：xz 要么拖一个 liblzma 的 C 依赖
        // 进来（musl 全静态构建就此泡汤），要么多一个纯 Rust 解码器。官方两种
        // 格式的内容一模一样，差的只是几十兆下载量。
        let name = format!("node-{ver}-{suffix}.tar.gz");
        let url = format!("{DIST}/{ver}/{name}");

        let scratch = fs::Scratch::new("node")?;
        let pkg = scratch.path().join(&name);
        download::to_file(&url, &pkg, &format!("Node {ver}"))?;

        let root = self.root();
        if root.exists() {
            fs::guard_removable(&root)?;
            std::fs::remove_dir_all(&root)
                .with_context(|| format!("清不掉旧的 {}", root.display()))?;
        }
        // 剥掉 node-v22.14.0-linux-x64/ 这层，安装路径就不随版本变。
        archive::untar_gz(&pkg, &root, 1)?;
        ui::ok(format!("Node {ver} 装到了 {}", root.display()));
        Ok(())
    }
}

/// Node 的架构写法：x64 / arm64。跟 Go 的 amd64 差一个词。
fn node_arch(a: Arch) -> &'static str {
    match a {
        Arch::X86_64 => "x64",
        Arch::Aarch64 => "arm64",
    }
}

fn normalize(v: &str) -> String {
    let v = v.trim();
    match v.strip_prefix('v') {
        Some(rest) => format!("v{rest}"),
        None => format!("v{v}"),
    }
}

#[derive(Deserialize)]
struct Release {
    version: String,
    /// LTS 版本是代号字符串（`"Jod"`），非 LTS 是 `false`。
    lts: serde_json::Value,
}

/// 最新的 LTS 版本号。
///
/// 不写死版本号：写死的那个迟早会过保，而「装上去就是个停止维护的版本」没人会
/// 注意到。取 LTS 而不是绝对最新，是因为大量工具链（electron、node-gyp 链路上
/// 的原生模块）对奇数号主版本的支持总是滞后。
fn latest_lts() -> Result<String> {
    let body = download::get_text(&format!("{DIST}/index.json"))
        .context("问不到 nodejs.org 上的版本清单")?;
    let all: Vec<Release> = serde_json::from_str(&body).context("nodejs.org 的版本清单解析不了")?;
    // 清单本身按版本从新到旧排，第一个 LTS 就是最新的 LTS。
    all.iter()
        .find(|r| r.lts.is_string())
        .map(|r| r.version.clone())
        .ok_or_else(|| anyhow::anyhow!("nodejs.org 的清单里一个 LTS 版本都没有"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 架构按_node_自己的叫法() {
        // Go 那边同一个东西叫 amd64。
        assert_eq!(node_arch(Arch::X86_64), "x64");
        assert_eq!(node_arch(Arch::Aarch64), "arm64");
    }

    #[test]
    fn 版本号带不带_v_都行() {
        assert_eq!(normalize("22.14.0"), "v22.14.0");
        assert_eq!(normalize("v22.14.0"), "v22.14.0");
    }

    #[test]
    fn brew_一个包就够别家要两个() {
        assert_eq!(NodeJs.pkgs(Pm::Brew), &["node"]);
        assert_eq!(NodeJs.pkgs(Pm::Apt), &["nodejs", "npm"]);
    }

    #[test]
    fn 只挑带代号的_lts_版本() {
        let body = r#"[
            {"version":"v25.0.0","lts":false},
            {"version":"v24.9.0","lts":false},
            {"version":"v22.14.0","lts":"Jod"},
            {"version":"v20.18.0","lts":"Iron"}
        ]"#;
        let all: Vec<Release> = serde_json::from_str(body).unwrap();
        let got = all.iter().find(|r| r.lts.is_string()).unwrap();
        assert_eq!(got.version, "v22.14.0");
    }
}
