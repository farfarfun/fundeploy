//! npm registry 换源。
//!
//! 直接写 `~/.npmrc`，不调 `npm config set`：换源常常发生在 node 装上之前，
//! 而且 pnpm、yarn(berry)、bun 读的都是同一个文件——调 npm 的 CLI 反而把一件
//! 跟 npm 本身无关的事绑在了 npm 存在这个前提上。

use std::path::PathBuf;

use anyhow::Result;

use super::{Mirror, Source};
use crate::common::env;
use crate::common::ini::Ini;
use crate::ui;

pub struct Npm;

impl Npm {
    fn file(&self) -> PathBuf {
        env::opt("FUNDEPLOY_NPMRC")
            .map(|p| env::expand_tilde(&p))
            .unwrap_or_else(|| env::home().join(".npmrc"))
    }
}

impl Source for Npm {
    fn name(&self) -> &str {
        "npm"
    }

    fn summary(&self) -> &str {
        "npm registry（pnpm / yarn / bun 都读这份配置）"
    }

    fn config_path(&self) -> PathBuf {
        self.file()
    }

    fn builtin(&self) -> Vec<Mirror> {
        vec![
            // 原 cnpm/taobao 源。registry.npm.taobao.org 的证书 2022 年就过期了，
            // 域名也只做跳转，所以这里只给新域名。
            Mirror::new(
                "npmmirror",
                "阿里 npmmirror",
                "https://registry.npmmirror.com/",
            ),
            Mirror::new(
                "huawei",
                "华为云",
                "https://mirrors.huaweicloud.com/repository/npm/",
            ),
            Mirror::new(
                "tencent",
                "腾讯云",
                "https://mirrors.cloud.tencent.com/npm/",
            ),
            Mirror::new("official", "官方", "https://registry.npmjs.org/").official(),
        ]
    }

    /// 量 `npm` 这个包自己的元数据：每个 registry 上必定有，响应也小。
    fn probe_url(&self, m: &Mirror) -> String {
        format!("{}/npm", m.url.trim_end_matches('/'))
    }

    fn current(&self) -> Option<String> {
        Ini::load(&self.file()).ok()?.get(None, "registry")
    }

    fn apply(&self, m: &Mirror) -> Result<()> {
        let path = self.file();
        let mut ini = Ini::load(&path)?;
        ini.set(None, "registry", &m.url);
        ini.save(&path)
    }

    fn reset(&self) -> Result<()> {
        let path = self.file();
        let mut ini = Ini::load(&path)?;
        ini.unset(None, "registry");
        if ini.is_trivial() && path.exists() {
            std::fs::remove_file(&path)?;
            ui::ok(format!("已删除 {}", path.display()));
            return Ok(());
        }
        ini.save(&path)
    }

    fn after_apply(&self, _m: &Mirror) {
        // 项目里的 .npmrc 优先级高于家目录这一份，而且 lockfile 里记着每个包的
        // 完整下载地址——换了源之后老 lockfile 仍然指向旧 registry。
        ui::hint("项目自带的 .npmrc 和 lock 文件里的地址优先，必要时删掉 lock 重装。");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::排队;

    fn 临时配置() -> (tempfile::TempDir, std::sync::MutexGuard<'static, ()>) {
        let g = 排队();
        let d = tempfile::tempdir().unwrap();
        std::env::set_var("FUNDEPLOY_NPMRC", d.path().join(".npmrc"));
        (d, g)
    }

    #[test]
    fn 换源写进_npmrc() {
        let (_d, _g) = 临时配置();
        let n = Npm;
        n.apply(&n.lookup("npmmirror").unwrap()).unwrap();
        assert_eq!(
            n.current().as_deref(),
            Some("https://registry.npmmirror.com/")
        );
    }

    #[test]
    fn 换源不碰_npmrc_里的鉴权行() {
        // .npmrc 里常放私有 scope 的 token，连带删掉的话下一次 install 会 401。
        let (_d, _g) = 临时配置();
        let n = Npm;
        std::fs::write(
            n.config_path(),
            "//registry.corp/:_authToken=abc\n@corp:registry=https://registry.corp/\n",
        )
        .unwrap();
        n.apply(&n.lookup("npmmirror").unwrap()).unwrap();
        let text = std::fs::read_to_string(n.config_path()).unwrap();
        assert!(text.contains("_authToken=abc"), "实际:\n{text}");
        assert!(text.contains("@corp:registry"), "实际:\n{text}");
    }

    #[test]
    fn scope_源不会被当成全局源读出来() {
        let (_d, _g) = 临时配置();
        let n = Npm;
        std::fs::write(n.config_path(), "@corp:registry=https://registry.corp/\n").unwrap();
        assert_eq!(n.current(), None);
    }

    #[test]
    fn reset_之后没有_registry_行() {
        let (_d, _g) = 临时配置();
        let n = Npm;
        n.apply(&n.lookup("tencent").unwrap()).unwrap();
        n.reset().unwrap();
        assert_eq!(n.current(), None);
    }

    #[test]
    fn reset_不删别人还在用的_npmrc() {
        let (_d, _g) = 临时配置();
        let n = Npm;
        std::fs::write(n.config_path(), "//r/:_authToken=abc\n").unwrap();
        n.apply(&n.lookup("tencent").unwrap()).unwrap();
        n.reset().unwrap();
        assert!(n.config_path().exists(), "文件里还有 token，不能删掉");
    }

    #[test]
    fn 探测地址量的是一个真包() {
        let n = Npm;
        assert_eq!(
            n.probe_url(&n.lookup("official").unwrap()),
            "https://registry.npmjs.org/npm"
        );
    }
}
