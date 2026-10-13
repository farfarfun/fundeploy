//! 服务层。
//!
//! 对外只有一个 [`Service`]：装、升、退、起、跑、停、重启、查、卸这九个动作，
//! 命令行照着调就行。
//!
//! 往里分两族，分界线是**谁管进程**：
//!
//! - [`upstream`]：上游 CLI 自带 `server start/stop/status`，PID 文件、守护、
//!   日志都是它自己的事，fundeploy 只负责装包、按顺序调它、核对端口。
//! - [`daemon`]：上游只给一个前台二进制，进程管理这摊事全落在 fundeploy 头上——
//!   下载附件、起后台进程、写 PID、停的时候 TERM 完再 KILL。
//!
//! 两族各自把生命周期实现一次，具体服务退化成一份声明。
//!
//! 两族都装不下的服务直接实现 [`Service`]（见 [`paperclip`]），并用
//! [`Service::extras`] 把九个动作之外的东西挂出来。

pub mod config;
pub mod daemon;
pub mod paperclip;
pub mod registry;
pub mod spec;
pub mod upstream;

use std::time::Duration;

use crate::common::port;
use crate::ui;

/// 等端口监听起来的上限。
///
/// 启动命令只要 fork 成功就返回 0，子进程几秒后崩掉它也不报错，所以必须自己等
/// 端口。这个值要能覆盖「装了一堆依赖的 Python 后端 import 完」的时间。
const START_TIMEOUT: Duration = Duration::from_secs(30);

/// 九个标准动作之外、只有某个服务才有的动作。
///
/// 上游 CLI 的本事常常不止起停装卸（paperclip 还有 doctor、health、backup、
/// channels）。把它们硬塞进九个动作会扭曲语义，丢掉又是实打实的功能缺失，
/// 所以给一个明确的挂点：服务声明有哪些，命令行照着放行。
pub struct Extra {
    pub name: &'static str,
    pub about: &'static str,
}

/// 一个服务能做的事。两族各自实现一遍，命令行只认这个。
pub trait Service {
    fn name(&self) -> &str;
    fn summary(&self) -> &str;
    /// 这个服务会占住的端口，列表里拿来提示。
    fn ports(&self) -> Vec<u16>;

    fn install(&self, version: Option<&str>) -> anyhow::Result<()>;
    fn upgrade(&self) -> anyhow::Result<()>;
    fn rollback(&self, version: &str) -> anyhow::Result<()>;
    fn start(&self) -> anyhow::Result<()>;
    fn run(&self) -> anyhow::Result<()>;
    fn stop(&self) -> anyhow::Result<()>;
    fn restart(&self) -> anyhow::Result<()>;
    fn status(&self) -> anyhow::Result<()>;
    fn uninstall(&self) -> anyhow::Result<()>;

    /// 这个服务额外支持的动作。绝大多数服务没有。
    fn extras(&self) -> &'static [Extra] {
        &[]
    }

    /// 执行一个额外动作。`args` 是动作名之后的全部参数。
    ///
    /// 名字由命令行对着 [`Service::extras`] 核过才会传进来，所以实现里不必再防
    /// 不认识的名字——走到默认实现只说明声明和实现对不上，那是编程错误。
    fn extra(&self, name: &str, args: &[String]) -> anyhow::Result<()> {
        let _ = args;
        anyhow::bail!("{} 声明了动作 {name} 却没有实现它", self.name())
    }
}

/// 停完之后核对端口是不是真的放开了。
///
/// 残留必须明确报出来：紧接着的 start 会因为端口被占而跳过该组件，于是服务看着
/// 是起的，跑的却是旧进程——而 stop 和 start 都报了成功。
fn warn_if_still_listening(label: &str, host: &str, port: u16) {
    if !port::is_listening(host, port) {
        return;
    }
    match port::listener_pid(port) {
        Some(pid) => ui::warn(format!(
            "{label} 的端口 {port} 仍被 PID {pid} 占着。\n确认要强停: kill -TERM {pid}"
        )),
        None => ui::warn(format!(
            "{label} 的端口 {port} 仍有人监听，但查不到 PID（多半属于别的用户）。"
        )),
    }
}

/// 卸载确认的跳过开关。
fn uninstall_env_var(name: &str) -> String {
    format!(
        "FUNDEPLOY_{}_UNINSTALL_YES",
        name.to_ascii_uppercase().replace(['-', '.'], "_")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 卸载确认的环境变量名由服务名推出() {
        assert_eq!(
            uninstall_env_var("funread-web"),
            "FUNDEPLOY_FUNREAD_WEB_UNINSTALL_YES"
        );
        assert_eq!(
            uninstall_env_var("airflow"),
            "FUNDEPLOY_AIRFLOW_UNINSTALL_YES"
        );
    }

    #[test]
    fn 注册表里没有重名服务() {
        let names: Vec<String> = registry::all()
            .iter()
            .map(|s| s.name().to_string())
            .collect();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "有重名: {names:?}");
    }

    #[test]
    fn 按名字能找到服务() {
        for name in registry::names() {
            assert!(registry::find(&name).is_some(), "找不到 {name}");
        }
        assert!(registry::find("__不存在的服务__").is_none());
    }

    #[test]
    fn 所有服务的端口在全局范围内不撞车() {
        // 同机常常同时跑好几个服务，端口撞了会表现成「另一个服务莫名其妙起不来」。
        let mut seen: Vec<(u16, String)> = Vec::new();
        for s in registry::all() {
            for p in s.ports() {
                if let Some((_, who)) = seen.iter().find(|(x, _)| *x == p) {
                    panic!("端口 {p} 被 {who} 和 {} 同时占用", s.name());
                }
                seen.push((p, s.name().to_string()));
            }
        }
    }
}
