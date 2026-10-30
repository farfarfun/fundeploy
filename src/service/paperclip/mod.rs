//! Paperclip。
//!
//! 装、升、诊断和后台服务都归官方 `paperclipai` CLI。它既不属于
//! [`upstream`](super::upstream)——那族的上游只管起停、装包归 fundeploy；也不属于
//! [`daemon`](super::daemon)——那族的上游只给一个前台二进制。Paperclip 两头都自己
//! 管：managed install 自带版本仓库和 shim，`service` 子命令会去写 systemd --user
//! 单元或 LaunchAgent。fundeploy 在它上面只补两件事：
//!
//! 1. 中文封装，以及把官方那些「用退出码传信息」「检测不到就假装成功」的接口
//!    翻译成明确的话；
//! 2. 没有服务管理器的机器上的后台托管。`systemd --user` 需要一个用户级 systemd
//!    实例，容器、WSL1 和没有登录会话的终端里不存在；官方 `service start/stop/status`
//!    在那种机器上只打印一条 reason 就以 0 退出（`resolveManager` 返回 null 而不是
//!    抛错），对调用方是彻底的假成功——返回 0、什么都没启动。官方给出的唯一出路
//!    `paperclipai run` 是前台、不自守护的。
//!
//! 为什么不走官方 install.sh：paperclip.ing 上那份是过期快照（给 CLI 传了 `install`
//! 并不接受的 `--no-prompt`、Node 下限写成 20、用了 bash 4 语法在 macOS 自带的 3.2 上
//! 崩），而 master 上那份正确的没有发布 .sha256，无从校验，它的 Node 引导还会在见到
//! `~/.nvm` 时直接 fail。只要 PATH 上已有够新的 node，install.sh 的 Node 引导块会被
//! 整个跳过，剩下的就是本模块执行的那一行 npx——终态一致，但不引入一个过期、且在装了
//! nvm 的机器上根本跑不起来的脚本依赖。

// 拆出去的四块。分界线是「跟 Paperclip 的生命周期是不是同一件事」：
//   prefetch  npm 预取与安装期进度测量——上游把 npm 的输出吞了，只能从外面观察
//   progress  进度条样式与时长格式化，没有任何 Paperclip 语义
//   plugin    插件目录，一张会随上游生态增补的静态表
//   log       托管那条路的日志读取，只在没有服务管理器的机器上用得到
// 四个子模块都 `use super::*`，所以这边的常量和类型它们直接用；反过来父模块用到
// 的少数几个入口在子模块里标了 pub(super)。
mod log;
mod plugin;
mod prefetch;
mod progress;

use anyhow::{anyhow, bail, Context, Result};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::{Extra, Service};
use crate::common::cmd::{self, EnvEdit};
use crate::common::fs::{dir_size, human_size, Scratch};
use crate::common::{env, port, proc, version};
use crate::ui;

// 这几个是主流程还直接用到的。各子模块里那些只有自己用的零件不在这儿露面——
// 它们连同用到它们的方法一起搬过去了，这正是拆分想达到的效果。
use log::print_tail;
use prefetch::{InstallWatch, Npmrc, WATCH_TICK};
use progress::{human_secs, progress_bar, STEADY_TICK};

/// 上游要求的 Node 下限。只有 npx 引导首次安装那一步真需要它——装完之后 shim 里
/// 是绝对路径，自带了它那份 node，PATH 上是不是旧版无关。
const NODE_MIN: &str = "24.11.0";

/// 官方源。npx 引导与 managed install 使用同一源，才能共享 npm 的 URL 缓存。
const NPM_REGISTRY: &str = "https://registry.npmjs.org";
const NPM_MIRROR: &str = "https://registry.npmmirror.com";
const NPM_MIRROR_TENCENT: &str = "https://mirrors.cloud.tencent.com/npm";
const NPM_ALLOW_SCRIPTS: &str = "@embedded-postgres/darwin-arm64,@embedded-postgres/darwin-x64,@embedded-postgres/linux-arm64,@embedded-postgres/linux-x64,opencode-ai,ssh2,protobufjs,cpu-features,@paperclipai/server";

/// 下载量与落盘体积的比值。
///
/// 实测 2026.1015.0-canary.9 在 linux-x64 上：要单独下的 tarball 431 个、合计
/// 0.97 GiB（数字来自灌完之后 `_cacache/index-v5` 里每条的 `size`），落盘 2.7 GiB。
/// 比值上不去是因为树里最大的几项本来就是压过的二进制（codex 那个 155 MiB 的包解开
/// 是 427 MiB）。
///
/// 写成比值而不是绝对值：依赖树一直在长（曾经的 1.4 GiB 现在是 2.7 GiB），而这个
/// 比值跟着树的构成走，比字节数稳得多。
///
/// 另有约 1 GiB 的 packument 元数据会进缓存，但那是解开之后的体积，线上是 gzip 传的
/// （百来 MiB 量级），而且同样命中缓存，所以不算进这个比值里。
const TARBALL_RATIO: f64 = 0.36;

/// 还没装过、无从现量时落盘量的垫底值，同上实测。
const MEASURED_DISK: u64 = 2_700 * 1024 * 1024;

/// 一批塞给 `npm cache add` 的包数。
///
/// 一批之内 npm 自己并发取，所以批越大越快；但进度只在批与批之间往前走，批越大
/// 屏幕上越像卡着。12 个是个折中：既有并发，又不会让人盯着一个数字看好几分钟。
const PREFETCH_BATCH: usize = 12;

/// 还没 onboard 过时的端口，跟上游的默认值一致。
const DEFAULT_SERVER_PORT: u16 = 8804;
const DEFAULT_PG_PORT: u16 = 5432;

/// 无人值守下确认降级的开关。
const DOWNGRADE_YES: &str = "FUNDEPLOY_PAPERCLIP_DOWNGRADE_YES";

const EXTRAS: &[Extra] = &[
    Extra {
        name: "install-canary",
        about: "装开发版（官方 managed install）",
    },
    Extra {
        name: "check",
        about: "只查有没有新版，什么都不改 [canary|prod]",
    },
    Extra {
        name: "prefetch",
        about: "把下一版要下的 tarball 先灌进 npm 缓存 [canary|prod] [--background]",
    },
    Extra {
        name: "backup",
        about: "备份数据库（官方 db:backup）",
    },
    Extra {
        name: "channels",
        about: "看发布通道和当前跟的那个",
    },
    Extra {
        name: "doctor",
        about: "官方诊断；加 --repair 试着自动修",
    },
    Extra {
        name: "health",
        about: "官方健康检查，不通时退出码非 0",
    },
    Extra {
        name: "onboard",
        about: "首次配置向导",
    },
    Extra {
        name: "logs",
        about: "看日志 [-f] [-n 行数]",
    },
    Extra {
        name: "plugin",
        about: "插件 list / install [包名] / installed",
    },
    Extra {
        name: "service-install",
        about: "注册后台服务并设开机自启",
    },
];

// ------------------------------------------------------------------ 发布通道

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Channel {
    Prod,
    Canary,
}

impl Channel {
    fn parse(s: &str) -> Result<Self> {
        match s {
            "canary" => Ok(Self::Canary),
            "prod" | "latest" | "stable" => Ok(Self::Prod),
            other => bail!("未知通道 {other}（支持 canary / prod）"),
        }
    }

    /// 传给官方 `update` 的通道选项。
    fn flag(self) -> &'static str {
        match self {
            Self::Prod => "--latest",
            Self::Canary => "--canary",
        }
    }

    /// 通道在 npm 上对应的 dist-tag。
    ///
    /// 只给 [`Paperclip::prefetch_target`] 在问不出官方版本时垫底用。它跟官方的通道
    /// 解析不一定同步——实测 canary 的 dist-tag 是 2026.1015.0-canary.9，而同一时刻
    /// `update --check --canary` 给的是 2026.1014.0-canary.11。
    fn dist_tag(self) -> &'static str {
        match self {
            Self::Prod => "latest",
            Self::Canary => "canary",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Prod => "prod",
            Self::Canary => "canary",
        }
    }
}

// -------------------------------------------------------------------- 配置

/// 实例 config.json 里本模块用得到的那几项。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Config {
    /// 数据库是不是服务自己带起来的那份 PostgreSQL。
    ///
    /// 是的话它就是服务的子进程，这件事有三处后果：外部的 `DATABASE_URL` 会让 CLI
    /// 连错库、服务停着就备份不了、服务被强停后它可能变成孤儿占住端口。
    embedded_pg: bool,
    pg_dir: PathBuf,
    pg_port: u16,
    host: String,
    port: u16,
}

/// 解析 config.json。给空串就得到一份默认值——还没 onboard 过是正常状态，不是故障。
fn parse_config(raw: &str, default_pg_dir: &Path) -> Config {
    let v: serde_json::Value = serde_json::from_str(raw).unwrap_or(serde_json::Value::Null);
    let db = &v["database"];
    let server = &v["server"];
    Config {
        embedded_pg: db["mode"].as_str() == Some("embedded-postgres"),
        pg_dir: db["embeddedPostgresDataDir"]
            .as_str()
            .map(env::expand_tilde)
            .unwrap_or_else(|| default_pg_dir.to_path_buf()),
        pg_port: json_port(&db["embeddedPostgresPort"]).unwrap_or(DEFAULT_PG_PORT),
        host: server["host"].as_str().unwrap_or("127.0.0.1").to_string(),
        port: json_port(&server["port"]).unwrap_or(DEFAULT_SERVER_PORT),
    }
}

fn json_port(v: &serde_json::Value) -> Option<u16> {
    v.as_u64()
        .and_then(|n| u16::try_from(n).ok())
        .filter(|p| *p > 0)
}

// ------------------------------------------------------------------- 主体

/// npm 取回参数。
///
/// npm 默认 fetch-retries=2、fetch-timeout=5min，而 paperclip 单个 tarball 能到 370M
/// （@openai/codex-linux-x64），跨境直连极易中途断流；一次失败整棵 1.4G 的依赖树都要
/// 重来，所以重试次数和超时默认都放宽。
struct NpmFetch {
    retries: u32,
    min_timeout: u64,
    max_timeout: u64,
    timeout: u64,
}

pub struct Paperclip {
    home: PathBuf,
    instance: String,
    run_dir: PathBuf,
    log_file: PathBuf,
    /// 没有服务管理器时由 fundeploy 托管后台进程。`PAPERCLIP_FALLBACK=off` 关掉它，
    /// 维持官方行为。
    fallback: bool,
    /// `PAPERCLIP_CHANNEL` 显式指定的通道，覆盖 install / upgrade 各自的默认值。
    channel: Option<Channel>,
    proxy: Option<String>,
    no_proxy: Option<String>,
    network_hint: bool,
    install_via_npx: bool,
    npm_registry: Option<String>,
    prefer_offline: bool,
    fetch: NpmFetch,
    start_timeout: Duration,
    stop_timeout: Duration,
    update_timeout: Duration,
    log_lines: usize,
}

impl Paperclip {
    pub fn from_env() -> Self {
        let home = env::opt("PAPERCLIP_HOME")
            .map(|v| env::expand_tilde(&v))
            .unwrap_or_else(|| env::home().join(".paperclip"));
        let instance = env::str_or("PAPERCLIP_INSTANCE_ID", "default");
        // 运行时文件落在 ${PAPERCLIP_HOME}/run/<实例>/：官方在那底下只用 cli/ 和
        // instances/，不会撞上。刻意不放仓库里——fundeploy 以 apt / Homebrew /
        // ~/.local 形式分发，运行时根本不存在「仓库工作区」。
        let run_dir = env::opt("PAPERCLIP_RUN_DIR")
            .map(|v| env::expand_tilde(&v))
            .unwrap_or_else(|| home.join("run").join(&instance));
        let log_file = env::opt("PAPERCLIP_LOG_FILE")
            .map(|v| env::expand_tilde(&v))
            .unwrap_or_else(|| run_dir.join("paperclip.log"));

        Self {
            home,
            instance,
            run_dir,
            log_file,
            fallback: env::str_or("PAPERCLIP_FALLBACK", "auto") != "off",
            channel: env::opt("PAPERCLIP_CHANNEL").and_then(|v| match Channel::parse(&v) {
                Ok(c) => Some(c),
                Err(e) => {
                    ui::warn(format!("{e}；PAPERCLIP_CHANNEL 当没设处理。"));
                    None
                }
            }),
            proxy: env::opt("PAPERCLIP_HTTPS_PROXY").and_then(checked_proxy),
            no_proxy: env::opt("PAPERCLIP_NO_PROXY"),
            network_hint: env::flag("PAPERCLIP_NETWORK_HINT", true),
            install_via_npx: env::flag("PAPERCLIP_INSTALL_VIA_NPX", false),
            npm_registry: env::opt("PAPERCLIP_NPM_REGISTRY")
                .as_deref()
                .and_then(checked_registry),
            prefer_offline: env::flag("PAPERCLIP_NPM_PREFER_OFFLINE", false),
            fetch: NpmFetch {
                retries: env::num("PAPERCLIP_NPM_FETCH_RETRIES", 5),
                min_timeout: env::num("PAPERCLIP_NPM_FETCH_RETRY_MINTIMEOUT", 20_000),
                max_timeout: env::num("PAPERCLIP_NPM_FETCH_RETRY_MAXTIMEOUT", 180_000),
                timeout: env::num("PAPERCLIP_NPM_FETCH_TIMEOUT", 1_200_000),
            },
            // 升级后要等 embedded PostgreSQL 冷启动、数据库迁移和服务就绪，低配或
            // 受限网络的机器 60 秒常不够，超时就报错会把「还在启动」说成「升级失败」。
            start_timeout: secs("PAPERCLIP_START_HEALTH_TIMEOUT_SEC", 180),
            // 与官方 systemd 单元的 TimeoutStopSec=300 对齐：SIGTERM 之后服务要排空
            // 正在跑的 heartbeat run，提前 KILL 会打断执行中的任务。
            stop_timeout: secs("PAPERCLIP_STOP_TIMEOUT_SEC", 300),
            update_timeout: secs("PAPERCLIP_UPDATE_HEALTH_TIMEOUT_SEC", 180),
            log_lines: env::num("PAPERCLIP_LOG_TAIL_LINES", 200),
        }
    }

    // --------------------------------------------------------------- 路径

    fn instance_dir(&self) -> PathBuf {
        self.home.join("instances").join(&self.instance)
    }

    fn config_path(&self) -> PathBuf {
        self.instance_dir().join("config.json")
    }

    fn pid_file(&self) -> PathBuf {
        self.run_dir.join("paperclip.pid")
    }

    fn config(&self) -> Config {
        let default_pg_dir = self.instance_dir().join("db");
        let raw = std::fs::read_to_string(self.config_path()).unwrap_or_default();
        parse_config(&raw, &default_pg_dir)
    }

    // ------------------------------------------------------------ 官方 CLI

    /// 官方 shim 的位置。
    ///
    /// managed install 固定装到 `~/.local/bin/paperclipai`，而那个目录可能还没进
    /// PATH（新装的机器上要重开 shell 才生效），所以先看文件再查 PATH。
    fn cli_path(&self) -> Option<PathBuf> {
        let shim = env::home().join(".local/bin/paperclipai");
        if is_executable(&shim) {
            return Some(shim);
        }
        let global = self.global_cli_path();
        if is_executable(&global) {
            return Some(global);
        }
        cmd::find("paperclipai").map(PathBuf::from)
    }

    fn global_prefix(&self) -> PathBuf {
        self.home.join("cli").join("npm-global")
    }

    fn global_cli_path(&self) -> PathBuf {
        self.global_prefix().join("bin/paperclipai")
    }

    fn global_service_shim_path(&self) -> PathBuf {
        self.global_prefix().join("bin/paperclipai-service")
    }

    fn write_global_service_shim(&self, node_env: &[EnvEdit]) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let configured_node = node_env
            .iter()
            .find(|(key, _)| key == "PATH")
            .and_then(|(_, path)| path.as_deref())
            .and_then(|path| path.split(':').next())
            .map(|bin| Path::new(bin).join("node"))
            .filter(|path| is_executable(path));
        let node = match configured_node {
            Some(node) => node,
            None => PathBuf::from(cmd::require("node", "Paperclip 服务需要 Node.js")?),
        };
        let quote = |value: &str| format!("'{}'", value.replace('\'', "'\\''"));
        let script = format!(
            "#!/bin/sh\nexec {} {} \"$@\"\n",
            quote(&node.display().to_string()),
            quote(&self.global_cli_path().display().to_string())
        );
        let shim = self.global_service_shim_path();
        let temporary = shim.with_extension(format!("tmp-{}", std::process::id()));
        std::fs::write(&temporary, script).context("写 Paperclip 服务启动器失败")?;
        std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o755))?;
        std::fs::rename(&temporary, shim).context("启用 Paperclip 服务启动器失败")
    }

    fn global_registry(&self) -> String {
        self.npm_registry
            .clone()
            .or_else(|| {
                std::fs::read_to_string(self.global_prefix().join(".fundeploy-registry"))
                    .ok()
                    .as_deref()
                    .and_then(checked_registry)
            })
            .unwrap_or_else(|| NPM_MIRROR.to_string())
    }

    fn global_channel(&self) -> Channel {
        std::fs::read_to_string(self.global_prefix().join(".fundeploy-channel"))
            .ok()
            .and_then(|s| Channel::parse(s.trim()).ok())
            .unwrap_or(Channel::Prod)
    }

    fn prefetch_registry(&self) -> String {
        if is_executable(&env::home().join(".local/bin/paperclipai")) || self.install_via_npx {
            NPM_REGISTRY.to_string()
        } else {
            self.global_registry()
        }
    }

    fn using_global_cli(&self) -> bool {
        self.cli_path().as_deref() == Some(self.global_cli_path().as_path())
    }

    fn require_cli(&self) -> Result<PathBuf> {
        self.cli_path().ok_or_else(|| {
            anyhow!("没找到 paperclipai，先执行: fundeploy service paperclip install")
        })
    }

    /// 调官方 CLI 时要带的环境。
    ///
    /// embedded 模式下必须把 `DATABASE_URL` 从环境里删掉：操作者 shell 里为别的项目
    /// export 过一个，CLI 就会拿它去连——连的是另一个库，而且谁也不会报错。
    fn cli_env(&self, cfg: &Config) -> Vec<EnvEdit> {
        let mut env = vec![
            (
                "PAPERCLIP_HOME".to_string(),
                Some(self.home.display().to_string()),
            ),
            (
                "PAPERCLIP_INSTANCE_ID".to_string(),
                Some(self.instance.clone()),
            ),
        ];
        if cfg.embedded_pg {
            env.push(("DATABASE_URL".to_string(), None));
        }
        if self.using_global_cli() {
            env.push((
                "PAPERCLIP_SHIM_PATH".to_string(),
                Some(self.global_service_shim_path().display().to_string()),
            ));
        }
        env
    }

    fn exec(&self, args: &[&str]) -> Result<i32> {
        self.exec_with(args, &[])
    }

    /// 跑一条官方命令，输出直通终端，返回退出码。
    fn exec_with(&self, args: &[&str], extra_env: &[EnvEdit]) -> Result<i32> {
        self.exec_inner(args, extra_env, false)
    }

    /// 同 [`Self::exec_with`]，另外盯着 managed install 的暂存目录报进度。
    /// 只用在真会去装 payload 的那几条命令上（见 [`InstallWatch`]）。
    fn exec_watching(&self, args: &[&str], extra_env: &[EnvEdit]) -> Result<i32> {
        self.exec_inner(args, extra_env, true)
    }

    fn exec_inner(&self, args: &[&str], extra_env: &[EnvEdit], watch: bool) -> Result<i32> {
        let bin = self.require_cli()?;
        let mut env = self.cli_env(&self.config());
        env.extend_from_slice(extra_env);
        if self.using_global_cli() {
            env.extend(self.node_env()?);
        }

        let mut argv = vec![bin.display().to_string()];
        argv.extend(args.iter().map(|s| (*s).to_string()));
        let code = if watch {
            self.spawn_watching(&argv, &env)?
        } else {
            cmd::spawn_status_in(&argv, &env, None)?
        };

        // payload 被删或损坏而 shim 还在时，官方 CLI 抛的是 node 的原始堆栈，看不出
        // 是安装坏了。失败之后补一次探测把它翻译成能照着做的事；正常路径零开销。
        if code != Some(0) && !cli_runnable(&bin) {
            ui::warn(format!(
                "{} 跑不起来，paperclipai 的安装可能已损坏。\n\
                 修复: fundeploy service paperclip install",
                bin.display()
            ));
        }
        Ok(code.unwrap_or(-1))
    }

    /// 跑一条会装 payload 的命令，中途按暂存目录报进度。
    ///
    /// stderr 不是终端就原样退回阻塞那条路：没人看的时候不该为了画一个隐藏的进度条
    /// 每两秒走一遍七万个文件，非交互环境（CI、重定向到日志）的行为也就跟以前逐字节
    /// 一致。
    fn spawn_watching(&self, argv: &[String], env: &[EnvEdit]) -> Result<Option<i32>> {
        if !std::io::IsTerminal::is_terminal(&std::io::stderr()) {
            return cmd::spawn_status_in(argv, env, None);
        }
        let mut watch = InstallWatch::new(&self.home);
        let code = cmd::spawn_status_ticking(argv, env, None, WATCH_TICK, &mut || watch.tick());
        // 子进程被打断时暂存目录可能还在，进度条也就还挂着；收尾不能只写在成功路径上。
        watch.finish();
        code
    }

    fn must(&self, args: &[&str]) -> Result<()> {
        self.must_with(args, &[])
    }

    fn must_with(&self, args: &[&str], extra_env: &[EnvEdit]) -> Result<()> {
        match self.exec_with(args, extra_env)? {
            0 => Ok(()),
            code => bail!("paperclipai {} 失败（退出码 {code}）", args.join(" ")),
        }
    }

    /// 跑一条 `--json` 命令并解析 stdout。
    ///
    /// 刻意不看退出码：官方拿它传信息而不只是成败（`update --check` 有可用更新时退
    /// 10，像 diff 那样）。把非零当失败，恰好就把唯一需要升级的那种情况判成「查不到
    /// 版本」。判据只有一个——stdout 能不能解析成 JSON。
    fn json(&self, args: &[&str]) -> Option<serde_json::Value> {
        let bin = self.cli_path()?;
        let mut c = std::process::Command::new(&bin);
        c.args(args)
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let mut cli_env = self.cli_env(&self.config());
        if self.using_global_cli() {
            cli_env.extend(self.node_env().ok()?);
        }
        cmd::apply_env(&mut c, &cli_env);
        serde_json::from_slice(&c.output().ok()?.stdout).ok()
    }

    fn installed_version(&self) -> Option<String> {
        let bin = self.cli_path()?;
        let node_env = if self.using_global_cli() {
            self.node_env().ok()?
        } else {
            Vec::new()
        };
        let out =
            cmd::try_run_with([bin.display().to_string().as_str(), "--version"], &node_env).ok()?;
        out.success()
            .then(|| {
                out.stdout
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_string()
            })
            .filter(|s| !s.is_empty())
    }

    // ------------------------------------------------------- 官方服务管理器

    fn supervisor(&self) -> Supervisor {
        let Some(v) = self.json(&["service", "status", "--json"]) else {
            return Supervisor {
                available: None,
                pid: None,
                detail: None,
                message: String::new(),
            };
        };
        Supervisor {
            // 只有明确的 supported:false 才算「没有」。字段缺失说明是旧版本或换了
            // 格式，那种情况下走官方路径、让它自己报错，比我们猜一个强。
            available: Some(v["supported"].as_bool() != Some(false)),
            pid: v["pid"]
                .as_i64()
                .and_then(|p| i32::try_from(p).ok())
                .filter(|p| *p > 0),
            detail: v["detail"].as_str().map(str::to_string),
            message: v["message"]
                .as_str()
                .unwrap_or_default()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
        }
    }

    /// 官方 supervisor 不可用时，该不该由 fundeploy 接管。
    fn require_fallback(&self, sup: &Supervisor) -> Result<()> {
        if self.fallback {
            return Ok(());
        }
        bail!(
            "{}（PAPERCLIP_FALLBACK=off）\n\
             可选: `fundeploy service paperclip run` 前台跑，或在有登录会话的 shell 里执行 \
             `fundeploy service paperclip service-install --enable-linger`",
            sup.reason()
        )
    }

    // ------------------------------------------------------------ 端口与健康

    /// 一次健康探测：数据库端口（embedded 时）、服务端口、`/api/health` 都通。
    ///
    /// 刻意用裸 HTTP 而不是官方 `paperclipai health`：后者每次都要起一个 node 进程
    /// 加载整个 CLI bundle，按秒轮询最多 180 次扛不住。`health` 作为用户命令另给。
    fn healthy(&self, cfg: &Config) -> bool {
        if cfg.embedded_pg && !port::is_listening("127.0.0.1", cfg.pg_port) {
            return false;
        }
        port::is_listening(&cfg.host, cfg.port) && health_endpoint_ok(cfg)
    }

    /// 等端口和 `/api/health` 就绪。超时返回 false，由调用方决定怎么报。
    ///
    /// 首探就通过时一声不吭：升级编排里 start 已经等过一轮，再打一遍只是噪音。
    fn wait_healthy(&self, cfg: &Config, timeout: Duration) -> bool {
        if self.healthy(cfg) {
            return true;
        }
        let mut targets = format!("Paperclip {} 和 /api/health", cfg.port);
        if cfg.embedded_pg {
            targets = format!("PostgreSQL {}、{targets}", cfg.pg_port);
        }
        ui::step(format!("检查 {targets}（最长 {} 秒）", timeout.as_secs()));

        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_secs(1));
            if self.healthy(cfg) {
                return true;
            }
        }
        false
    }

    /// 收掉 embedded PostgreSQL。
    ///
    /// 它是服务的子进程，但托管进程被收掉之后它可能变成孤儿继续占着端口，换 payload
    /// 或重启时就会失败。所有平台都要收，不是 macOS 专属。
    ///
    /// 停不下来就停手，不升级成 SIGKILL：那会让下次启动走一轮 WAL 崩溃恢复，不该由
    /// 一条 upgrade 顺手触发。
    fn stop_embedded_pg(&self, cfg: &Config) -> Result<()> {
        let Some(pid) = postmaster_pid(&cfg.pg_dir.join("postmaster.pid")) else {
            return Ok(());
        };
        if !proc::is_alive(pid) {
            return Ok(());
        }
        // 认准再动手：PID 会被复用，这个文件可能是上次崩溃留下的。
        let cmdline = process_cmdline(pid).unwrap_or_default();
        let dir = cfg.pg_dir.display().to_string();
        if !cmdline.contains("postgres") || !cmdline.contains(&dir) {
            bail!("PID {pid} 跑的不是 {dir} 的 PostgreSQL，没有动它");
        }

        ui::step(format!("停止 embedded PostgreSQL（PID {pid}）"));
        proc::term(pid);
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if !proc::is_alive(pid) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(300));
        }
        bail!("embedded PostgreSQL 在 30 秒内没停下来，升级中止")
    }

    // ------------------------------------------------------- fundeploy 托管

    /// fundeploy 托管的那个进程的 PID；没在跑返回 None。
    fn fallback_pid(&self) -> Option<i32> {
        let pid = proc::read_pid_file(&self.pid_file())?;
        if !proc::is_alive(pid) {
            return None;
        }
        // PID 会被复用，而这份文件可能是上次重启前留下的。只认命令行里确实是
        // paperclipai 的进程，否则宁可当成「没在跑」，也不能朝陌生进程发信号。
        process_cmdline(pid)?.contains("paperclipai").then_some(pid)
    }

    fn start_fallback(&self, cfg: &Config, reason: &str) -> Result<()> {
        let bin = self.require_cli()?;
        if let Some(pid) = self.fallback_pid() {
            ui::info(format!(
                "Paperclip 已由 fundeploy 在后台运行（PID {pid}，日志 {}）。",
                self.log_file.display()
            ));
            return Ok(());
        }
        // 端口上已经有别的进程（比如一个 `run` 起的前台实例）时再起一个，只会抢不到
        // 端口后自己退出，白留一份 PID 文件。
        if port::is_listening(&cfg.host, cfg.port) {
            match port::listener_pid(cfg.port) {
                Some(other) => bail!(
                    "端口 {} 已被 PID {other} 占着（可能是 `fundeploy service paperclip run` 的前台实例），先停掉再试",
                    cfg.port
                ),
                None => bail!(
                    "端口 {} 已经有人在听（查不到 PID，多半属于别的用户），先腾出来再试",
                    cfg.port
                ),
            }
        }

        let mut spawn_env = self.cli_env(cfg);
        spawn_env.extend(self.node_env()?);
        // 刻意不设 PAPERCLIP_SERVICE_MANAGED=1：它只用来让 run 跳过「同实例已在
        // supervisor 下运行」的检查，而这里恰好要保留那层保护。

        ui::step(reason);
        ui::step(format!(
            "改由 fundeploy 托管后台进程；PID {}，日志 {}",
            self.pid_file().display(),
            self.log_file.display()
        ));

        let argv = [
            bin.display().to_string(),
            "run".to_string(),
            "--instance".to_string(),
            self.instance.clone(),
        ];
        // cwd 对齐官方单元的 WorkingDirectory=%h：别让服务继承调用者的目录，否则
        // agent 的 fallback cwd 会随在哪儿执行 start 而变。
        let pid = proc::spawn_detached(&argv, &env::home(), &spawn_env, &self.log_file)?;
        proc::write_pid_file(&self.pid_file(), pid)?;

        if self.wait_healthy(cfg, self.start_timeout) {
            ui::ok(format!(
                "Paperclip 已启动（PID {pid}，http://{}:{}），健康检查通过。",
                cfg.host, cfg.port
            ));
            return Ok(());
        }

        if !proc::is_alive(pid) {
            let _ = std::fs::remove_file(self.pid_file());
            ui::warn("进程已经退出，日志末尾：");
            print_tail(&self.log_file, 40);
            bail!(
                "Paperclip 启动失败（完整日志: {}）",
                self.log_file.display()
            );
        }
        ui::warn(format!("进程还在（PID {pid}），日志末尾："));
        print_tail(&self.log_file, 40);
        bail!(
            "Paperclip 没在 {} 秒内通过健康检查（用 status / logs 确认，stop 停止）",
            self.start_timeout.as_secs()
        )
    }

    /// 停掉 fundeploy 托管的进程并清掉 PID 文件。返回有没有真停掉一个进程。
    ///
    /// `quiet`：没东西可停时不出声，供官方 stop 之后的收尾调用——那条路径已经有输出了。
    fn stop_fallback(&self, quiet: bool) -> Result<bool> {
        let Some(pid) = self.fallback_pid() else {
            if !quiet {
                ui::info("Paperclip 没在运行（没有 fundeploy 托管的进程）。");
            }
            let _ = std::fs::remove_file(self.pid_file());
            return Ok(false);
        };
        ui::step(format!(
            "停止 fundeploy 托管的 Paperclip（PID {pid}，最长等 {} 秒排空运行中的任务）",
            self.stop_timeout.as_secs()
        ));
        // 整组停：spawn_detached 做过 setsid，这个 PID 就是组 leader，而服务会派生
        // 数据库和 agent 子进程，只停父进程会留下孤儿。
        proc::stop_pid(pid, self.stop_timeout, true)?;
        let _ = std::fs::remove_file(self.pid_file());
        if !quiet {
            ui::ok("已停止 fundeploy 托管的 Paperclip。");
        }
        Ok(true)
    }

    fn ensure_port_free(&self) -> Result<()> {
        let cfg = self.config();
        super::force_free_port("Paperclip", &cfg.host, cfg.port);
        if port::is_listening(&cfg.host, cfg.port) {
            bail!(
                "Paperclip 的端口 {} 仍被占用，已中止重启；请确认占用者后再试",
                cfg.port
            );
        }
        Ok(())
    }

    /// 官方 `service stop` 报错之后，判断这是不是「本来就停着」。
    ///
    /// 停着的服务再停一次该是个 no-op，但官方 stop 把服务管理器的原始错误直接升成了
    /// 失败：launchd 后端的 `stop()` 是一句裸的 `launchctl bootout`，job 没加载时它以
    /// ESRCH(3) 退出（`Boot-out failed: 3: No such process`），而同一个 bootout 在官方
    /// 的 install / uninstall 里都带着 `.catch()`，只有 stop 没有；systemd 后端的
    /// `systemctl --user stop` 对没加载的单元同样非零退出。
    ///
    /// 只有核实过才能说「已经停了」——真停不下来（没权限、卡在退出中、job 还加载着会
    /// 被 KeepAlive 拉回来）必须保留失败语义，否则又是一次假成功。
    ///
    /// 不过报失败之前先动一次手：端口上还坐着东西就按端口把它收掉（见
    /// [`super::force_free_port`]），这跟三族服务的 stop 收尾是同一套做法。
    fn confirm_stopped(&self, code: i32, reaped: bool) -> Result<()> {
        let cfg = self.config();
        let sup = self.supervisor();
        let listening = port::is_listening(&cfg.host, cfg.port);
        if let Some(evidence) = still_running(&sup, listening, cfg.port) {
            ui::warn(format!("官方 stop 没把它停下来：{evidence}"));
            // 只按端口强停，不去碰服务管理器里那个 PID：那条进程有 KeepAlive 守着，
            // 杀掉只会被立刻拉起来，把「停不下来」变成更难看的「停了又起」。
            super::force_free_port("Paperclip", &cfg.host, cfg.port);

            // 重新问一遍状态：强停可能已经解决问题，也可能根本没碰到真正的占用者。
            let sup = self.supervisor();
            let listening = port::is_listening(&cfg.host, cfg.port);
            if let Some(evidence) = still_running(&sup, listening, cfg.port) {
                bail!("paperclipai service stop 失败（退出码 {code}），强停也没解决：{evidence}");
            }
            ui::ok("已强制停止 Paperclip（官方 stop 没停下来，按端口收的）。");
            return Ok(());
        }
        ui::ok(if reaped {
            "已停止 fundeploy 托管的 Paperclip。"
        } else {
            "Paperclip 本来就没在跑，不用停。"
        });
        ui::hint(format!(
            "上面官方 service stop 的报错（退出码 {code}）不改变结果：服务管理器里没有在跑的实例，\
             端口 {} 也没人在听。服务没加载时 launchctl bootout / systemctl stop 报的就是这种错，\
             官方 stop 没把它当成 no-op。",
            cfg.port
        ));
        Ok(())
    }

    fn status_fallback(&self, cfg: &Config, supervisor_unavailable: Option<&str>) {
        let listening = port::is_listening(&cfg.host, cfg.port);
        let healthy = health_endpoint_ok(cfg);
        let mut card = ui::Card::new("Paperclip");

        let (pst, ptext) = match self.fallback_pid() {
            Some(pid) => (ui::State::Up, format!("运行中 · PID {pid}")),
            // PID 文件还在但进程没了：和「本来就没跑」不是一回事，后者不用清理。
            None if self.pid_file().exists() => {
                (ui::State::Down, "没在运行 · PID 文件已失效".to_string())
            }
            None => (ui::State::Idle, "没在运行".to_string()),
        };
        card = card.row_state("fundeploy 托管", pst, ptext);

        card = match self.installed_version() {
            Some(v) => card.row("版本", v),
            None => card.row_state("版本", ui::State::Idle, "未安装"),
        };
        card = card.row_state(
            format!("端口 {}", cfg.port),
            if listening {
                ui::State::Up
            } else {
                ui::State::Idle
            },
            yes_no(listening, "监听中", "没监听"),
        );
        // 端口在听但健康检查不过，是「起来了又崩在里面」——这个组合最该被看见，
        // 所以单独标红，而不是跟「没在跑」一个成色。
        let hst = match (listening, healthy) {
            (_, true) => ui::State::Up,
            (true, false) => ui::State::Down,
            (false, false) => ui::State::Idle,
        };
        card = card.row_state("健康检查", hst, yes_no(healthy, "通过", "不通过"));
        if let Some(why) = supervisor_unavailable {
            card = card
                .row_state("官方管理器", ui::State::Idle, "不可用")
                .note(why);
        }

        let badge = match (listening, healthy) {
            (true, true) => (ui::State::Up, "在运行"),
            (true, false) => (ui::State::Partial, "端口在听，健康检查不过"),
            _ => (ui::State::Idle, "未运行"),
        };
        card.badge(badge.0, badge.1).print();

        // 这两条都比框里任何字段长，摆框外免得把卡片撑到满屏或者被省略成认不出。
        ui::hint(format!(
            "健康检查 http://{}:{}/api/health",
            cfg.host, cfg.port
        ));
        ui::hint(format!("日志 {}", self.log_file.display()));
    }

    // --------------------------------------------------------------- Node

    /// 保证子进程的 PATH 上有个够新的 node，返回要追加的环境改动。
    fn node_env(&self) -> Result<Vec<EnvEdit>> {
        let current = node_version();
        if current
            .as_deref()
            .is_some_and(|v| !version::is_newer(NODE_MIN, v))
        {
            return Ok(Vec::new());
        }
        if let Some(bin) = nvm_node_bin(NODE_MIN) {
            let path = format!(
                "{}:{}",
                bin.display(),
                std::env::var("PATH").unwrap_or_default()
            );
            return Ok(vec![("PATH".to_string(), Some(path))]);
        }
        bail!(
            "需要 Node.js {NODE_MIN}+，当前 {}。可执行: nvm install 24",
            current.unwrap_or_else(|| "没装".to_string())
        )
    }

    fn npm_command(&self) -> Result<(String, Vec<EnvEdit>)> {
        let node_env = self.node_env()?;
        if let Some((_, Some(path))) = node_env.iter().find(|(key, _)| key == "PATH") {
            if let Some(bin) = path.split(':').next() {
                let npm = Path::new(bin).join("npm");
                if is_executable(&npm) {
                    return Ok((npm.display().to_string(), node_env));
                }
            }
        }
        Ok((
            cmd::require("npm", "它随 Node.js 一起装：https://nodejs.org/")?,
            node_env,
        ))
    }

    // ---------------------------------------------------------------- npm

    // ------------------------------------------------------------ 安装与升级

    fn install_channel(&self, ch: Channel) -> Result<()> {
        if !is_executable(&env::home().join(".local/bin/paperclipai")) && !self.install_via_npx {
            return self.install_global_with_fallback(ch);
        }
        if self.using_global_cli() {
            bail!("已有 npm 镜像安装，不能同时使用 managed install；先卸载旧版再切换");
        }
        let npmrc = Npmrc::write(NPM_REGISTRY);
        let mut env = self.npm_env(npmrc.as_ref());
        self.print_network_hint(ch);

        // 执行器优先级：本机已装的 CLI > npx。
        //
        // `npx paperclipai@latest` 会把整棵依赖树再下一份到 ~/.npm/_npx（实测 1.4G），
        // 而它唯一的用途是执行一次 install 子命令；install 自己会解析目标版本，跟执行
        // 它的那个 CLI 的版本无关，所以复用已有 CLI 不会装到旧版。
        // PAPERCLIP_INSTALL_VIA_NPX=1 是本机 CLI 坏掉时的逃生门。
        let reuse = if self.install_via_npx {
            None
        } else {
            self.cli_path().filter(|b| cli_runnable(b))
        };

        let mut argv: Vec<String> = Vec::new();
        match reuse {
            Some(bin) => {
                ui::step("复用本机 paperclipai 执行 managed install（省掉 npx 那份重复下载）");
                argv.push(bin.display().to_string());
            }
            None => {
                // 只有这条路需要版本门槛：此时还没有 shim，npx 用的就是 PATH 上那个 node。
                env.extend(self.node_env()?);
                cmd::require("npx", "它随 Node.js 一起装：https://nodejs.org/")?;
                for a in [
                    "npx",
                    "--yes",
                    "--registry",
                    NPM_REGISTRY,
                    "paperclipai@latest",
                ] {
                    argv.push(a.to_string());
                }
            }
        }
        argv.push("install".to_string());
        argv.push("--yes".to_string());
        if ch == Channel::Canary {
            argv.push("--canary".to_string());
        }

        let mut full = self.cli_env(&self.config());
        full.extend(env);
        // 走 npx 引导时，npx 自己那份下载是 npm 直接打在终端上的（那会儿暂存目录还不
        // 存在，进度条不出现），紧接着的 managed install 那段沉默才由 watch 接管。
        match self.spawn_watching(&argv, &full)? {
            Some(0) => {}
            other => bail!(
                "managed install 失败（退出码 {}）",
                other.map_or_else(|| "信号终止".to_string(), |c| c.to_string())
            ),
        }
        self.must(&["--version"])
    }

    fn install_global_with_fallback(&self, ch: Channel) -> Result<()> {
        let primary = self.global_registry();
        let mut registries = vec![primary.as_str()];
        if self.npm_registry.is_none() {
            for registry in [NPM_MIRROR, NPM_MIRROR_TENCENT, NPM_REGISTRY] {
                if !registries.contains(&registry) {
                    registries.push(registry);
                }
            }
        }
        let mut error = None;
        for registry in registries {
            match self.install_global(ch, registry) {
                Ok(()) => return Ok(()),
                Err(e) => {
                    ui::warn(format!("从 {registry} 安装失败: {e}"));
                    error = Some(e);
                }
            }
        }
        Err(error.expect("至少有一个 npm 源"))
    }

    fn install_global(&self, ch: Channel, registry: &str) -> Result<()> {
        let (npm, node_env) = self.npm_command()?;
        let npmrc = Npmrc::write(registry);
        let mut env = self.npm_env(npmrc.as_ref());
        env.extend(node_env);
        let prefix = self.global_prefix();
        ui::step(format!(
            "从 {registry} 安装 Paperclip {} 到 {}",
            ch.name(),
            prefix.display()
        ));
        let spec = format!("paperclipai@{}", ch.dist_tag());
        let registry_arg = format!("--registry={registry}");
        let scope_arg = format!("--@paperclipai:registry={registry}");
        let prefix_arg = prefix.display().to_string();
        let code = cmd::spawn_status_in(
            [
                &npm,
                "install",
                "--global",
                "--prefix",
                &prefix_arg,
                &spec,
                &registry_arg,
                &scope_arg,
                "--no-audit",
                "--no-fund",
                &format!("--allow-scripts={NPM_ALLOW_SCRIPTS}"),
            ],
            &env,
            None,
        )?;
        if code != Some(0) {
            bail!(
                "npm 安装失败（退出码 {}）",
                code.map_or_else(|| "信号终止".to_string(), |c| c.to_string())
            );
        }
        self.must(&["--version"])?;
        self.write_global_service_shim(&env)?;
        std::fs::write(prefix.join(".fundeploy-registry"), registry)
            .context("保存 Paperclip npm 镜像地址失败")?;
        std::fs::write(prefix.join(".fundeploy-channel"), ch.name())
            .context("保存 Paperclip 安装通道失败")
    }

    fn check(&self, ch: Channel) -> Result<()> {
        let Some(c) = self.update_check(ch) else {
            bail!(
                "取不到 {} 通道的版本信息（网络不通，或者还没装 paperclipai）",
                ch.name()
            );
        };
        ui::info(format!("当前版本: {}", or_unknown(&c.current)));
        ui::info(format!("通道目标: {}", or_unknown(&c.target)));
        if c.downgrade {
            ui::info("结论: 目标比当前旧，执行 upgrade 会是一次降级（要确认）");
        } else if c.available {
            ui::info(format!(
                "结论: 有新版可升，执行: PAPERCLIP_CHANNEL={} fundeploy service paperclip upgrade",
                ch.name()
            ));
            ui::hint(format!(
                "升级最久的一段是下载。想先把它跑完、等到真升级时只剩解包: \
                 fundeploy service paperclip prefetch {} --background",
                ch.name()
            ));
        } else {
            ui::info("结论: 已经是该通道的最新版");
        }
        Ok(())
    }

    fn update_check(&self, ch: Channel) -> Option<UpdateCheck> {
        if self.using_global_cli() {
            let (npm, node_env) = self.npm_command().ok()?;
            let registry = self.global_registry();
            let npmrc = Npmrc::write(&registry);
            let mut env = self.npm_env(npmrc.as_ref());
            env.extend(node_env);
            let spec = format!("paperclipai@{}", ch.dist_tag());
            let out = cmd::try_run_with(
                [
                    &npm,
                    "view",
                    &spec,
                    "version",
                    "--registry",
                    &registry,
                    "--json",
                ],
                &env,
            )
            .ok()?;
            if !out.success() {
                return None;
            }
            let target = prefetch::npm_view_version(&out.stdout)?;
            let current = self.installed_version()?;
            return Some(UpdateCheck {
                available: version::is_newer(&target, &current),
                downgrade: version::is_newer(&current, &target),
                current,
                target,
            });
        }
        let v = self.json(&["update", ch.flag(), "--check", "--json"])?;
        Some(UpdateCheck {
            current: v["currentVersion"].as_str().unwrap_or_default().to_string(),
            target: v["targetVersion"].as_str().unwrap_or_default().to_string(),
            available: v["updateAvailable"].as_bool().unwrap_or(false),
            downgrade: v["downgrade"].as_bool().unwrap_or(false),
        })
    }

    // ---------------------------------------------------------------- 预热

    fn upgrade_channel(&self, ch: Channel) -> Result<()> {
        let npmrc = Npmrc::write(NPM_REGISTRY);
        let npm = self.npm_env(npmrc.as_ref());
        if !self.using_global_cli() {
            self.print_network_hint(ch);
        }

        let mut args: Vec<&str> = vec![ch.flag()];
        // 先问官方要不要动，别自己比版本号。问不出来（离线、还没装）就不阻塞，往下走。
        if let Some(c) = self.update_check(ch) {
            if c.downgrade {
                // canary 机器上 --latest 指向的 stable 往往比当前版本旧；官方 update
                // 把这种情况当显式降级、要 -y 确认，不补的话非交互环境里会卡住。
                ui::warn(format!(
                    "{} 通道的 {} 比当前的 {} 旧，这是一次降级。",
                    ch.name(),
                    or_unknown(&c.target),
                    or_unknown(&c.current)
                ));
                ui::confirm_destructive(
                    &format!(
                        "确认把 Paperclip 从 {} 降到 {}？（不会备份数据库，要备份先执行 backup）",
                        or_unknown(&c.current),
                        or_unknown(&c.target)
                    ),
                    DOWNGRADE_YES,
                )?;
                args.push("-y");
            } else if !c.available {
                ui::info(format!(
                    "已经是 {} 通道的最新版 {}，跳过更新（没有重启服务；要重启用 restart）",
                    ch.name(),
                    or_unknown(&c.current)
                ));
                return Ok(());
            } else {
                ui::step(format!(
                    "{} 通道有新版本: {} → {}",
                    ch.name(),
                    or_unknown(&c.current),
                    or_unknown(&c.target)
                ));
            }
        }
        self.apply_update(&args, &npm)
    }

    /// 停服 → 一次官方 update → doctor --repair → 重启 → 健康检查。upgrade 和
    /// rollback 共用这套编排。
    ///
    /// 刻意不做升级前备份，还给官方 update 传 `--no-backup` 关掉它自己那次：实测一次
    /// 备份 370M、耗时数十秒，而 update 换的是 payload，不动数据目录；payload 要退回去
    /// 有 rollback。备份是单独的 backup 动作，需要就先执行一次再升级。
    fn apply_update(&self, update_args: &[&str], npm: &[EnvEdit]) -> Result<()> {
        let cfg = self.config();

        // 两条通道都要停：官方 service stop 在没有服务管理器的机器上会假成功（返回 0
        // 且什么都没停），fundeploy 托管的那个只能从 PID 文件收。这里刻意尽力而为、
        // 不因为停不掉就退出——真正的护栏是紧随其后的端口检查。
        let _ = self.exec(&["service", "stop"]);
        let _ = self.stop_fallback(true);

        // 假成功会让上面两步什么都没停下，继续下去就是在服务运行中替换 payload；用
        // `run` 前台起的实例正是这种情况。所以无条件确认端口真的放开了。
        if port::is_listening(&cfg.host, cfg.port) {
            bail!(
                "Paperclip 还在监听 {}（可能是 `fundeploy service paperclip run` 起的前台实例，\
                 不归服务管理器管），先停掉再试",
                cfg.port
            );
        }
        if cfg.embedded_pg {
            self.stop_embedded_pg(&cfg)?;
        }

        let mut argv = vec!["update"];
        argv.extend_from_slice(update_args);
        argv.push("--no-backup");
        // update 是整条命令里最长的一段沉默：上游从头到尾只在最后 emit 一行。
        let update_result = if self.using_global_cli() {
            let ch = if update_args.contains(&"--canary") {
                Channel::Canary
            } else if update_args.contains(&"--latest") {
                Channel::Prod
            } else {
                bail!("npm 镜像安装没有 managed payload，无法使用官方 rollback");
            };
            self.install_global_with_fallback(ch).map(|_| 0)
        } else {
            self.exec_watching(&argv, npm)
        };
        let failed = match update_result {
            Ok(0) => None,
            Ok(code) => Some(format!("退出码 {code}")),
            Err(e) => Some(e.to_string()),
        };
        if let Some(reason) = failed {
            // 尽力把服务拉起来，但保留 update 本身的失败语义。
            if let Err(e) = self.start() {
                ui::warn(format!("换包失败后想把服务拉起来，也没成功: {e}"));
            }
            bail!("Paperclip 更新失败: {reason}");
        }

        // 只对 managed install 修复 install store；npm 安装没有那套 store，运行修复
        // 反而可能重建旧 shim，把服务入口切回 managed payload。
        if !self.using_global_cli() && self.exec(&["doctor", "--repair", "--yes"])? != 0 {
            ui::warn(
                "doctor --repair 没全过，继续启动；起来之后可执行 \
                 `fundeploy service paperclip doctor` 看详情",
            );
        }

        self.start()?;

        // payload 换过之后配置可能也变了，重新读一次。
        let cfg = self.config();
        if self.wait_healthy(&cfg, self.update_timeout) {
            ui::ok("Paperclip 升级完成，健康检查通过。");
            return Ok(());
        }
        // payload 已经换完、start 也执行过了，服务很可能只是还在起——先把状态打出来，
        // 别把「仍在启动」报成「升级失败」。
        ui::warn("健康检查没通过，当前状态：");
        let _ = self.status();
        bail!(
            "Paperclip 升级后没在 {} 秒内通过健康检查（payload 已经换好，服务可能还在启动，\
             用 status / logs 确认）",
            self.update_timeout.as_secs()
        )
    }

    // -------------------------------------------------------------- 其他动作

    fn backup(&self, args: &[&str]) -> Result<()> {
        let cfg = self.config();
        // embedded PostgreSQL 是服务的子进程，服务停着它就没在跑，db:backup 必然连不上。
        // 在这儿说清楚，别把上游的连接错误丢给人自己猜。
        if cfg.embedded_pg && !port::is_listening("127.0.0.1", cfg.pg_port) {
            bail!(
                "embedded PostgreSQL 没在监听 {}（它随 Paperclip 一起停），先 start 再备份",
                cfg.pg_port
            );
        }
        self.must(&prepend(&["db:backup"], args))
    }

    fn onboard(&self, args: &[&str]) -> Result<()> {
        let mut argv = prepend(&["onboard"], args);
        // 无人值守下向导会停在第一个问题上等输入，表现是「卡死」。
        if !ui::interactive() && !args.iter().any(|a| *a == "--yes" || *a == "-y") {
            argv.push("--yes");
        }
        self.must(&argv)
    }
}

// --------------------------------------------------------------- Service

impl Service for Paperclip {
    fn name(&self) -> &str {
        "paperclip"
    }

    fn summary(&self) -> &str {
        "Paperclip 本机服务"
    }

    fn ports(&self) -> Vec<u16> {
        vec![self.config().port]
    }

    fn install(&self, version: Option<&str>) -> Result<()> {
        let ch = match version {
            None => self.channel.unwrap_or(Channel::Prod),
            Some(v) => Channel::parse(v).context(
                "paperclip 的 managed install 自己决定装哪个版本，钉不到指定版本；\
                 这里的 --version 只认通道",
            )?,
        };
        self.install_channel(ch)
    }

    fn upgrade(&self) -> Result<()> {
        // managed install 默认跟 canary；npm 安装跟随首次安装时选的通道。
        let default = if self.using_global_cli() {
            self.global_channel()
        } else {
            Channel::Canary
        };
        self.upgrade_channel(self.channel.unwrap_or(default))
    }

    fn rollback(&self, version: &str) -> Result<()> {
        // 官方只在 install store 里留上一份 payload，没有「回到任意版本」这回事。
        if !matches!(version, "previous" | "prev" | "last" | "-") {
            bail!(
                "paperclip 只能退回 install store 里留着的上一份 payload，退不到指定版本。\n\
                 执行: fundeploy service paperclip rollback previous"
            );
        }
        if self.using_global_cli() {
            bail!("npm 镜像安装不保留 managed payload，无法 rollback；可重新安装指定通道");
        }
        let npmrc = Npmrc::write(NPM_REGISTRY);
        let npm = self.npm_env(npmrc.as_ref());
        self.apply_update(&["--rollback"], &npm)
    }

    fn start(&self) -> Result<()> {
        let sup = self.supervisor();
        if sup.available != Some(false) {
            if self.using_global_cli() {
                self.write_global_service_shim(&self.node_env()?)?;
            }
            self.must(&["service", "start"])?;
            // launchd 返回成功只表示 job 已加载，入口丢失或进程崩溃仍可能没起来。
            if !self.wait_healthy(&self.config(), self.start_timeout) {
                bail!(
                    "Paperclip 没在 {} 秒内通过健康检查，请用 status / logs 查看原因",
                    self.start_timeout.as_secs()
                );
            }
            return Ok(());
        }
        self.require_fallback(&sup)?;
        self.start_fallback(&self.config(), &sup.reason())
    }

    fn run(&self) -> Result<()> {
        let bin = self.require_cli()?;
        // 官方 run 自带的「同实例已在 supervisor 下运行」检查覆盖不到 fundeploy 托管的
        // 进程（那种机器上根本没有 supervisor），不挡住就会起第二个实例去抢同一个端口。
        if let Some(pid) = self.fallback_pid() {
            bail!(
                "Paperclip 已由 fundeploy 在后台运行（PID {pid}），\
                 先执行: fundeploy service paperclip stop"
            );
        }
        let mut env = self.cli_env(&self.config());
        env.extend(self.node_env()?);
        let argv = [bin.display().to_string(), "run".to_string()];
        match cmd::spawn_status_in(argv, &env, None)? {
            // 被信号终止就是 Ctrl+C，那是前台运行的正常收场。
            Some(0) | None => Ok(()),
            Some(code) => bail!("paperclipai run 退出（退出码 {code}）"),
        }
    }

    fn stop(&self) -> Result<()> {
        let sup = self.supervisor();
        if sup.available != Some(false) {
            let code = self.exec(&["service", "stop"])?;
            // 无条件再收一次 fundeploy 托管的进程：机器从「没有 supervisor」变成「有」
            // 之后，上次 start 留下的那个不归 supervisor 管辖，只能从 PID 文件收尾。
            // 官方那步失败时也要收——失败的常见原因恰恰是服务管理器里没有这个服务，
            // 那种机器上真在跑的就是托管的那个。
            let reaped = self.stop_fallback(true)?;
            if code != 0 {
                // 官方 stop 分不清「停不下来」和「本来就停着」，自己核实一遍。
                return self.confirm_stopped(code, reaped);
            }
            return self.ensure_port_free();
        }
        // 关掉兜底时也不该假成功：官方 stop 在这种机器上什么都不会停。
        self.require_fallback(&sup)?;
        self.stop_fallback(false)?;
        self.ensure_port_free()
    }

    fn restart(&self) -> Result<()> {
        self.stop()?;
        ui::blank();
        self.start()
    }

    fn status(&self) -> Result<()> {
        let sup = self.supervisor();
        let cfg = self.config();
        if sup.available != Some(false) {
            let code = self.exec(&["service", "status"])?;
            // 只在确实留着 PID 文件时才追加托管那段，免得正常机器上多出无关输出。
            if !self.pid_file().exists() {
                return match code {
                    0 => Ok(()),
                    c => bail!("paperclipai service status 失败（退出码 {c}）"),
                };
            }
            ui::blank();
            self.status_fallback(&cfg, None);
            return Ok(());
        }
        // 官方管理器不可用的原因是上游给的一长段英文，塞进卡片会把框撑到满屏，
        // 所以进框外的脚注，原文一个字不动——那是诊断信息。
        let reason = sup.reason();
        self.status_fallback(&cfg, Some(&reason));
        Ok(())
    }

    fn uninstall(&self) -> Result<()> {
        ui::info(format!(
            "Paperclip 的数据目录 {} 不会删。",
            self.home.display()
        ));
        ui::confirm_destructive(
            "确认卸载 Paperclip 的官方服务和 CLI？",
            &super::uninstall_env_var("paperclip"),
        )?;
        // 先停再卸：官方 service uninstall 在没有服务管理器的机器上同样假成功，不先
        // 收掉 fundeploy 托管的进程就会在服务运行中把它的 payload 删掉。
        self.stop_fallback(false)?;
        self.must(&["service", "uninstall"])?;
        if self.using_global_cli() {
            let prefix = self.global_prefix().display().to_string();
            let (npm, mut npm_env) = self.npm_command()?;
            npm_env.push(("npm_config_audit".to_string(), Some("false".to_string())));
            match cmd::spawn_status_in(
                [
                    &npm,
                    "uninstall",
                    "--global",
                    "--prefix",
                    &prefix,
                    "paperclipai",
                ],
                &npm_env,
                None,
            )? {
                Some(0) => {}
                other => bail!("npm 卸载 paperclipai 失败（退出码 {other:?}）"),
            }
        } else {
            self.must(&["uninstall"])?;
        }
        ui::ok(format!(
            "已卸载 Paperclip 的服务和 CLI；数据留在 {}。",
            self.home.display()
        ));
        Ok(())
    }

    fn extras(&self) -> &'static [Extra] {
        EXTRAS
    }

    fn extra(&self, name: &str, args: &[String]) -> Result<()> {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        match name {
            "install-canary" => self.install_channel(Channel::Canary),
            "check" => {
                let ch = match args.first() {
                    Some(s) => Channel::parse(s)?,
                    None => self.channel.unwrap_or(Channel::Canary),
                };
                self.check(ch)
            }
            "prefetch" => self.prefetch(&args),
            "backup" => self.backup(&args),
            "channels" => self.must(&prepend(&["channels"], &args)),
            "doctor" => self.must(&prepend(&["doctor"], &args)),
            "health" => self.must(&prepend(&["health"], &args)),
            "onboard" => self.onboard(&args),
            "logs" => self.logs(&args),
            "plugin" => self.plugin(&args),
            "service-install" => self.must(&prepend(&["service", "install"], &args)),
            other => bail!("paperclip 没有 {other} 这个动作"),
        }
    }
}

// --------------------------------------------------------------- 小零件

/// 官方 `service status --json` 的判读。
struct Supervisor {
    /// 有没有可用的服务管理器。`None` 表示问不出来——那时候不猜，让官方命令自己报错。
    available: Option<bool>,
    pid: Option<i32>,
    /// 服务在服务管理器眼里的状态。launchd 给 `running` / `loaded`，job 没加载时
    /// （`launchctl print` 失败）这个字段根本不存在；systemd 给的是 ActiveState。
    detail: Option<String>,
    message: String,
}

impl Supervisor {
    fn reason(&self) -> String {
        if self.message.is_empty() {
            "没检测到可用的服务管理器".to_string()
        } else {
            self.message.clone()
        }
    }
}

/// 服务在服务管理器眼里是不是已经彻底停了（没在跑，而且不会被自己拉回来）。
///
/// 「没有 pid」不等于「停了」：官方的 LaunchAgent 带 `KeepAlive <true/>`，job 只要还
/// 加载着，进程死了 launchd 也会在 ThrottleInterval 之后重新拉起来。两种后端给的值
/// 不一样——launchd 的 `loaded` 表示加载着但此刻没进程，而 job 没加载时它根本不带
/// detail；systemd 给的是 ActiveState，没加载的单元是 `inactive`。
fn supervisor_idle(detail: Option<&str>) -> bool {
    match detail {
        None => true,
        Some(d) => matches!(d, "inactive" | "failed" | "dead" | "not-found"),
    }
}

/// 官方 `service stop` 报错之后核实服务到底停了没有。返回 `Some(还在跑的证据)` 就是
/// 真的没停下来。
fn still_running(sup: &Supervisor, listening: bool, port: u16) -> Option<String> {
    if let Some(pid) = sup.pid {
        return Some(format!("服务还在跑（PID {pid}）"));
    }
    // 状态问不出来（`service status --json` 没给出能解析的 JSON）时不替它下结论：
    // 手上只剩一个端口探测，而端口号是从可能已经读坏的 config.json 来的。
    if sup.available.is_none() {
        return Some("问不出服务管理器里的状态，核实不了是不是真停了".to_string());
    }
    if !supervisor_idle(sup.detail.as_deref()) {
        return Some(format!(
            "服务此刻没进程，但在服务管理器里还加载着（状态 {}），会被重新拉起来",
            sup.detail.as_deref().unwrap_or("未知")
        ));
    }
    if listening {
        return Some(format!(
            "端口 {port} 还有人在听（可能是 `fundeploy service paperclip run` 起的前台实例，不归服务管理器管）"
        ));
    }
    None
}

/// 官方 `update --check --json` 的判读。
struct UpdateCheck {
    current: String,
    target: String,
    available: bool,
    downgrade: bool,
}

fn secs(key: &str, default: u64) -> Duration {
    Duration::from_secs(env::num(key, default))
}

/// 一段输出的末尾几行，用来在报错里引上游的原话。
fn last_lines(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

fn or_unknown(s: &str) -> &str {
    if s.is_empty() {
        "未知"
    } else {
        s
    }
}

fn yes_no(cond: bool, yes: &'static str, no: &'static str) -> &'static str {
    if cond {
        yes
    } else {
        no
    }
}

/// 把子命令拼在参数前面。
fn prepend<'a>(head: &[&'a str], rest: &[&'a str]) -> Vec<&'a str> {
    let mut v = head.to_vec();
    v.extend_from_slice(rest);
    v
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

fn cli_runnable(bin: &Path) -> bool {
    cmd::try_run([bin.display().to_string().as_str(), "--version"]).is_ok_and(|o| o.success())
}

/// `postmaster.pid` 的第一行是 PID，后面还有数据目录、启动时间等若干行，所以不能直接
/// 用 [`proc::read_pid_file`]——它认的是整个文件只有一个数。
fn postmaster_pid(path: &Path) -> Option<i32> {
    let raw = std::fs::read_to_string(path).ok()?;
    raw.lines()
        .next()?
        .trim()
        .parse::<i32>()
        .ok()
        .filter(|p| *p > 0)
}

/// 进程的完整命令行。Linux 直接读 /proc，其余平台退回 ps。
fn process_cmdline(pid: i32) -> Option<String> {
    #[cfg(target_os = "linux")]
    if let Ok(raw) = std::fs::read(format!("/proc/{pid}/cmdline")) {
        // /proc 里参数之间是 NUL。
        return Some(
            String::from_utf8_lossy(&raw)
                .replace('\0', " ")
                .trim()
                .to_string(),
        );
    }
    let out = cmd::try_run(["ps", "-p", &pid.to_string(), "-o", "command="]).ok()?;
    out.success().then(|| out.stdout.trim().to_string())
}

fn health_endpoint_ok(cfg: &Config) -> bool {
    let host = match cfg.host.as_str() {
        "" | "0.0.0.0" | "*" => "127.0.0.1",
        "::" => "::1",
        h => h,
    };
    // IPv6 字面量在 URL 里要加方括号。
    let authority = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(3))
        .build()
        .get(&format!("http://{authority}:{}/api/health", cfg.port))
        .call()
        .is_ok()
}

fn node_version() -> Option<String> {
    let out = cmd::try_run(["node", "--version"]).ok()?;
    out.success()
        .then(|| out.stdout.trim().trim_start_matches('v').to_string())
        .filter(|v| !v.is_empty())
}

/// nvm 装的那些 node 里挑一个够新的，返回它的 bin 目录。
///
/// 刻意不去 source nvm.sh：那东西只对 shell 有意义，而 nvm 的布局
/// （`$NVM_DIR/versions/node/v<版本>/bin`）是稳定的。直接挑目录既不用起 shell，挑中
/// 哪一个也是确定的——官方 install.sh 恰恰相反，它见到 ~/.nvm 就直接 fail。
fn nvm_node_bin(min: &str) -> Option<PathBuf> {
    let root = env::opt("NVM_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| env::home().join(".nvm"))
        .join("versions")
        .join("node");

    let mut best: Option<(String, PathBuf)> = None;
    for entry in std::fs::read_dir(root).ok()?.flatten() {
        let v = entry
            .file_name()
            .to_string_lossy()
            .trim_start_matches('v')
            .to_string();
        if version::is_newer(min, &v) {
            continue; // 比下限旧
        }
        let bin = entry.path().join("bin");
        if !bin.join("node").exists() {
            continue;
        }
        let better = match &best {
            None => true,
            Some((cur, _)) => version::is_newer(&v, cur),
        };
        if better {
            best = Some((v, bin));
        }
    }
    best.map(|(_, bin)| bin)
}

/// 代理只做隧道（HTTPS 经 CONNECT 透传，TLS 仍是端到端），所以本机 `http://` 代理是
/// 正常用法，这里不强制 https。但限定已知 scheme，免得 registry 地址或一段 shell 被
/// 误填进来之后静默生效。
fn checked_proxy(raw: String) -> Option<String> {
    const OK: [&str; 4] = ["http://", "https://", "socks5://", "socks5h://"];
    if OK.iter().any(|p| raw.starts_with(p)) {
        return Some(raw);
    }
    ui::warn(format!(
        "忽略 PAPERCLIP_HTTPS_PROXY（{raw}）：只支持 http:// https:// socks5:// socks5h://"
    ));
    None
}

fn checked_registry(raw: &str) -> Option<String> {
    let registry = raw.trim_end_matches('/');
    if registry.starts_with("https://")
        && registry.len() > "https://".len()
        && !registry.chars().any(char::is_whitespace)
        && !registry.contains('@')
    {
        return Some(registry.to_string());
    }
    ui::warn("忽略 PAPERCLIP_NPM_REGISTRY：需要不含账号密码的 HTTPS 地址。继续使用 npmjs.org。");
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 残留端口没有清掉就不能继续重启() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let dir = tempfile::tempdir().unwrap();
        let mut pc = Paperclip::from_env();
        pc.home = dir.path().to_path_buf();
        std::fs::create_dir_all(pc.instance_dir()).unwrap();
        std::fs::write(
            pc.config_path(),
            format!(r#"{{"server":{{"host":"127.0.0.1","port":{port}}}}}"#),
        )
        .unwrap();
        // 测试进程自身受保护，不会被杀掉；端口清理失败必须阻止后续 start。
        let err = pc.ensure_port_free().unwrap_err().to_string();
        assert!(err.contains("已中止重启"), "{err}");
        drop(listener);
        pc.ensure_port_free().unwrap();
    }

    #[test]
    fn 镜像地址必须是安全的_https() {
        assert_eq!(
            checked_registry("https://registry.npmmirror.com/"),
            Some("https://registry.npmmirror.com".into())
        );
        for bad in [
            "http://mirror",
            "https://",
            "https://user@host",
            "https://host\nfoo",
        ] {
            assert!(checked_registry(bad).is_none());
        }
    }

    #[test]
    fn 通道只认那两个写法() {
        assert_eq!(Channel::parse("canary").unwrap(), Channel::Canary);
        for s in ["prod", "latest", "stable"] {
            assert_eq!(Channel::parse(s).unwrap(), Channel::Prod);
        }
        // 版本号不是通道。managed install 钉不到具体版本，这里必须拦住而不是
        // 把它当成某个通道悄悄装一个别的版本。
        let err = Channel::parse("2026.1008.0").unwrap_err().to_string();
        assert!(err.contains("canary / prod"), "实际: {err}");
    }

    #[test]
    fn 没有配置文件时用默认端口() {
        // 还没 onboard 过是正常状态：install 之前本来就没有配置，这里不能崩。
        let cfg = parse_config("", Path::new("/tmp/db"));
        assert_eq!(cfg.port, DEFAULT_SERVER_PORT);
        assert_eq!(cfg.pg_port, DEFAULT_PG_PORT);
        assert_eq!(cfg.host, "127.0.0.1");
        assert!(!cfg.embedded_pg);
        assert_eq!(cfg.pg_dir, Path::new("/tmp/db"));
    }

    #[test]
    fn 读得出_embedded_模式和端口() {
        let cfg = parse_config(
            r#"{"database":{"mode":"embedded-postgres","embeddedPostgresPort":54329,
                "embeddedPostgresDataDir":"~/pgdata"},
                "server":{"host":"0.0.0.0","port":9999}}"#,
            Path::new("/tmp/db"),
        );
        assert!(cfg.embedded_pg);
        assert_eq!(cfg.pg_port, 54329);
        assert_eq!(cfg.pg_dir, env::home().join("pgdata"));
        assert_eq!(cfg.host, "0.0.0.0");
        assert_eq!(cfg.port, 9999);
    }

    #[test]
    fn 外部数据库不算_embedded() {
        // 这条决定了三件事：要不要删 DATABASE_URL、备份前要不要查端口、
        // 换 payload 前要不要去收 postmaster。认错一边都会出事。
        let cfg = parse_config(
            r#"{"database":{"mode":"postgres","connectionString":"postgresql://x/y"}}"#,
            Path::new("/tmp/db"),
        );
        assert!(!cfg.embedded_pg);
    }

    #[test]
    fn 配置坏掉时退回默认值而不是崩掉() {
        // 写到一半断电、或者上游换了结构，都会得到这种内容。status 这种只读命令
        // 必须还能打出东西来。
        for junk in ["", "{", "null", "[]", r#"{"server":{"port":"八千"}}"#] {
            let cfg = parse_config(junk, Path::new("/tmp/db"));
            assert_eq!(cfg.port, DEFAULT_SERVER_PORT, "junk={junk}");
        }
    }

    #[test]
    fn 端口为零当作没配() {
        let cfg = parse_config(r#"{"server":{"port":0}}"#, Path::new("/tmp/db"));
        assert_eq!(cfg.port, DEFAULT_SERVER_PORT);
    }

    #[test]
    fn 代理只认已知的_scheme() {
        assert_eq!(
            checked_proxy("http://127.0.0.1:7890".into()).as_deref(),
            Some("http://127.0.0.1:7890")
        );
        assert_eq!(
            checked_proxy("socks5h://127.0.0.1:1080".into()).as_deref(),
            Some("socks5h://127.0.0.1:1080")
        );
        // 填错的话必须当没填：静默生效的结果是所有下载都走进一个不存在的代理。
        assert_eq!(checked_proxy("127.0.0.1:7890".into()), None);
        assert_eq!(checked_proxy("; rm -rf /".into()), None);
    }

    #[test]
    fn postmaster_的_pid_只取第一行() {
        // 这个文件不止一行，直接整篇解析会失败，于是 embedded PostgreSQL 永远收不掉。
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("postmaster.pid");
        std::fs::write(&p, "12345\n/home/u/db\n1728000000\n5432\n").unwrap();
        assert_eq!(postmaster_pid(&p), Some(12345));
        std::fs::write(&p, "乱码\n").unwrap();
        assert_eq!(postmaster_pid(&p), None);
        assert_eq!(postmaster_pid(&dir.path().join("nope")), None);
    }

    #[test]
    fn 额外动作的名字不重复() {
        // 重名的那个会被 match 的第一条分支吃掉，另一条永远走不到。
        let mut names: Vec<&str> = EXTRAS.iter().map(|e| e.name).collect();
        let n = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), n, "额外动作重名");
        assert!(EXTRAS.iter().all(|e| !e.about.is_empty()));
    }

    #[test]
    fn 不认识的额外动作有明确报错() {
        // 这条分支必须在动手之前就拦住：往下走就是拿一个拼错的子命令去调官方 CLI。
        let err = Paperclip::from_env()
            .extra("__不存在__", &[])
            .unwrap_err()
            .to_string();
        assert!(err.contains("__不存在__"), "实际: {err}");
    }

    #[test]
    fn 官方_stop_报错之后分得清本来就停着和还没停下来() {
        let sup = |pid: Option<i32>, detail: Option<&str>| Supervisor {
            available: Some(true),
            pid,
            detail: detail.map(str::to_string),
            message: String::new(),
        };

        // launchd 的 job 没加载时 status 既没有 pid 也没有 detail，端口也空着：
        // bootout 报的那句 No such process 不是故障，stop 该以成功收场。
        assert_eq!(still_running(&sup(None, None), false, 8804), None);
        // systemd 那边没加载的单元是 inactive，同理。
        assert_eq!(
            still_running(&sup(None, Some("inactive")), false, 8804),
            None
        );

        // 有 pid 就是没停下来，必须保留失败语义，不能因为「停过了」就报成功。
        assert!(
            still_running(&sup(Some(123), None), false, 8804).is_some_and(|e| e.contains("123"))
        );
        // 此刻没进程但 job 还加载着：KeepAlive 会把它拉回来，这不叫停了。
        for d in ["loaded", "running", "active", "activating", "deactivating"] {
            assert!(
                still_running(&sup(None, Some(d)), false, 8804).is_some(),
                "detail={d}"
            );
        }
        // 服务管理器说没在跑，可端口还有人听——多半是 run 起的前台实例，也不算停了。
        assert!(still_running(&sup(None, None), true, 8804).is_some_and(|e| e.contains("8804")));
        // 状态问不出来时宁可报失败：此时端口号也可能是从读坏的配置里来的默认值。
        let unknown = Supervisor {
            available: None,
            pid: None,
            detail: None,
            message: String::new(),
        };
        assert!(still_running(&unknown, false, 8804).is_some());
    }

    #[test]
    fn 回退只接受_previous_这类说法() {
        let pc = Paperclip::from_env();
        // 给版本号必须明确说「退不到指定版本」，而不是默默退到上一份。
        let err = pc.rollback("2026.1008.0").unwrap_err().to_string();
        assert!(err.contains("退不到指定版本"), "实际: {err}");
    }

    #[test]
    fn 报错里只引上游输出的末尾几行() {
        assert_eq!(last_lines("a\nb\nc\nd", 2), "c\nd");
        // 空行不占配额：npm 的错误输出里空行很多，占着就把有用的那几行挤出去了。
        assert_eq!(
            last_lines("npm error 1\n\n\nnpm error 2\n", 2),
            "npm error 1\nnpm error 2"
        );
        assert_eq!(last_lines("", 3), "");
    }
}
