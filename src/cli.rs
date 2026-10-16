//! 命令行解析与分发。

use anyhow::{bail, Result};
use clap::{CommandFactory, Parser, Subcommand};

use crate::ai;
use crate::common::port;
use crate::dev;
use crate::doctor;
use crate::port_kill;
use crate::selfup;
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
    /// 升级 fundeploy 自己（包管理器装的就调它自己的升级命令）
    Upgrade {
        /// 装指定版本。比当前旧也照做，这是回退的用法。brew / apt 装的点不了版本。
        #[arg(long, value_name = "版本")]
        version: Option<String>,
        /// 只看有没有新版本，不动手
        #[arg(long)]
        check: bool,
    },
    /// 列出能管的东西
    List,
    /// 这是什么、谁做的
    About,
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
    /// 这个动作从菜单选中之后，还得再问一次参数吗。
    ///
    /// [`Action::menu`] 只能摆出不带参数的占位，带必填参数的动作必须由
    /// [`pick_and_run`] 补问，否则会拿着空值一路走到底。这里是那份名单的唯一出处
    /// ——`pick_and_run` 和测试都读它，所以加了新的带参数动作时，改完这里测试会
    /// 立刻提醒你去补那条分支。
    fn needs_input(&self) -> bool {
        matches!(self, Action::Rollback { .. })
    }

    /// 交互菜单里列出的动作。
    ///
    /// 顺序是「常用的在前」，不是 clap 里的声明顺序。但**必须一个不少**——菜单是
    /// 唯一能发现这些动作的地方，漏掉的那个等于不存在。有测试钉着它和 clap 一致。
    fn menu() -> Vec<Action> {
        vec![
            Action::Status,
            Action::Start,
            Action::Stop,
            Action::Restart,
            Action::Install { version: None },
            Action::Upgrade,
            Action::Run,
            // 版本号在选中之后再问，这里只占位。
            Action::Rollback {
                version: String::new(),
            },
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
    // 必须在任何交互之前：inquire 不走 ui::color()，默认配色会在重定向时把 ANSI
    // 码漏进管道。
    ui::init_theme();
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
        Some(Command::Upgrade { version, check }) => selfup::run(version.as_deref(), check),
        Some(Command::About) => {
            about();
            Ok(())
        }
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

/// 先问一句「看全部，还是挑一个」。
///
/// service / tool 这两域有「不点名就是总览」的用法（`fundeploy service status`），
/// 而菜单从前直接跳到「选一个服务」，那条路在交互里根本走不到。
fn pick_overview(overview: &str) -> Result<bool> {
    let menu = [
        format!("status  {overview}"),
        "挑一个  装、起、停、查…".to_string(),
    ];
    let picked = ui::select("要做什么", &menu)?.ok_or_else(|| anyhow::anyhow!("已取消。"))?;
    Ok(picked.starts_with("status"))
}

// 下面这几张表是各域动作菜单的**唯一出处**: 菜单由它生成, 测试也拿它和 clap 的
// 子命令对照。写成「命令名 + 说明」两列而不是两份清单, 就不会出现「菜单里改了、
// 测试里忘了」这种漂移。

/// 换源这一域的动作。
const SOURCE_MENU: [(&str, &str); 7] = [
    ("status", "看当前配的是哪个"),
    ("list", "列出可用的源"),
    ("test", "测所有源的延迟"),
    ("use", "自动换到最快的"),
    ("reset", "恢复官方源"),
    ("add", "加一个自定义源"),
    ("rm", "删掉一个自定义源"),
];

/// AI 命令行那一域的动作。
const AI_MENU: [(&str, &str); 4] = [
    ("status", "装没装、装在哪儿、哪个版本"),
    ("install", "安装"),
    ("upgrade", "升到最新版"),
    ("uninstall", "卸载"),
];

/// 开发工具链那一域的动作。
const DEV_MENU: [(&str, &str); 4] = [
    ("status", "装没装、装在哪儿、哪个版本"),
    ("install", "安装"),
    ("upgrade", "升到最新版"),
    ("uninstall", "卸载"),
];

/// 技能仓库那一域的动作。
const SKILLS_MENU: [(&str, &str); 5] = [
    ("list", "查远端，列出每个仓库在本机的状态"),
    ("sync", "把缺的装上、已有的更新到最新"),
    ("install", "装一个"),
    ("update", "更新一个"),
    ("rm", "删掉一个"),
];

/// 把动作表渲染成菜单行，命令名按显示宽度对齐。
fn action_menu(items: &[(&str, &str)]) -> Vec<String> {
    let w = items.iter().map(|(k, _)| k.len()).max().unwrap_or(0) + 2;
    items
        .iter()
        .map(|(k, d)| format!("{}{d}", ui::pad(k, w)))
        .collect()
}

/// 先问「看全部，还是挑一个」。挑了就返回那个名字。
///
/// `overview` 是「不点名」那条路的说明——每一域都有这条路，而菜单从前要么直接
/// 跳进列表、要么直接执行默认动作，另一半在交互里走不到。
fn pick_name(names: Vec<String>, prompt: &str, overview: &str) -> Result<Option<String>> {
    let mut menu = vec![format!("（全部）  {overview}")];
    menu.extend(names);
    let picked = ui::select(prompt, &menu)?.ok_or_else(|| anyhow::anyhow!("已取消。"))?;
    if picked.starts_with("（全部）") {
        return Ok(None);
    }
    Ok(Some(
        picked.split_whitespace().next().unwrap_or("").to_string(),
    ))
}

fn pick_ai_action() -> Result<AiAction> {
    let menu = action_menu(&AI_MENU);
    let picked = ui::select("要做什么", &menu)?.ok_or_else(|| anyhow::anyhow!("已取消。"))?;
    // 装法不在交互里问：不给就是 official，要点名装法命令行上 `--method` 更顺手。
    let how = AiHow { method: None };
    Ok(match picked.split_whitespace().next().unwrap_or("status") {
        "install" => AiAction::Install {
            how,
            args: Vec::new(),
        },
        "upgrade" => AiAction::Upgrade { how },
        "uninstall" => AiAction::Uninstall { how, yes: false },
        _ => AiAction::Status,
    })
}

fn pick_dev_action() -> Result<DevAction> {
    let menu = action_menu(&DEV_MENU);
    let picked = ui::select("要做什么", &menu)?.ok_or_else(|| anyhow::anyhow!("已取消。"))?;
    let how = DevHow {
        method: None,
        version: None,
    };
    Ok(match picked.split_whitespace().next().unwrap_or("status") {
        "install" => DevAction::Install { how, force: false },
        "upgrade" => DevAction::Upgrade { how },
        "uninstall" => DevAction::Uninstall {
            how: DevHow {
                method: None,
                version: None,
            },
            yes: false,
        },
        _ => DevAction::Status,
    })
}

fn pick_skills_action() -> Result<SkillsAction> {
    let menu = action_menu(&SKILLS_MENU);
    let picked = ui::select("要做什么", &menu)?.ok_or_else(|| anyhow::anyhow!("已取消。"))?;
    let head = picked.split_whitespace().next().unwrap_or("list");
    // 这三个都要点名仓库，选中之后才问。
    let name = match head {
        "install" | "update" | "rm" => {
            let n = ui::input("仓库名", "")?;
            if n.is_empty() {
                bail!("没给仓库名。");
            }
            Some(n)
        }
        _ => None,
    };
    Ok(match head {
        "sync" => SkillsAction::Sync,
        "install" => SkillsAction::Install {
            name: name.unwrap(),
        },
        "update" => SkillsAction::Update {
            name: name.unwrap(),
        },
        "rm" => SkillsAction::Rm {
            name,
            all: false,
            yes: false,
        },
        _ => SkillsAction::List,
    })
}

/// GitHub 诊断那一域。`--fix` / `--undo` 以前在交互里走不到。
fn pick_doctor_action() -> Result<()> {
    let menu = [
        "诊断  只看，不改任何配置".to_string(),
        "修    按诊断结果自己挑一条改法".to_string(),
        "撤销  把之前改过的配置还回去".to_string(),
    ];
    let picked = ui::select("要做什么", &menu)?.ok_or_else(|| anyhow::anyhow!("已取消。"))?;
    match picked.split_whitespace().next().unwrap_or("诊断") {
        "修" => doctor::github::fix(doctor::github::Fix::Auto),
        "撤销" => doctor::github::undo(),
        _ => doctor::github::diagnose(),
    }
}

/// 自升级。默认只看不动手——菜单是随手点开的，真替换自己该是明确的决定。
fn pick_upgrade_action() -> Result<()> {
    let menu = [
        "check    只看有没有新版，不动手".to_string(),
        "upgrade  升上去".to_string(),
    ];
    let picked = ui::select("要做什么", &menu)?.ok_or_else(|| anyhow::anyhow!("已取消。"))?;
    let check = !picked.starts_with("upgrade");
    selfup::run(None, check)
}

fn pick_source_action() -> Result<SourceAction> {
    let menu = action_menu(&SOURCE_MENU);
    let picked = ui::select("要做什么", &menu)?.ok_or_else(|| anyhow::anyhow!("已取消。"))?;
    Ok(match picked.split_whitespace().next().unwrap_or("status") {
        "list" => SourceAction::List,
        "test" => SourceAction::Test,
        "use" => SourceAction::Use { mirror: None },
        "reset" => SourceAction::Reset,
        // 这两个要带值，选中之后才问。
        "add" => {
            let name = ui::input("源的名字", "")?;
            let url = ui::input("地址", "")?;
            if name.is_empty() || url.is_empty() {
                bail!("名字和地址都得给。");
            }
            // 备注是可选的，交互里不问——真要写，命令行上 `--label` 更方便。
            SourceAction::Add {
                name,
                url,
                label: None,
            }
        }
        "rm" => {
            let name = ui::input("要删哪个（自定义源的名字）", "")?;
            if name.is_empty() {
                bail!("没给名字。");
            }
            SourceAction::Rm { name }
        }
        _ => SourceAction::Status,
    })
}

/// 不带任何参数时。
///
/// 无人值守下必须打 help 然后正常退出，不能停在菜单上等输入——这条路径会被
/// 装机脚本和 CI 走到，卡住的表现是「安装脚本挂死」，比报错难查得多。
/// 无参数时那张菜单。每行的第一个词必须是一个真实的子命令名。
///
/// 和 clap 的 `Command` 是两份定义，天然会漂移——`upgrade` 和 `list` 就这么漏过
/// 一阵子：菜单里没有，`--help` 里有。所以提成常量，由
/// [`tests::菜单和命令两边都不许漏`] 双向钉住。
const MENU: [&str; 11] = [
    "service    服务（装、起、停、查）",
    "tool       工具（装、升、卸、查）",
    "source     换源（pip、npm、brew、cargo、go）",
    "port-kill  腾端口（查出谁占着，停掉它）",
    "doctor     诊断（GitHub 连不上）",
    "dev        开发工具链（go、nodejs、pnpm、rust、uv、python）",
    "ai         AI 命令行（claude、codex、cursor）",
    "skills     技能仓库（同步 Gitee 上的那批）",
    "upgrade    升级 fundeploy 自己",
    "list       列出能管的东西",
    "about      这是什么、谁做的",
];

fn no_args() -> Result<()> {
    if !ui::interactive() {
        Cli::command().print_help()?;
        println!();
        return Ok(());
    }
    // 只有「人主动打开菜单」这一条路径配得上一段 banner。
    ui::brand();
    ui::blank();
    let domains: Vec<String> = MENU.iter().map(|s| s.to_string()).collect();
    let picked = ui::select("要管什么", &domains)?.ok_or_else(|| anyhow::anyhow!("已取消。"))?;
    // 逐个列全、不留 `_ =>` 兜底: 兜底会把「菜单加了项却忘了加分支」变成「选了
    // upgrade 却跳出工具列表」, 不报错也查不出来。
    match picked.split_whitespace().next().unwrap_or("") {
        // 先问「看全部」还是「挑一个」：不点名服务的 status（所有服务的状态总览）
        // 只有这条路能走到，否则它在交互里等于不存在。
        "service" => match pick_overview("所有服务的状态总览")? {
            true => {
                status_all();
                Ok(())
            }
            false => {
                let svc = pick_one(service::registry::all(), "选一个服务", |s| {
                    (s.name().to_string(), s.summary().to_string())
                })?;
                pick_and_run(svc.as_ref())
            }
        },
        "tool" => match pick_overview("所有工具的状态")? {
            true => {
                tool_status_all();
                Ok(())
            }
            false => {
                let t = pick_one(tool::registry::all(), "选一个工具", |t| {
                    (t.name().to_string(), t.summary().to_string())
                })?;
                pick_and_run_tool(t.as_ref())
            }
        },
        "source" => {
            let s = pick_one(source::registry::all(), "给哪个换源", |s| {
                (s.name().to_string(), s.summary().to_string())
            })?;
            let action = pick_source_action()?;
            source_cmd(Some(s.name().to_string()), Some(action))
        }
        "port-kill" => {
            // `--list` 那条路（只看谁占着、不动手）以前在交互里走不到。
            let menu = [
                "kill  查出谁占着，停掉它".to_string(),
                "list  只看谁占着，不动手".to_string(),
            ];
            let how = ui::select("要做什么", &menu)?.ok_or_else(|| anyhow::anyhow!("已取消。"))?;
            let line = ui::input("端口号（多个用空格隔开）", "")?;
            let ports = port_kill::parse_ports(&line)?;
            if how.starts_with("list") {
                port_kill::list(&ports)
            } else {
                port_kill::kill(&ports, false)
            }
        }
        "doctor" => pick_doctor_action(),
        "dev" => {
            let name = pick_name(dev::registry::names(), "哪个工具链", "全部的状态")?;
            match name {
                None => dev_cmd(None, None),
                Some(n) => {
                    let action = pick_dev_action()?;
                    dev_cmd(Some(n), Some(action))
                }
            }
        }
        "ai" => {
            // `all` 是 ai_cmd 认的特殊名字，一次对所有 AI 命令行执行。
            let mut names = vec!["all".to_string()];
            names.extend(ai::registry::names());
            let name = pick_name(names, "哪个 AI 命令行", "都列一遍")?;
            match name {
                None => ai::list(),
                Some(n) => {
                    let action = pick_ai_action()?;
                    ai_cmd(Some(n), Some(action))
                }
            }
        }
        "skills" => {
            let action = pick_skills_action()?;
            skills_cmd(Some(action))
        }
        "upgrade" => pick_upgrade_action(),
        "list" => {
            list_all();
            Ok(())
        }
        "about" => {
            about();
            Ok(())
        }
        other => bail!("菜单项 {other} 没有对应的处理分支，这是 fundeploy 自己的 bug"),
    }
}

fn list_all() {
    // 一律先 ui::pad 再上色: pad 算的是终端显示宽度, 带着 ANSI 转义码进去, 那些
    // 不可见的字节会被当成列宽, 整张表歪得莫名其妙。
    ui::heading("服务");
    for s in service::registry::all() {
        let ports: Vec<String> = s.ports().iter().map(u16::to_string).collect();
        println!(
            "  {}{}{}",
            ui::strong(ui::pad(s.name(), 16)),
            s.summary(),
            ui::muted(format!("（端口 {}）", ports.join("、")))
        );
    }
    ui::blank();
    ui::heading("工具");
    for t in tool::registry::all() {
        println!("  {}{}", ui::strong(ui::pad(t.name(), 16)), t.summary());
    }
    ui::blank();
    ui::heading("可换源");
    for s in source::registry::all() {
        println!("  {}{}", ui::strong(ui::pad(s.name(), 16)), s.summary());
    }
}

/// `fundeploy about`：这是什么、谁做的、去哪儿找。
///
/// 内容和 README 的「关于 farfarfun」一节对齐。fundeploy 目前基本只在自己人里
/// 用，这条命令是顺手把组织带出去的地方。
fn about() {
    ui::brand();
    ui::blank();
    ui::info("专注实用工具库的开源组织，涵盖云存储、数据处理、AI、多媒体与开发工具链。");
    ui::blank();
    ui::Card::new("farfarfun")
        .row("组织主页", "https://github.com/farfarfun")
        .row("PyPI", "https://pypi.org/user/niuliangtao/")
        .row("联系", "farfarfun@qq.com")
        .row("本项目", "https://github.com/farfarfun/fundeploy")
        .row("许可", "MIT")
        .print();
    ui::blank();
    ui::hint("能管什么: fundeploy list");
}

fn list_tools() {
    ui::heading("工具");
    for t in tool::registry::all() {
        println!("  {}{}", ui::strong(ui::pad(t.name(), 16)), t.summary());
    }
}

/// 所有服务的状态总览（`fundeploy service status`）。
///
/// 只探端口、不碰 PID 文件：这条命令的用处是「扫一眼这台机器上什么在跑」，得快。
/// 单个服务的细节（PID、版本、日志路径）归 `fundeploy service <服务> status`。
///
/// 端口被占不等于「我们起的那份在跑」——别人的进程也可能正好坐在同一个端口上。
/// 这里刻意不去分辨：总览回答的是「这个口通不通」，认归属是单服务 status 的事。
fn status_all() {
    ui::heading("服务状态");
    for s in service::registry::all() {
        let ports = s.ports();
        let up = ports.iter().filter(|&&p| port::is_taken(p)).count();
        let (kind, label) = match (up, ports.len()) {
            (_, 0) => (ui::State::Idle, "没有端口".to_string()),
            (0, _) => (ui::State::Idle, "没在跑".to_string()),
            (u, n) if u == n => (ui::State::Up, "在跑".to_string()),
            (u, n) => (ui::State::Partial, format!("部分 {u}/{n}")),
        };
        let list: Vec<String> = ports.iter().map(u16::to_string).collect();
        println!(
            "  {}{}{}",
            ui::strong(ui::pad(s.name(), 16)),
            ui::muted(ui::pad(&list.join(" "), 22)),
            ui::state(kind, label)
        );
    }
    ui::blank();
    ui::hint("细节: fundeploy service <服务> status");
}

/// 所有工具的状态（`fundeploy tool status`）。
///
/// 工具没有端口可探，[`Tool`] 也只给了会自己打印的 `status()`，所以这里逐个调。
/// 一个工具查不了不该挡住后面的——这条命令是来看全局的。
///
/// 不在这儿补标题：每个工具的 `status()` 自己就会打一个，再加一层会变成重复两行。
fn tool_status_all() {
    for (i, t) in tool::registry::all().iter().enumerate() {
        if i > 0 {
            ui::blank();
        }
        if let Err(e) = t.status() {
            ui::warn(format!("{} 查不了: {e}", t.name()));
        }
    }
}

fn service_cmd(name: Option<String>, action: Option<Action>) -> Result<()> {
    let Some(name) = name else {
        // 不给服务名时 clap 会把 `fundeploy service status` 里的 status 解析成
        // action。以前这儿只看 name，解析出来的 action 被整个丢掉——给了动作却
        // 一声不吭地打出一张清单，退出码还是 0。
        return match action {
            Some(Action::Status) => {
                status_all();
                Ok(())
            }
            // 其余动作一律拒绝而不是对所有服务来一遍：`service stop` 看着像
            // 「停一下」，真跑起来是把这台机器上十几个服务全停了。
            Some(_) => bail!(
                "这个动作要点名服务: fundeploy service <服务> <动作>\n\
                 可选: {}\n\
                 不点名只支持 status（所有服务的状态总览）。",
                service::registry::names().join("、")
            ),
            None => {
                list_all();
                Ok(())
            }
        };
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
        // 同 service_cmd：`fundeploy tool status` 的 status 会被解析成 action。
        return match action {
            Some(ToolAction::Status) => {
                tool_status_all();
                Ok(())
            }
            Some(_) => bail!(
                "这个动作要点名工具: fundeploy tool <工具> <动作>\n\
                 可选: {}\n\
                 不点名只支持 status。",
                tool::registry::names().join("、")
            ),
            None => {
                list_tools();
                Ok(())
            }
        };
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
        // 带必填参数的动作，菜单里只占了个位，这会儿才问。
        Some(a) if a.needs_input() => {
            let version = ui::input("回退到哪个版本（例如 1.1.109）", "")?;
            if version.is_empty() {
                bail!("没给版本号。");
            }
            Action::Rollback { version }.apply(svc)
        }
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
    fn 菜单和命令两边都不许漏() {
        // 无参数菜单是手写的第二份命令清单，和 clap 的定义会漂移。双向断言:
        // 漏一边就红。`upgrade` 和 `list` 曾经只在 --help 里有、菜单里没有。
        let commands: Vec<String> = Cli::command()
            .get_subcommands()
            // help 是 clap 自动生成的，不该出现在菜单上。
            .filter(|c| c.get_name() != "help")
            .map(|c| c.get_name().to_string())
            .collect();

        for item in MENU {
            let head = item.split_whitespace().next().unwrap_or("");
            assert!(
                commands.contains(&head.to_string()),
                "菜单项 {head} 不是真实命令；可选: {commands:?}"
            );
        }
        for name in &commands {
            assert!(
                MENU.iter().any(|m| m.starts_with(name.as_str())),
                "命令 {name} 没进无参数菜单"
            );
        }
    }

    #[test]
    fn 不点名时的_status_是动作而不是服务名() {
        // 这是 `fundeploy service status` 那个回归的根: clap 把 status 解析成
        // action、name 留空，而旧代码只看 name，于是把 action 整个丢掉、打了张
        // 清单还返回 0。
        let cli = Cli::try_parse_from(["fundeploy", "service", "status"]).unwrap();
        match cli.command {
            Some(Command::Service { name, action }) => {
                assert!(name.is_none(), "status 不该被当成服务名");
                assert!(
                    matches!(action, Some(Action::Status)),
                    "status 没被解析成 Status 动作"
                );
            }
            _ => panic!("fundeploy service status 没被解析成 Service 命令"),
        }
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

    /// 菜单项形如 `install  安装`，取前面那个命令名。
    fn 首词(s: &str) -> String {
        s.split_whitespace().next().unwrap_or("").to_string()
    }

    #[test]
    fn 交互菜单列全了每个域的动作() {
        // 每个域的动作菜单都是手写的第二份清单，和 clap 的定义天然会漂移。漏掉的
        // 动作在交互里等于不存在——而菜单正是「不知道有哪些命令」时唯一能翻的地方。
        // 已经漏过：service 的 rollback、source 的 add/rm，以及 ai / dev / skills
        // 整域（菜单从前直接跳列表，一个动作都到不了）。
        for (域, 菜单) in [
            ("source", &SOURCE_MENU[..]),
            ("ai", &AI_MENU[..]),
            ("dev", &DEV_MENU[..]),
            ("skills", &SKILLS_MENU[..]),
        ] {
            let mut mine: Vec<String> = 菜单.iter().map(|(k, _)| k.to_string()).collect();
            mine.sort();
            assert_eq!(子命令(域), mine, "{域} 的交互菜单和 clap 对不上");
        }
    }

    #[test]
    fn 服务和工具的动作菜单不漏项() {
        // 和下面那条的区别：STANDARD / TOOL_STANDARD 守的是「拼错子命令时的报错
        // 文案」，这条守的是**交互里真正列出来的那些**。rollback 曾经只在前者里
        // 有——报错认得它，菜单里却找不到。
        let mut mine: Vec<String> = Action::menu().iter().map(|a| 首词(&a.label())).collect();
        mine.sort();
        assert_eq!(子命令("service"), mine, "service 的动作菜单漏项了");

        let mut mine: Vec<String> = ToolAction::menu()
            .iter()
            .map(|a| 首词(&a.label()))
            .collect();
        mine.sort();
        assert_eq!(子命令("tool"), mine, "tool 的动作菜单漏项了");
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
    fn 菜单里要补参数的动作只有_rollback() {
        // 菜单选完就执行，带必填参数的动作混进来会拿着空值一路走到底。rollback 是
        // 唯一的例外: pick_and_run 看 needs_input() 把它分流出去, 先问版本号、空的
        // 就报错。这条断言的是那份名单本身——改了 needs_input 却忘了在
        // pick_and_run 里接一下, 这里会红。
        let 要补: Vec<String> = Action::menu()
            .iter()
            .filter(|a| a.needs_input())
            .map(|a| 首词(&a.label()))
            .collect();
        assert_eq!(要补, ["rollback"]);
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
