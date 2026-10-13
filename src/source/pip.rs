//! PyPI 换源。
//!
//! 直接写 pip 的用户级配置文件，不调 `python -m pip config set`：那条路要求机器上
//! 先有一个能跑的 python，而「换源」恰恰常常是装 python 之前就要做的事——uv、
//! poetry、pdm 读的也是同一个文件，它们都不需要 pip 存在。

use std::path::PathBuf;

use anyhow::Result;

use super::{Mirror, Source};
use crate::common::env;
use crate::common::ini::Ini;
use crate::common::platform::{Os, Platform};
use crate::ui;

const SECTION: &str = "global";

pub struct Pip;

/// 从索引地址里取出主机名，给 `trusted-host` 用。
fn host_of(url: &str) -> Option<&str> {
    let rest = url.split_once("://")?.1;
    let rest = rest.rsplit_once('@').map(|(_, h)| h).unwrap_or(rest);
    let host = rest.split('/').next()?;
    // trusted-host 要的是主机名，带端口也认，但不能带路径。
    (!host.is_empty()).then_some(host)
}

impl Pip {
    fn file(&self) -> PathBuf {
        if let Some(p) = env::opt("FUNDEPLOY_PIP_CONFIG") {
            return env::expand_tilde(&p);
        }
        // macOS 上 pip 的用户级配置在 Application Support 下，不是 ~/.config。
        // 写错地方的症状是「配好了，pip 还是走官方源」。
        if matches!(Platform::current(), Ok(p) if p.os == Os::MacOs) {
            return env::home().join("Library/Application Support/pip/pip.conf");
        }
        dirs::config_dir()
            .unwrap_or_else(|| env::home().join(".config"))
            .join("pip")
            .join("pip.conf")
    }
}

impl Source for Pip {
    fn name(&self) -> &str {
        "pip"
    }

    fn summary(&self) -> &str {
        "PyPI（pip / uv / poetry / pdm 都读这份配置）"
    }

    fn config_path(&self) -> PathBuf {
        self.file()
    }

    fn builtin(&self) -> Vec<Mirror> {
        vec![
            Mirror::new(
                "aliyun",
                "阿里云",
                "https://mirrors.aliyun.com/pypi/simple/",
            ),
            Mirror::new(
                "tuna",
                "清华大学",
                "https://pypi.tuna.tsinghua.edu.cn/simple/",
            ),
            Mirror::new(
                "ustc",
                "中国科大",
                "https://pypi.mirrors.ustc.edu.cn/simple/",
            ),
            Mirror::new(
                "tencent",
                "腾讯云",
                "https://mirrors.cloud.tencent.com/pypi/simple/",
            ),
            Mirror::new(
                "huawei",
                "华为云",
                "https://mirrors.huaweicloud.com/repository/pypi/simple/",
            ),
            Mirror::new(
                "nju",
                "南京大学",
                "https://mirrors.nju.edu.cn/pypi/web/simple/",
            ),
            Mirror::new(
                "bfsu",
                "北京外国语",
                "https://mirrors.bfsu.edu.cn/pypi/web/simple/",
            ),
            Mirror::new(
                "sjtu",
                "上海交大",
                "https://mirror.sjtu.edu.cn/pypi/web/simple/",
            ),
            Mirror::new("official", "官方", "https://pypi.org/simple/").official(),
        ]
    }

    /// 量一个确定存在的小包的索引页，不量索引根。
    ///
    /// 索引根在不少镜像上是整张包列表（几十 MB 的 HTML），而且有的站对它返回
    /// 403。`setuptools` 是 PEP 503 索引里必定有的一项。
    fn probe_url(&self, m: &Mirror) -> String {
        format!("{}/setuptools/", m.url.trim_end_matches('/'))
    }

    fn current(&self) -> Option<String> {
        Ini::load(&self.file())
            .ok()?
            .get(Some(SECTION), "index-url")
    }

    fn apply(&self, m: &Mirror) -> Result<()> {
        let path = self.file();
        let mut ini = Ini::load(&path)?;
        ini.set(Some(SECTION), "index-url", &m.url);

        // 明文 HTTP 的源 pip 默认拒绝使用，必须同时把主机列进 trusted-host。
        // 这是在关掉 pip 自己的一道拦截，所以只对当前这个源开，不留着。
        match (m.url.starts_with("http://"), host_of(&m.url)) {
            (true, Some(h)) => ini.set(Some(SECTION), "trusted-host", h),
            _ => ini.unset(Some(SECTION), "trusted-host"),
        }
        ini.save(&path)
    }

    fn reset(&self) -> Result<()> {
        let path = self.file();
        let mut ini = Ini::load(&path)?;
        ini.unset(Some(SECTION), "index-url");
        ini.unset(Some(SECTION), "extra-index-url");
        ini.unset(Some(SECTION), "trusted-host");
        if ini.is_trivial() && path.exists() {
            // 只剩一个空的 [global]，留着没有意义，还会让人以为这儿配了东西。
            std::fs::remove_file(&path)?;
            ui::ok(format!("已删除 {}", path.display()));
            super::brew::sync_pip_index();
            return Ok(());
        }
        ini.save(&path)?;
        super::brew::sync_pip_index();
        Ok(())
    }

    fn after_apply(&self, _m: &Mirror) {
        ui::hint("uv / poetry / pdm 读的是同一份配置，不用再配一遍。");
        // brew 装 Python 类配方时读的是另一个变量，见 [`super::brew`]。
        super::brew::sync_pip_index();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::排队;

    fn 临时配置() -> (tempfile::TempDir, std::sync::MutexGuard<'static, ()>) {
        let g = 排队();
        let d = tempfile::tempdir().unwrap();
        std::env::set_var("FUNDEPLOY_PIP_CONFIG", d.path().join("pip.conf"));
        (d, g)
    }

    #[test]
    fn 取主机名() {
        assert_eq!(host_of("https://a.b/simple/"), Some("a.b"));
        assert_eq!(host_of("http://a.b:8080/simple/"), Some("a.b:8080"));
        // 带凭据的内网源，@ 前面的部分不能混进 trusted-host。
        assert_eq!(host_of("http://u:p@a.b/simple/"), Some("a.b"));
        assert_eq!(host_of("不是地址"), None);
    }

    #[test]
    fn 换源写进_global_节() {
        let (_d, _g) = 临时配置();
        let p = Pip;
        p.apply(&p.lookup("aliyun").unwrap()).unwrap();
        assert_eq!(
            p.current().as_deref(),
            Some("https://mirrors.aliyun.com/pypi/simple/")
        );
        let text = std::fs::read_to_string(p.config_path()).unwrap();
        assert!(text.contains("[global]"), "实际:\n{text}");
    }

    #[test]
    fn 换源不动文件里别的设置() {
        let (_d, _g) = 临时配置();
        let p = Pip;
        std::fs::write(
            p.config_path(),
            "[global]\ntimeout = 120\n\n[install]\nuser = true\n",
        )
        .unwrap();
        p.apply(&p.lookup("tuna").unwrap()).unwrap();
        let text = std::fs::read_to_string(p.config_path()).unwrap();
        assert!(text.contains("timeout = 120"), "实际:\n{text}");
        assert!(text.contains("user = true"), "实际:\n{text}");
    }

    #[test]
    fn 明文源才配_trusted_host_并且换回去要清掉() {
        let (_d, _g) = 临时配置();
        let p = Pip;
        p.apply(&Mirror::new("土", "内网", "http://a.b/simple/"))
            .unwrap();
        let ini = Ini::load(&p.config_path()).unwrap();
        assert_eq!(
            ini.get(Some("global"), "trusted-host").as_deref(),
            Some("a.b")
        );

        // 换到 https 源之后那行必须消失：留着等于对那台主机永久关掉校验。
        p.apply(&p.lookup("tuna").unwrap()).unwrap();
        let ini = Ini::load(&p.config_path()).unwrap();
        assert_eq!(ini.get(Some("global"), "trusted-host"), None);
    }

    #[test]
    fn reset_之后文件里不再有索引设置() {
        let (_d, _g) = 临时配置();
        let p = Pip;
        p.apply(&p.lookup("aliyun").unwrap()).unwrap();
        p.reset().unwrap();
        assert_eq!(p.current(), None);
    }

    #[test]
    fn reset_保留别人的设置而不是删文件() {
        let (_d, _g) = 临时配置();
        let p = Pip;
        std::fs::write(p.config_path(), "[global]\ntimeout = 120\n").unwrap();
        p.apply(&p.lookup("aliyun").unwrap()).unwrap();
        p.reset().unwrap();
        let text = std::fs::read_to_string(p.config_path()).unwrap();
        assert!(text.contains("timeout = 120"), "实际:\n{text}");
    }

    #[test]
    fn 没配过的时候当前源是空的() {
        let (_d, _g) = 临时配置();
        assert_eq!(Pip.current(), None);
        // 文件压根不存在也不该报错。
        Pip.status().unwrap();
    }

    #[test]
    fn 探测地址指向一个确定存在的包() {
        let p = Pip;
        let m = p.lookup("tuna").unwrap();
        assert_eq!(
            p.probe_url(&m),
            "https://pypi.tuna.tsinghua.edu.cn/simple/setuptools/"
        );
        // 源地址不带结尾斜杠时也不能拼出双斜杠。
        assert_eq!(
            p.probe_url(&Mirror::new("x", "x", "https://a/simple")),
            "https://a/simple/setuptools/"
        );
    }
}
