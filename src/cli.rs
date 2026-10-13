//! 命令行解析与分发。

use anyhow::{bail, Result};
use clap::{CommandFactory, Parser, Subcommand};

use crate::ai;
use crate::dev;
use crate::doctor;
use crate::port_kill;
use crate::service::{self, Service};
use crate::skills;
use crate::source;
use crate::tool::{self, Tool};
use crate::ui;
use crate::Extra;

#[derive(Parser)]
#[command(
    name = "fundeploy",
    version,
    about = "开发环境、工具与服务管理",
    arg_required_else_help = false,
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// 服务管理
    #[command(visible_alias = "svc")]
    Service {
        /// 服务名。不给就列出所有服务。
        name: Option<String>,
        #[command(subcommand)]
        action: Option<Action>,
    },
    /// 工具管理
    Tool {
        /// 工具名。不给就列出所有工具。
        name: Option<String>,
        #[command(subcommand)]
        action: Option<ToolAction>,
    },
    /// 换源（pip / npm / brew / cargo / go）
    #[command(visible_alias = "src")]
    Source {
        /// 哪个工具。不给就看全部的当前状态。
        name: Option<String>,
        #[command(subcommand)]
        action: Option<SourceAction>,
    },
    /// 腾端口：查出谁占着，停掉它
    PortKill {
        /// 端口号，可以给多个
        #[arg(required = true, value_name = "PORT")]
        ports: Vec<u16>,
        /// 只看占用情况，不动手
        #[arg(short, long)]
        list: bool,
        /// 不问直接停
        #[arg(short, long)]
        yes: bool,
    },
    /// 诊断：网页打得开、命令行却不动的那类问题
    Doctor {
        #[command(subcommand)]
        what: DoctorWhat,
    },
    /// 开发工具链（go / nodejs / pnpm / rust / uv / python）
    Dev {
        /// 工具名。不给就列出全部的当前状态。
        name: Option<String>,
        #[command(subcommand)]
        action: Option<DevAction>,
    },
    /// AI 命令行工具（claude / codex / cursor）
    Ai {
        /// 工具名，或 all 对三样都来一遍。不给就列出全部的当前状态。
        name: Option<String>,
        #[command(subcommand)]
        action: Option<AiAction>,
    },
    /// 技能仓库同步（Gitee 上的 farfarfun-skills）
    Skills {
        #[command(subcommand)]
        action: Option<SkillsAction>,
    },
    /// 列出能管的东西
    List,
}

#[derive(Subcommand, Clone)]
enum AiAction {
    /// 装没装、装在哪儿、哪个版本
    Status,
    /// 安装
    Install {
        #[command(flatten)]
        how: AiHow,
        /// `--` 之后的参数原样交给官方安装器
        #[arg(trailing_var_arg = true, value_name = "参数")]
        args: Vec<String>,
    },
    /// 升到最新版
    Upgrade {
        #[command(flatten)]
        how: AiHow,
    },
    /// 卸载。不点名装法就每条路都走一遍。
    Uninstall {
        #[command(flatten)]
        how: AiHow,
        /// 不问直接卸
        #[arg(short, long)]
        yes: bool,
    },
}

#[derive(clap::Args, Clone)]
struct AiHow {
    /// 装法。不给走 official。
    #[arg(long, value_enum, value_name = "装法")]
    method: Option<ai::Method>,
}

#[derive(Subcommand, Clone)]
enum DevAction {
    /// 装没装、装在哪儿、哪个版本
    Status,
    /// 安装
    Install {
        #[command(flatten)]
        how: DevHow,
        /// python：已存在时删掉重建；其余工具无此动作
        #[arg(long)]
        force: bool,
    },
    /// 升到最新版
    Upgrade {
        #[command(flatten)]
        how: DevHow,
    },
    /// 卸载。不点名装法就两边都清。
    Uninstall {
        #[command(flatten)]
        how: DevHow,
        /// 不问直接卸
        #[arg(short, long)]
        yes: bool,
    },
}

#[derive(clap::Args, Clone)]
struct DevHow {
    /// pkg=系统包管理器，source=官方包装到 ~/opt。不给看 FUNDEPLOY_INSTALL_METHOD，再不给用 pkg。
    #[arg(long, value_enum, value_name = "装法")]
    method: Option<dev::Method>,
    /// 钉版本。go/nodejs/rust 只在 source 装法下有效；python 用它选 Python 版本。
    #[arg(long, value_name = "版本")]
    version: Option<String>,
}

#[derive(Subcommand, Clone)]
enum SkillsAction {
    /// 查远端，列出每个仓库在本机的状态
    #[command(visible_alias = "ls")]
    List,
    /// 把缺的装上、已有的更新到最新
    Sync,
    /// 装一个
    Install {
        /// 仓库名
        name: String,
    },
    /// 更新一个
    Update {
        /// 仓库名
        name: String,
    },
    /// 删掉本地的仓库（有本地改动的会被拒绝）
    Rm {
        /// 仓库名；不给就要配 --all
        name: Option<String>,
        /// 删掉本地全部已装的
        #[arg(long, conflicts_with = "name")]
        all: bool,
        /// 不问直接删
        #[arg(short, long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum DoctorWhat {
    /// GitHub 连不上 / clone 卡住
    Github {
        /// 改到某条通道上；只给 --fix 不给通道名就按诊断结果自动挑
        #[arg(long, value_enum, value_name = "通道", num_args = 0..=1, default_missing_value = "auto")]
        fix: Option<doctor::github::Fix>,
        /// 撤掉 fundeploy 写进 git / ssh 配置里的东西
        #[arg(long, conflicts_with = "fix")]
        undo: bool,
    },
}

#[derive(Subcommand, Clone)]
enum SourceAction {
    /// 列出可用的源
    #[command(visible_alias = "ls")]
    List,
    /// 看当前配的是哪个
    Status,
    /// 测所有源的延迟
    Test,
    /// 换源。不给名字就自动选最快的那个。
    Use {
        /// 源名，或者直接给一个 http(s) 地址
        mirror: Option<String>,
    },
    /// 恢复官方源
    Reset,
    /// 添一个自己的源
    Add {
        /// 短名，以后用它点名
        name: String,
        /// 地址
        url: String,
        /// 备注，列表里显示
        #[arg(long)]
        label: Option<String>,
    },
    /// 删掉自己添的源
    Rm {
        /// 短名
        name: String,
    },
}

#[derive(Subcommand, Clone)]
enum Action {
    /// 安装
    Install {
        /// 钉到某个版本，不给就装最新版
        #[arg(long)]
        version: Option<String>,
    },
    /// 升到最新版（升级前在跑的话，换完包自动拉起来）
    Upgrade,
    /// 回退到指定版本
    Rollback {
        /// 版本号，例如 1.1.109
        version: String,
    },
    /// 后台启动
    Start,
    /// 前台运行（Ctrl+C 退出）
    Run,
    /// 停止
    Stop,
    /// 重启
    Restart,
    /// 看状态
    Status,
    /// 卸载
    Uninstall,

    /// 服务自己声明的额外动作（见 [`Service::extras`]），原样收下交给它。
    ///
    /// 参数不在这里解析：额外动作多半是转调上游 CLI，`-f`、`--repair` 这些该由
    /// 上游自己认，clap 插一手只会把它不认识的选项拦下来。
    #[command(external_subcommand)]
    Extra(Vec<String>),
}

/// 九个标准动作的名字。
///
/// [`Action::Extra`] 一旦生效，clap 就不再为拼错的子命令报错了——`statsu` 会被
/// 原样收下。这份名单是用来把那条报错补回去的，有测试守着它和 clap 不脱节。
const STANDARD: [&str; 9] = [
    "install",
    "upgrade",
    "rollback",
    "start",
    "run",
    "stop",
    "restart",
    "status",
    "uninstall",
];

impl Action {
    fn label(&self) -> String {
        match self {
            Action::Install { .. } => "install  安装".to_string(),
            Action::Upgrade => "upgrade  升到最新版".to_string(),
            Action::Rollback { .. } => "rollback 回退到指定版本".to_string(),
            Action::Start => "start    后台启动".to_string(),
            Action::Run => "run      前台运行".to_string(),
            Action::Stop => "stop     停止".to_string(),
            Action::Restart => "restart  重启".to_string(),
            Action::Status => "status   看状态".to_string(),
            Action::Uninstall => "uninstall 卸载".to_string(),
            Action::Extra(argv) => argv.join(" "),
        }
    }

    /// 菜单里给的那几个。install/rollback 要带参数，菜单里只放不带参数的形态。
    fn menu() -> Vec<Action> {
        vec![
            Action::Status,
            Action::Start,
            Action::Stop,
            Action::Restart,
            Action::Install { version: None },
            Action::Upgrade,
            Action::Run,
            Action::Uninstall,
        ]
    }

    fn apply(&self, svc: &dyn Service) -> Result<()> {
        match self {
            Action::Install { version } => svc.install(version.as_deref()),
            Action::Upgrade => svc.upgrade(),
            Action::Rollback { version } => svc.rollback(version),
            Action::Start => svc.start(),
            Action::Run => svc.run(),
            Action::Stop => svc.stop(),
            Action::Restart => svc.restart(),
            Action::Status => svc.status(),
            Action::Uninstall => svc.uninstall(),
            Action::Extra(argv) => run_extra(svc, argv),
        }
    }
}

#[derive(Subcommand, Clone)]
enum ToolAction {
    /// 安装
    Install {
        /// 钉到某个版本，不给就装最新版
        #[arg(long)]
        version: Option<String>,
    },
    /// 升到最新版
    Upgrade,
    /// 看状态
    Status,
    /// 卸载
    Uninstall,

    /// 工具自己声明的额外动作（见 [`Tool::extras`]）。
    #[command(external_subcommand)]
    Extra(Vec<String>),
}

/// 工具的四个标准动作。作用同 [`STANDARD`]。
const TOOL_STANDARD: [&str; 4] = ["install", "upgrade", "status", "uninstall"];

impl ToolAction {
    fn label(&self) -> String {
        match self {
            ToolAction::Install { .. } => "install  安装".to_string(),
            ToolAction::Upgrade => "upgrade  升到最新版".to_string(),
            ToolAction::Status => "status   看状态".to_string(),
            ToolAction::Uninstall => "uninstall 卸载".to_string(),
            ToolAction::Extra(argv) => argv.join(" "),
        }
    }

    fn menu() -> Vec<ToolAction> {
        vec![
            ToolAction::Status,
            ToolAction::Install { version: None },
            ToolAction::Upgrade,
            ToolAction::Uninstall,
        ]
    }

    fn apply(&self, t: &dyn Tool) -> Result<()> {
        match self {
            ToolAction::Install { version } => t.install(version.as_deref()),
            ToolAction::Upgrade => t.upgrade(),
            ToolAction::Status => t.status(),
            ToolAction::Uninstall => t.uninstall(),
            ToolAction::Extra(argv) => {
                let (name, args) = split_extra(t.name(), "tool", t.extras(), &TOOL_STANDARD, argv)?;
                t.extra(name, args)
            }
        }
    }
}

/// 执行一个服务声明的额外动作。
fn run_extra(svc: &dyn Service, argv: &[String]) -> Result<()> {
    let (name, args) = split_extra(svc.name(), "service", svc.extras(), &STANDARD, argv)?;
    svc.extra(name, args)
}

/// 把 `额外动作名 参数...` 拆开，顺便核一遍这个名字确实被声明过。
///
/// 必须核：clap 的 external_subcommand 会原样收下任何没见过的词，不核的话
/// `statsu` 这种手滑会一路传下去，变成一条拿拼错的子命令去调上游 CLI 的调用——
/// 报错也就成了上游的用法提示，跟「打错了」八竿子打不着。
fn split_extra<'a>(
    who: &str,
    domain: &str,
    extras: &[Extra],
    standard: &[&str],
    argv: &'a [String],
) -> Result<(&'a str, &'a [String])> {
    let Some((name, args)) = argv.split_first() else {
        bail!("没给动作名。看全部动作: fundeploy {domain} {who} --help");
    };
    if !extras.iter().any(|e| e.name == name) {
        let known: Vec<&str> = standard
            .iter()
            .copied()
            .chain(extras.iter().map(|e| e.name))
            .collect();
        bail!("{who} 没有 {name} 这个动作。可选: {}", known.join("、"));
    }
    Ok((name, args))
}

pub fn cli_main() -> Result<()> {
    match Cli::parse().command {
        Some(Command::List) => {
            list_all();
            Ok(())
        }
        Some(Command::Service { name, action }) => service_cmd(name, action),
        Some(Command::Tool { name, action }) => tool_cmd(name, action),
        Some(Command::Source { name, action }) => source_cmd(name, action),
        Some(Command::PortKill { ports, list, yes }) => {
            if list {
                port_kill::list(&ports)
            } else {
                port_kill::kill(&ports, yes)
            }
        }
        Some(Command::Doctor { what }) => match what {
            DoctorWhat::Github { fix, undo } => match (fix, undo) {
                (_, true) => doctor::github::undo(),
                (Some(f), _) => doctor::github::fix(f),
                // 不带动作时只诊断，不改配置：这条命令多半是在「出了问题想看看」
                // 的时候跑的，默认就动别人的 git 配置太横了。
                _ => doctor::github::diagnose(),
            },
        },
        Some(Command::Dev { name, action }) => dev_cmd(name, action),
        Some(Command::Ai { name, action }) => ai_cmd(name, action),
        Some(Command::Skills { action }) => skills_cmd(action),
        None => no_args(),
    }
}

fn dev_cmd(name: Option<String>, action: Option<DevAction>) -> Result<()> {
    let Some(name) = name else {
        for d in dev::registry::all() {
            dev::status(d.as_ref())?;
            ui::blank();
        }
        dev::python::list()?;
        ui::blank();
        ui::hint("装: fundeploy dev <go|nodejs|pnpm|rust|uv|python> install [--method pkg|source]");
        return Ok(());
    };

    // 不给动作时只报状态，不动手：这一域的每个动作都会往机器上写东西。
    let action = action.unwrap_or(DevAction::Status);

    if name == "python" || name == "python-env" || name == "py" {
        return match action {
            DevAction::Status => dev::python::list(),
            DevAction::Install { how, force } => {
                dev::python::install(how.version.as_deref(), force)
            }
            DevAction::Upgrade { how } => dev::python::update(how.version.as_deref()),
            DevAction::Uninstall { how, yes } => {
                dev::python::uninstall(how.version.as_deref(), yes)
            }
        };
    }

    let d = dev::registry::find(&name).ok_or_else(|| {
        anyhow::anyhow!(
            "没有 {name} 这样东西。可选: {}、python",
            dev::registry::names().join("、")
        )
    })?;
    match action {
        DevAction::Status => dev::status(d.as_ref()),
        DevAction::Install { how, force } => {
            if force {
                bail!("--force 只对 python 有意义。");
            }
            dev::install(d.as_ref(), how.method, how.version.as_deref())
        }
        DevAction::Upgrade { how } => dev::upgrade(d.as_ref(), how.method),
        DevAction::Uninstall { how, yes } => dev::uninstall(d.as_ref(), how.method, yes),
    }
}

fn ai_cmd(name: Option<String>, action: Option<AiAction>) -> Result<()> {
    let Some(name) = name else {
        return ai::list();
    };
    if name == "all" {
        return match action.unwrap_or(AiAction::Status) {
            AiAction::Status => ai::list(),
            AiAction::Install { how, args } => {
                ai::each(how.method, |t| t.install(how.method, &args))
            }
            AiAction::Upgrade { how } => ai::each(how.method, |t| t.upgrade(how.method)),
            AiAction::Uninstall { how, yes } => {
                ai::each(how.method, |t| t.uninstall(how.method, yes))
            }
        };
    }
    let t = ai::registry::find(&name).ok_or_else(|| {
        anyhow::anyhow!(
            "没有 {name} 这样东西。可选: {}",
            ai::registry::names().join("、")
        )
    })?;
    // 跟 dev 一样：不给动作时只报状态，不动手。
    match action.unwrap_or(AiAction::Status) {
        AiAction::Status => t.status(),
        AiAction::Install { how, args } => t.install(how.method, &args),
        AiAction::Upgrade { how } => t.upgrade(how.method),
        AiAction::Uninstall { how, yes } => t.uninstall(how.method, yes),
    }
}

fn skills_cmd(action: Option<SkillsAction>) -> Result<()> {
    match action {
        Some(SkillsAction::List) | None => skills::list(),
        Some(SkillsAction::Sync) => skills::sync(),
        Some(SkillsAction::Install { name }) => skills::install(&name),
        Some(SkillsAction::Update { name }) => skills::update(&name),
        Some(SkillsAction::Rm { name, all, yes }) => skills::remove(name.as_deref(), all, yes),
    }
}

fn source_cmd(name: Option<String>, action: Option<SourceAction>) -> Result<()> {
    let Some(name) = name else {
        // 不点名就把五个的现状一次报全：人想知道的通常是「我这台机器配了什么」，
        // 而不是「有哪些能配」。
        for s in source::registry::all() {
            s.status()?;
            ui::blank();
        }
        ui::hint("换源: fundeploy source <pip|npm|brew|cargo|go> use [源名]");
        return Ok(());
    };

    let s = source::registry::find(&name)?;
    let action = match action {
        Some(a) => a,
        None if ui::interactive() => pick_source_action()?,
        // 无人值守下默认看状态，而不是默默换源——换源会动配置文件。
        None => SourceAction::Status,
    };

    match action {
        SourceAction::List => s.list(),
        SourceAction::Status => s.status(),
        SourceAction::Test => s.test(),
        SourceAction::Use { mirror } => s.switch(mirror.as_deref()),
        SourceAction::Reset => s.reset(),
        SourceAction::Add { name, url, label } => {
            source::store::add(s.name(), &name, &url, label.as_deref())?;
            ui::ok(format!("已添加 {name} -> {url}"));
            ui::hint(format!("换过去: fundeploy source {} use {name}", s.name()));
            Ok(())
        }
        SourceAction::Rm { name } => {
            if source::store::remove(s.name(), &name)? {
                ui::ok(format!("已删除 {name}"));
            } else {
                // 内置源删不掉，但说「没有这个源」会让人以为是打错了名字。
                ui::warn(format!(
                    "{} 里没有你自己添的 {name}（内置源删不掉）。",
                    s.name()
                ));
            }
            Ok(())
        }
    }
}

fn pick_source_action() -> Result<SourceAction> {
    let menu = [
        "status  看当前配的是哪个".to_string(),
        "list    列出可用的源".to_string(),
        "test    测所有源的延迟".to_string(),
        "use     自动换到最快的".to_string(),
        "reset   恢复官方源".to_string(),
    ];
    let picked = ui::select("要做什么", &menu)?.ok_or_else(|| anyhow::anyhow!("已取消。"))?;
    Ok(match picked.split_whitespace().next().unwrap_or("status") {
        "list" => SourceAction::List,
        "test" => SourceAction::Test,
        "use" => SourceAction::Use { mirror: None },
        "reset" => SourceAction::Reset,
        _ => SourceAction::Status,
    })
}

/// 不带任何参数时。
///
/// 无人值守下必须打 help 然后正常退出，不能停在菜单上等输入——这条路径会被
/// 装机脚本和 CI 走到，卡住的表现是「安装脚本挂死」，比报错难查得多。
fn no_args() -> Result<()> {
    if !ui::interactive() {
        Cli::command().print_help()?;
        println!();
        return Ok(());
    }
    let domains = [
        "service    服务（装、起、停、查）".to_string(),
        "tool       工具（装、升、卸、查）".to_string(),
        "source     换源（pip、npm、brew、cargo、go）".to_string(),
        "port-kill  腾端口（查出谁占着，停掉它）".to_string(),
        "doctor     诊断（GitHub 连不上）".to_string(),
        "dev        开发工具链（go、nodejs、pnpm、rust、uv、python）".to_string(),
        "ai         AI 命令行（claude、codex、cursor）".to_string(),
        "skills     技能仓库（同步 Gitee 上的那批）".to_string(),
    ];
    let picked = ui::select("要管什么", &domains)?.ok_or_else(|| anyhow::anyhow!("已取消。"))?;
    match picked.split_whitespace().next().unwrap_or("") {
        "service" => {
            let svc = pick_one(service::registry::all(), "选一个服务", |s| {
                (s.name().to_string(), s.summary().to_string())
            })?;
            pick_and_run(svc.as_ref())
        }
        "source" => {
            let s = pick_one(source::registry::all(), "给哪个换源", |s| {
                (s.name().to_string(), s.summary().to_string())
            })?;
            let action = pick_source_action()?;
            source_cmd(Some(s.name().to_string()), Some(action))
        }
        "port-kill" => {
            let line = ui::input("端口号（多个用空格隔开）", "")?;
            port_kill::kill(&port_kill::parse_ports(&line)?, false)
        }
        "doctor" => doctor::github::diagnose(),
        "dev" => dev_cmd(None, None),
        "ai" => ai::list(),
        "skills" => skills::list(),
        _ => {
            let t = pick_one(tool::registry::all(), "选一个工具", |t| {
                (t.name().to_string(), t.summary().to_string())
            })?;
            pick_and_run_tool(t.as_ref())
        }
    }
}

fn list_all() {
    ui::heading("服务");
    for s in service::registry::all() {
        let ports: Vec<String> = s.ports().iter().map(u16::to_string).collect();
        println!(
            "  {:<14} {}（端口 {}）",
            s.name(),
            s.summary(),
            ports.join("、")
        );
    }
    ui::blank();
    ui::heading("工具");
    for t in tool::registry::all() {
        println!("  {:<14} {}", t.name(), t.summary());
    }
    ui::blank();
    ui::heading("可换源");
    for s in source::registry::all() {
        println!("  {:<14} {}", s.name(), s.summary());
    }
}

fn list_tools() {
    ui::heading("工具");
    for t in tool::registry::all() {
        println!("  {:<14} {}", t.name(), t.summary());
    }
}

fn service_cmd(name: Option<String>, action: Option<Action>) -> Result<()> {
    let Some(name) = name else {
        list_all();
        return Ok(());
    };

    let svc = service::registry::find(&name).ok_or_else(|| {
        anyhow::anyhow!(
            "没有叫 {name} 的服务。可选: {}",
            service::registry::names().join("、")
        )
    })?;

    match action {
        Some(a) => a.apply(svc.as_ref()),
        None => pick_and_run(svc.as_ref()),
    }
}

fn tool_cmd(name: Option<String>, action: Option<ToolAction>) -> Result<()> {
    let Some(name) = name else {
        list_tools();
        return Ok(());
    };

    let t = tool::registry::find(&name).ok_or_else(|| {
        anyhow::anyhow!(
            "没有叫 {name} 的工具。可选: {}",
            tool::registry::names().join("、")
        )
    })?;

    match action {
        Some(a) => a.apply(t.as_ref()),
        None => pick_and_run_tool(t.as_ref()),
    }
}

/// 从一张表里选一个。`describe` 给出 (名字, 一行说明)。
fn pick_one<T>(all: Vec<T>, prompt: &str, describe: impl Fn(&T) -> (String, String)) -> Result<T> {
    let labels: Vec<String> = all
        .iter()
        .map(|x| {
            let (name, about) = describe(x);
            format!("{name:<14} {about}")
        })
        .collect();
    let picked = ui::select(prompt, &labels)?.ok_or_else(|| anyhow::anyhow!("已取消。"))?;
    let idx = labels
        .iter()
        .position(|l| l == picked)
        .ok_or_else(|| anyhow::anyhow!("已取消。"))?;
    Ok(all.into_iter().nth(idx).unwrap())
}

/// 没给动作时的提示。额外动作不在 `--help` 里（clap 不认识它们），不在这儿说
/// 就等于没有。
fn no_action_hint(who: &str, domain: &str, extras: &[Extra]) -> String {
    let mut msg = format!(
        "要做什么？例如 `fundeploy {domain} {who} status`（看全部动作: `fundeploy {domain} {who} --help`）"
    );
    if !extras.is_empty() {
        let names: Vec<&str> = extras.iter().map(|e| e.name).collect();
        msg.push_str(&format!("\n{who} 还支持: {}", names.join("、")));
    }
    msg
}

/// 额外动作跟标准动作并列进菜单：对使用者来说它们同样是「这东西能做的事」。
fn action_labels(standard: Vec<String>, extras: &[Extra]) -> Vec<String> {
    let mut labels = standard;
    labels.extend(extras.iter().map(|e| format!("{:<9}{}", e.name, e.about)));
    labels
}

fn pick_and_run(svc: &dyn Service) -> Result<()> {
    if !ui::interactive() {
        bail!(no_action_hint(svc.name(), "service", svc.extras()));
    }

    let actions = Action::menu();
    let labels = action_labels(actions.iter().map(Action::label).collect(), svc.extras());

    let picked = ui::select(&format!("{} 要做什么", svc.name()), &labels)?
        .ok_or_else(|| anyhow::anyhow!("已取消。"))?;
    let idx = labels.iter().position(|l| l == picked).unwrap();
    match actions.get(idx) {
        Some(a) => a.apply(svc),
        // 越过标准动作的下标落在 extras 上，顺序跟上面拼 labels 时一致。
        None => svc.extra(svc.extras()[idx - actions.len()].name, &[]),
    }
}

fn pick_and_run_tool(t: &dyn Tool) -> Result<()> {
    if !ui::interactive() {
        bail!(no_action_hint(t.name(), "tool", t.extras()));
    }

    let actions = ToolAction::menu();
    let labels = action_labels(actions.iter().map(ToolAction::label).collect(), t.extras());

    let picked = ui::select(&format!("{} 要做什么", t.name()), &labels)?
        .ok_or_else(|| anyhow::anyhow!("已取消。"))?;
    let idx = labels.iter().position(|l| l == picked).unwrap();
    match actions.get(idx) {
        Some(a) => a.apply(t),
        None => t.extra(t.extras()[idx - actions.len()].name, &[]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 命令定义本身是合法的() {
        // clap 的参数冲突（重复的长选项、子命令重名）只在运行时才炸，
        // 这条断言把它提到编译测试阶段。
        Cli::command().debug_assert();
    }

    #[test]
    fn 解析服务动作() {
        let cli = Cli::try_parse_from(["fundeploy", "service", "funread-web", "start"]).unwrap();
        match cli.command {
            Some(Command::Service { name, action }) => {
                assert_eq!(name.as_deref(), Some("funread-web"));
                assert!(matches!(action, Some(Action::Start)));
            }
            _ => panic!("解析成了别的命令"),
        }
    }

    #[test]
    fn rollback_必须给版本号() {
        // 不给版本号就回退是没有意义的，这里要在解析阶段就挡住。
        assert!(Cli::try_parse_from(["fundeploy", "service", "x", "rollback"]).is_err());
        assert!(Cli::try_parse_from(["fundeploy", "service", "x", "rollback", "1.1.109"]).is_ok());
    }

    #[test]
    fn install_的版本号是可选的() {
        let cli = Cli::try_parse_from(["fundeploy", "service", "x", "install"]).unwrap();
        match cli.command {
            Some(Command::Service {
                action: Some(Action::Install { version }),
                ..
            }) => {
                assert_eq!(version, None);
            }
            _ => panic!("解析成了别的命令"),
        }
        let cli =
            Cli::try_parse_from(["fundeploy", "service", "x", "install", "--version", "1.2.3"])
                .unwrap();
        match cli.command {
            Some(Command::Service {
                action: Some(Action::Install { version }),
                ..
            }) => {
                assert_eq!(version.as_deref(), Some("1.2.3"));
            }
            _ => panic!("解析成了别的命令"),
        }
    }

    #[test]
    fn 不带参数不报错() {
        // 交互菜单的入口。解析阶段必须放行，否则无人值守下连 help 都打不出来。
        let cli = Cli::try_parse_from(["fundeploy"]).unwrap();
        assert!(cli.command.is_none());
    }

    #[test]
    fn 不认识的子命令要报错() {
        assert!(Cli::try_parse_from(["fundeploy", "没这个命令"]).is_err());
    }

    /// clap 那边某个域实际有哪些子命令。
    fn 子命令(域: &str) -> Vec<String> {
        let cmd = Cli::command();
        let d = cmd
            .find_subcommand(域)
            .unwrap_or_else(|| panic!("没有 {域} 子命令"));
        let mut names: Vec<String> = d
            .get_subcommands()
            .map(|c| c.get_name().to_string())
            .filter(|n| n != "help")
            .collect();
        names.sort();
        names
    }

    #[test]
    fn 标准动作名单和_clap_不脱节() {
        // STANDARD 是手抄的，加了动作忘了抄就会在报错里漏掉它。
        let mut mine = STANDARD.to_vec();
        mine.sort_unstable();
        assert_eq!(子命令("service"), mine);

        let mut mine = TOOL_STANDARD.to_vec();
        mine.sort_unstable();
        assert_eq!(子命令("tool"), mine);
    }

    #[test]
    fn 解析工具动作() {
        let cli = Cli::try_parse_from(["fundeploy", "tool", "open-pencil", "status"]).unwrap();
        match cli.command {
            Some(Command::Tool { name, action }) => {
                assert_eq!(name.as_deref(), Some("open-pencil"));
                assert!(matches!(action, Some(ToolAction::Status)));
            }
            _ => panic!("解析成了别的命令"),
        }
    }

    #[test]
    fn 工具没有起停这类动作() {
        // 工具是没有进程的东西，`start` 应当像拼错了一样被挡下，而不是被
        // external_subcommand 收下再传到工具里。
        let err = tool_cmd(
            Some("open-pencil".into()),
            Some(ToolAction::Extra(vec!["start".into()])),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("start"), "实际: {err}");
        assert!(err.contains("install"), "实际: {err}");
    }

    #[test]
    fn 工具的额外动作被放行() {
        let cli = Cli::try_parse_from(["fundeploy", "tool", "open-pencil", "mcp-config"]).unwrap();
        match cli.command {
            Some(Command::Tool {
                action: Some(ToolAction::Extra(argv)),
                ..
            }) => assert_eq!(argv, ["mcp-config"]),
            _ => panic!("解析成了别的命令"),
        }
    }

    #[test]
    fn 不存在的工具名报错里列出可选项() {
        let err = tool_cmd(Some("没这个工具".into()), Some(ToolAction::Status))
            .unwrap_err()
            .to_string();
        assert!(err.contains("open-pencil"), "实际: {err}");
    }

    #[test]
    fn 工具菜单里的动作都不需要额外参数() {
        for a in ToolAction::menu() {
            assert!(!matches!(a, ToolAction::Extra(_)));
        }
    }

    #[test]
    fn 没给动作时的提示里带上额外动作() {
        let extras = tool::registry::find("open-pencil").unwrap().extras();
        let msg = no_action_hint("open-pencil", "tool", extras);
        assert!(msg.contains("mcp-config"), "实际: {msg}");
        assert!(msg.contains("fundeploy tool open-pencil"), "实际: {msg}");
    }

    #[test]
    fn 没见过的动作被原样收下交给服务() {
        // external_subcommand 生效后 clap 不再报错，核名字这件事归 run_extra。
        let cli = Cli::try_parse_from(["fundeploy", "service", "paperclip", "doctor", "--repair"])
            .unwrap();
        match cli.command {
            Some(Command::Service {
                action: Some(Action::Extra(argv)),
                ..
            }) => assert_eq!(argv, ["doctor", "--repair"]),
            _ => panic!("解析成了别的命令"),
        }
    }

    #[test]
    fn 手滑拼错的动作报错里列出可选项() {
        // 不核名字的话，`statsu` 会被当成额外动作一路传到上游 CLI，
        // 报出来的是上游的用法提示，跟「打错了」八竿子打不着。
        let err = service_cmd(
            Some("funread-web".into()),
            Some(Action::Extra(vec!["statsu".into()])),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("statsu"), "实际: {err}");
        assert!(err.contains("status"), "实际: {err}");
    }

    #[test]
    fn 声明了额外动作的服务在报错里也列出它们() {
        let err = service_cmd(
            Some("paperclip".into()),
            Some(Action::Extra(vec!["没这个".into()])),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("doctor"), "实际: {err}");
    }

    #[test]
    fn 菜单里的动作都不需要额外参数() {
        // 菜单选完就直接执行，带必填参数的动作混进来会在运行时才暴露。
        for a in Action::menu() {
            assert!(
                !matches!(a, Action::Rollback { .. }),
                "rollback 需要版本号，不能放进菜单"
            );
        }
    }

    #[test]
    fn 无人值守下不带参数打_help_而不是卡住() {
        // 装机脚本和 CI 会走到这条路径，卡在菜单上的表现是「安装挂死」。
        std::env::set_var("NONINTERACTIVE", "1");
        no_args().unwrap();
        std::env::remove_var("NONINTERACTIVE");
    }

    #[test]
    fn 解析腾端口() {
        let cli = Cli::try_parse_from(["fundeploy", "port-kill", "8080", "8081", "-y"]).unwrap();
        match cli.command {
            Some(Command::PortKill { ports, list, yes }) => {
                assert_eq!(ports, [8080, 8081]);
                assert!(!list);
                assert!(yes);
            }
            _ => panic!("解析成了别的命令"),
        }
    }

    #[test]
    fn 腾端口必须给端口号() {
        // 不给端口就执行等于「停掉所有端口上的进程」，必须在解析阶段挡住。
        assert!(Cli::try_parse_from(["fundeploy", "port-kill"]).is_err());
        // 端口号超出 u16 也该在这儿被拒，而不是跑起来再说。
        assert!(Cli::try_parse_from(["fundeploy", "port-kill", "70000"]).is_err());
    }

    #[test]
    fn 不存在的服务名报错里列出可选项() {
        let err = service_cmd(Some("没这个服务".into()), Some(Action::Status))
            .unwrap_err()
            .to_string();
        assert!(err.contains("funread-web"), "实际: {err}");
    }
}
