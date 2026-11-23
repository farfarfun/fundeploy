//! Funmill 任务执行 API 及其第三方编排后端（Dagu / Windmill）。
//!
//! 除九个标准服务动作外，把 `funmill` CLI 的完整能力都挂到 `fundeploy service funmill`
//! 底下，既能整套管，也能单独升级/启停/重启编排后端（尤其是 `dagu`）或按原样透传参数：
//!
//! - `services`：列出可安装的第三方编排后端（`funmill services`）
//! - `install-backend [dagu|windmill] [--force]`：安装或强制重装第三方编排后端
//! - `upgrade-backend [dagu|windmill]`：升级编排后端（`dagu` 拉取 `latest`；运行中自动停服再拉起）
//! - `dagu <install|upgrade|start|stop|restart|status> [--force]`：直接管理 Dagu 后端
//! - `windmill <install|upgrade|start|stop|restart|status> [--force]`：直接管理 Windmill 后端
//! - `start-one` / `stop-one` / `restart-one` / `status-one`：单组件启停、重启、查状态
//! - `run-api`：仅前台运行 Funmill API（不连带启停编排后端）
//! - `server <start|stop|restart|status|run> [api|dagu|windmill]`：与 `funmill server` 同构
//! - `cli [--config <路径>] <参数...>`：原样透传任意参数给 `funmill`

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Result};

use super::registry::env_file_value;
use super::spec::{CliShape, Component, Pkg, ServiceSpec};
use super::upstream::{stop_upstream_component, Upstream, UpstreamStop};
use super::{Extra, START_TIMEOUT};
use crate::args;
use crate::common::{cmd, env, port, proc};
use crate::ui;

const KNOWN_BACKENDS: [&str; 2] = ["dagu", "windmill"];
const KNOWN_COMPONENTS: [&str; 3] = ["api", "dagu", "windmill"];

pub const EXTRAS: &[Extra] = &[
    Extra {
        name: "services",
        about: "列出 funmill 可安装的第三方编排后端",
    },
    Extra {
        name: "install-backend",
        about: "安装第三方编排后端: install-backend [dagu|windmill] [--force]",
    },
    Extra {
        name: "upgrade-backend",
        about: "升级编排后端（dagu 拉取 latest，运行中自动停服再拉起）: upgrade-backend [dagu|windmill]",
    },
    Extra {
        name: "dagu",
        about: "管理 dagu 后端: dagu <install|upgrade|start|stop|restart|status> [--force]",
    },
    Extra {
        name: "windmill",
        about: "管理 windmill 后端: windmill <install|upgrade|start|stop|restart|status> [--force]",
    },
    Extra {
        name: "start-one",
        about: "单独后台启动一个组件: start-one [api|dagu|windmill]",
    },
    Extra {
        name: "stop-one",
        about: "单独停止一个组件: stop-one [api|dagu|windmill]",
    },
    Extra {
        name: "restart-one",
        about: "单独重启一个组件: restart-one [api|dagu|windmill]",
    },
    Extra {
        name: "status-one",
        about: "单独查看一个组件状态: status-one [api|dagu|windmill]",
    },
    Extra {
        name: "run-api",
        about: "仅前台运行 Funmill API（不连带启停编排后端）",
    },
    Extra {
        name: "server",
        about: "执行 funmill server 生命周期命令: server <start|stop|restart|status|run> [组件]",
    },
    Extra {
        name: "cli",
        about: "原样透传任意参数给 funmill 命令: cli [--config <路径>] <参数...>",
    },
];

/// funmill 有三处个性：
/// 1. API 和编排后端由同一个 pip 包提供，但后端二进制还得让 `funmill install <后端>` 去下；
/// 2. 所有配置按「配置文件（`api.env`） > 环境变量 > 默认值」解析；
/// 3. 强制 `X-API-Key` 鉴权且上游不给默认值。
pub struct Funmill {
    pub(super) spec: ServiceSpec,
    /// 要编排的第三方后端；None 表示后端归别处管。
    pub(super) backend: Option<String>,
    /// `FUNMILL_HOME`，默认 `~/.farfarfun/funmill`。
    home: PathBuf,
    /// 默认配置文件路径：`FUNMILL_CONFIG` 或 `<home>/api/api.env`。
    config_file: PathBuf,
    api_port: u16,
    backend_port: u16,
}

impl Funmill {
    pub fn from_env() -> Self {
        let home = env::opt("FUNMILL_HOME")
            .map(|v| env::expand_tilde(&v))
            .unwrap_or_else(|| env::home().join(".farfarfun/funmill"));
        let config_file = env::opt("FUNMILL_CONFIG")
            .map(|v| env::expand_tilde(&v))
            .unwrap_or_else(|| home.join("api/api.env"));

        let pkg = env::str_or("FUNMILL_PACKAGE", "funmill-api");
        let version = env::opt("FUNMILL_VERSION");
        let api_port = env::port("FUNMILL_API_PORT", 8812);
        let backend_port = env::port("FUNMILL_BACKEND_PORT", 8813);

        // 与 funmill 上游一致：配置文件优先级高于环境变量。
        let backend =
            setting_from(&config_file, "FUNMILL_BACKEND").unwrap_or_else(|| "dagu".to_string());
        // none 表示这台机器的编排后端归别处管（已经在用外部实例）。
        let backend = (backend != "none").then_some(backend);

        let mut components = Vec::new();
        if let Some(b) = &backend {
            components.push(
                Component::new(
                    "backend",
                    &format!("编排后端 {b}"),
                    Pkg::pip(&pkg, "funmill"),
                    "127.0.0.1",
                    backend_port,
                )
                .version(version.clone())
                .cli_shape(
                    CliShape::new()
                        .start(args!["start", b.clone()])
                        .stop(args!["stop", b.clone()])
                        .status(args!["status", b.clone()]),
                ),
            );
        }

        let mut api = Component::new(
            "api",
            "Funmill API",
            Pkg::pip(&pkg, "funmill"),
            "127.0.0.1",
            api_port,
        )
        .version(version)
        .cli_shape(
            CliShape::new()
                .start(args!["start"])
                .run(args!["run"])
                .stop(args!["stop"])
                .status(args!["status"]),
        );
        if let Some(b) = &backend {
            api = api.env("FUNMILL_BACKEND", b);
        }
        components.push(api);

        let spec = ServiceSpec::new("funmill", "funmill 任务执行 API", components)
            .entry_at("api", "/docs")
            // 端口是上游写死的，CLI 不收 --host/--port；这里的端口只拿来探测死活。
            .note("端口由上游固定，改不了；FUNMILL_*_PORT 只影响 fundeploy 的探测。")
            .note("PID 和日志归 funmill 自己管，在 ~/.farfarfun/funmill/services/<名字>/ 下。");

        Self {
            spec,
            backend,
            home,
            config_file,
            api_port,
            backend_port,
        }
    }

    // --------------------------------------------------------------- 配置与路径

    fn effective_config_path(&self, override_path: Option<&str>) -> PathBuf {
        match override_path {
            Some(p) => env::expand_tilde(p),
            None => self.config_file.clone(),
        }
    }

    fn ensure_config_with(&self, config_override: Option<&str>) -> Result<()> {
        let cfg_path = self.effective_config_path(config_override);
        if let Some(explicit) = config_override {
            if !cfg_path.is_file() {
                bail!("指定的 funmill 配置文件不存在: {explicit}");
            }
        }
        if setting_from(&cfg_path, "FUNMILL_API_KEY").is_none() {
            bail!(
                "funmill 强制 X-API-Key 鉴权，启动前先 export FUNMILL_API_KEY=... \
                 或在 {} 中配置 FUNMILL_API_KEY（上游没有默认值）",
                cfg_path.display()
            );
        }
        Ok(())
    }

    fn require_cli(&self) -> Result<()> {
        if cmd::has("funmill") {
            return Ok(());
        }
        bail!("命令 funmill 不在 PATH 上\n先执行 `fundeploy service funmill install`")
    }

    fn backend_binary(&self, backend: &str) -> PathBuf {
        self.home.join("services").join(backend).join(backend)
    }

    /// 读编排后端已装版本。目前 Dagu 支持 `dagu version`；Windmill 不强求。
    fn backend_version(&self, backend: &str) -> Option<String> {
        if backend != "dagu" {
            return None;
        }
        let bin = self.backend_binary("dagu");
        if !bin.is_file() {
            return None;
        }
        let out = cmd::try_run([bin.to_string_lossy().as_ref(), "version"]).ok()?;
        out.success()
            .then(|| out.stdout.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    fn api_env_edits(&self) -> Vec<(String, String)> {
        match &self.backend {
            Some(b) => vec![("FUNMILL_BACKEND".to_string(), b.clone())],
            None => Vec::new(),
        }
    }

    /// 查单个组件在 funmill 眼里是否正在运行。
    fn component_status_ok(&self, target: &str, config: Option<&str>) -> bool {
        let argv = build_component_argv(config, "status", target);
        cmd::try_run(&argv).is_ok_and(|o| o.success())
    }

    fn component_port(&self, target: &str) -> u16 {
        if target == "api" {
            self.api_port
        } else {
            self.backend_port
        }
    }

    fn component_label(&self, target: &str) -> String {
        if target == "api" {
            "Funmill API".to_string()
        } else {
            format!("编排后端 {target}")
        }
    }

    fn is_component_running(&self, target: &str, config: Option<&str>) -> bool {
        if self.component_status_ok(target, config) {
            return true;
        }
        let p = self.component_port(target);
        if target == "api" || self.backend.as_deref() == Some(target) {
            port::is_listening("127.0.0.1", p)
        } else {
            false
        }
    }

    // --------------------------------------------------------------- 目标解析

    fn resolve_backend_target(&self, raw: Option<&str>, action: &str) -> Result<String> {
        if let Some(name) = raw {
            if name.starts_with('-') {
                bail!(
                    "不认识的选项 {name}。用法: fundeploy service funmill {action} [dagu|windmill]"
                );
            }
            if name == "backend" {
                return self.backend.clone().ok_or_else(|| {
                    anyhow!("FUNMILL_BACKEND=none，没有默认编排后端，请明确指定 dagu 或 windmill")
                });
            }
            return Ok(name.to_string());
        }

        if let Some(b) = &self.backend {
            if !ui::interactive() {
                return Ok(b.clone());
            }
            let mut choices = vec![b.clone()];
            for k in KNOWN_BACKENDS {
                if k != b {
                    choices.push(k.to_string());
                }
            }
            let Some(picked) = ui::select("选择编排后端", &choices)? else {
                bail!("已取消。");
            };
            return Ok(picked.clone());
        }

        if ui::interactive() {
            let choices: Vec<String> = KNOWN_BACKENDS.iter().map(|s| (*s).to_string()).collect();
            let Some(picked) = ui::select("选择编排后端", &choices)? else {
                bail!("已取消。");
            };
            return Ok(picked.clone());
        }

        bail!(
            "FUNMILL_BACKEND=none，请明确指定编排后端，例如: fundeploy service funmill {action} dagu"
        )
    }

    fn resolve_component_target(&self, raw: Option<&str>, action: &str) -> Result<String> {
        if let Some(name) = raw {
            if name.starts_with('-') {
                bail!(
                    "不认识的选项 {name}。用法: fundeploy service funmill {action} <api|dagu|windmill>"
                );
            }
            if name == "backend" {
                return self.backend.clone().ok_or_else(|| {
                    anyhow!(
                        "FUNMILL_BACKEND=none，没有默认编排后端，请明确指定 api、dagu 或 windmill"
                    )
                });
            }
            return Ok(name.to_string());
        }

        if ui::interactive() {
            let choices: Vec<String> = KNOWN_COMPONENTS.iter().map(|s| (*s).to_string()).collect();
            let Some(picked) = ui::select("选择 Funmill 组件", &choices)? else {
                bail!("已取消。");
            };
            return Ok(picked.clone());
        }

        bail!(
            "用法: fundeploy service funmill {action} <组件>\n可选: {}",
            KNOWN_COMPONENTS.join("、")
        )
    }

    // --------------------------------------------------------------- 动作实现

    fn services_cmd(&self, args: &[String]) -> Result<()> {
        self.require_cli()?;
        let (config, rest) = parse_config_flag(args)?;
        if !rest.is_empty() {
            bail!("services 不收额外参数: {}", rest.join(" "));
        }
        let argv = build_funmill_argv(config.as_deref(), &["services"]);
        run_funmill(&argv, &[])
    }

    fn install_backend_cmd(&self, args: &[String]) -> Result<()> {
        self.require_cli()?;
        let (config, rest) = parse_config_flag(args)?;
        let mut force = false;
        let mut target: Option<&str> = None;
        for a in &rest {
            match a.as_str() {
                "--force" | "-f" => force = true,
                s if s.starts_with('-') => {
                    bail!("install-backend 不认识选项 {s}（支持 --force、--config <路径>）")
                }
                s if target.is_none() => target = Some(s),
                s => bail!("install-backend 多余参数: {s}"),
            }
        }
        let target = self.resolve_backend_target(target, "install-backend")?;
        if force {
            return self.upgrade_backend(&target, config.as_deref());
        }
        ui::step(format!("安装编排后端 {target}"));
        let argv = build_funmill_argv(config.as_deref(), &["install", &target]);
        run_funmill(&argv, &[])
    }

    fn upgrade_backend_cmd(&self, args: &[String]) -> Result<()> {
        self.require_cli()?;
        let (config, rest) = parse_config_flag(args)?;
        if rest.len() > 1 {
            bail!("用法: fundeploy service funmill upgrade-backend [dagu|windmill]");
        }
        let target =
            self.resolve_backend_target(rest.first().map(String::as_str), "upgrade-backend")?;
        self.upgrade_backend(&target, config.as_deref())
    }

    /// 升级/强制重装编排后端（`funmill install <backend> --force`）。
    ///
    /// Dagu 在 `--force` 下会从 npm 拉取该平台 `latest` 并覆盖二进制，运行中覆盖可能触发
    /// ETXTBSY 或新版本未生效；因此运行中时先停服、升级后再自动拉起。
    fn upgrade_backend(&self, target: &str, config: Option<&str>) -> Result<()> {
        self.require_cli()?;
        let before = self.backend_version(target);
        let was_running = self.is_component_running(target, config);

        if was_running {
            ui::step(format!("升级前编排后端 {target} 正在运行，先停止"));
            self.stop_component_one(target, config)?;
            ui::blank();
        }

        ui::step(format!("通过 funmill 升级编排后端 {target}（--force）"));
        let argv = build_funmill_argv(config, &["install", target, "--force"]);
        run_funmill(&argv, &[]).map_err(|e| {
            if was_running {
                e.context(format!(
                    "升级 {target} 失败，而它已经被停掉了。\n\
                     修好之后执行 `fundeploy service funmill start-one {target}` 拉起来。"
                ))
            } else {
                e
            }
        })?;

        let after = self.backend_version(target);
        if before.is_some() || after.is_some() {
            ui::info(format!(
                "{target}: {} -> {}",
                before.as_deref().unwrap_or("未安装"),
                after.as_deref().unwrap_or("未知")
            ));
        }

        if was_running {
            ui::blank();
            ui::step(format!("升级前编排后端 {target} 正在运行，重新启动"));
            self.start_component_one(target, config)?;
        } else {
            ui::ok(format!("{target} 升级完成（升级前未运行，没有自动启动）。"));
        }
        Ok(())
    }

    fn start_component_one(&self, target: &str, config: Option<&str>) -> Result<()> {
        self.require_cli()?;
        if target == "api" {
            self.ensure_config_with(config)?;
        }
        let label = self.component_label(target);
        let listen_port = self.component_port(target);
        if self.is_component_running(target, config) {
            ui::info(format!(
                "{label} 127.0.0.1:{listen_port} 已在运行，跳过启动。"
            ));
            return Ok(());
        }

        ui::step(format!("启动 {label}，监听 127.0.0.1:{listen_port}"));
        let argv = build_component_argv(config, "start", target);
        let env_edits = if target == "api" {
            self.api_env_edits()
        } else {
            Vec::new()
        };
        run_funmill(&argv, &env_edits)?;

        let sp = ui::Spinner::new(format!("等 {label} 监听 127.0.0.1:{listen_port}"));
        let listening =
            port::wait_for_listener_ticking("127.0.0.1", listen_port, START_TIMEOUT, || sp.tick());
        if listening {
            sp.done(format!("{label} 起来了，127.0.0.1:{listen_port} 在听"));
            Ok(())
        } else {
            sp.give_up();
            bail!("{label} 的启动命令已返回，但端口 {listen_port} 迟迟没被监听。")
        }
    }

    fn stop_component_one(&self, target: &str, config: Option<&str>) -> Result<()> {
        self.require_cli()?;
        let label = self.component_label(target);
        let listen_port = self.component_port(target);
        ui::step(format!("停止 {label}"));

        let stop_argv = build_component_argv(config, "stop", target);
        let status_argv = build_component_argv(config, "status", target);
        let match_tokens = if target == "api" {
            vec![
                "funmill".to_string(),
                "funmill-api".to_string(),
                "funmill_api".to_string(),
            ]
        } else {
            vec![target.to_ascii_lowercase()]
        };
        stop_upstream_component(&UpstreamStop {
            label: &label,
            host: "127.0.0.1",
            port: listen_port,
            stop_argv: &stop_argv,
            status_argv: Some(&status_argv),
            env: &[],
            match_tokens: &match_tokens,
        })?;
        ui::ok(format!("{label} 已停止。"));
        Ok(())
    }

    fn restart_component_one(&self, target: &str, config: Option<&str>) -> Result<()> {
        self.stop_component_one(target, config)?;
        ui::blank();
        self.start_component_one(target, config)
    }

    fn status_component_one(&self, target: &str, config: Option<&str>) -> Result<()> {
        self.require_cli()?;
        let argv = build_component_argv(config, "status", target);
        // 上游用非零退出码表示「未运行」，这不是故障。
        let _ = cmd::spawn_status(&argv, &[])?;
        Ok(())
    }

    fn run_api_cmd(&self, args: &[String]) -> Result<()> {
        self.require_cli()?;
        let (config, rest) = parse_config_flag(args)?;
        if !rest.is_empty() {
            bail!("run-api 不收额外参数: {}", rest.join(" "));
        }
        self.ensure_config_with(config.as_deref())?;
        if port::is_listening("127.0.0.1", self.api_port) {
            bail!(
                "Funmill API 的端口 {} 已被占用；run-api 不接管已有进程。\n\
                 要前台跑，先 `fundeploy service funmill stop-one api`。",
                self.api_port
            );
        }
        ui::step("前台启动 Funmill API（Ctrl+C 退出）");
        let argv = build_funmill_argv(config.as_deref(), &["run"]);
        let env_edits = self.api_env_edits();
        let guard = proc::SignalGuard::install();
        let code = cmd::spawn_status(&argv, &env_edits);
        drop(guard);
        match code {
            Ok(Some(0)) | Ok(None) => Ok(()),
            Ok(Some(c)) => bail!("Funmill API 退出，退出码 {c}"),
            Err(e) => Err(e),
        }
    }

    fn component_action_cmd(&self, action: &str, args: &[String]) -> Result<()> {
        let (config, rest) = parse_config_flag(args)?;
        if rest.len() > 1 {
            bail!("用法: fundeploy service funmill {action} [api|dagu|windmill]");
        }
        let target = self.resolve_component_target(rest.first().map(String::as_str), action)?;
        match action {
            "start-one" => self.start_component_one(&target, config.as_deref()),
            "stop-one" => self.stop_component_one(&target, config.as_deref()),
            "restart-one" => self.restart_component_one(&target, config.as_deref()),
            "status-one" => self.status_component_one(&target, config.as_deref()),
            other => bail!("funmill 不认识单组件动作 {other}"),
        }
    }

    /// `fundeploy service funmill dagu <subcmd>` / `windmill <subcmd>`。
    fn backend_subcmd(&self, backend: &str, args: &[String]) -> Result<()> {
        let (config, rest) = parse_config_flag(args)?;
        let (sub, tail): (String, &[String]) = match rest.split_first() {
            Some((head, tail)) => (head.clone(), tail),
            None if ui::interactive() => {
                let menu = vec![
                    format!("status   查看 {backend} 状态"),
                    format!("start    后台启动 {backend}"),
                    format!("stop     停止 {backend}"),
                    format!("restart  重启 {backend}"),
                    format!("install  安装 {backend}"),
                    format!("upgrade  升级 {backend}（--force 拉取最新）"),
                ];
                let Some(picked) = ui::select(&format!("funmill › {backend}"), &menu)? else {
                    return Ok(());
                };
                let head = picked.split_whitespace().next().unwrap_or("status");
                (head.to_string(), &[])
            }
            None => bail!(
                "用法: fundeploy service funmill {backend} <install|upgrade|start|stop|restart|status> [--force]"
            ),
        };

        match sub.as_str() {
            "install" => {
                let mut forwarded = Vec::new();
                if let Some(cfg) = &config {
                    forwarded.push("--config".to_string());
                    forwarded.push(cfg.clone());
                }
                forwarded.push(backend.to_string());
                forwarded.extend(tail.iter().cloned());
                self.install_backend_cmd(&forwarded)
            }
            "upgrade" => {
                if !tail.is_empty() {
                    bail!("用法: fundeploy service funmill {backend} upgrade");
                }
                self.upgrade_backend(backend, config.as_deref())
            }
            "start" => {
                if !tail.is_empty() {
                    bail!("用法: fundeploy service funmill {backend} start");
                }
                self.start_component_one(backend, config.as_deref())
            }
            "stop" => {
                if !tail.is_empty() {
                    bail!("用法: fundeploy service funmill {backend} stop");
                }
                self.stop_component_one(backend, config.as_deref())
            }
            "restart" => {
                if !tail.is_empty() {
                    bail!("用法: fundeploy service funmill {backend} restart");
                }
                self.restart_component_one(backend, config.as_deref())
            }
            "status" => {
                if !tail.is_empty() {
                    bail!("用法: fundeploy service funmill {backend} status");
                }
                self.status_component_one(backend, config.as_deref())
            }
            other => bail!(
                "{backend} 不支持动作 {other}。可选: install、upgrade、start、stop、restart、status"
            ),
        }
    }

    /// `fundeploy service funmill server <start|stop|restart|status|run> [api|dagu|windmill]`。
    ///
    /// 与上游 `funmill server` 同构，且内部转成单组件调用，因此无论本机装的是旧版还是
    /// `v1.0.31+` 的 `funmill-api` 都能直接工作。
    fn server_cmd(&self, args: &[String]) -> Result<()> {
        let (config, rest) = parse_config_flag(args)?;
        let (sub, tail): (String, &[String]) = match rest.split_first() {
            Some((head, tail)) => (head.clone(), tail),
            None if ui::interactive() => {
                let menu = vec![
                    "status   查看组件状态".to_string(),
                    "start    后台启动组件".to_string(),
                    "stop     停止组件".to_string(),
                    "restart  重启组件".to_string(),
                    "run      前台运行 Funmill API".to_string(),
                ];
                let Some(picked) = ui::select("funmill › server", &menu)? else {
                    return Ok(());
                };
                let head = picked.split_whitespace().next().unwrap_or("status");
                (head.to_string(), &[])
            }
            None => bail!(
                "用法: fundeploy service funmill server <start|stop|restart|status|run> [api|dagu|windmill]"
            ),
        };

        if sub == "run" {
            if !tail.is_empty() {
                bail!("用法: fundeploy service funmill server run");
            }
            let mut forwarded = Vec::new();
            if let Some(cfg) = config {
                forwarded.push("--config".to_string());
                forwarded.push(cfg);
            }
            return self.run_api_cmd(&forwarded);
        }

        if tail.len() > 1 {
            bail!("用法: fundeploy service funmill server {sub} [api|dagu|windmill]");
        }
        // 与 `funmill server <action>` 对齐：省略组件名时在命令行下默认指向 api，
        // 交互菜单无参进入时允许选择组件。
        let target = match tail.first() {
            Some(t) => self.resolve_component_target(Some(t.as_str()), &format!("server {sub}"))?,
            None if rest.is_empty() && ui::interactive() => {
                self.resolve_component_target(None, &format!("server {sub}"))?
            }
            None => "api".to_string(),
        };

        match sub.as_str() {
            "start" => self.start_component_one(&target, config.as_deref()),
            "stop" => self.stop_component_one(&target, config.as_deref()),
            "restart" => self.restart_component_one(&target, config.as_deref()),
            "status" => self.status_component_one(&target, config.as_deref()),
            other => bail!("server 不支持动作 {other}。可选: start、stop、restart、status、run"),
        }
    }

    fn cli_cmd(&self, args: &[String]) -> Result<()> {
        self.require_cli()?;
        let mut argv = vec!["funmill".to_string()];
        if args.is_empty() {
            if ui::interactive() {
                let line = ui::input("传给 funmill 的参数（留空显示 --help）", "")?;
                let parts: Vec<String> = line.split_whitespace().map(str::to_string).collect();
                if parts.is_empty() {
                    argv.push("--help".to_string());
                } else {
                    argv.extend(parts);
                }
            } else {
                argv.push("--help".to_string());
            }
        } else {
            argv.extend(args.iter().cloned());
        }
        run_funmill(&argv, &[])
    }
}

impl Upstream for Funmill {
    fn spec(&self) -> &ServiceSpec {
        &self.spec
    }

    /// 上游强制 X-API-Key 鉴权，而且没有默认值。缺了照样能起来，但之后每个请求
    /// 都是 401——与其让人对着一个「启动成功」的服务查半天，不如在这儿就拦住。
    /// 与上游一致：环境变量和 `api.env` 配置文件里任一处配置了都放行。
    fn ensure_config(&self) -> Result<()> {
        self.ensure_config_with(None)
    }

    /// 编排后端是个二进制，pip 包里没有，得让上游自己去下。
    fn post_install(&self) -> Result<()> {
        let Some(b) = &self.backend else {
            return Ok(());
        };
        if env::flag("FUNMILL_UPGRADE_BACKEND", false) {
            ui::step(format!("升级编排后端 {b}（FUNMILL_UPGRADE_BACKEND=1）"));
            return run_funmill(
                &[
                    "funmill".to_string(),
                    "install".to_string(),
                    b.clone(),
                    "--force".to_string(),
                ],
                &[],
            );
        }
        ui::step(format!("安装编排后端 {b}"));
        run_funmill(
            &["funmill".to_string(), "install".to_string(), b.clone()],
            &[],
        )
    }

    fn upgrade(&self) -> Result<()> {
        let before_backend = self
            .backend
            .as_deref()
            .and_then(|b| self.backend_version(b));
        // 复用默认升级流程（停服 -> 升 funmill-api -> post_install -> 起服）。
        upgrade_upstream_default(self)?;

        if let Some(b) = &self.backend {
            if env::flag("FUNMILL_UPGRADE_BACKEND", false) {
                let after_backend = self.backend_version(b);
                if before_backend.is_some() || after_backend.is_some() {
                    ui::info(format!(
                        "编排后端 {b}: {} -> {}",
                        before_backend.as_deref().unwrap_or("未安装"),
                        after_backend.as_deref().unwrap_or("未知")
                    ));
                }
            } else {
                ui::hint(format!(
                    "如需把编排后端 {b} 也升级到最新版，执行: fundeploy service funmill upgrade-backend {b}"
                ));
            }
        }
        Ok(())
    }

    fn extra_status(&self) -> Result<()> {
        ui::blank();
        ui::heading("配置与后端");
        let cfg_desc = if self.config_file.is_file() {
            format!("{}（已存在）", self.config_file.display())
        } else {
            format!("{}（未创建，走环境变量）", self.config_file.display())
        };
        ui::kv("配置文件", Some(&cfg_desc));
        ui::kv("数据目录", Some(&self.home.display().to_string()));

        let dagu_bin = self.backend_binary("dagu");
        if dagu_bin.is_file() {
            let ver = self
                .backend_version("dagu")
                .unwrap_or_else(|| "已安装".to_string());
            ui::kv("dagu", Some(&format!("{ver} · {}", dagu_bin.display())));
        } else if self.backend.as_deref() == Some("dagu") {
            ui::kv("dagu", None);
        }

        let windmill_bin = self.backend_binary("windmill");
        if windmill_bin.is_file() {
            ui::kv("windmill", Some(&windmill_bin.display().to_string()));
        } else if self.backend.as_deref() == Some("windmill") {
            ui::kv("windmill", None);
        }
        Ok(())
    }

    fn extras(&self) -> &'static [Extra] {
        EXTRAS
    }

    fn extra(&self, name: &str, args: &[String]) -> Result<()> {
        match name {
            "services" => self.services_cmd(args),
            "install-backend" => self.install_backend_cmd(args),
            "upgrade-backend" => self.upgrade_backend_cmd(args),
            "dagu" => self.backend_subcmd("dagu", args),
            "windmill" => self.backend_subcmd("windmill", args),
            "start-one" | "stop-one" | "restart-one" | "status-one" => {
                self.component_action_cmd(name, args)
            }
            "run-api" => self.run_api_cmd(args),
            "server" => self.server_cmd(args),
            "cli" => self.cli_cmd(args),
            other => bail!("funmill 没有 {other} 这个动作"),
        }
    }
}

/// 调 `Upstream::upgrade` 的默认实现（避免在覆盖方法里递归调用自身）。
fn upgrade_upstream_default(f: &Funmill) -> Result<()> {
    struct DefaultUpgrade<'a>(&'a Funmill);
    impl Upstream for DefaultUpgrade<'_> {
        fn spec(&self) -> &ServiceSpec {
            self.0.spec()
        }
        fn ensure_config(&self) -> Result<()> {
            self.0.ensure_config()
        }
        fn post_install(&self) -> Result<()> {
            self.0.post_install()
        }
        fn extra_status(&self) -> Result<()> {
            self.0.extra_status()
        }
    }
    DefaultUpgrade(f).upgrade()
}

/// 读取一个 funmill 设置：配置文件优先于环境变量（与上游 `funmill.api.config.load` 一致）。
fn setting_from(config_file: &Path, key: &str) -> Option<String> {
    env_file_value(config_file, key).or_else(|| env::opt(key))
}

/// 从参数列表里剥出 `--config <路径>` 或 `--config=<路径>`。
fn parse_config_flag(args: &[String]) -> Result<(Option<String>, Vec<String>)> {
    let mut config = None;
    let mut rest = Vec::with_capacity(args.len());
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--config" {
            let val = iter
                .next()
                .ok_or_else(|| anyhow!("--config 后面需要给出配置文件路径"))?;
            config = Some(val.clone());
        } else if let Some(val) = arg.strip_prefix("--config=") {
            if val.is_empty() {
                bail!("--config= 后面需要给出配置文件路径");
            }
            config = Some(val.to_string());
        } else {
            rest.push(arg.clone());
        }
    }
    Ok((config, rest))
}

fn build_funmill_argv(config: Option<&str>, subargs: &[&str]) -> Vec<String> {
    let mut argv = vec!["funmill".to_string()];
    if let Some(cfg) = config {
        argv.push("--config".to_string());
        argv.push(cfg.to_string());
    }
    argv.extend(subargs.iter().map(|s| (*s).to_string()));
    argv
}

/// 组装单组件命令：`api` 省略位置参数，第三方后端带上名字。
/// 刻意用顶层 `start/stop/status` 而非 `server start`，兼容旧版和 `v1.0.31+` 全部 `funmill-api`。
fn build_component_argv(config: Option<&str>, action: &str, target: &str) -> Vec<String> {
    if target == "api" {
        build_funmill_argv(config, &[action])
    } else {
        build_funmill_argv(config, &[action, target])
    }
}

fn run_funmill(argv: &[String], extra_env: &[(String, String)]) -> Result<()> {
    match cmd::spawn_status(argv, extra_env)? {
        Some(0) => Ok(()),
        other => bail!(
            "{} 失败（退出码 {}）",
            argv.join(" "),
            other.map_or("信号终止".to_string(), |c| c.to_string())
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::env::test_env::EnvGuard;

    #[test]
    fn 解析_config_选项并保留其余参数() {
        let args: Vec<String> = vec![
            "--config".into(),
            "/tmp/api.env".into(),
            "dagu".into(),
            "--force".into(),
        ];
        let (cfg, rest) = parse_config_flag(&args).unwrap();
        assert_eq!(cfg.as_deref(), Some("/tmp/api.env"));
        assert_eq!(rest, ["dagu", "--force"]);

        let eq_args: Vec<String> = vec!["--config=/tmp/b.env".into(), "status".into()];
        let (cfg, rest) = parse_config_flag(&eq_args).unwrap();
        assert_eq!(cfg.as_deref(), Some("/tmp/b.env"));
        assert_eq!(rest, ["status"]);

        let missing: Vec<String> = vec!["--config".into()];
        assert!(parse_config_flag(&missing).is_err());
    }

    #[test]
    fn 组装单组件命令时_api_不传多余位置参数() {
        assert_eq!(
            build_component_argv(None, "start", "api"),
            ["funmill", "start"]
        );
        assert_eq!(
            build_component_argv(None, "start", "dagu"),
            ["funmill", "start", "dagu"]
        );
        assert_eq!(
            build_component_argv(Some("/etc/a.env"), "stop", "dagu"),
            ["funmill", "--config", "/etc/a.env", "stop", "dagu"]
        );
    }

    #[test]
    fn api_env_里的_api_key_和_backend_也能被识别() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("api.env");
        std::fs::write(
            &cfg,
            "FUNMILL_API_KEY=from-file\nFUNMILL_BACKEND=windmill\n",
        )
        .unwrap();
        let cfg_s = cfg.to_string_lossy().into_owned();
        let _g = EnvGuard::set(&[
            ("FUNMILL_CONFIG", Some(&cfg_s)),
            ("FUNMILL_API_KEY", None),
            ("FUNMILL_BACKEND", None),
        ]);

        let f = Funmill::from_env();
        assert_eq!(f.backend.as_deref(), Some("windmill"));
        assert!(f.ensure_config().is_ok());
    }

    #[test]
    fn 额外动作的名字不重样() {
        let mut names: Vec<&str> = EXTRAS.iter().map(|e| e.name).collect();
        let raw_len = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), raw_len);
    }
}
