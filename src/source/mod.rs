//! 换源：把包管理器指到国内镜像。
//!
//! 国内直连 pypi.org / registry.npmjs.org / GitHub 的体感是几十 KB/s 到完全超时，
//! 装一套开发环境能卡一下午。各家镜像站把这些仓库同步到了国内，换过去就是分钟级
//! 和小时级的差别。
//!
//! 五个包管理器的换源做法完全不同——pip 改 INI、npm 改 `.npmrc`、cargo 改 TOML、
//! go 改它自己的 env 文件、brew 改环境变量加 git remote。但要做的**事**是同一串：
//! 列出候选、量一遍延迟、挑一个、写进配置、能查、能撤。所以这里只有一个 trait：
//! 各家实现「这个镜像的地址是什么」「配置写在哪、怎么写」这四五个方法，
//! 列表、测速、排序、选最快、确认、回显，全在默认方法里写一遍。
//!
//! # 为什么不是一条 `curl | sh`
//!
//! 社区里这类脚本（HomebrewCN 是其中流传最广的一个）通常直接追加 export 到 shell
//! 配置，再靠关键字 sed 删除旧的。换三次源就留三份互相覆盖的 export，而按关键字
//! 删行会误伤任何恰好提到那个词的行。这里一律走 [`crate::common::block`] 的标记
//! 区块：改是替换，删只删自己那段。

pub mod brew;
pub mod cargo;
pub mod go;
pub mod npm;
pub mod pip;
pub mod probe;
pub mod registry;
pub mod store;

use std::time::Duration;

use anyhow::{bail, Result};

use crate::common::env;
use crate::ui;

/// 一个候选镜像。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mirror {
    /// 短名，命令行上用它点名。
    pub name: String,
    /// 给人看的来源，比如「清华大学」。
    pub label: String,
    /// 主地址。大多数包管理器要写进配置的就是它，测速也量它。
    pub url: String,
    /// 少数源才用得上的附加地址（brew 的 git remote、api domain）。
    pub extras: Vec<(String, String)>,
    /// 用户自己加的。只有它能被 `rm` 掉。
    pub custom: bool,
    /// 官方源。`reset` 回到的就是它，列表里也不该把它当成「加速」。
    pub official: bool,
}

impl Mirror {
    pub fn new(name: &str, label: &str, url: &str) -> Self {
        Self {
            name: name.to_string(),
            label: label.to_string(),
            url: url.to_string(),
            extras: Vec::new(),
            custom: false,
            official: false,
        }
    }

    pub fn official(mut self) -> Self {
        self.official = true;
        self
    }

    pub fn with(mut self, key: &str, value: &str) -> Self {
        self.extras.push((key.to_string(), value.to_string()));
        self
    }

    pub fn extra(&self, key: &str) -> Option<&str> {
        self.extras
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

/// 一类能换源的东西。
pub trait Source {
    fn name(&self) -> &str;
    fn summary(&self) -> &str;

    /// 内置镜像表。
    fn builtin(&self) -> Vec<Mirror>;

    /// 配置文件在哪。给 status 显示用。
    fn config_path(&self) -> std::path::PathBuf;

    /// 当前生效的源地址。读真实配置，不记在自己这边。
    ///
    /// 不另存一份「我设成了什么」：人会绕过 fundeploy 直接改配置文件，也会用
    /// `npm config set` 这类原生命令改。记一份副本的结果是它和真实配置说的不是
    /// 一回事，而人信的是我们打印的那个。
    fn current(&self) -> Option<String>;

    /// 把源写进配置。
    fn apply(&self, m: &Mirror) -> Result<()>;

    /// 撤回，恢复官方源。
    fn reset(&self) -> Result<()>;

    /// 测速用的地址。默认量主地址本身。
    ///
    /// 要挑一个**小而必定存在**的路径：量首字节时间不必下整个索引，但服务端
    /// 得真的回一个 200，否则量到的是「404 回得有多快」。
    fn probe_url(&self, m: &Mirror) -> String {
        m.url.clone()
    }

    /// 换完源之后额外说一句。比如要不要重开 shell。
    fn after_apply(&self, _m: &Mirror) {}

    // ===== 以下是写一遍就不用再写的部分 =====

    /// 内置的加上用户自己加的。
    fn mirrors(&self) -> Vec<Mirror> {
        let mut all = self.builtin();
        for m in store::custom(self.name()) {
            // 同名时用户加的盖掉内置的：镜像站改了路径，不该逼人等我们发版。
            match all.iter().position(|x| x.name == m.name) {
                Some(i) => all[i] = m,
                None => all.push(m),
            }
        }
        all
    }

    /// 按名字找一个镜像。给的像个 URL 就当成一次性的匿名源收下。
    fn lookup(&self, key: &str) -> Result<Mirror> {
        let key = key.trim();
        if let Some(m) = self.mirrors().into_iter().find(|m| m.name == key) {
            return Ok(m);
        }
        if key.starts_with("http://") || key.starts_with("https://") {
            return Ok(Mirror::new("自定义", "命令行直接给的地址", key));
        }
        let names: Vec<String> = self.mirrors().into_iter().map(|m| m.name).collect();
        bail!(
            "{} 没有叫 {key} 的源。可选: {}\n也可以直接给一个 http(s) 地址。",
            self.name(),
            names.join("、")
        )
    }

    fn list(&self) -> Result<()> {
        ui::heading(format!("{} 可用的源", self.name()));
        let current = self.current();
        for m in self.mirrors() {
            let mark = if current.as_deref() == Some(m.url.as_str()) {
                "*"
            } else {
                " "
            };
            let tag = match (m.custom, m.official) {
                (true, _) => " [自定义]",
                (_, true) => " [官方]",
                _ => "",
            };
            println!(
                "{mark} {} {} {}{tag}",
                ui::pad(&m.name, 12),
                ui::pad(&m.label, 16),
                m.url
            );
        }
        Ok(())
    }

    /// 全部量一遍延迟，快的在前。不可达的排在最后。
    fn bench(&self) -> Result<Vec<probe::Timing>> {
        let mirrors = self.mirrors();
        if mirrors.is_empty() {
            bail!("{} 没有任何候选源", self.name());
        }
        let urls: Vec<String> = mirrors.iter().map(|m| self.probe_url(m)).collect();
        ui::step(format!("测 {} 个源的延迟…", mirrors.len()));
        let results = probe::race(&urls, probe_timeout(), probe_jobs());

        let mut out: Vec<probe::Timing> = mirrors
            .into_iter()
            .zip(results)
            .map(|(mirror, outcome)| probe::Timing { mirror, outcome })
            .collect();
        // 可达的按延迟升序；不可达的保持原顺序排在后面。
        out.sort_by_key(|t| t.outcome.millis().unwrap_or(u128::MAX));
        Ok(out)
    }

    /// 测速并把结果打出来。
    fn test(&self) -> Result<()> {
        let ranked = self.bench()?;
        let current = self.current();
        ui::blank();
        for t in &ranked {
            let mark = if current.as_deref() == Some(t.mirror.url.as_str()) {
                "*"
            } else {
                " "
            };
            println!(
                "{mark} {} {} {}",
                ui::pad(&t.mirror.name, 12),
                ui::pad(&t.outcome.describe(), 10),
                t.mirror.url
            );
        }
        Ok(())
    }

    /// 换源。`key` 为 None 时测速选最快的那个。
    fn switch(&self, key: Option<&str>) -> Result<()> {
        let m = match key {
            Some(k) => self.lookup(k)?,
            None => {
                let ranked = self.bench()?;
                ui::blank();
                for t in ranked.iter().take(5) {
                    println!("  {} {}", ui::pad(&t.mirror.name, 12), t.outcome.describe());
                }
                ui::blank();
                let best = ranked
                    .into_iter()
                    .find(|t| t.outcome.millis().is_some())
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "{} 的候选源一个都连不上。先确认网络本身是通的。",
                            self.name()
                        )
                    })?;
                best.mirror
            }
        };

        if m.url.starts_with("http://") {
            // 明文 HTTP 下载的包，中间任何一跳都能换掉内容。pip 还得额外配
            // trusted-host 才肯用，等于把它自己的那道拦截也关了。
            ui::warn(format!("{} 是明文 HTTP，传输中可被篡改。", m.url));
        }

        ui::step(format!("{} -> {} ({})", self.name(), m.name, m.url));
        self.apply(&m)?;
        ui::ok(format!("已写入 {}", self.config_path().display()));
        self.after_apply(&m);
        Ok(())
    }

    fn status(&self) -> Result<()> {
        ui::heading(self.name());
        ui::kv("配置文件", Some(&self.config_path().display().to_string()));
        match self.current() {
            Some(url) => {
                let known = self.mirrors().into_iter().find(|m| m.url == url);
                let who = known
                    .map(|m| format!("{url}  （{} {}）", m.name, m.label))
                    .unwrap_or(url);
                ui::kv("当前源", Some(&who));
            }
            None => {
                ui::kv("当前源", Some("未设置（走该工具自己的默认值）"));
            }
        }
        Ok(())
    }
}

/// 单次探测的超时。
fn probe_timeout() -> Duration {
    Duration::from_secs(env::num("FUNDEPLOY_SOURCE_PROBE_TIMEOUT", 5))
}

/// 同时探测几个。
fn probe_jobs() -> usize {
    env::num::<usize>("FUNDEPLOY_SOURCE_PROBE_JOBS", 8).clamp(1, 32)
}

/// 测试里改环境变量前先排队。
///
/// 环境变量是整个进程共用的，而 `cargo test` 默认并行跑。各家的用例都靠
/// `FUNDEPLOY_*_CONFIG` 把配置文件指到临时目录，不排队就会互相指走——而且是
/// 偶发失败。整个 `source` 域共用这一把锁，不是每个模块各一把：pip 换源会
/// 顺带改 brew 的区块，分开锁就挡不住这种跨模块的用例。
#[cfg(test)]
pub(crate) fn 排队() -> std::sync::MutexGuard<'static, ()> {
    static M: std::sync::Mutex<()> = std::sync::Mutex::new(());
    M.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct 假源;

    impl Source for 假源 {
        fn name(&self) -> &str {
            "假源"
        }
        fn summary(&self) -> &str {
            "测试用"
        }
        fn builtin(&self) -> Vec<Mirror> {
            vec![
                Mirror::new("a", "甲", "https://a.example/"),
                Mirror::new("b", "乙", "https://b.example/").with("git", "https://b.example/x.git"),
                Mirror::new("official", "官方", "https://o.example/").official(),
            ]
        }
        fn config_path(&self) -> std::path::PathBuf {
            "/dev/null".into()
        }
        fn current(&self) -> Option<String> {
            None
        }
        fn apply(&self, _m: &Mirror) -> Result<()> {
            Ok(())
        }
        fn reset(&self) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn 按名字找得到() {
        assert_eq!(假源.lookup("b").unwrap().label, "乙");
        assert_eq!(假源.lookup("  b  ").unwrap().label, "乙");
    }

    #[test]
    fn 直接给地址也收() {
        let m = 假源.lookup("https://自己的镜像/simple/").unwrap();
        assert_eq!(m.url, "https://自己的镜像/simple/");
        assert!(!m.custom);
    }

    #[test]
    fn 名字不认识时报错里列出可选项() {
        let err = 假源.lookup("没这个").unwrap_err().to_string();
        assert!(err.contains('a'), "实际: {err}");
        assert!(err.contains("official"), "实际: {err}");
        // 不像 URL 的东西不能被静默当成地址收下——那样换完源是个连不上的地方。
        assert!(假源.lookup("tuna.tsinghua.edu.cn").is_err());
    }

    #[test]
    fn 附加地址按键取() {
        let m = 假源.lookup("b").unwrap();
        assert_eq!(m.extra("git"), Some("https://b.example/x.git"));
        assert_eq!(m.extra("没这个"), None);
        assert_eq!(假源.lookup("a").unwrap().extra("git"), None);
    }

    #[test]
    fn 每个源都标出了官方那一个() {
        // reset 要回到官方源，表里没有标出来的话就无处可回。
        for s in registry::all() {
            let n = s.builtin().iter().filter(|m| m.official).count();
            assert_eq!(n, 1, "{} 的官方源有 {n} 个，应当恰好一个", s.name());
        }
    }

    #[test]
    fn 内置源的名字和地址都不重样() {
        for s in registry::all() {
            let all = s.builtin();
            for (field, mut v) in [
                (
                    "名字",
                    all.iter().map(|m| m.name.clone()).collect::<Vec<_>>(),
                ),
                (
                    "地址",
                    all.iter().map(|m| m.url.clone()).collect::<Vec<_>>(),
                ),
            ] {
                let total = v.len();
                v.sort();
                v.dedup();
                assert_eq!(v.len(), total, "{} 的内置源有重复的{field}", s.name());
            }
        }
    }

    #[test]
    fn 内置源的地址都是绝对_https_地址() {
        for s in registry::all() {
            for m in s.builtin() {
                // 两种值不是纯 URL：go 的 GOPROXY 形如 `https://x,direct`，
                // cargo 的索引带 `sparse+` 协议前缀。剥到真地址再看。
                let first = m
                    .url
                    .trim_start_matches("sparse+")
                    .split(',')
                    .next()
                    .unwrap();
                assert!(
                    first.starts_with("https://"),
                    "{} 的 {} 不是 https: {}",
                    s.name(),
                    m.name,
                    m.url
                );
            }
        }
    }

    #[test]
    fn 探测参数能被环境变量调且有上限() {
        std::env::set_var("FUNDEPLOY_SOURCE_PROBE_JOBS", "1000");
        assert_eq!(probe_jobs(), 32);
        std::env::set_var("FUNDEPLOY_SOURCE_PROBE_JOBS", "0");
        assert_eq!(probe_jobs(), 1);
        std::env::remove_var("FUNDEPLOY_SOURCE_PROBE_JOBS");
    }
}
