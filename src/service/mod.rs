//! 服务层。
//!
//! 对外只有一个 [`Service`]：装、升、退、起、跑、停、重启、查、卸这九个动作，
//! 命令行照着调就行。
//!
//! 往里分三族，分界线是**谁管进程**和**包从哪来**：
//!
//! | | 进程归上游管 | 进程归 fundeploy 管 |
//! |---|---|---|
//! | **包来自 PyPI/npm** | [`upstream`] | [`venv`] |
//! | **包来自 GitHub release** | — | [`daemon`] |
//!
//! - [`upstream`]：上游 CLI 自带 `server start/stop/status`，PID 文件、守护、
//!   日志都是它自己的事，fundeploy 只负责装包、按顺序调它、核对端口。
//! - [`daemon`]：上游只给一个前台二进制，进程管理这摊事全落在 fundeploy 头上——
//!   下载附件、起后台进程、写 PID、停的时候 TERM 完再 KILL。
//! - [`venv`]：同样由 fundeploy 管进程，但包是 pip 装进一个独占虚拟环境的。
//!   认进程的方式也因此不同：venv 里的控制台脚本跑起来 `/proc/<pid>/exe` 是
//!   python 解释器，只能认「属于这个 venv」而不是某个具体文件。
//!
//! 三族各自把生命周期实现一次，具体服务退化成一份声明。
//!
//! 三族都装不下的服务直接实现 [`Service`]（见 [`paperclip`]），并用
//! [`Service::extras`] 把九个动作之外的东西挂出来。

pub mod airflow;
pub mod celery;
pub mod config;
pub mod daemon;
pub mod paperclip;
pub mod registry;
pub mod spec;
pub mod upstream;
pub mod venv;

use std::time::Duration;

use crate::common::{port, proc};
use crate::ui;

/// 等端口监听起来的上限。
///
/// 启动命令只要 fork 成功就返回 0，子进程几秒后崩掉它也不报错，所以必须自己等
/// 端口。这个值要能覆盖「装了一堆依赖的 Python 后端 import 完」的时间。
const START_TIMEOUT: Duration = Duration::from_secs(30);

/// stop 收尾强停时，SIGTERM 之后等多久再上 SIGKILL。
///
/// 比正常停止的宽限短得多：走到这一步，这个进程已经收过一轮停止请求了
/// （daemon/venv 自己的 TERM→KILL，或者上游 stop 发的那个 TERM），还赖着
/// 不走就别再陪它等下去。
const FORCE_GRACE: Duration = Duration::from_secs(5);

/// 在终端里也跳过「要不要按端口强停」这一问的开关。
///
/// 无人值守时本来就不问（见 [`ui::confirm_default_yes`]），这个变量是给盯着
/// 终端、但不想每次 restart 都按一下回车的人用的。
const ENV_KILL_PORT_YES: &str = "FUNDEPLOY_STOP_KILL_PORT_YES";

pub use crate::Extra;

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

/// 停完之后核对端口是不是真的放开了，没放开就按端口查出占用者停掉，动手前问一声。
///
/// 残留不能只是报出来：紧接着的 start 会因为端口被占而跳过该组件，于是服务看着
/// 是起的，跑的却是旧进程——而 stop 和 start 都报了成功。
///
/// 要问的原因是这个 PID 是从端口反查来的，端口是全机共享的，它完全可能是别人的
/// 进程（同机另装了一份、或者端口号撞了）。所以把谁要被停报在问题里，让人有机会
/// 拦住。但这一问不能挡住自动化：无人值守时不问直接停，默认选项也是「停」——
/// 详见 [`ui::confirm_default_yes`]。
fn force_free_port(label: &str, host: &str, port: u16) {
    if !port::is_listening(host, port) {
        return;
    }
    // 一次拿全占用者，而不是先 listener_pid 再 who —— 那是对 /proc 的两趟全盘
    // 扫描挨在一起，第二趟问的还是第一趟已经看过的东西。
    let hs = port::holders(port);
    let Some(h) = hs.iter().find(|h| h.protos.contains(&"tcp")) else {
        ui::warn(format!(
            "{label} 的端口 {port} 仍有人监听，但查不到 PID（多半属于别的用户），强停不了。加 sudo 再跑一次。"
        ));
        return;
    };
    let pid = h.pid;
    if let Some(why) = proc::protected(pid) {
        ui::warn(format!(
            "{label} 的端口 {port} 被 PID {pid} 占着，但它{why}，没有动它。"
        ));
        return;
    }

    // 占用者一律摆在句末：带进程名时它以全角「）」收尾，后面再接字就得夹个空格，
    // 中文里看着像漏排版。
    let who = describe(h);
    ui::warn(format!("{label} 的端口 {port} 停完还占着，占用者是 {who}"));
    if !ui::confirm_default_yes(
        &format!("强制停掉它，腾出端口 {port} 吗"),
        ENV_KILL_PORT_YES,
    ) {
        ui::warn(format!(
            "没有动 {who}；{label} 的端口 {port} 还占着，接下来的 start 会起不来。\
             要自己停: kill -TERM {pid}"
        ));
        return;
    }

    ui::step(format!("强制停掉 {who}"));
    // 绝不按进程组发信号：这个 PID 是从端口反查来的，它的进程组里很可能坐着
    // 调用者自己的 shell。
    if let Err(e) = proc::stop_pid(pid, FORCE_GRACE, false) {
        ui::warn(format!("停不下来: {e}"));
    }

    // 复查端口而不是复查 PID：进程没了不等于端口腾出来了，同一个端口上还可能压着
    // 别的用户的进程，非 root 时根本看不见它。
    if port::is_listening(host, port) {
        ui::warn(format!(
            "{label} 的端口 {port} 还占着。占用者多半不属于当前用户，加 sudo 再跑一次。"
        ));
    } else {
        ui::ok(format!("{label} 的端口 {port} 已腾出。"));
    }
}

/// 把占用者说清楚：有进程名就带上。问人「要不要停掉 PID 12345」，光一个数字
/// 没法判断；带上 `gunicorn` 才认得出那是不是自己要停的东西。
fn describe(h: &port::Holder) -> String {
    if h.comm.is_empty() {
        format!("PID {}", h.pid)
    } else {
        format!("PID {}（{}）", h.pid, h.comm)
    }
}

/// 卸载确认的跳过开关。实现和 tool 那边共用一份（见
/// [`crate::common::env::uninstall_yes_var`]）：变量名是对外承诺，两份实现漂开
/// 一点，无人值守脚本就会静默卡在确认上。
pub(crate) use crate::common::env::uninstall_yes_var as uninstall_env_var;

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn 没人占的端口不用腾也不报错() {
        let port = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        force_free_port("测试", "127.0.0.1", port);
    }

    #[test]
    fn 占用者的提示里带着进程名() {
        // 问「要停掉 PID 12345 吗」时，光一个数字没法判断该不该答应。
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let me = std::process::id() as i32;

        let h = port::holders(port)
            .into_iter()
            .find(|h| h.pid == me)
            .expect("自己持有的 socket 必须能反查到");
        let s = describe(&h);

        assert!(s.contains(&me.to_string()), "实际: {s}");
        assert!(s.contains('（'), "没带上进程名: {s}");
        drop(listener);
    }

    #[test]
    fn 自己占的端口不会把自己杀掉() {
        // 没有 proc::protected 那道闸，这条测试会把测试进程自己干掉。
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        force_free_port("测试", "127.0.0.1", port);

        assert!(
            port::is_listening("127.0.0.1", port),
            "端口 {port} 不该被腾掉"
        );
        drop(listener);
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
