//! crates.io 换源。
//!
//! cargo 的换源不是改一个地址，而是声明「把 crates-io 这个源替换成另一个」，
//! 要写三个 TOML 表。而 `~/.cargo/config.toml` 里通常还有 `[build]`、
//! `[target.*]`、`[net]`——都是人手写的，带注释。
//!
//! 所以这里不解析整份 TOML 再写回去（那样注释和顺序全没了），而是用
//! [`crate::common::block`] 划一块自己的地方。代价是：文件里已经有
//! `[source.crates-io]` 时必须停下来报错——TOML 不允许同名表出现两次，
//! 硬写进去会让 cargo 连配置都读不了，那时候连 `cargo --version` 都跑不起来。

use std::path::PathBuf;

use anyhow::{bail, Result};

use super::{Mirror, Source};
use crate::common::block::Marker;
use crate::common::env;
use crate::ui;

/// 替换源在配置里的名字。
const ALIAS: &str = "fundeploy-mirror";

pub struct Cargo;

impl Cargo {
    fn file(&self) -> PathBuf {
        if let Some(p) = env::opt("FUNDEPLOY_CARGO_CONFIG") {
            return env::expand_tilde(&p);
        }
        let home = env::opt("CARGO_HOME")
            .map(|p| env::expand_tilde(&p))
            .unwrap_or_else(|| env::home().join(".cargo"));
        home.join("config.toml")
    }

    fn marker(&self) -> Marker {
        Marker::new("cargo mirror")
    }

    /// 区块之外是不是已经有人声明了 `[source.crates-io]`。
    fn conflicts(&self) -> Result<bool> {
        let path = self.file();
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Ok(false);
        };
        let ours = self.marker().read(&path).unwrap_or_default();
        Ok(text
            .lines()
            .filter(|l| !ours.iter().any(|o| o == *l))
            .any(|l| l.trim().starts_with("[source.crates-io]")))
    }
}

impl Source for Cargo {
    fn name(&self) -> &str {
        "cargo"
    }

    fn summary(&self) -> &str {
        "crates.io 索引与下载"
    }

    fn config_path(&self) -> PathBuf {
        self.file()
    }

    fn builtin(&self) -> Vec<Mirror> {
        // 一律用 sparse 协议：git 索引要先把整个 crates.io-index 仓库克隆下来
        // （几百 MB），sparse 只按需取用到的那几个 crate 的元数据。
        // cargo 1.68 起 sparse 是稳定且默认的。
        vec![
            Mirror::new(
                "rsproxy",
                "字节 rsproxy",
                "sparse+https://rsproxy.cn/index/",
            ),
            Mirror::new(
                "aliyun",
                "阿里云",
                "sparse+https://mirrors.aliyun.com/crates.io-index/",
            ),
            Mirror::new(
                "ustc",
                "中国科大",
                "sparse+https://mirrors.ustc.edu.cn/crates.io-index/",
            ),
            Mirror::new(
                "tuna",
                "清华大学",
                "sparse+https://mirrors.tuna.tsinghua.edu.cn/crates.io-index/",
            ),
            Mirror::new("official", "官方", "sparse+https://index.crates.io/").official(),
        ]
    }

    /// 去掉 `sparse+` 前缀再量——那个前缀是给 cargo 看的，不是 URL 的一部分。
    /// `config.json` 是 sparse 索引的入口文件，每个镜像上都有，而且只有几十字节。
    fn probe_url(&self, m: &Mirror) -> String {
        let base = m.url.trim_start_matches("sparse+").trim_end_matches('/');
        format!("{base}/config.json")
    }

    fn current(&self) -> Option<String> {
        let body = self.marker().read(&self.file())?;
        body.iter().find_map(|l| {
            let (k, v) = l.split_once('=')?;
            (k.trim() == "registry").then(|| v.trim().trim_matches('"').to_string())
        })
    }

    fn apply(&self, m: &Mirror) -> Result<()> {
        if self.conflicts()? {
            bail!(
                "{} 里已经有 [source.crates-io] 了，不能再写一份（TOML 不允许同名表）。\n\
                 先把那一段删掉再重试，或者直接把它的 replace-with 改成你要的源。",
                self.file().display()
            );
        }
        // 官方源就是「不替换」，写一条 replace-with 指回 crates-io 是自指。
        if m.official {
            return self.reset();
        }
        let body = vec![
            "[source.crates-io]".to_string(),
            format!("replace-with = \"{ALIAS}\""),
            String::new(),
            format!("[source.{ALIAS}]"),
            format!("registry = \"{}\"", m.url),
        ];
        self.marker().write(&self.file(), &body)
    }

    fn reset(&self) -> Result<()> {
        if self.marker().remove(&self.file())? {
            ui::ok("已恢复 crates.io 官方源。");
        } else {
            ui::info("本来就没配过镜像。");
        }
        Ok(())
    }

    fn after_apply(&self, _m: &Mirror) {
        // 换源只影响索引和下载，已经解析好的 Cargo.lock 里记的是 crate 名和版本，
        // 不记来源，所以老项目不用动。但 registry 缓存是按源分开存的。
        ui::hint("已有项目直接 cargo build 即可；首次会重新拉一遍索引。");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::排队;

    fn 临时配置() -> (tempfile::TempDir, std::sync::MutexGuard<'static, ()>) {
        let g = 排队();
        let d = tempfile::tempdir().unwrap();
        std::env::set_var("FUNDEPLOY_CARGO_CONFIG", d.path().join("config.toml"));
        (d, g)
    }

    /// 写出来的东西必须是合法 TOML，而且字段确实在 cargo 认的位置上。
    fn 解析(c: &Cargo) -> toml::Value {
        let text = std::fs::read_to_string(c.config_path()).unwrap();
        toml::from_str(&text).unwrap_or_else(|e| panic!("写出来的不是合法 TOML: {e}\n{text}"))
    }

    #[test]
    fn 换源写出合法且位置正确的_toml() {
        let (_d, _g) = 临时配置();
        let c = Cargo;
        c.apply(&c.lookup("rsproxy").unwrap()).unwrap();

        let v = 解析(&c);
        assert_eq!(
            v["source"]["crates-io"]["replace-with"].as_str(),
            Some(ALIAS)
        );
        assert_eq!(
            v["source"][ALIAS]["registry"].as_str(),
            Some("sparse+https://rsproxy.cn/index/")
        );
        assert_eq!(
            c.current().as_deref(),
            Some("sparse+https://rsproxy.cn/index/")
        );
    }

    #[test]
    fn 换第二次是替换而不是再写一份同名表() {
        // 再写一份的话文件就不是合法 TOML 了，cargo 任何命令都会直接报错退出。
        let (_d, _g) = 临时配置();
        let c = Cargo;
        c.apply(&c.lookup("rsproxy").unwrap()).unwrap();
        c.apply(&c.lookup("ustc").unwrap()).unwrap();
        let v = 解析(&c);
        assert_eq!(
            v["source"][ALIAS]["registry"].as_str(),
            Some("sparse+https://mirrors.ustc.edu.cn/crates.io-index/")
        );
    }

    #[test]
    fn 不动文件里原有的设置() {
        let (_d, _g) = 临时配置();
        let c = Cargo;
        std::fs::write(
            c.config_path(),
            "# 我的设置\n[build]\njobs = 4\n\n[net]\ngit-fetch-with-cli = true\n",
        )
        .unwrap();
        c.apply(&c.lookup("aliyun").unwrap()).unwrap();
        let text = std::fs::read_to_string(c.config_path()).unwrap();
        assert!(text.contains("# 我的设置"), "实际:\n{text}");
        let v = 解析(&c);
        assert_eq!(v["build"]["jobs"].as_integer(), Some(4));
        assert!(v["net"]["git-fetch-with-cli"].as_bool().unwrap());
    }

    #[test]
    fn 已有同名表时报错而不是写成坏文件() {
        let (_d, _g) = 临时配置();
        let c = Cargo;
        std::fs::write(
            c.config_path(),
            "[source.crates-io]\nreplace-with = \"自己配的\"\n",
        )
        .unwrap();
        let err = c.apply(&c.lookup("ustc").unwrap()).unwrap_err().to_string();
        assert!(err.contains("source.crates-io"), "实际: {err}");
        // 报了错就不能动过文件。
        let text = std::fs::read_to_string(c.config_path()).unwrap();
        assert!(text.contains("自己配的"));
    }

    #[test]
    fn 自己写的那段不算冲突() {
        let (_d, _g) = 临时配置();
        let c = Cargo;
        c.apply(&c.lookup("ustc").unwrap()).unwrap();
        assert!(!c.conflicts().unwrap(), "自己上次写的不该把自己挡住");
    }

    #[test]
    fn 选官方源等于撤掉替换() {
        // 写一条 replace-with = "crates-io" 是自指，cargo 会直接报错。
        let (_d, _g) = 临时配置();
        let c = Cargo;
        c.apply(&c.lookup("ustc").unwrap()).unwrap();
        c.apply(&c.lookup("official").unwrap()).unwrap();
        assert_eq!(c.current(), None);
        let text = std::fs::read_to_string(c.config_path()).unwrap_or_default();
        assert!(!text.contains("replace-with"), "实际:\n{text}");
    }

    #[test]
    fn reset_之后还是合法_toml() {
        let (_d, _g) = 临时配置();
        let c = Cargo;
        std::fs::write(c.config_path(), "[build]\njobs = 4\n").unwrap();
        c.apply(&c.lookup("tuna").unwrap()).unwrap();
        c.reset().unwrap();
        assert_eq!(解析(&c)["build"]["jobs"].as_integer(), Some(4));
        assert_eq!(c.current(), None);
    }

    #[test]
    fn 探测地址去掉了_sparse_前缀() {
        let c = Cargo;
        assert_eq!(
            c.probe_url(&c.lookup("rsproxy").unwrap()),
            "https://rsproxy.cn/index/config.json"
        );
    }

    #[test]
    fn cargo_home_能把配置指到别处() {
        let _g = 排队();
        let d = tempfile::tempdir().unwrap();
        std::env::remove_var("FUNDEPLOY_CARGO_CONFIG");
        std::env::set_var("CARGO_HOME", d.path());
        assert_eq!(Cargo.config_path(), d.path().join("config.toml"));
        std::env::remove_var("CARGO_HOME");
    }
}
