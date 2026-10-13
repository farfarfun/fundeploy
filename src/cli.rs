//! 命令行解析与分发。

use anyhow::{bail, Result};
use clap::{CommandFactory, Parser, Subcommand};

use crate::service::{self, Service};
use crate::ui;

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
    /// 列出能管的东西
    List,
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

/// 执行一个服务声明的额外动作。
///
/// 名字先对着 [`Service::extras`] 核一遍再往下传：clap 的 external_subcommand 会
/// 原样收下任何没见过的词，不核的话 `statsu` 这种手滑会一路传到服务里，变成一条
/// 拿拼错的子命令去调上游 CLI 的调用——报错也就成了上游的用法提示，与手滑无关。
fn run_extra(svc: &dyn Service, argv: &[String]) -> Result<()> {
    let Some((name, args)) = argv.split_first() else {
        bail!(
            "没给动作名。看全部动作: fundeploy service {} --help",
            svc.name()
        );
    };
    let extras = svc.extras();
    if !extras.iter().any(|e| e.name == name) {
        let known: Vec<&str> = STANDARD
            .iter()
            .copied()
            .chain(extras.iter().map(|e| e.name))
            .collect();
        bail!(
            "{} 没有 {name} 这个动作。可选: {}",
            svc.name(),
            known.join("、")
        );
    }
    svc.extra(name, args)
}

pub fn cli_main() -> Result<()> {
    match Cli::parse().command {
        Some(Command::List) => {
            list_services();
            Ok(())
        }
        Some(Command::Service { name, action }) => service_cmd(name, action),
        None => no_args(),
    }
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
    let svc = pick_service()?;
    pick_and_run(svc.as_ref())
}

fn list_services() {
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
}

fn service_cmd(name: Option<String>, action: Option<Action>) -> Result<()> {
    let Some(name) = name else {
        list_services();
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

fn pick_service() -> Result<Box<dyn Service>> {
    let all = service::registry::all();
    let labels: Vec<String> = all
        .iter()
        .map(|s| format!("{:<14} {}", s.name(), s.summary()))
        .collect();

    let picked = ui::select("选一个服务", &labels)?.ok_or_else(|| anyhow::anyhow!("已取消。"))?;
    let idx = labels
        .iter()
        .position(|l| l == picked)
        .ok_or_else(|| anyhow::anyhow!("已取消。"))?;
    Ok(all.into_iter().nth(idx).unwrap())
}

fn pick_and_run(svc: &dyn Service) -> Result<()> {
    if !ui::interactive() {
        let mut msg = format!(
            "要做什么？例如 `fundeploy service {0} status`（看全部动作: `fundeploy service {0} --help`）",
            svc.name()
        );
        // 额外动作不在 --help 里（clap 不认识它们），不在这儿说就等于没有。
        if !svc.extras().is_empty() {
            let names: Vec<&str> = svc.extras().iter().map(|e| e.name).collect();
            msg.push_str(&format!("\n{} 还支持: {}", svc.name(), names.join("、")));
        }
        bail!(msg);
    }

    let actions = Action::menu();
    let mut labels: Vec<String> = actions.iter().map(Action::label).collect();
    // 额外动作跟标准动作并列进菜单：对使用者来说它们同样是「这个服务能做的事」。
    labels.extend(
        svc.extras()
            .iter()
            .map(|e| format!("{:<9}{}", e.name, e.about)),
    );

    let picked = ui::select(&format!("{} 要做什么", svc.name()), &labels)?
        .ok_or_else(|| anyhow::anyhow!("已取消。"))?;
    let idx = labels.iter().position(|l| l == picked).unwrap();
    match actions.get(idx) {
        Some(a) => a.apply(svc),
        // 越过标准动作的下标落在 extras 上，顺序跟上面拼 labels 时一致。
        None => svc.extra(svc.extras()[idx - actions.len()].name, &[]),
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

    #[test]
    fn 标准动作名单和_clap_不脱节() {
        // STANDARD 是手抄的，加了动作忘了抄就会在报错里漏掉它。
        let cmd = Cli::command();
        let svc = cmd.find_subcommand("service").expect("没有 service 子命令");
        let mut from_clap: Vec<&str> = svc
            .get_subcommands()
            .map(|c| c.get_name())
            .filter(|n| *n != "help")
            .collect();
        from_clap.sort_unstable();
        let mut mine = STANDARD.to_vec();
        mine.sort_unstable();
        assert_eq!(from_clap, mine);
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
    fn 不存在的服务名报错里列出可选项() {
        let err = service_cmd(Some("没这个服务".into()), Some(Action::Status))
            .unwrap_err()
            .to_string();
        assert!(err.contains("funread-web"), "实际: {err}");
    }
}
