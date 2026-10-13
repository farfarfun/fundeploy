//! GitHub Releases 查询。
//!
//! 不要求登录。设了 `GITHUB_TOKEN` / `GH_TOKEN` 就带上（见 `download::get_text`），
//! 匿名限流是每小时 60 次，所以每个动作最多问一次，不做轮询。

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use super::download;

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

fn api(path: &str) -> String {
    format!("https://api.github.com/repos/{path}")
}

/// `latest` 指向的那条 release。
pub fn latest(repo: &str) -> Result<Release> {
    let body = download::get_text(&api(&format!("{repo}/releases/latest")))?;
    serde_json::from_str(&body).with_context(|| format!("解析 {repo} 的 latest release 失败"))
}

/// 指定 tag 的 release。
pub fn by_tag(repo: &str, tag: &str) -> Result<Release> {
    // 这里最常见的失败是 404——指定的版本根本不存在。不加这句的话，报出来的
    // 只有一个 API URL 和 `status code 404`，看不出是「版本写错了」。
    let body = download::get_text(&api(&format!("{repo}/releases/tags/{tag}")))
        .with_context(|| format!("{repo} 上取不到 {tag}（多半是没有这个版本）"))?;
    serde_json::from_str(&body).with_context(|| format!("解析 {repo} 的 {tag} 失败"))
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
}
