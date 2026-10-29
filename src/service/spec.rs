//! 服务的声明式描述。
//!
//! 一个服务由若干**组件**组成（后端、前端、管理端……）。组件之间只有两种关系：
//! 启动顺序（声明顺序）和地址引用（前端要知道后端在哪）。除此之外每个组件都是
//! 独立进程，由各自的上游 CLI 自己管 PID 和日志。
//!
//! 上游 CLI 的调用形态差异很大，没有共同约定，所以不去猜，而是把每个动作的
//! 参数模板直接声明出来：
//!
//! ```ignore
//! // funfluid-api：子命令带一个 service 位置参数，不接受 --host/--port
//! CliShape::new()
//!     .start(args!["start", "gunicorn", "-d"])
//!     .stop(args!["stop", "all"])
//!
//! // funread-api：动作在 server 子组下，接受 --host/--port
//! CliShape::new()
//!     .start(args!["server", "start", "--host", Host, "--port", Port])
//!     .stop(args!["server", "stop", "all"])
//! ```

use anyhow::{bail, Result};

/// 参数模板里的一项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arg {
    /// 字面量。
    Lit(String),
    /// 本组件的监听地址。
    Host,
    /// 本组件的监听端口。
    Port,
    /// 另一个组件的 `http://host:port`，按 key 引用。
    SiblingUrl(String),
    /// 另一个组件的监听地址。
    SiblingHost(String),
    /// 另一个组件的端口。
    SiblingPort(String),
}

impl From<&str> for Arg {
    fn from(s: &str) -> Self {
        Arg::Lit(s.to_string())
    }
}

impl From<String> for Arg {
    fn from(s: String) -> Self {
        Arg::Lit(s)
    }
}

/// 构造参数模板。`args!["start", "--port", Port]` 这样写。
#[macro_export]
macro_rules! args {
    [$($x:expr),* $(,)?] => {
        vec![$($crate::service::spec::Arg::from($x)),*]
    };
}

/// 上游 CLI 在各动作下的调用形态。空模板表示该动作不支持。
#[derive(Debug, Clone, Default)]
pub struct CliShape {
    pub start: Vec<Arg>,
    pub run: Vec<Arg>,
    pub stop: Vec<Arg>,
    pub status: Vec<Arg>,
}

impl CliShape {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start(mut self, a: Vec<Arg>) -> Self {
        self.start = a;
        self
    }

    pub fn run(mut self, a: Vec<Arg>) -> Self {
        self.run = a;
        self
    }

    pub fn stop(mut self, a: Vec<Arg>) -> Self {
        self.stop = a;
        self
    }

    pub fn status(mut self, a: Vec<Arg>) -> Self {
        self.status = a;
        self
    }
}

/// 组件的分发方式。
#[derive(Debug, Clone)]
pub enum Pkg {
    /// Python 包。`cli` 是它提供的可执行文件名。
    Pip { name: String, cli: String },
    /// 全局 npm 包。包名可能带 scope，而 `cli` 通常不带。
    Npm {
        name: String,
        cli: String,
        /// 无 scope 的包也可以复用已有 scope 的私有源配置。
        registry_scope: Option<String>,
    },
}

impl Pkg {
    pub fn pip(name: &str, cli: &str) -> Self {
        Pkg::Pip {
            name: name.into(),
            cli: cli.into(),
        }
    }

    pub fn npm(name: &str, cli: &str) -> Self {
        Pkg::Npm {
            name: name.into(),
            cli: cli.into(),
            registry_scope: None,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Pkg::Pip { name, .. } | Pkg::Npm { name, .. } => name,
        }
    }

    pub fn cli(&self) -> &str {
        match self {
            Pkg::Pip { cli, .. } | Pkg::Npm { cli, .. } => cli,
        }
    }

    pub fn kind_label(&self) -> &'static str {
        match self {
            Pkg::Pip { .. } => "python",
            Pkg::Npm { .. } => "npm",
        }
    }
}

/// 一个组件。
#[derive(Debug, Clone)]
pub struct Component {
    /// 组件内唯一标识，供地址引用用（`backend` / `frontend` / `admin` …）。
    pub key: String,
    /// 打给人看的名字。
    pub label: String,
    pub pkg: Pkg,
    pub host: String,
    pub port: u16,
    /// 钉死的版本；None 表示装最新版。
    pub version: Option<String>,
    pub cli: CliShape,
    /// 跑这个组件时要额外导出的环境变量。
    pub env: Vec<(String, String)>,
}

impl Component {
    pub fn new(key: &str, label: &str, pkg: Pkg, host: &str, port: u16) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            pkg,
            host: host.into(),
            port,
            version: None,
            cli: CliShape::new(),
            env: Vec::new(),
        }
    }

    pub fn cli_shape(mut self, c: CliShape) -> Self {
        self.cli = c;
        self
    }

    pub fn version(mut self, v: Option<String>) -> Self {
        self.version = v.filter(|s| !s.is_empty());
        self
    }

    pub fn env(mut self, k: &str, v: &str) -> Self {
        self.env.push((k.into(), v.into()));
        self
    }

    /// 别人要连它时该用的地址。
    ///
    /// `0.0.0.0` / `::` 是「监听所有网卡」的写法，不是可连接的目的地址，换成回环。
    pub fn connect_host(&self) -> &str {
        match self.host.as_str() {
            "0.0.0.0" | "*" | "" => "127.0.0.1",
            "::" | "[::]" => "[::1]",
            h => h,
        }
    }

    pub fn url(&self) -> String {
        format!("http://{}:{}", self.connect_host(), self.port)
    }

    /// 监听地址是否对外开放。start 时据此提醒防火墙。
    pub fn is_public(&self) -> bool {
        !matches!(
            self.host.as_str(),
            "127.0.0.1" | "localhost" | "::1" | "[::1]" | ""
        )
    }

    /// 把参数模板渲染成实际 argv（不含程序名）。
    pub fn render(&self, template: &[Arg], all: &[Component]) -> Result<Vec<String>> {
        let sibling = |key: &str| -> Result<&Component> {
            all.iter()
                .find(|c| c.key == key)
                .ok_or_else(|| anyhow::anyhow!("组件 {} 引用了不存在的组件 {key}", self.key))
        };

        let mut out = Vec::with_capacity(template.len());
        for a in template {
            out.push(match a {
                Arg::Lit(s) => s.clone(),
                Arg::Host => self.host.clone(),
                Arg::Port => self.port.to_string(),
                Arg::SiblingUrl(k) => sibling(k)?.url(),
                // 上游要的是「去哪儿连」，所以通配地址要换成回环，和 url() 一致。
                Arg::SiblingHost(k) => sibling(k)?.connect_host().to_string(),
                Arg::SiblingPort(k) => sibling(k)?.port.to_string(),
            });
        }
        Ok(out)
    }
}

/// 一个服务。
#[derive(Debug, Clone)]
pub struct ServiceSpec {
    pub name: String,
    pub summary: String,
    /// 声明顺序 = 启动顺序；停止按逆序。
    pub components: Vec<Component>,
    /// help 末尾的额外说明。
    pub notes: Vec<String>,
    /// 装完/起完提示访问哪个组件的地址。None 表示不提示。
    pub entry_component: Option<String>,
    /// 入口地址的路径部分（有些服务的首页不在 `/` 上）。
    pub entry_path: Option<String>,
}

impl ServiceSpec {
    pub fn new(name: &str, summary: &str, components: Vec<Component>) -> Self {
        Self {
            name: name.into(),
            summary: summary.into(),
            components,
            notes: Vec::new(),
            entry_component: None,
            entry_path: None,
        }
    }

    pub fn note(mut self, s: impl Into<String>) -> Self {
        self.notes.push(s.into());
        self
    }

    pub fn entry(mut self, key: &str) -> Self {
        self.entry_component = Some(key.into());
        self
    }

    /// 入口不在 `/` 上时用这个，`path` 以 `/` 开头。
    pub fn entry_at(mut self, key: &str, path: &str) -> Self {
        self.entry_component = Some(key.into());
        self.entry_path = Some(path.into());
        self
    }

    pub fn component(&self, key: &str) -> Option<&Component> {
        self.components.iter().find(|c| c.key == key)
    }

    /// 入口地址（装完让操作者打开的那个）。
    pub fn entry_url(&self) -> Option<String> {
        let key = self.entry_component.as_deref()?;
        let base = self.component(key)?.url();
        Some(match &self.entry_path {
            Some(p) => format!("{base}{p}"),
            None => base,
        })
    }

    /// 自检：key 不重复、端口不冲突、地址引用都能解析。
    ///
    /// 这些错误全是声明写错，属于编程错误而非运行时故障，所以有一个单测会把
    /// 注册表里每个服务都跑一遍这个检查。
    pub fn validate(&self) -> Result<()> {
        if self.components.is_empty() {
            bail!("服务 {} 没有任何组件", self.name);
        }

        for (i, c) in self.components.iter().enumerate() {
            if self.components[..i].iter().any(|o| o.key == c.key) {
                bail!("服务 {} 的组件 key 重复: {}", self.name, c.key);
            }
            if let Some(dup) = self.components[..i].iter().find(|o| o.port == c.port) {
                bail!(
                    "服务 {} 的组件 {} 与 {} 端口冲突: {}",
                    self.name,
                    c.key,
                    dup.key,
                    c.port
                );
            }
            // 渲染一遍所有模板，把悬空的地址引用逼出来。
            for t in [&c.cli.start, &c.cli.run, &c.cli.stop, &c.cli.status] {
                c.render(t, &self.components)?;
            }
        }

        if let Some(key) = &self.entry_component {
            if self.component(key).is_none() {
                bail!("服务 {} 的入口组件 {key} 不存在", self.name);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comp(key: &str, port: u16) -> Component {
        Component::new(key, key, Pkg::pip("p", "c"), "127.0.0.1", port)
    }

    #[test]
    fn 渲染字面量与自身地址() {
        let c = Component::new("backend", "后端", Pkg::pip("a", "a"), "0.0.0.0", 18811);
        let all = vec![c.clone()];
        let t = args!["server", "start", "--host", Arg::Host, "--port", Arg::Port];
        let got = c.render(&t, &all).unwrap();
        assert_eq!(
            got,
            ["server", "start", "--host", "0.0.0.0", "--port", "18811"]
        );
    }

    #[test]
    fn 渲染兄弟组件的地址() {
        let backend = Component::new("backend", "后端", Pkg::pip("a", "a"), "127.0.0.1", 18811);
        let frontend = Component::new("frontend", "前端", Pkg::npm("b", "b"), "127.0.0.1", 8811);
        let all = vec![backend, frontend.clone()];

        let t = args![
            "server",
            "start",
            "--backend",
            Arg::SiblingUrl("backend".into())
        ];
        let got = frontend.render(&t, &all).unwrap();
        assert_eq!(
            got,
            ["server", "start", "--backend", "http://127.0.0.1:18811"]
        );
    }

    #[test]
    fn 悬空的地址引用在渲染时报错() {
        let c = comp("frontend", 8811);
        let all = vec![c.clone()];
        let t = args!["--backend", Arg::SiblingUrl("backend".into())];
        let err = c.render(&t, &all).unwrap_err().to_string();
        assert!(err.contains("不存在的组件 backend"), "实际: {err}");
    }

    #[test]
    fn 通配地址在展示时换成回环() {
        let mut c = comp("x", 8080);
        c.host = "0.0.0.0".into();
        assert_eq!(c.url(), "http://127.0.0.1:8080");
        c.host = "::".into();
        assert_eq!(c.url(), "http://[::1]:8080");
    }

    #[test]
    fn 对外开放的地址被识别() {
        let mut c = comp("x", 8080);
        assert!(!c.is_public());
        c.host = "localhost".into();
        assert!(!c.is_public());
        c.host = "0.0.0.0".into();
        assert!(c.is_public());
        c.host = "192.168.1.5".into();
        assert!(c.is_public());
    }

    #[test]
    fn 空版本号当作不钉版本() {
        let c = comp("x", 1).version(Some(String::new()));
        assert_eq!(c.version, None);
        let c = comp("x", 1).version(Some("1.2.3".into()));
        assert_eq!(c.version.as_deref(), Some("1.2.3"));
    }

    #[test]
    fn 校验通过的最小服务() {
        ServiceSpec::new("s", "", vec![comp("a", 1)])
            .validate()
            .unwrap();
    }

    #[test]
    fn 没有组件的服务校验失败() {
        let err = ServiceSpec::new("s", "", vec![])
            .validate()
            .unwrap_err()
            .to_string();
        assert!(err.contains("没有任何组件"), "实际: {err}");
    }

    #[test]
    fn 重复的组件_key_校验失败() {
        let err = ServiceSpec::new("s", "", vec![comp("a", 1), comp("a", 2)])
            .validate()
            .unwrap_err()
            .to_string();
        assert!(err.contains("key 重复"), "实际: {err}");
    }

    #[test]
    fn 端口冲突校验失败() {
        // 同一个服务的两个组件抢同一个端口，start 时表现为「第二个组件起不来」
        // 或更糟：第二个组件看到端口已占用就跳过，于是它根本没起但报告成功。
        let err = ServiceSpec::new("s", "", vec![comp("a", 8080), comp("b", 8080)])
            .validate()
            .unwrap_err()
            .to_string();
        assert!(err.contains("端口冲突"), "实际: {err}");
    }

    #[test]
    fn 校验会把模板里的悬空引用逼出来() {
        let mut f = comp("frontend", 2);
        f.cli = CliShape::new().start(args!["--backend", Arg::SiblingUrl("nope".into())]);
        let err = ServiceSpec::new("s", "", vec![f])
            .validate()
            .unwrap_err()
            .to_string();
        assert!(err.contains("不存在的组件 nope"), "实际: {err}");
    }

    #[test]
    fn 不存在的入口组件校验失败() {
        let err = ServiceSpec::new("s", "", vec![comp("a", 1)])
            .entry("b")
            .validate()
            .unwrap_err()
            .to_string();
        assert!(err.contains("入口组件 b 不存在"), "实际: {err}");
    }

    #[test]
    fn 入口地址取自指定组件() {
        let spec = ServiceSpec::new("s", "", vec![comp("a", 1), comp("b", 8811)]).entry("b");
        assert_eq!(spec.entry_url().as_deref(), Some("http://127.0.0.1:8811"));
    }
}
