//! Go module proxy 换源。
//!
//! `go env -w` 写的就是 `$GOENV` 指向的那个文件（默认
//! `~/.config/go/env`），格式是最朴素的 `KEY=VALUE`。这里在 go 装了的时候调它，
//! 没装的时候直接写文件——换源是装 go 之前就该做好的事，不然 `go install`
//! 第一步就卡住。
//!
//! # GOSUMDB 也得一起换
//!
//! 校验和数据库 `sum.golang.org` 在国内不通，而它是**另一个**地址，不走
//! GOPROXY。只换 GOPROXY 的症状很具体：下载飞快，然后卡在
//! `verifying module: ... dial tcp: i/o timeout`，看起来像代理没生效。

use std::path::PathBuf;

use anyhow::{Context, Result};

use super::{Mirror, Source};
use crate::common::ini::Ini;
use crate::common::{cmd, env};
use crate::ui;

/// 国内可用的校验和数据库镜像（Google 官方在国内的镜像站）。
const CN_SUMDB: &str = "sum.golang.google.cn";
const DEFAULT_SUMDB: &str = "sum.golang.org";

pub struct Go;

impl Go {
    fn file(&self) -> PathBuf {
        if let Some(p) = env::opt("FUNDEPLOY_GO_ENV") {
            return env::expand_tilde(&p);
        }
        if let Some(p) = env::opt("GOENV") {
            return env::expand_tilde(&p);
        }
        dirs::config_dir()
            .unwrap_or_else(|| env::home().join(".config"))
            .join("go")
            .join("env")
    }

    /// 装了 go 就用它自己的命令写：它知道 GOENV 在哪，也会顺手校验键名。
    /// 没装就直接写文件，等 go 装上之后这份配置照样生效。
    fn set(&self, pairs: &[(&str, &str)]) -> Result<()> {
        if env::opt("FUNDEPLOY_GO_ENV").is_none() && cmd::has("go") {
            let mut argv = vec!["go".to_string(), "env".to_string(), "-w".to_string()];
            argv.extend(pairs.iter().map(|(k, v)| format!("{k}={v}")));
            return cmd::run(argv);
        }
        let path = self.file();
        let mut ini = Ini::load(&path)?;
        for (k, v) in pairs {
            // go 自己写的是 `KEY=VALUE`（等号两边没空格），这里跟着它的格式，
            // 免得同一个文件里两种风格混着。
            ini.unset(None, k);
            ini.set(None, k, v);
        }
        ini.save(&path)?;
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        std::fs::write(&path, text.replace(" = ", "="))
            .with_context(|| format!("写不了 {}", path.display()))
    }
}

impl Source for Go {
    fn name(&self) -> &str {
        "go"
    }

    fn summary(&self) -> &str {
        "Go module proxy（GOPROXY / GOSUMDB）"
    }

    fn config_path(&self) -> PathBuf {
        self.file()
    }

    fn builtin(&self) -> Vec<Mirror> {
        // 末尾的 `direct` 是兜底：代理上没有的模块（私有仓库）直接去源站拉，
        // 少了它，任何代理没收录的模块都会直接失败。
        vec![
            Mirror::new("goproxycn", "七牛 goproxy.cn", "https://goproxy.cn,direct")
                .with("sumdb", CN_SUMDB),
            Mirror::new("goproxyio", "goproxy.io", "https://goproxy.io,direct")
                .with("sumdb", CN_SUMDB),
            Mirror::new(
                "aliyun",
                "阿里云",
                "https://mirrors.aliyun.com/goproxy/,direct",
            )
            .with("sumdb", CN_SUMDB),
            Mirror::new(
                "tencent",
                "腾讯云",
                "https://mirrors.cloud.tencent.com/go/,direct",
            )
            .with("sumdb", CN_SUMDB),
            Mirror::new("official", "官方", "https://proxy.golang.org,direct").official(),
        ]
    }

    /// GOPROXY 的值是逗号分隔的列表，量第一个代理。
    /// `@v/list` 是 module proxy 协议里必有的接口，回的是版本号列表，很小。
    fn probe_url(&self, m: &Mirror) -> String {
        let first = m.url.split(',').next().unwrap_or(&m.url);
        format!(
            "{}/github.com/pkg/errors/@v/list",
            first.trim_end_matches('/')
        )
    }

    fn current(&self) -> Option<String> {
        Ini::load(&self.file()).ok()?.get(None, "GOPROXY")
    }

    fn apply(&self, m: &Mirror) -> Result<()> {
        let sumdb = m.extra("sumdb").unwrap_or(DEFAULT_SUMDB);
        self.set(&[("GOPROXY", &m.url), ("GOSUMDB", sumdb)])
    }

    fn reset(&self) -> Result<()> {
        // 写回默认值，而不是删掉这两行：删掉的话，环境变量里要是还留着老值，
        // 它就会重新冒出来；写死成默认值则是明确的。
        self.set(&[
            ("GOPROXY", "https://proxy.golang.org,direct"),
            ("GOSUMDB", DEFAULT_SUMDB),
        ])
    }

    fn after_apply(&self, _m: &Mirror) {
        if std::env::var_os("GOPROXY").is_some() {
            // 环境变量优先级高于配置文件，设了它这次写的就不生效。
            ui::warn("当前 shell 里设了 GOPROXY，它会盖掉刚写的配置。把它从 shell 配置里去掉。");
        }
        ui::hint("私有仓库用 GOPRIVATE 排除，例如 go env -w GOPRIVATE=git.corp.com/*");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::排队;

    fn 临时配置() -> (tempfile::TempDir, std::sync::MutexGuard<'static, ()>) {
        let g = 排队();
        let d = tempfile::tempdir().unwrap();
        // 指了这个变量就走写文件那条路，测试里绝不去调真的 go。
        std::env::set_var("FUNDEPLOY_GO_ENV", d.path().join("env"));
        (d, g)
    }

    #[test]
    fn 换源同时写_goproxy_和_gosumdb() {
        // 只写 GOPROXY 的话会卡在校验和下载上，症状跟换源没生效一模一样。
        let (_d, _g) = 临时配置();
        let go = Go;
        go.apply(&go.lookup("goproxycn").unwrap()).unwrap();
        let ini = Ini::load(&go.config_path()).unwrap();
        assert_eq!(
            ini.get(None, "GOPROXY").as_deref(),
            Some("https://goproxy.cn,direct")
        );
        assert_eq!(ini.get(None, "GOSUMDB").as_deref(), Some(CN_SUMDB));
    }

    #[test]
    fn 官方源用官方的校验和库() {
        let (_d, _g) = 临时配置();
        let go = Go;
        go.apply(&go.lookup("official").unwrap()).unwrap();
        let ini = Ini::load(&go.config_path()).unwrap();
        assert_eq!(ini.get(None, "GOSUMDB").as_deref(), Some(DEFAULT_SUMDB));
    }

    #[test]
    fn 写出来的格式和_go_自己写的一致() {
        // go 写的是 KEY=VALUE。混进 `KEY = VALUE` 的话 go 读得出来，
        // 但下一次 `go env -w` 会把文件整理成它的格式，两种风格来回变。
        let (_d, _g) = 临时配置();
        let go = Go;
        go.apply(&go.lookup("aliyun").unwrap()).unwrap();
        let text = std::fs::read_to_string(go.config_path()).unwrap();
        assert!(text.contains("GOPROXY=https://"), "实际:\n{text}");
        assert!(!text.contains("GOPROXY ="), "实际:\n{text}");
    }

    #[test]
    fn 不碰文件里别的_go_设置() {
        let (_d, _g) = 临时配置();
        let go = Go;
        std::fs::write(
            go.config_path(),
            "GOPATH=/data/go\nGOPRIVATE=git.corp.com/*\n",
        )
        .unwrap();
        go.apply(&go.lookup("tencent").unwrap()).unwrap();
        let ini = Ini::load(&go.config_path()).unwrap();
        assert_eq!(ini.get(None, "GOPATH").as_deref(), Some("/data/go"));
        assert_eq!(
            ini.get(None, "GOPRIVATE").as_deref(),
            Some("git.corp.com/*")
        );
    }

    #[test]
    fn 换第二次不留下两行_goproxy() {
        let (_d, _g) = 临时配置();
        let go = Go;
        go.apply(&go.lookup("goproxycn").unwrap()).unwrap();
        go.apply(&go.lookup("aliyun").unwrap()).unwrap();
        let text = std::fs::read_to_string(go.config_path()).unwrap();
        assert_eq!(text.matches("GOPROXY=").count(), 1, "实际:\n{text}");
    }

    #[test]
    fn reset_写回官方值而不是把行删掉() {
        let (_d, _g) = 临时配置();
        let go = Go;
        go.apply(&go.lookup("goproxycn").unwrap()).unwrap();
        go.reset().unwrap();
        assert_eq!(
            go.current().as_deref(),
            Some("https://proxy.golang.org,direct")
        );
    }

    #[test]
    fn 探测地址取列表里第一个代理() {
        let go = Go;
        assert_eq!(
            go.probe_url(&go.lookup("goproxycn").unwrap()),
            "https://goproxy.cn/github.com/pkg/errors/@v/list"
        );
        // 带结尾斜杠的源不能拼出双斜杠。
        assert_eq!(
            go.probe_url(&go.lookup("aliyun").unwrap()),
            "https://mirrors.aliyun.com/goproxy/github.com/pkg/errors/@v/list"
        );
    }

    #[test]
    fn 每个国内源都配了能用的校验和库() {
        for m in Go.builtin() {
            if m.official {
                continue;
            }
            assert_eq!(m.extra("sumdb"), Some(CN_SUMDB), "{} 漏了 sumdb", m.name);
        }
    }
}
