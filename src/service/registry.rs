//! 服务声明。
//!
//! 一个服务通常就是一份声明，生命周期全部由所属族的默认实现提供。只有真有个性
//! 的服务（见 [`funread_web`]）才需要自己的类型去覆盖钩子。
//!
//! 所有端口、地址、包名、版本都可以用环境变量盖掉，变量名的规律是
//! `<服务名大写>_<组件名大写>_<字段>`，例如 `FUNREAD_WEB_BACKEND_PORT`。

use anyhow::Result;
use std::path::PathBuf;

use super::config;
use super::daemon::{self, DaemonSpec, Github, Proc};
use super::spec::{Arg, CliShape, Component, Pkg, ServiceSpec};
use super::upstream::{self, Upstream};
use super::Service;
use crate::args;
use crate::common::platform::{self, Platform};
use crate::ui;

pub fn all() -> Vec<Box<dyn Service>> {
    vec![
        upstream::declared(funfluid_web()),
        upstream::declared(funlesson_web()),
        upstream::declared(fungame_web()),
        upstream::declared(funflix_web()),
        upstream::boxed(funread_web()),
        daemon::boxed(new_api()),
        daemon::declared(code_server()),
        daemon::boxed(sub2api()),
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

impl Upstream for FunreadWeb {
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

// ---------------------------------------------------------------- new-api

/// new-api 自己要守着，而且端口最终由它读的 `.env` 说了算。
pub struct NewApi {
    spec: DaemonSpec,
}

fn new_api() -> NewApi {
    let home = PathBuf::from(env_str(
        "NEW_API_SERVICE_HOME",
        &home().join("opt/new-api").to_string_lossy(),
    ));
    let data = home.join("data");
    let port = env_port("NEW_API_PORT", 8801);
    let host = env_str("NEW_API_HOST", "127.0.0.1");

    // 上游给三种附件，macOS 不分架构。
    let asset = |tag: &str, p: Platform| {
        let ver = tag.trim_start_matches(['v', 'V']);
        match (p.os, p.arch) {
            (platform::Os::MacOs, _) => format!("new-api-macos-v{ver}"),
            (platform::Os::Linux, platform::Arch::Aarch64) => format!("new-api-arm64-v{ver}"),
            (platform::Os::Linux, platform::Arch::X86_64) => format!("new-api-v{ver}"),
        }
    };

    // 必须在数据目录里跑：上游启动时执行 godotenv.Load(".env")，走的是相对路径。
    let mut proc = Proc::new("main", "new-api", "new-api", &host, port, data.clone());

    // godotenv 不覆盖已存在的环境变量。有 .env 就把 PORT 让给它，同时从继承环境里
    // 删掉 PORT——操作者 shell 里 export 过一个，.env 那行就白写了，而且没人报错。
    let force = env_opt("NEW_API_FORCE_SCRIPT_PORT").as_deref() == Some("1");
    proc = if data.join(".env").is_file() && !force {
        proc.unset("PORT")
    } else {
        proc.env("PORT", port.to_string())
    };

    let spec = DaemonSpec::new(
        "new-api",
        "new-api 中转网关",
        home,
        Github {
            repo: env_str("NEW_API_GITHUB_REPO", "QuantumNous/new-api"),
            asset: Box::new(asset),
            // 附件是裸二进制，没有归档可剥。
            strip: 0,
            binary_name: "new-api".into(),
            scan: 40,
        },
        vec![proc],
    )
    .entry(format!("http://{host}:{port}"))
    .note("SQL_DSN 等配置写在 data/.env 里，它的 PORT 优先于 NEW_API_PORT。");

    NewApi { spec }
}

impl daemon::Daemon for NewApi {
    fn spec(&self) -> &DaemonSpec {
        &self.spec
    }

    /// `.env` 里的 PORT 才是真正生效的那个（见上面 godotenv 的说明）。
    /// 不按它探测的话，start 会在一个没人监听的端口上空等三十秒然后报失败。
    fn effective_port(&self, p: &Proc) -> u16 {
        env_file_value(&self.spec.data_dir().join(".env"), "PORT")
            .and_then(|v| v.parse().ok())
            .filter(|&x| x > 0)
            .unwrap_or(p.port)
    }
}

/// 从 `KEY=VALUE` 风格的文件里取一个值。取最后一条，和 shell 里重复赋值的语义一致。
fn env_file_value(path: &PathBuf, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines()
        .rev()
        .filter_map(|line| {
            let line = line.trim();
            let line = line.strip_prefix("export ").unwrap_or(line);
            let (k, v) = line.split_once('=')?;
            (k.trim() == key).then(|| v.to_string())
        })
        .map(|v| {
            // 去掉行尾注释和包裹的引号。
            let v = v.split('#').next().unwrap_or("").trim();
            v.trim_matches(['"', '\'']).to_string()
        })
        .find(|v| !v.is_empty())
}

// ---------------------------------------------------------------- code-server

fn code_server() -> DaemonSpec {
    let workspace = home();
    let home = PathBuf::from(env_str(
        "CODE_SERVER_SERVICE_HOME",
        &workspace.join("opt/code-server").to_string_lossy(),
    ));
    let host = env_str("CODE_SERVER_HOST", "127.0.0.1");
    let port = env_port("CODE_SERVER_PORT", 8080);

    let asset = |tag: &str, p: Platform| {
        let ver = tag.trim_start_matches(['v', 'V']);
        format!(
            "code-server-{ver}-{}-{}.tar.gz",
            p.os.macos_style(),
            p.arch.go()
        )
    };

    // 工作区就是家目录：code-server 打开的是 cwd，装在 opt 下不代表要在那儿写代码。
    let mut proc = Proc::new(
        "main",
        "code-server",
        "bin/code-server",
        &host,
        port,
        workspace,
    )
    .args(vec!["--bind-addr".into(), format!("{host}:{port}")]);
    // 走环境变量而不是命令行：/proc/<pid>/cmdline 是全机可读的。
    if let Some(pw) = env_opt("PASSWORD") {
        proc = proc.env("PASSWORD", pw);
    }

    DaemonSpec::new(
        "code-server",
        "浏览器里的 VS Code",
        home,
        Github {
            repo: env_str("CODE_SERVER_GITHUB_REPO", "coder/code-server"),
            asset: Box::new(asset),
            // 归档里是 code-server-<版本>-<平台>/ 一层壳，剥掉它安装路径才不随版本变。
            strip: 1,
            binary_name: "code-server".into(),
            scan: 20,
        },
        vec![proc],
    )
    .entry(format!("http://{host}:{port}"))
    .note("没设 PASSWORD 的话，登录密码在首次启动的日志里（log/main.log）。")
}

// ---------------------------------------------------------------- sub2api

/// sub2api 要在启动前把环境变量文件和示例配置放好。
pub struct Sub2Api {
    spec: DaemonSpec,
}

fn sub2api() -> Sub2Api {
    let home = PathBuf::from(env_str(
        "SUB2API_SERVICE_HOME",
        &home().join("opt/sub2api").to_string_lossy(),
    ));
    let data = home.join("data");
    let env_file = data.join("sub2api.env");

    // 优先级：显式环境变量 > 环境变量文件 > 默认值。
    // 默认绑 127.0.0.1 而不是 0.0.0.0 是刻意的：这里存着上游渠道的 key 和用户
    // token，默认对外开等于把它们挂到公网上。
    let host = env_opt("SUB2API_HOST")
        .or_else(|| env_file_value(&env_file, "SERVER_HOST"))
        .unwrap_or_else(|| "127.0.0.1".into());
    let port = env_opt("SUB2API_PORT")
        .or_else(|| env_file_value(&env_file, "SERVER_PORT"))
        .and_then(|v| v.parse().ok())
        .filter(|&x: &u16| x > 0)
        .unwrap_or(8802);

    let asset = |tag: &str, p: Platform| {
        let ver = tag.trim_start_matches(['v', 'V']);
        format!("sub2api_{ver}_{}_{}.tar.gz", p.os.go(), p.arch.go())
    };

    // 归档里是 sub2api + deploy/ 平铺在根上，没有壳要剥；在 app/ 下跑，
    // 上游才找得到同级的 deploy/。
    let proc = Proc::new("main", "sub2api", "sub2api", &host, port, home.join("app"))
        .env("DATA_DIR", data.to_string_lossy().into_owned())
        .env("SERVER_HOST", &host)
        .env("SERVER_PORT", port.to_string())
        .env("RUN_MODE", env_str("SUB2API_RUN_MODE", "simple"))
        .env("SIMPLE_MODE_CONFIRM", "true")
        .env("GIN_MODE", "release");

    let spec = DaemonSpec::new(
        "sub2api",
        "sub2api 订阅转 API",
        home,
        Github {
            repo: env_str("SUB2API_GITHUB_REPO", "Wei-Shaw/sub2api"),
            asset: Box::new(asset),
            strip: 0,
            binary_name: "sub2api".into(),
            scan: 30,
        },
        vec![proc],
    )
    .entry(format!("http://{host}:{port}"))
    .note("库地址、JWT 密钥、管理员口令写在 data/sub2api.env 里。");

    Sub2Api { spec }
}

impl Sub2Api {
    fn env_file(&self) -> PathBuf {
        self.spec.data_dir().join("sub2api.env")
    }

    /// 放一份注释掉的环境变量模板。
    ///
    /// 整份注释掉等于「这台机器没有特殊配置」。这里面该填什么库、什么口令不是
    /// 安装脚本能替人决定的，写个默认值进去只会让人在连错库之后才发现。
    fn seed_env_file(&self) -> Result<()> {
        let path = self.env_file();
        let body = "# sub2api 的运行环境变量，由 fundeploy 生成。整份是注释，去掉 # 即生效。\n\
                    # 这个文件会被 sub2api 读走，权限是 0600——里面放的是口令和密钥。\n\
                    # 监听地址由 fundeploy 的 SUB2API_HOST / SUB2API_PORT 决定，\n\
                    # 没设时才回落到这里的 SERVER_HOST / SERVER_PORT。\n\
                    #\n\
                    # SERVER_HOST=127.0.0.1\n\
                    # SERVER_PORT=8802\n\
                    # DATABASE_HOST=127.0.0.1\n\
                    # DATABASE_PORT=5432\n\
                    # DATABASE_USER=postgres\n\
                    # DATABASE_PASSWORD=\n\
                    # DATABASE_NAME=sub2api\n\
                    # REDIS_HOST=127.0.0.1\n\
                    # REDIS_PORT=6379\n\
                    # REDIS_PASSWORD=\n\
                    # JWT_SECRET=换成一串随机值\n\
                    # TOTP_ENCRYPTION_KEY=换成一串随机值\n\
                    # ADMIN_EMAIL=admin@sub2api.local\n\
                    # ADMIN_PASSWORD=换掉我\n";
        match config::write_if_absent(&path, body) {
            Ok(true) => ui::step(format!("已生成 {}（库地址和口令写这里）", path.display())),
            Ok(false) => {}
            Err(e) => ui::warn(format!("{e}（库地址和口令要写在这里）")),
        }
        Ok(())
    }
}

impl daemon::Daemon for Sub2Api {
    fn spec(&self) -> &DaemonSpec {
        &self.spec
    }

    fn ensure_config(&self) -> Result<()> {
        self.seed_env_file()
    }

    fn post_install(&self) -> Result<()> {
        self.seed_env_file()?;
        // 上游把一份完整的示例配置打在 deploy/ 里，放到 data/ 下人才找得到；
        // app/ 每次升级会被整个换掉，留在那儿等于没留。
        let from = self.spec.app_dir().join("deploy/config.example.yaml");
        let to = self.spec.data_dir().join("config.example.yaml");
        if from.is_file() && !to.exists() {
            if let Err(e) = std::fs::copy(&from, &to) {
                ui::warn(format!("复制示例配置失败: {e}"));
            }
        }
        Ok(())
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

    /// 上游自带守护那一族的全部声明。`all()` 里只剩 `dyn Service`，
    /// 拿不到 spec，组件级的自检只能在这里对着原始声明做。
    fn upstream_specs() -> Vec<ServiceSpec> {
        vec![
            funfluid_web(),
            funlesson_web(),
            fungame_web(),
            funflix_web(),
            funread_web().spec,
        ]
    }

    #[test]
    fn 每个上游服务的声明都自检通过() {
        // 声明写错（key 重复、端口撞车、引用了不存在的组件）属于编程错误，
        // 不该等到操作者跑 start 时才炸。
        for spec in upstream_specs() {
            spec.validate()
                .unwrap_or_else(|e| panic!("{} 的声明有问题: {e}", spec.name));
        }
    }

    #[test]
    fn 每个上游服务都至少声明了_start_和_stop() {
        for spec in upstream_specs() {
            for c in &spec.components {
                assert!(
                    !c.cli.start.is_empty(),
                    "{}/{} 没有 start",
                    spec.name,
                    c.key
                );
                assert!(!c.cli.stop.is_empty(), "{}/{} 没有 stop", spec.name, c.key);
            }
        }
    }

    #[test]
    fn 每个上游服务的最后一个组件都能前台跑() {
        // run 把最后一个组件放前台，它必须有 run 形态，否则 run 命令直接不可用。
        for spec in upstream_specs() {
            let last = spec.components.last().unwrap();
            assert!(
                !last.cli.run.is_empty(),
                "{} 的 {} 没有 run",
                spec.name,
                last.key
            );
        }
    }

    #[test]
    fn new_api_按平台拼附件名() {
        // 三种附件名互不相同，而且 macOS 不分架构——拼错了就是 404。
        let s = new_api().spec;
        let cases = [
            (
                platform::Os::Linux,
                platform::Arch::X86_64,
                "new-api-v0.12.6",
            ),
            (
                platform::Os::Linux,
                platform::Arch::Aarch64,
                "new-api-arm64-v0.12.6",
            ),
            (
                platform::Os::MacOs,
                platform::Arch::Aarch64,
                "new-api-macos-v0.12.6",
            ),
        ];
        for (os, arch, want) in cases {
            assert_eq!((s.source.asset)("v0.12.6", Platform { os, arch }), want);
        }
    }

    #[test]
    fn code_server_和_sub2api_的附件名各按各家的写法() {
        let p = Platform {
            os: platform::Os::MacOs,
            arch: platform::Arch::Aarch64,
        };
        // code-server 用 macos-arm64，sub2api 用 darwin_arm64。
        assert_eq!(
            (code_server().source.asset)("v4.112.0", p),
            "code-server-4.112.0-macos-arm64.tar.gz"
        );
        assert_eq!(
            (sub2api().spec.source.asset)("v0.1.144", p),
            "sub2api_0.1.144_darwin_arm64.tar.gz"
        );
    }

    #[test]
    fn new_api_有_env_时把端口让给它() {
        // godotenv 不覆盖已存在的环境变量，所以这时必须把 PORT 从环境里删掉，
        // 否则操作者 shell 里 export 过的那个会静默压住 .env 里的配置。
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("data")).unwrap();
        std::fs::write(dir.path().join("data/.env"), "PORT=9999\n").unwrap();
        std::env::set_var("NEW_API_SERVICE_HOME", dir.path());

        let s = new_api();
        assert_eq!(s.spec.procs[0].env, [("PORT".to_string(), None)]);
        assert_eq!(
            daemon::Daemon::effective_port(&s, &s.spec.procs[0]),
            9999,
            ".env 里的 PORT 才是真正生效的那个"
        );

        std::env::remove_var("NEW_API_SERVICE_HOME");
    }

    #[test]
    fn new_api_没有_env_时自己注入端口() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("NEW_API_SERVICE_HOME", dir.path());

        let s = new_api();
        assert_eq!(
            s.spec.procs[0].env,
            [("PORT".to_string(), Some("8801".to_string()))]
        );
        assert_eq!(daemon::Daemon::effective_port(&s, &s.spec.procs[0]), 8801);

        std::env::remove_var("NEW_API_SERVICE_HOME");
    }

    #[test]
    fn 从_env_文件里取值() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join(".env");
        std::fs::write(
            &f,
            "# PORT=1\nexport SERVER_HOST = \"0.0.0.0\"\nPORT=3000 # 注释\nPORT=4000\n",
        )
        .unwrap();
        // 重复赋值取最后一条，和 shell 的语义一致。
        assert_eq!(env_file_value(&f, "PORT").as_deref(), Some("4000"));
        // export 前缀、引号、行尾注释都要剥掉。
        assert_eq!(
            env_file_value(&f, "SERVER_HOST").as_deref(),
            Some("0.0.0.0")
        );
        assert_eq!(env_file_value(&f, "没有这个"), None);
        assert_eq!(env_file_value(&dir.path().join("没有"), "PORT"), None);
    }

    #[test]
    fn sub2api_默认只绑本机() {
        // 它存着上游渠道的 key 和用户 token，默认对外开等于把这些挂到公网上。
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("SUB2API_SERVICE_HOME", dir.path());
        let s = sub2api().spec;
        assert_eq!(s.procs[0].host, "127.0.0.1");
        assert_eq!(s.procs[0].port, 8802);
        std::env::remove_var("SUB2API_SERVICE_HOME");
    }
}
