//! HTTP 下载，以及受限网络下的 GitHub 镜像改写。
//!
//! 国内直连 GitHub 经常几 KB/s 甚至直接超时，所以允许用环境变量把下载重定向到
//! 加速前缀或 raw 镜像。改写只对 GitHub 族主机生效——别的域名原样放过，免得
//! 一个为加速设的变量把无关请求也绕到第三方去。

use anyhow::{bail, Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::path::Path;
use std::time::Duration;

use crate::ui;

/// 单次请求的超时。GitHub 偶发会把连接挂住不返回，没有超时就表现为命令卡死。
const TIMEOUT: Duration = Duration::from_secs(60);

/// 建连超时。
const CONNECT: Duration = Duration::from_secs(15);

/// 探测时的建连超时。
///
/// 「连不上就换个源」的第一下只是探路，等满 15 秒没有意义——结果都是换条路
/// 再来一次，那 15 秒纯粹是人盯着光标干等。
const PROBE_CONNECT: Duration = Duration::from_secs(8);

/// 改写策略。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rewrite {
    /// 直连。
    Off,
    /// 在原 URL 前拼一个加速前缀（ghproxy 之类）。
    HubProxy(String),
    /// 只把 raw.githubusercontent.com 换成另一个基址。
    MirrorRaw(String),
}

impl Rewrite {
    /// 从环境变量读策略。
    ///
    /// 前缀/基址必须是 https://。这两个变量能把本程序所有 GitHub 下载重定向到
    /// 任意主机，而「复制这行 ghproxy 就能提速」正是最容易从随手搜到的博客里
    /// 抄走的配置——至少不能允许它降级成明文 http。
    pub fn from_env() -> Self {
        let hub = env_https("FUNDEPLOY_GITHUB_HUB_PROXY_PREFIX");
        if let Some(p) = hub {
            return Rewrite::HubProxy(p);
        }
        let mode = std::env::var("FUNDEPLOY_GITHUB_DOWNLOAD_MODE").unwrap_or_default();
        if mode == "mirror_raw" {
            if let Some(b) = env_https("FUNDEPLOY_GITHUB_RAW_MIRROR_BASE") {
                return Rewrite::MirrorRaw(b);
            }
        }
        if mode == "hub_proxy" {
            ui::warn(
                "FUNDEPLOY_GITHUB_DOWNLOAD_MODE=hub_proxy 但没设 \
                 FUNDEPLOY_GITHUB_HUB_PROXY_PREFIX，这次不改写。",
            );
        }
        Rewrite::Off
    }

    /// 应用到一个 URL 上。不该改的原样返回。
    pub fn apply(&self, url: &str) -> String {
        let Some(host) = https_host(url) else {
            return url.to_string();
        };
        if !is_github_host(&host) {
            return url.to_string();
        }
        match self {
            Rewrite::Off => url.to_string(),
            // 已经带过前缀就不再套一层，否则反复执行会越拼越长。
            Rewrite::HubProxy(p) if url.starts_with(p.as_str()) => url.to_string(),
            Rewrite::HubProxy(p) => format!("{p}{url}"),
            Rewrite::MirrorRaw(_) if host != "raw.githubusercontent.com" => url.to_string(),
            Rewrite::MirrorRaw(base) => {
                let path = https_path(url).unwrap_or_else(|| "/".into());
                format!("{}{path}", base.trim_end_matches('/'))
            }
        }
    }
}

fn env_https(name: &str) -> Option<String> {
    let v = std::env::var(name).ok()?;
    let v = v.trim();
    if v.is_empty() {
        return None;
    }
    if !v.starts_with("https://") {
        ui::warn(format!("{name} 不是 https:// 开头（{v}），忽略。"));
        return None;
    }
    Some(v.to_string())
}

fn is_github_host(host: &str) -> bool {
    matches!(
        host,
        "github.com" | "www.github.com" | "raw.githubusercontent.com" | "api.github.com"
    )
}

/// https URL 的主机名，小写。不是 https 就返回 None。
fn https_host(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let host = rest.split('/').next()?;
    if host.is_empty() {
        return None;
    }
    Some(host.to_ascii_lowercase())
}

/// https URL 里主机名之后的部分，以 / 开头。
fn https_path(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    match rest.find('/') {
        Some(i) => Some(rest[i..].to_string()),
        None => Some("/".into()),
    }
}

/// 直连 GitHub 时提示一次可以配镜像。
pub fn hint_mirror() {
    if std::env::var("FUNDEPLOY_GITHUB_DOWNLOAD_HINT").unwrap_or_default() == "0" {
        return;
    }
    if Rewrite::from_env() != Rewrite::Off {
        return;
    }
    ui::warn(
        "当前直连 GitHub。网络受限时可设 FUNDEPLOY_GITHUB_HUB_PROXY_PREFIX，\
         或 FUNDEPLOY_GITHUB_DOWNLOAD_MODE=mirror_raw 搭配 FUNDEPLOY_GITHUB_RAW_MIRROR_BASE。\
         设 FUNDEPLOY_GITHUB_DOWNLOAD_HINT=0 隐藏本行。",
    );
}

fn agent_connecting(connect: Duration) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(connect)
        .timeout(TIMEOUT)
        .build()
}

fn agent() -> ureq::Agent {
    agent_connecting(CONNECT)
}

/// 取一段文本（API 响应）。
pub fn get_text(url: &str) -> Result<String> {
    get_text_connecting(url, CONNECT)
}

/// 和 [`get_text`] 一样，但建连超时短一些。
///
/// 用在「这一个源连不上就换下一个」的探测上。
pub fn get_text_probe(url: &str) -> Result<String> {
    get_text_connecting(url, PROBE_CONNECT)
}

fn get_text_connecting(url: &str, connect: Duration) -> Result<String> {
    let url = Rewrite::from_env().apply(url);
    let mut req = agent_connecting(connect)
        .get(&url)
        .set("User-Agent", "fundeploy")
        .set("Accept", "application/vnd.github+json");
    if let Some(t) = github_token(&url) {
        req = req.set("Authorization", &format!("Bearer {t}"));
    }
    let resp = req.call().with_context(|| format!("请求失败: {url}"))?;
    resp.into_string()
        .with_context(|| format!("读取响应失败: {url}"))
}

/// 这次请求该带的 GitHub token，没有就匿名发。
///
/// 匿名调 API 是每小时 60 次，且按出口 IP 算——公司网、校园网、CI runner 都是
/// 一堆机器共用一个出口，装几个服务就撞上限，然后表现成「查不到最新版本」。
/// 带上 token 是每小时 5000 次。
///
/// 读 `GITHUB_TOKEN` / `GH_TOKEN` 而不是自己起个名字：gh CLI 和 GitHub Actions
/// 已经在设这两个，配过一次的机器不用再为我们配第二遍。
///
/// 只认 api.github.com，而且判的是**改写之后**的地址：镜像前缀会把 URL 变成
/// `https://某代理/https://api.github.com/…`，照旧附 Authorization 等于把用户的
/// 凭据递给那个第三方代理。
fn github_token(url: &str) -> Option<String> {
    if https_host(url).as_deref() != Some("api.github.com") {
        return None;
    }
    ["GITHUB_TOKEN", "GH_TOKEN"]
        .into_iter()
        .filter_map(|n| std::env::var(n).ok())
        .map(|v| v.trim().to_string())
        .find(|v| !v.is_empty())
}

/// 下载到文件，带进度条。
///
/// 先写临时文件再 rename：下载中断不会留下一个长度正确、内容残缺的目标文件，
/// 那种残骸下一次执行会被当成「已经下好了」。
pub fn to_file(url: &str, dest: &Path, label: &str) -> Result<()> {
    let url = Rewrite::from_env().apply(url);
    let resp = agent()
        .get(&url)
        .set("User-Agent", "fundeploy")
        .call()
        .with_context(|| format!("下载失败: {url}"))?;

    let total: u64 = resp
        .header("Content-Length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("建目录失败: {}", parent.display()))?;
    }
    let tmp = dest.with_extension("part");
    let mut out = File::create(&tmp).with_context(|| format!("写入失败: {}", tmp.display()))?;

    let bar = progress(total, label);
    let mut reader = resp.into_reader();
    let mut buf = vec![0u8; 64 * 1024];
    let mut done: u64 = 0;
    loop {
        let n = reader.read(&mut buf).context("读取响应流失败")?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])
            .with_context(|| format!("写入失败: {}", tmp.display()))?;
        done += n as u64;
        bar.set_position(done);
    }
    out.flush().ok();
    drop(out);
    bar.finish_and_clear();

    std::fs::rename(&tmp, dest)
        .with_context(|| format!("重命名失败: {} -> {}", tmp.display(), dest.display()))?;
    Ok(())
}

fn progress(total: u64, label: &str) -> ProgressBar {
    if !std::io::IsTerminal::is_terminal(&std::io::stderr()) {
        return ProgressBar::hidden();
    }
    let bar = if total > 0 {
        let b = ProgressBar::new(total);
        b.set_style(
            ProgressStyle::with_template(
                "{msg} [{bar:30.cyan/blue}] {bytes}/{total_bytes} {bytes_per_sec}",
            )
            .unwrap()
            .progress_chars("=>-"),
        );
        b
    } else {
        // 没有 Content-Length（分块传输）时进度条没有分母，只能报已下载量。
        let b = ProgressBar::new_spinner();
        b.set_style(
            ProgressStyle::with_template("{msg} {spinner} {bytes} {bytes_per_sec}").unwrap(),
        );
        b
    };
    bar.set_message(label.to_string());
    bar
}

/// 文件的 sha256，小写十六进制。
pub fn sha256(path: &Path) -> Result<String> {
    let f = File::open(path).with_context(|| format!("打开失败: {}", path.display()))?;
    let mut reader = BufReader::new(f);
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf).context("读取失败")?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// 从 `checksums.txt` 里找某个文件的期望 sha256。
///
/// 格式是 `<sha256>  <filename>`，文件名前可能有个 `*`（二进制模式标记）。
pub fn expected_sha256(checksums: &str, filename: &str) -> Option<String> {
    for line in checksums.lines() {
        let mut parts = line.split_whitespace();
        let (Some(sum), Some(name)) = (parts.next(), parts.next()) else {
            continue;
        };
        if name.trim_start_matches('*') == filename {
            return Some(sum.to_ascii_lowercase());
        }
    }
    None
}

/// 校验下载下来的文件。
///
/// 查不到校验和时只警告不拦——上游不是每个 release 都带 checksums.txt，硬拦会让
/// 安装在完全正常的版本上失败。但对得上却不匹配必须当硬错误。
pub fn verify(path: &Path, checksums: Option<&str>, filename: &str) -> Result<()> {
    let Some(sums) = checksums else {
        ui::warn("未取到 checksums.txt，跳过校验。");
        return Ok(());
    };
    let Some(want) = expected_sha256(sums, filename) else {
        ui::warn(format!(
            "checksums.txt 里没有 {filename} 的条目，跳过校验。"
        ));
        return Ok(());
    };
    let got = sha256(path)?;
    if got != want {
        bail!("{filename} 校验失败\n  期望 {want}\n  实际 {got}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 非_github_主机不改写() {
        let r = Rewrite::HubProxy("https://ghproxy.test/".into());
        assert_eq!(r.apply("https://example.com/a"), "https://example.com/a");
        // http 的也不碰：改写只认 https，顺带避免把明文请求搬到别处。
        assert_eq!(r.apply("http://github.com/a"), "http://github.com/a");
    }

    #[test]
    fn hub_前缀拼在原_url_前面() {
        let r = Rewrite::HubProxy("https://ghproxy.test/".into());
        assert_eq!(
            r.apply("https://github.com/o/r/releases/download/v1/x"),
            "https://ghproxy.test/https://github.com/o/r/releases/download/v1/x"
        );
    }

    #[test]
    fn 已经带了前缀就不再套一层() {
        // 不防这一下的话，同一个 URL 过两遍改写会变成 proxy/proxy/github…
        let r = Rewrite::HubProxy("https://ghproxy.test/".into());
        let once = r.apply("https://github.com/o/r");
        assert_eq!(r.apply(&once), once);
    }

    #[test]
    fn mirror_raw_只换_raw_主机() {
        let r = Rewrite::MirrorRaw("https://raw.test/gh".into());
        assert_eq!(
            r.apply("https://raw.githubusercontent.com/o/r/main/f.txt"),
            "https://raw.test/gh/o/r/main/f.txt"
        );
        // release 附件不在 raw 上，mirror_raw 模式下应原样直连。
        assert_eq!(
            r.apply("https://github.com/o/r/releases/download/v1/x"),
            "https://github.com/o/r/releases/download/v1/x"
        );
    }

    #[test]
    fn mirror_raw_的基址末尾斜杠不会造成双斜杠() {
        let r = Rewrite::MirrorRaw("https://raw.test/gh/".into());
        assert_eq!(
            r.apply("https://raw.githubusercontent.com/o/r/main/f"),
            "https://raw.test/gh/o/r/main/f"
        );
    }

    #[test]
    fn 主机名大小写不敏感() {
        let r = Rewrite::HubProxy("https://p/".into());
        assert_eq!(
            r.apply("https://GitHub.com/a"),
            "https://p/https://GitHub.com/a"
        );
    }

    #[test]
    fn off_策略原样返回() {
        assert_eq!(
            Rewrite::Off.apply("https://github.com/a"),
            "https://github.com/a"
        );
    }

    #[test]
    fn checksums_解析() {
        let s = "aa11  foo.tar.gz\nbb22 *bar.tar.gz\n";
        assert_eq!(expected_sha256(s, "foo.tar.gz"), Some("aa11".into()));
        // goreleaser 的二进制模式会在文件名前加 *。
        assert_eq!(expected_sha256(s, "bar.tar.gz"), Some("bb22".into()));
        assert_eq!(expected_sha256(s, "none"), None);
    }

    #[test]
    fn checksums_里的空行和残行不会_panic() {
        let s = "\n   \nonlyonecolumn\naa11  ok\n";
        assert_eq!(expected_sha256(s, "ok"), Some("aa11".into()));
    }

    #[test]
    fn 校验和对不上要报错() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("x.bin");
        std::fs::write(&f, b"hello").unwrap();
        let real = sha256(&f).unwrap();
        assert_eq!(
            real,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );

        verify(&f, Some(&format!("{real}  x.bin")), "x.bin").unwrap();
        let err = verify(&f, Some("dead  x.bin"), "x.bin")
            .unwrap_err()
            .to_string();
        assert!(err.contains("校验失败"), "实际: {err}");
    }

    #[test]
    fn 查不到校验和时放行() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("x.bin");
        std::fs::write(&f, b"hello").unwrap();
        verify(&f, None, "x.bin").unwrap();
        verify(&f, Some("aa  别的文件"), "x.bin").unwrap();
    }

    /// 临时改几个环境变量，出作用域还原。
    ///
    /// 环境变量是进程级的而 cargo test 多线程跑，所以这里既要排队也要还原：
    /// CI 和装过 gh 的机器上 GITHUB_TOKEN 本来就可能有值，不还原会殃及同进程里
    /// 别的测试。
    struct 临时环境 {
        旧值: Vec<(&'static str, Option<String>)>,
        _锁: std::sync::MutexGuard<'static, ()>,
    }

    impl 临时环境 {
        fn 设(pairs: &[(&'static str, Option<&str>)]) -> Self {
            static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
            // panic 会毒化这把锁，但它保护的只是「别同时跑」，没有要保持一致的
            // 共享状态，直接接着用。
            let _锁 = LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let 旧值 = pairs
                .iter()
                .map(|(k, v)| {
                    let prev = std::env::var(k).ok();
                    match v {
                        Some(v) => std::env::set_var(k, v),
                        None => std::env::remove_var(k),
                    }
                    (*k, prev)
                })
                .collect();
            Self { 旧值, _锁 }
        }
    }

    impl Drop for 临时环境 {
        fn drop(&mut self) {
            for (k, v) in &self.旧值 {
                match v {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
        }
    }

    #[test]
    fn 有_token_就带上没有就匿名() {
        let _e = 临时环境::设(&[("GITHUB_TOKEN", Some("t1")), ("GH_TOKEN", None)]);
        assert_eq!(
            github_token("https://api.github.com/repos/o/r/releases/latest"),
            Some("t1".into())
        );

        std::env::remove_var("GITHUB_TOKEN");
        assert_eq!(github_token("https://api.github.com/x"), None);
    }

    #[test]
    fn gh_token_作为备选() {
        // 值为空白等于没设，要继续往下找——否则 `export GITHUB_TOKEN=` 这种
        // 清空写法会把真正有值的 GH_TOKEN 挡住。
        let _e = 临时环境::设(&[("GITHUB_TOKEN", Some("  ")), ("GH_TOKEN", Some("t2"))]);
        assert_eq!(github_token("https://api.github.com/x"), Some("t2".into()));
    }

    #[test]
    fn token_不会跟着请求跑到镜像代理上去() {
        let _e = 临时环境::设(&[("GITHUB_TOKEN", Some("secret")), ("GH_TOKEN", None)]);
        let 改写后 = Rewrite::HubProxy("https://ghproxy.test/".into())
            .apply("https://api.github.com/repos/o/r/releases/latest");
        assert_eq!(github_token(&改写后), None, "凭据不能递给第三方代理");
        // 其它主机同理。
        assert_eq!(github_token("https://example.com/api.github.com/x"), None);
        assert_eq!(github_token("https://api.github.com.evil.test/x"), None);
    }
}
