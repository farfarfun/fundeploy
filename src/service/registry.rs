//! 服务声明。
//!
//! 一个服务通常就是一份 [`ServiceSpec`]，生命周期全部由 [`Service`] 的默认实现
//! 提供。只有真有个性的服务（见 [`funread_web`]）才需要自己的类型去覆盖钩子。
//!
//! 所有端口、地址、包名、版本都可以用环境变量盖掉，变量名的规律是
//! `<服务名大写>_<组件名大写>_<字段>`，例如 `FUNREAD_WEB_BACKEND_PORT`。

use anyhow::Result;
use std::path::PathBuf;

use super::config;
use super::spec::{Arg, CliShape, Component, Pkg, ServiceSpec};
use super::{Declared, Service};
use crate::args;
use crate::ui;

pub fn all() -> Vec<Box<dyn Service>> {
    vec![
        Box::new(Declared(funfluid_web())),
        Box::new(Declared(funlesson_web())),
        Box::new(Declared(fungame_web())),
        Box::new(Declared(funflix_web())),
        Box::new(funread_web()),
    ]
}

pub fn names() -> Vec<String> {
    all().iter().map(|s| s.name().to_string()).collect()
}

pub fn find(name: &str) -> Option<Box<dyn Service>> {
    all().into_iter().find(|s| s.name() == name)
}

// ---------------------------------------------------------------- 环境变量

fn env_str(key: &str, default: &str) -> String {
    match std::env::var(key) {
        Ok(v) if !v.trim().is_empty() => v,
        _ => default.to_string(),
    }
}

fn env_opt(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

/// 读端口。值不合法时**用默认值并明确报出来**——静默回退的话，操作者会盯着
/// 一个他以为改了的端口，而服务起在另一个端口上。
fn env_port(key: &str, default: u16) -> u16 {
    let Some(raw) = env_opt(key) else {
        return default;
    };
    match raw.trim().parse::<u16>() {
        Ok(p) if p > 0 => p,
        _ => {
            ui::warn(format!(
                "{key}={raw} 不是合法端口，按默认值 {default} 处理。"
            ));
            default
        }
    }
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

// ---------------------------------------------------------------- funfluid-web

fn funfluid_web() -> ServiceSpec {
    // 后端的子命令带一个 service 位置参数（gunicorn / celery / …），这里只管
    // gunicorn 那一个。
    let service = env_str("FUNFLUID_WEB_BACKEND_SERVICE", "gunicorn");

    let backend = Component::new(
        "backend",
        "后端 funfluid-api",
        Pkg::pip(
            &env_str("FUNFLUID_WEB_BACKEND_PACKAGE", "funfluid-api"),
            "funfluid-api",
        ),
        &env_str("FUNFLUID_WEB_BACKEND_HOST", "127.0.0.1"),
        env_port("FUNFLUID_WEB_BACKEND_PORT", 18806),
    )
    .version(env_opt("FUNFLUID_WEB_BACKEND_VERSION"))
    .cli_shape(
        CliShape::new()
            .start(args!["start", service.clone(), "-d"])
            .run(args!["run", service.clone()])
            // 停的目标必须是 all。funfluid-api 自己守着一个 watch 循环，只有
            // 目标为 all 时才会把那个循环一起停掉；否则循环会在半分钟后把服务
            // 重新拉起来，表现为「停了又活」。
            .stop(args!["stop", "all"])
            .status(args!["status", service.clone()]),
    );

    let frontend = Component::new(
        "frontend",
        "前端 funfluid-web",
        Pkg::npm(
            &env_str("FUNFLUID_WEB_FRONTEND_PACKAGE", "@farfarfun/funfluid-web"),
            "funfluid-web",
        ),
        &env_str("FUNFLUID_WEB_FRONTEND_HOST", "127.0.0.1"),
        env_port("FUNFLUID_WEB_FRONTEND_PORT", 8806),
    )
    .version(env_opt("FUNFLUID_WEB_FRONTEND_VERSION"))
    .cli_shape(
        CliShape::new()
            .start(args![
                "start",
                "--port",
                Arg::Port,
                "--host",
                Arg::Host,
                "-d"
            ])
            .run(args!["run", "--port", Arg::Port, "--host", Arg::Host])
            .stop(args!["stop"])
            .status(args!["status"]),
    );

    ServiceSpec::new("funfluid-web", "funfluid 前后端", vec![backend, frontend]).entry("frontend")
}

// ---------------------------------------------------------------- funlesson-web

fn funlesson_web() -> ServiceSpec {
    let backend = Component::new(
        "backend",
        "后端 funlesson-api",
        Pkg::pip(
            &env_str("FUNLESSON_WEB_BACKEND_PACKAGE", "funlesson-api"),
            "funlesson-api",
        ),
        &env_str("FUNLESSON_WEB_BACKEND_HOST", "127.0.0.1"),
        env_port("FUNLESSON_WEB_BACKEND_PORT", 18812),
    )
    .version(env_opt("FUNLESSON_WEB_BACKEND_VERSION"))
    .cli_shape(server_subcommand_shape(vec![]));

    let frontend = Component::new(
        "frontend",
        "前端 funlesson-web",
        Pkg::npm(
            &env_str("FUNLESSON_WEB_FRONTEND_PACKAGE", "funlesson-web"),
            "funlesson-web",
        ),
        &env_str("FUNLESSON_WEB_FRONTEND_HOST", "127.0.0.1"),
        env_port("FUNLESSON_WEB_FRONTEND_PORT", 8812),
    )
    .version(env_opt("FUNLESSON_WEB_FRONTEND_VERSION"))
    .cli_shape(server_subcommand_shape(args![
        "--backend",
        Arg::SiblingUrl("backend".into())
    ]));

    ServiceSpec::new("funlesson-web", "funlesson 前后端", vec![backend, frontend]).entry("frontend")
}

// ---------------------------------------------------------------- fungame-web

fn fungame_web() -> ServiceSpec {
    let backend = Component::new(
        "backend",
        "后端 fungame-backend",
        Pkg::pip(
            &env_str("FUNGAME_WEB_BACKEND_PACKAGE", "fungame-backend"),
            "fungame-backend",
        ),
        &env_str("FUNGAME_WEB_BACKEND_HOST", "127.0.0.1"),
        env_port("FUNGAME_WEB_BACKEND_PORT", 8809),
    )
    .version(env_opt("FUNGAME_WEB_BACKEND_VERSION"))
    .cli_shape(server_subcommand_shape(vec![]));

    // B 端和 C 端都是独立的前端进程，各自连同一个后端。
    let upstream = args![
        "--api-upstream-host",
        Arg::SiblingHost("backend".into()),
        "--api-upstream-port",
        Arg::SiblingPort("backend".into())
    ];

    let admin = Component::new(
        "admin",
        "管理端 fungame-admin",
        Pkg::npm(
            &env_str("FUNGAME_WEB_ADMIN_PACKAGE", "@fungame/admin-cli"),
            "fungame-admin",
        ),
        &env_str("FUNGAME_WEB_ADMIN_HOST", "127.0.0.1"),
        env_port("FUNGAME_WEB_ADMIN_PORT", 8807),
    )
    .version(env_opt("FUNGAME_WEB_ADMIN_VERSION"))
    .cli_shape(server_subcommand_shape(upstream.clone()));

    let client = Component::new(
        "client",
        "用户端 fungame-client",
        Pkg::npm(
            &env_str("FUNGAME_WEB_CLIENT_PACKAGE", "@fungame/client-cli"),
            "fungame-client",
        ),
        &env_str("FUNGAME_WEB_CLIENT_HOST", "127.0.0.1"),
        env_port("FUNGAME_WEB_CLIENT_PORT", 8808),
    )
    .version(env_opt("FUNGAME_WEB_CLIENT_VERSION"))
    .cli_shape(server_subcommand_shape(upstream));

    ServiceSpec::new(
        "fungame-web",
        "fungame 后端 + 管理端 + 用户端",
        vec![backend, admin, client],
    )
    .entry("client")
    .note("管理端在上面那个 admin 的地址上，和用户端是两个进程。")
}

// ---------------------------------------------------------------- funflix-web

fn funflix_web() -> ServiceSpec {
    let backend_cfg = env_opt("FUNFLIX_WEB_BACKEND_CONFIG");
    let frontend_cfg = env_opt("FUNFLIX_WEB_FRONTEND_CONFIG");

    let mut backend_start = args!["--host", Arg::Host, "--port", Arg::Port];
    if let Some(c) = &backend_cfg {
        backend_start.extend(args!["--config", c.clone()]);
    }
    let mut frontend_start = args![
        "--host",
        Arg::Host,
        "--port",
        Arg::Port,
        "--backend",
        Arg::SiblingUrl("backend".into())
    ];
    if let Some(c) = &frontend_cfg {
        frontend_start.extend(args!["--config", c.clone()]);
    }

    let backend = Component::new(
        "backend",
        "后端 funflix-api",
        Pkg::pip(
            &env_str("FUNFLIX_WEB_BACKEND_PACKAGE", "funflix-api"),
            "funflix-api",
        ),
        &env_str("FUNFLIX_WEB_BACKEND_HOST", "127.0.0.1"),
        env_port("FUNFLIX_WEB_BACKEND_PORT", 18810),
    )
    .version(env_opt("FUNFLIX_WEB_BACKEND_VERSION"))
    .cli_shape(server_shape_with(backend_start, vec![], vec![]));

    let frontend = Component::new(
        "frontend",
        "前端 funflix-web",
        Pkg::npm(
            &env_str("FUNFLIX_WEB_FRONTEND_PACKAGE", "@farfarfun/funflix-web"),
            "funflix-web",
        ),
        &env_str("FUNFLIX_WEB_FRONTEND_HOST", "127.0.0.1"),
        env_port("FUNFLIX_WEB_FRONTEND_PORT", 8810),
    )
    .version(env_opt("FUNFLIX_WEB_FRONTEND_VERSION"))
    .cli_shape(server_shape_with(frontend_start, vec![], vec![]));

    ServiceSpec::new("funflix-web", "funflix 前后端", vec![backend, frontend]).entry("frontend")
}

// ---------------------------------------------------------------- funread-web

pub fn funread_web() -> FunreadWeb {
    let backend_cfg = env_opt("FUNREAD_WEB_BACKEND_CONFIG").map(PathBuf::from);
    let frontend_cfg = env_opt("FUNREAD_WEB_FRONTEND_CONFIG").map(PathBuf::from);

    let backend_port = env_port("FUNREAD_WEB_BACKEND_PORT", 18811);
    let frontend_port = env_port("FUNREAD_WEB_FRONTEND_PORT", 8811);

    let mut backend_start = args!["--host", Arg::Host, "--port", Arg::Port];
    // stop/status 不接受 --host（上游只给了 --config 和 --port）：--config 用来定位
    // PID 文件，--port 是 PID 文件丢了之后的兜底，两个都要传。
    let mut backend_addr = args!["--port", Arg::Port];
    if let Some(c) = &backend_cfg {
        let c = c.to_string_lossy().into_owned();
        backend_start.extend(args!["--config", c.clone()]);
        backend_addr.extend(args!["--config", c]);
    }

    let mut frontend_start = args![
        "--host",
        Arg::Host,
        "--port",
        Arg::Port,
        "--backend",
        Arg::SiblingUrl("backend".into())
    ];
    let mut frontend_addr = args!["--port", Arg::Port];
    if let Some(c) = &frontend_cfg {
        let c = c.to_string_lossy().into_owned();
        frontend_start.extend(args!["--config", c.clone()]);
        frontend_addr.extend(args!["--config", c]);
    }

    let backend = Component::new(
        "backend",
        "后端 funread-api",
        Pkg::pip(
            &env_str("FUNREAD_WEB_BACKEND_PACKAGE", "funread-api"),
            "funread-api",
        ),
        &env_str("FUNREAD_WEB_BACKEND_HOST", "127.0.0.1"),
        backend_port,
    )
    .version(env_opt("FUNREAD_WEB_BACKEND_VERSION"))
    .cli_shape(server_shape_with(
        backend_start,
        backend_addr.clone(),
        backend_addr,
    ));

    let frontend = Component::new(
        "frontend",
        "前端 funread-web",
        Pkg::npm(
            &env_str("FUNREAD_WEB_FRONTEND_PACKAGE", "@farfarfun/funread-web"),
            "funread-web",
        ),
        &env_str("FUNREAD_WEB_FRONTEND_HOST", "127.0.0.1"),
        frontend_port,
    )
    .version(env_opt("FUNREAD_WEB_FRONTEND_VERSION"))
    .cli_shape(server_shape_with(
        frontend_start,
        frontend_addr.clone(),
        frontend_addr,
    ));

    let spec = ServiceSpec::new("funread-web", "funread 前后端", vec![backend, frontend])
        .entry_at("frontend", "/web")
        .note("管理端在 /admin。")
        .note("还没有账号就先: funread-api accounts create-admin <用户名>");

    FunreadWeb {
        spec,
        backend_cfg,
        frontend_cfg,
    }
}

/// funread-web 要在启动前把配置文件准备好，所以有自己的类型。
pub struct FunreadWeb {
    spec: ServiceSpec,
    backend_cfg: Option<PathBuf>,
    frontend_cfg: Option<PathBuf>,
}

impl FunreadWeb {
    fn shared_config_path(&self) -> PathBuf {
        home().join(".farfarfun/funread/config.toml")
    }

    /// 前后端共用的那份根配置。
    ///
    /// funread 1.1.116 起，库地址和管理口令只从这个文件读，没有环境变量可走。
    /// 生成的是一份**整体注释掉的模板**：这台机器该连哪个库不是安装脚本能替人
    /// 决定的事，写一个默认值进去只会让人在连错库之后才发现。
    fn ensure_shared_config(&self) -> Result<()> {
        let path = self.shared_config_path();
        let body = r#"# funread 的共享根配置，由 fundeploy 首次安装时生成。
# 前后端与核心库管线都读这一份；各服务目录下的 config.toml 优先级更高。
# 解析顺序：命令行参数 > 服务自己的 config.toml > 本文件 > funsecret > 兜底。
#
# 下面整份是注释，等于「这台机器没有特殊配置」。要用的话去掉对应的 # 即可。

# [database]
# 不填就按 funsecret 的 funread/cache/source/db_url > 本地 SQLite 解析。
# url = "postgresql+psycopg2://user:password@host/dbname"

# [auth]
# password        管理端（/admin）口令。不填＝管理端无鉴权全开。
# session_secret  会话签名密钥。不填会在 api 的配置目录下自动生成一个。
# reader_public   只读的阅读端 GET 是否免鉴权，默认 false。
# register_open   注册入口开不开，默认 true（闸门是邀请码）。
# password = "..."
# reader_public = false
# register_open = true
"#;
        match config::write_if_absent(&path, body) {
            Ok(true) => ui::step(format!(
                "已生成共享配置: {}（库地址和口令写这里）",
                path.display()
            )),
            Ok(false) => {}
            // 写不出来不该挡住启动：库地址也可能是从 funsecret 来的。
            Err(e) => ui::warn(format!("{e}（库地址和口令要写在这里）")),
        }
        Ok(())
    }

    /// 单个组件自己的 config.toml。
    ///
    /// 上游的 `server start` 要求这个文件存在，缺了只报一行 config file not found，
    /// 内容本身可以是空壳。
    fn ensure_component_config(&self, key: &str, explicit: &Option<PathBuf>, fallback: &str) {
        // 操作者点名了配置文件，就由他自己负责，不去生成。
        if explicit.is_some() {
            return;
        }
        let Some(c) = self.spec.component(key) else {
            return;
        };

        let dir = config::cli_config_dir(c.pkg.cli(), &home().join(fallback));
        let path = dir.join("config.toml");
        let shared = self.shared_config_path();
        let body = format!(
            "# 由 fundeploy 生成（{label}）。funread 的 server start 要求这个文件存在，\n\
             # 缺了只会报一行 config file not found —— 内容本身可以是空壳。\n\
             # host/port 以 fundeploy 的 FUNREAD_WEB_* 环境变量为准：start 时会用显式\n\
             # --host/--port 盖掉这里的值，改监听地址请改环境变量，不是改这个文件。\n\
             #\n\
             # 这份只放本服务自己的设置。库地址、口令那些两端共用的，写在上一层的\n\
             # {shared} 里（本文件优先级更高，这里写了就盖掉它）。\n\
             [server]\n\
             host = \"{host}\"\n\
             port = {port}\n",
            label = c.label,
            shared = shared.display(),
            host = c.host,
            port = c.port,
        );

        match config::write_if_absent(&path, &body) {
            Ok(true) => ui::step(format!("已生成 {} 的配置: {}", c.label, path.display())),
            Ok(false) => {}
            Err(e) => ui::warn(format!(
                "{e}，{} 的 server start 可能仍会报 config file not found",
                c.label
            )),
        }
    }
}

impl Service for FunreadWeb {
    fn spec(&self) -> &ServiceSpec {
        &self.spec
    }

    fn ensure_config(&self) -> Result<()> {
        self.ensure_shared_config()?;
        self.ensure_component_config("backend", &self.backend_cfg, ".farfarfun/funread/api");
        self.ensure_component_config("frontend", &self.frontend_cfg, ".farfarfun/funread/web");
        Ok(())
    }

    fn post_install(&self) -> Result<()> {
        // 装完就把根配置放好，让人能在启动前先去填库地址。
        self.ensure_shared_config()
    }
}

// ---------------------------------------------------------------- 形态模板

/// 动作挂在 `server` 子组下、start/run 接受 `--host/--port` 的形态。
/// `extra` 是追加在 start/run 后面的参数（比如 `--backend <url>`）。
fn server_subcommand_shape(extra: Vec<Arg>) -> CliShape {
    let mut addr = args!["--host", Arg::Host, "--port", Arg::Port];
    addr.extend(extra);
    server_shape_with(addr, vec![], vec![])
}

/// 同上，但 start/run、stop、status 的参数各自给定。
fn server_shape_with(start: Vec<Arg>, stop: Vec<Arg>, status: Vec<Arg>) -> CliShape {
    let mut start_args = args!["server", "start"];
    start_args.extend(start.clone());
    let mut run_args = args!["server", "run"];
    run_args.extend(start);
    let mut stop_args = args!["server", "stop"];
    stop_args.extend(stop);
    let mut status_args = args!["server", "status"];
    status_args.extend(status);

    CliShape::new()
        .start(start_args)
        .run(run_args)
        .stop(stop_args)
        .status(status_args)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(spec: &ServiceSpec, key: &str, pick: fn(&CliShape) -> &Vec<Arg>) -> Vec<String> {
        let c = spec.component(key).unwrap();
        let mut v = c.render(pick(&c.cli), &spec.components).unwrap();
        v.insert(0, c.pkg.cli().to_string());
        v
    }

    #[test]
    fn funfluid_后端停的是_all() {
        // 不带 all 的话，funfluid-api 自己那个 watch 循环不会停，半分钟后会把
        // 服务重新拉起来——表现为「停了又活」，而 stop 报的是成功。
        let spec = funfluid_web();
        assert_eq!(
            argv(&spec, "backend", |c| &c.stop),
            ["funfluid-api", "stop", "all"]
        );
    }

    #[test]
    fn funfluid_后端带_service_位置参数而不是地址参数() {
        let spec = funfluid_web();
        let start = argv(&spec, "backend", |c| &c.start);
        assert_eq!(start, ["funfluid-api", "start", "gunicorn", "-d"]);
        assert!(!start.iter().any(|a| a == "--port"), "上游不接受 --port");
    }

    #[test]
    fn funfluid_前端带守护参数() {
        let spec = funfluid_web();
        assert_eq!(
            argv(&spec, "frontend", |c| &c.start),
            [
                "funfluid-web",
                "start",
                "--port",
                "8806",
                "--host",
                "127.0.0.1",
                "-d"
            ]
        );
        // 前台形态不能带 -d，否则 run 会立刻返回、把后端留成孤儿。
        assert!(!argv(&spec, "frontend", |c| &c.run)
            .iter()
            .any(|a| a == "-d"));
    }

    #[test]
    fn funlesson_前端连得上后端() {
        let spec = funlesson_web();
        assert_eq!(
            argv(&spec, "frontend", |c| &c.start),
            [
                "funlesson-web",
                "server",
                "start",
                "--host",
                "127.0.0.1",
                "--port",
                "8812",
                "--backend",
                "http://127.0.0.1:18812"
            ]
        );
    }

    #[test]
    fn fungame_的两个前端都指向同一个后端() {
        let spec = fungame_web();
        for key in ["admin", "client"] {
            let a = argv(&spec, key, |c| &c.start);
            let i = a.iter().position(|x| x == "--api-upstream-port").unwrap();
            assert_eq!(a[i + 1], "8809", "{key} 的上游端口不对");
        }
    }

    #[test]
    fn fungame_是三个组件且用户端是入口() {
        let spec = fungame_web();
        assert_eq!(spec.components.len(), 3);
        assert_eq!(spec.entry_url().as_deref(), Some("http://127.0.0.1:8808"));
    }

    #[test]
    fn funread_的_stop_不带_host() {
        // 上游的 stop/status 只认 --config 和 --port，多给 --host 会直接报参数错误。
        let spec = funread_web().spec;
        let stop = argv(&spec, "backend", |c| &c.stop);
        assert_eq!(stop, ["funread-api", "server", "stop", "--port", "18811"]);
        assert!(!stop.iter().any(|a| a == "--host"));
    }

    #[test]
    fn funread_的入口带_web_路径() {
        let spec = funread_web().spec;
        assert_eq!(
            spec.entry_url().as_deref(),
            Some("http://127.0.0.1:8811/web")
        );
    }

    #[test]
    fn 端口能用环境变量盖掉() {
        std::env::set_var("FUNLESSON_WEB_FRONTEND_PORT", "19999");
        let spec = funlesson_web();
        assert_eq!(spec.component("frontend").unwrap().port, 19999);
        std::env::remove_var("FUNLESSON_WEB_FRONTEND_PORT");
    }

    #[test]
    fn 非法端口退回默认值而不是崩掉() {
        std::env::set_var("__FD_TEST_PORT", "八千");
        assert_eq!(env_port("__FD_TEST_PORT", 1234), 1234);
        std::env::set_var("__FD_TEST_PORT", "0");
        assert_eq!(env_port("__FD_TEST_PORT", 1234), 1234);
        std::env::set_var("__FD_TEST_PORT", "70000");
        assert_eq!(env_port("__FD_TEST_PORT", 1234), 1234);
        std::env::remove_var("__FD_TEST_PORT");
    }

    #[test]
    fn 空环境变量当作没设置() {
        std::env::set_var("__FD_TEST_STR", "   ");
        assert_eq!(env_str("__FD_TEST_STR", "默认"), "默认");
        assert_eq!(env_opt("__FD_TEST_STR"), None);
        std::env::remove_var("__FD_TEST_STR");
    }

    #[test]
    fn 每个服务都至少声明了_start_和_stop() {
        for s in all() {
            for c in &s.spec().components {
                assert!(!c.cli.start.is_empty(), "{}/{} 没有 start", s.name(), c.key);
                assert!(!c.cli.stop.is_empty(), "{}/{} 没有 stop", s.name(), c.key);
            }
        }
    }

    #[test]
    fn 每个服务的最后一个组件都能前台跑() {
        // run 把最后一个组件放前台，它必须有 run 形态，否则 run 命令直接不可用。
        for s in all() {
            let last = s.spec().components.last().unwrap();
            assert!(
                !last.cli.run.is_empty(),
                "{} 的 {} 没有 run",
                s.name(),
                last.key
            );
        }
    }

    #[test]
    fn 所有服务的端口在全局范围内也不撞车() {
        // 同机常常同时跑好几个服务，端口撞了会表现成「另一个服务莫名其妙起不来」。
        let mut seen: Vec<(u16, String)> = Vec::new();
        for s in all() {
            for c in &s.spec().components {
                if let Some((_, who)) = seen.iter().find(|(p, _)| *p == c.port) {
                    panic!(
                        "端口 {} 被 {} 和 {}/{} 同时占用",
                        c.port,
                        who,
                        s.name(),
                        c.key
                    );
                }
                seen.push((c.port, format!("{}/{}", s.name(), c.key)));
            }
        }
    }
}
