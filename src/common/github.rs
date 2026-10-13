//! Releases 查询。
//!
//! 不要求登录。设了 `GITHUB_TOKEN` / `GH_TOKEN` 就带上（见 `download::get_text`），
//! 匿名限流是每小时 60 次，所以每个动作最多问一次，不做轮询。
//!
//! Gitee 也在这儿，是因为两家 release 的 JSON 字段恰好同名（`tag_name`、
//! `assets[].name`、`assets[].browser_download_url`），下面的结构体原样就能解析
//! 两边的响应，差的只有一个 API 基址。它只用在 fundeploy 自己身上——见
//! [`Source`]。

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;

use super::download;
use crate::ui;

#[derive(Debug, Clone, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

impl Release {
    /// 版本号，即去掉 `v` 前缀的 tag。
    pub fn version(&self) -> &str {
        self.tag_name.trim_start_matches(['v', 'V'])
    }

    pub fn asset(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.name == name)
    }

    /// release 自带的校验和清单的下载地址。
    ///
    /// 名字各家不一样：goreleaser 出 `checksums.txt`，Tauri 那边常见
    /// `SHA256SUMS.txt`。里面的格式倒是一致的（`<sha256>  <文件名>`），所以只是
    /// 多认几个名字的事——认不出来的后果是白白跳过校验。
    pub fn checksums_url(&self) -> Option<&str> {
        const NAMES: &[&str] = &["checksums.txt", "sha256sums.txt", "sha256sums", "checksums"];
        self.assets
            .iter()
            .find(|a| NAMES.contains(&a.name.to_ascii_lowercase().as_str()))
            .map(|a| a.browser_download_url.as_str())
    }
}

/// 把用户给的版本号规范成 tag：`1.2.3` 和 `v1.2.3` 都接受，统一成 `v1.2.3`。
pub fn tag_of(version: &str) -> Result<String> {
    let v = version.trim().trim_start_matches(['v', 'V']);
    if v.is_empty() {
        bail!("版本号是空的");
    }
    Ok(format!("v{v}"))
}

/// 从哪个站点取 release。
///
/// 只对 fundeploy 自己的 release 有意义：Gitee 上那份是 CI 从 GitHub 整仓镜像
/// 过去的，连 release 附件一起搬（见 `.github/workflows/sync-gitee.yml`）。别人
/// 的仓库我们没镜像，所以这个模块里查第三方 release 的那几个函数一律直连 GitHub。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    GitHub,
    Gitee,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::GitHub => "github",
            Source::Gitee => "gitee",
        }
    }

    /// 查一条 release 的 API 地址。`tag` 为 `None` 就是 latest。
    fn release_url(self, repo: &str, tag: Option<&str>) -> String {
        let path = match tag {
            Some(t) => format!("{repo}/releases/tags/{t}"),
            None => format!("{repo}/releases/latest"),
        };
        match self {
            Source::GitHub => format!("https://api.github.com/repos/{path}"),
            Source::Gitee => format!("https://gitee.com/api/v5/repos/{path}"),
        }
    }

    /// `FUNDEPLOY_RELEASE_SOURCE` 点的名。没设或者设岔了都返回 `None`，由调用方
    /// 决定默认怎么走。
    pub fn from_env() -> Option<Self> {
        let v = super::env::opt("FUNDEPLOY_RELEASE_SOURCE")?;
        match v.trim().to_ascii_lowercase().as_str() {
            "" => None,
            "github" => Some(Source::GitHub),
            "gitee" => Some(Source::Gitee),
            other => {
                ui::warn(format!(
                    "FUNDEPLOY_RELEASE_SOURCE={other} 不认识（只认 github / gitee），这次按默认来。"
                ));
                None
            }
        }
    }
}

fn fetch(src: Source, repo: &str, tag: Option<&str>, probe: bool) -> Result<Release> {
    let url = src.release_url(repo, tag);
    let get = if probe {
        download::get_text_probe
    } else {
        download::get_text
    };
    // 最常见的失败是 404——指定的版本根本不存在。不加这句的话，报出来的只有
    // 一个 API URL 和 `status code 404`，看不出是「版本写错了」。
    let body = get(&url).with_context(|| match tag {
        Some(t) => format!("{repo} 上取不到 {t}（多半是没有这个版本）"),
        None => format!("取不到 {repo} 的最新版本"),
    })?;
    serde_json::from_str(&body)
        .with_context(|| format!("解析 {} 上 {repo} 的 release 失败", src.label()))
}

fn api(path: &str) -> String {
    format!("https://api.github.com/repos/{path}")
}

/// `latest` 指向的那条 release。
pub fn latest(repo: &str) -> Result<Release> {
    fetch(Source::GitHub, repo, None, false)
}

/// 指定 tag 的 release。
pub fn by_tag(repo: &str, tag: &str) -> Result<Release> {
    fetch(Source::GitHub, repo, Some(tag), false)
}

/// [`fetch_release`] 的结果：拿到了什么，以及从哪儿拿的。
pub struct Fetched {
    pub release: Release,
    pub source: Source,
    /// 是不是 GitHub 没通才落到这儿的。点名点过来的不算——这两件事对用户的
    /// 含义完全不同，一个是「网络出问题了」，一个是「你自己设的」。
    pub fell_back: bool,
}

/// 取 fundeploy 自己的一条 release，GitHub 不通就换 Gitee 上那份镜像。
///
/// `FUNDEPLOY_RELEASE_SOURCE` 点了名就只用那一个，失败直接报错——点名的人要的
/// 就是确定性，背着他换一个源，下次出问题根本说不清这份是从哪儿下的。
///
/// 没点名则先 GitHub：它是权威源，Gitee 那份是镜像过去的，万一哪天同步断了，
/// 默认走 GitHub 的人不会被静悄悄钉在旧版本上。
///
/// 回落不区分失败原因。真正值得区分的只有「连不上」该换、「没这个版本」不必换，
/// 而后者本来就是一次快速的 404，多问 Gitee 一次也就几十毫秒——为这点事去拆
/// ureq 的错误类型不划算。
pub fn fetch_release(repo: &str, tag: Option<&str>) -> Result<Fetched> {
    if let Some(source) = Source::from_env() {
        return fetch(source, repo, tag, false).map(|release| Fetched {
            release,
            source,
            fell_back: false,
        });
    }

    let first = match fetch(Source::GitHub, repo, tag, true) {
        Ok(release) => {
            return Ok(Fetched {
                release,
                source: Source::GitHub,
                fell_back: false,
            })
        }
        Err(e) => e,
    };
    fetch(Source::Gitee, repo, tag, false)
        .map(|release| Fetched {
            release,
            source: Source::Gitee,
            fell_back: true,
        })
        .map_err(|second| {
            anyhow!(
                "GitHub 和 Gitee 镜像都取不到 {repo} 的 release。\n  \
                 github: {first:#}\n  gitee: {second:#}"
            )
        })
}

/// 最近若干条 release，新的在前。
pub fn recent(repo: &str, limit: usize) -> Result<Vec<Release>> {
    let body = download::get_text(&api(&format!("{repo}/releases?per_page={limit}")))?;
    serde_json::from_str(&body).with_context(|| format!("解析 {repo} 的 release 列表失败"))
}

/// 找出最近一条**带有我们要的那个附件**的 release。
///
/// 不能直接用 `latest`：上游常常先打 tag、CI 过一会儿才传二进制，这期间 latest
/// 的 assets 是空的，照着它装只会 404。`want` 按 tag 算附件名——不同版本的附件名
/// 里嵌着版本号，所以必须逐条算。
pub fn latest_with_asset(
    repo: &str,
    limit: usize,
    want: impl Fn(&str) -> String,
) -> Result<(Release, String)> {
    let all = recent(repo, limit)?;
    for r in all {
        if r.draft {
            continue;
        }
        let name = want(&r.tag_name);
        if r.asset(&name).is_some() {
            return Ok((r, name));
        }
    }
    bail!("{repo} 最近 {limit} 条 release 里都没有当前平台的附件")
}

/// release 附件的直链。API 给的 `browser_download_url` 优先；它拿不到时按约定拼。
pub fn asset_url(repo: &str, tag: &str, asset: &str) -> String {
    format!("https://github.com/{repo}/releases/download/{tag}/{asset}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, assets: &[&str]) -> Release {
        Release {
            tag_name: tag.into(),
            draft: false,
            prerelease: false,
            assets: assets
                .iter()
                .map(|n| Asset {
                    name: (*n).into(),
                    browser_download_url: format!("https://github.com/o/r/d/{n}"),
                })
                .collect(),
        }
    }

    #[test]
    fn tag_规范化接受两种写法() {
        assert_eq!(tag_of("1.2.3").unwrap(), "v1.2.3");
        assert_eq!(tag_of("v1.2.3").unwrap(), "v1.2.3");
        assert_eq!(tag_of("  V1.2.3 ").unwrap(), "v1.2.3");
        assert!(tag_of("").is_err());
        assert!(tag_of("v").is_err());
    }

    #[test]
    fn 版本号是去掉_v_的_tag() {
        assert_eq!(release("v0.12.6", &[]).version(), "0.12.6");
        // 有些仓库的 tag 本来就不带 v。
        assert_eq!(release("0.12.6", &[]).version(), "0.12.6");
    }

    #[test]
    fn 按名字找附件() {
        let r = release("v1", &["a.tar.gz", "checksums.txt"]);
        assert!(r.asset("a.tar.gz").is_some());
        assert!(r.asset("b.tar.gz").is_none());
        assert_eq!(
            r.checksums_url(),
            Some("https://github.com/o/r/d/checksums.txt")
        );
        assert_eq!(release("v1", &["a"]).checksums_url(), None);
    }

    #[test]
    fn 校验和清单的几种命名都认() {
        for n in ["checksums.txt", "SHA256SUMS.txt", "sha256sums"] {
            assert!(release("v1", &[n]).checksums_url().is_some(), "没认出 {n}");
        }
        // 带平台后缀的那种不认：它只覆盖一部分附件，拿来校验别的会落空。
        assert_eq!(
            release("v1", &["checksums-linux.txt"]).checksums_url(),
            None
        );
    }

    #[test]
    fn 缺字段的_json_能解析() {
        // GitHub 对老 release 不一定回全部字段；少了 assets 不该让整个命令失败。
        let r: Release = serde_json::from_str(r#"{"tag_name":"v1.0.0"}"#).unwrap();
        assert_eq!(r.tag_name, "v1.0.0");
        assert!(r.assets.is_empty());
        assert!(!r.draft);
    }

    #[test]
    fn 附件直链按约定拼() {
        assert_eq!(
            asset_url("o/r", "v1.2.3", "x.tar.gz"),
            "https://github.com/o/r/releases/download/v1.2.3/x.tar.gz"
        );
    }

    #[test]
    fn 两个源的_api_基址() {
        let g = Source::GitHub;
        let e = Source::Gitee;
        assert_eq!(
            g.release_url("o/r", None),
            "https://api.github.com/repos/o/r/releases/latest"
        );
        assert_eq!(
            g.release_url("o/r", Some("v1.2.3")),
            "https://api.github.com/repos/o/r/releases/tags/v1.2.3"
        );
        assert_eq!(
            e.release_url("o/r", None),
            "https://gitee.com/api/v5/repos/o/r/releases/latest"
        );
        assert_eq!(
            e.release_url("o/r", Some("v1.2.3")),
            "https://gitee.com/api/v5/repos/o/r/releases/tags/v1.2.3"
        );
    }

    /// 整个「Gitee 能当备用源」的前提就是这一条：两家 release 的 JSON 字段同名，
    /// 所以上面那个 `Release` 不用改就能解析 Gitee 的响应。哪天 Gitee 改了字段，
    /// 表现会是回落之后反而解析失败——那比直接连不上更难查，所以钉在这儿。
    ///
    /// 这段是从 `gitee.com/api/v5/repos/farfarfun/fundeploy/releases/latest`
    /// 实际取回来的，只删掉了与此无关的字段。
    #[test]
    fn gitee_的_release_响应能用同一个结构体解析() {
        let body = r#"{
          "id": 1203682,
          "tag_name": "v0.3.1",
          "target_commitish": "master",
          "prerelease": false,
          "name": "v0.3.1",
          "body": "v0.3.1",
          "assets": [
            {"browser_download_url": "https://gitee.com/farfarfun/fundeploy/releases/download/v0.3.1/fundeploy-x86_64-unknown-linux-musl.tar.gz",
             "name": "fundeploy-x86_64-unknown-linux-musl.tar.gz"},
            {"browser_download_url": "https://gitee.com/farfarfun/fundeploy/releases/download/v0.3.1/SHA256SUMS",
             "name": "SHA256SUMS"},
            {"browser_download_url": "https://gitee.com/farfarfun/fundeploy/archive/refs/tags/v0.3.1.tar.gz",
             "name": "v0.3.1.tar.gz"}
          ]
        }"#;

        let r: Release = serde_json::from_str(body).expect("Gitee 的 release JSON 解析失败");
        assert_eq!(r.version(), "0.3.1");
        // Gitee 不回 draft 这个字段，靠 serde 的 default 兜住。
        assert!(!r.draft);

        let want = "fundeploy-x86_64-unknown-linux-musl.tar.gz";
        assert_eq!(
            r.asset(want).map(|a| a.browser_download_url.as_str()),
            Some("https://gitee.com/farfarfun/fundeploy/releases/download/v0.3.1/fundeploy-x86_64-unknown-linux-musl.tar.gz")
        );
        // 本项目的校验和清单叫 SHA256SUMS，不带扩展名，得认得出来。
        assert_eq!(
            r.checksums_url(),
            Some("https://gitee.com/farfarfun/fundeploy/releases/download/v0.3.1/SHA256SUMS")
        );
    }

    #[test]
    fn gitee_自动塞进来的源码包不会被当成平台附件() {
        // Gitee 会把 `<tag>.zip` / `<tag>.tar.gz` 两个源码归档也列进 assets，
        // GitHub 不会。按全名精确匹配就不受影响，这儿把这件事钉死。
        let r = Release {
            tag_name: "v0.3.1".into(),
            draft: false,
            prerelease: false,
            assets: ["v0.3.1.tar.gz", "v0.3.1.zip"]
                .iter()
                .map(|n| Asset {
                    name: (*n).into(),
                    browser_download_url: format!("https://gitee.com/o/r/archive/{n}"),
                })
                .collect(),
        };
        assert!(r
            .asset("fundeploy-x86_64-unknown-linux-musl.tar.gz")
            .is_none());
        assert_eq!(r.checksums_url(), None);
    }

    #[test]
    fn 源可以用环境变量点名() {
        let _锁 = 排队();
        for (v, want) in [
            ("github", Some(Source::GitHub)),
            ("gitee", Some(Source::Gitee)),
            ("  GiTee ", Some(Source::Gitee)),
            // 空值等于没设：`export FUNDEPLOY_RELEASE_SOURCE=` 这种清空写法
            // 该回到默认，而不是报错。
            ("", None),
            // 不认识的值只警告不拦——为一个拼错的环境变量让升级整个失败不值当。
            ("gitlab", None),
        ] {
            std::env::set_var("FUNDEPLOY_RELEASE_SOURCE", v);
            assert_eq!(Source::from_env(), want, "值是 {v:?}");
        }
        std::env::remove_var("FUNDEPLOY_RELEASE_SOURCE");
        assert_eq!(Source::from_env(), None);
    }

    /// 环境变量是进程级的，而 cargo test 多线程跑。
    fn 排队() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }
}
