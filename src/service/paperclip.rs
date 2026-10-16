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

use anyhow::{anyhow, bail, Context, Result};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::{Extra, Service};
use crate::common::cmd::{self, EnvEdit};
use crate::common::fs::{dir_size, human_size, Scratch};
use crate::common::{env, port, proc, version};
use crate::ui;

/// 上游要求的 Node 下限。只有 npx 引导首次安装那一步真需要它——装完之后 shim 里
/// 是绝对路径，自带了它那份 node，PATH 上是不是旧版无关。
const NODE_MIN: &str = "24.11.0";

/// 官方源。用途只有一个：npx 引导那一跳的 registry。managed install 自己也把它钉死
/// （见 [`Paperclip::npm_env`]），这里跟着写一遍只是让那一跳落在同一个源上。
const NPM_REGISTRY: &str = "https://registry.npmjs.org";

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
        cmd::find("paperclipai").map(PathBuf::from)
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
        cmd::apply_env(&mut c, &self.cli_env(&self.config()));
        serde_json::from_slice(&c.output().ok()?.stdout).ok()
    }

    fn installed_version(&self) -> Option<String> {
        let bin = self.cli_path()?;
        let out = cmd::try_run([bin.display().to_string().as_str(), "--version"]).ok()?;
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

    fn status_fallback(&self, cfg: &Config, supervisor_unavailable: Option<String>) {
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
        if let Some(why) = &supervisor_unavailable {
            card = card
                .row_state("官方管理器", ui::State::Idle, "不可用")
                .note(why.clone());
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

    fn logs_fallback(&self, args: &[&str]) -> Result<()> {
        if !self.log_file.exists() {
            bail!(
                "没有 fundeploy 托管的日志（{} 不存在）；这个实例多半不是用 \
                 `fundeploy service paperclip start` 起的",
                self.log_file.display()
            );
        }
        let (lines, follow) = parse_log_args(args, self.log_lines);
        for line in tail_lines(&self.log_file, lines) {
            println!("{line}");
        }
        if follow {
            follow_log(&self.log_file)?;
        }
        Ok(())
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

    // ---------------------------------------------------------------- npm

    /// 装包期间要透给 npm 的环境改动。
    ///
    /// 官方 managed install 把 registry 钉死在 registry.npmjs.org：它另写一份只含
    /// registry 的临时 npmrc、用 `npm_config_userconfig` 强制 npm 子进程改用它，命令行
    /// 上再传一次 `--registry`。这是刻意的防投毒设计，有上游测试守着，所以 `~/.npmrc`
    /// 里的 registry / proxy / fetch-* 对它一概无效（文件被整体替换），
    /// `npm_config_registry` 也压不过命令行。唯一透得进去的是环境变量——它以
    /// `{...process.env, npm_config_userconfig}` 起 npm。因此这里只设它没有在命令行上
    /// 显式传过的那些键：取回重试与超时，以及代理。
    fn npm_env(&self, npmrc: Option<&Npmrc>) -> Vec<EnvEdit> {
        let mut e: Vec<EnvEdit> = vec![
            ("npm_config_allow_scripts".to_string(), None),
            (
                "npm_config_fetch_retries".to_string(),
                Some(self.fetch.retries.to_string()),
            ),
            (
                "npm_config_fetch_retry_mintimeout".to_string(),
                Some(self.fetch.min_timeout.to_string()),
            ),
            (
                "npm_config_fetch_retry_maxtimeout".to_string(),
                Some(self.fetch.max_timeout.to_string()),
            ),
            (
                "npm_config_fetch_timeout".to_string(),
                Some(self.fetch.timeout.to_string()),
            ),
        ];
        if let Some(rc) = npmrc {
            e.push((
                "npm_config_userconfig".to_string(),
                Some(rc.path.display().to_string()),
            ));
        }
        if self.prefer_offline {
            e.push((
                "npm_config_prefer_offline".to_string(),
                Some("true".to_string()),
            ));
        }
        let Some(proxy) = &self.proxy else {
            return e;
        };
        ui::info(format!("npm 下载走代理 {proxy}"));
        for key in [
            "npm_config_proxy",
            "npm_config_https_proxy",
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "http_proxy",
            "https_proxy",
        ] {
            e.push((key.to_string(), Some(proxy.clone())));
        }
        if let Some(no) = &self.no_proxy {
            for key in ["NO_PROXY", "no_proxy"] {
                e.push((key.to_string(), Some(no.clone())));
            }
        }
        e
    }

    /// 装之前把这次要下多少、能省掉多少说清楚。
    ///
    /// 下载量和落盘量必须分开报：实测同一棵树是 **1.0 GiB 下载 / 2.7 GiB 落盘**，
    /// 差了近三倍。这行原来只报落盘量，读起来像是每次升级都得下 2.7 GiB。
    ///
    /// 绝对值仍然不写死（依赖树一直在长）：落盘量按本机当前 payload 现量，下载量按
    /// [`TARBALL_RATIO`] 折算。而真正决定这次要等多久的不是总量，是缓存热不热，所以
    /// 还要去量一眼 `_cacache`——那是这条提示里唯一能让人当场做点什么的数字。
    fn print_network_hint(&self, ch: Channel) {
        if !self.network_hint {
            return;
        }
        let measured = dir_size(&self.home.join("cli").join("current"));
        let disk = if measured > 0 {
            measured
        } else {
            MEASURED_DISK
        };
        let download = (disk as f64 * TARBALL_RATIO) as u64;
        ui::warn(format!(
            "Paperclip 的依赖树很大：全量约 {} 下载 / {} 落盘{}。而且每次升级都是从零拼一棵\
             新的——上游装到一个临时目录再原子换过去，换来的是失败不留残骸、回滚只动一个\
             符号链接。",
            human_size(download),
            human_size(disk),
            if measured > 0 {
                ""
            } else {
                "（本机还没装过，按上次实测估）"
            },
        ));
        ui::hint(cache_state(download));
        ui::hint(
            "下载量的七成是各家 agent CLI 的二进制（codex / claude-agent-sdk / copilot / \
             opencode，而且每个都有两份：上游的版本范围对不上，同一个工具的两个版本并存），\
             升级时它们大多不变——缓存热了之后 canary 日更通常只下一两百 MiB。所以别清 \
             ~/.npm/_cacache。",
        );
        ui::hint(format!(
            "想先把下载跑完再装: fundeploy service paperclip prefetch {} --background",
            ch.name()
        ));
        if self.proxy.is_none() {
            ui::hint(
                "官方 managed install 把 registry 钉死在 npmjs.org，换镜像对它无效（刻意的\
                 防投毒设计，有上游测试守着），受限网络请走代理：\
                 PAPERCLIP_HTTPS_PROXY=http://127.0.0.1:7890。",
            );
        }
        ui::hint("设 PAPERCLIP_NETWORK_HINT=0 隐藏以上几行。");
    }

    // ------------------------------------------------------------ 安装与升级

    fn install_channel(&self, ch: Channel) -> Result<()> {
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
        let v = self.json(&["update", ch.flag(), "--check", "--json"])?;
        Some(UpdateCheck {
            current: v["currentVersion"].as_str().unwrap_or_default().to_string(),
            target: v["targetVersion"].as_str().unwrap_or_default().to_string(),
            available: v["updateAvailable"].as_bool().unwrap_or(false),
            downgrade: v["downgrade"].as_bool().unwrap_or(false),
        })
    }

    // ---------------------------------------------------------------- 预热

    /// 把目标版本要下的 tarball 预先灌进 npm 缓存。
    ///
    /// # 为什么这件事做得到
    ///
    /// npm 的缓存是内容寻址的，键就是 tarball 的完整 URL（实测
    /// `make-fetch-happen:request-cache:https://registry.npmjs.org/…tgz`），谁灌进去
    /// 的都算：`npm cache add` 之后再去装同一个包，npm 在同一个键上命中，一个字节都
    /// 不再下（实测日志里是 `npm http cache … 0ms (cache hit)`）。
    ///
    /// 所以必须跟 install 走同一个 registry——键里带着主机名，换个源灌进去的那份对
    /// 上游那条命令就是不存在（实测用 npmmirror 灌出来的键是 `cdn.npmmirror.com/…`）。
    ///
    /// # 为什么值得做
    ///
    /// managed install 是一条前台命令、全程不打输出，而那条命令里最久的一段就是下载
    /// （冷缓存实测约 1 GiB）。下载这件事可以提前、可以挪后台、断了重来也只是重下没
    /// 命中的那几个；install 那条命令却必须一口气跑完，中间断了就是整棵树重拼。
    fn prefetch(&self, args: &[&str]) -> Result<()> {
        let (ch, background) = prefetch_args(args, self.channel.unwrap_or(Channel::Canary))?;
        if background {
            return self.prefetch_background(ch);
        }
        let r = self.prefetch_now(ch);
        // 收掉自己那份 PID 文件，成败都收：PID 会被系统回收，留着它会让下一次
        // `--background` 把一个毫不相干的新进程当成「还有预热在跑」而拒绝开工。
        let pid_file = self.run_dir.join("prefetch.pid");
        if proc::read_pid_file(&pid_file).is_some_and(|p| p == std::process::id() as i32) {
            let _ = std::fs::remove_file(&pid_file);
        }
        r
    }

    fn prefetch_now(&self, ch: Channel) -> Result<()> {
        cmd::require("npm", "它随 Node.js 一起装：https://nodejs.org/")?;
        let npmrc = Npmrc::write(NPM_REGISTRY);
        let env = self.npm_env(npmrc.as_ref());

        let version = self.prefetch_target(ch, &env)?;
        let lock = self.resolve_tree(&version, &env)?;
        let picked = prefetch_specs(&lock, &NpmPlatform::current());
        if !picked.foreign.is_empty() {
            // 一个都不该有：这棵树里 539 个 resolved 全在钉死的那个 registry 上。真冒
            // 出来一个，说明上游改了依赖的写法，那时候该来看一眼而不是默默去下。
            ui::warn(format!(
                "有 {} 个包的下载地址不在 {} 上，跳过不灌: {}",
                picked.foreign.len(),
                NPM_REGISTRY,
                picked.foreign.join(" ")
            ));
        }
        if picked.specs.is_empty() {
            bail!("从 lockfile 里没挑出一个要下的包，解析结果不对（期待几百个）");
        }
        ui::step(format!(
            "{version}: 本机要下 {} 个包（另外 {} 个是别的平台、{} 个装在父包的 tarball 里）",
            picked.specs.len(),
            picked.other_platform,
            picked.bundled
        ));
        self.fill_cache(&picked.specs, &env)
    }

    /// 一批批灌缓存，并把进度报出来。
    fn fill_cache(&self, specs: &[String], env: &[EnvEdit]) -> Result<()> {
        let total = specs.len();
        let cache = npm_cacache();
        let before = dir_size(&cache);

        let bar = progress_bar();
        bar.set_style(prefetch_style());
        bar.set_length(total as u64);
        // 第一批跑完才有字节数可报，而第一批里只要有个 155 MiB 的 codex 就是好几分钟，
        // 那几分钟里这行得先说清楚它在干什么。
        bar.set_message("灌缓存".to_string());
        bar.enable_steady_tick(STEADY_TICK);
        let mut next = Instant::now();
        let mut failed = 0usize;
        let mut first_err: Option<String> = None;

        for chunk in specs.chunks(PREFETCH_BATCH) {
            let mut argv = vec![
                "npm".to_string(),
                "cache".to_string(),
                "add".to_string(),
                "--registry".to_string(),
                NPM_REGISTRY.to_string(),
            ];
            argv.extend(chunk.iter().cloned());
            // 输出收起来：成功时 `npm cache add` 一个字都不打（实测），失败时要的是
            // 末尾那几行汇总，而不是 12 个包的 npm 日志盖掉进度条。
            match cmd::try_run_with(&argv, env) {
                Ok(o) if o.success() => {}
                Ok(o) => {
                    failed += chunk.len();
                    first_err.get_or_insert_with(|| last_lines(&o.stderr, 3));
                }
                Err(e) => {
                    failed += chunk.len();
                    first_err.get_or_insert_with(|| e.to_string());
                }
            }
            bar.inc(chunk.len() as u64);
            // 量缓存跟 InstallWatch 用同一套限频：一趟 dir_size 在几 GiB 的缓存上要
            // 几百毫秒，而缓存全命中的时候每批几乎是瞬间完事，不限频就全花在量上了。
            if Instant::now() >= next {
                let t0 = Instant::now();
                let added = dir_size(&cache).saturating_sub(before);
                bar.set_message(format!("已灌 {}", human_size(added)));
                next = Instant::now() + MEASURE_EVERY.max(t0.elapsed() * 10);
            }
            if bar.is_hidden() {
                // 非终端（后台跑、重定向到日志）那条路上进度条是隐藏的，日志里得留下
                // 点东西，否则后台那几十分钟在日志里是一片空白。
                ui::info(format!("已灌 {}/{total} 个包", bar.position()));
            }
        }
        bar.finish_and_clear();

        let added = dir_size(&cache).saturating_sub(before);
        if let Some(e) = first_err {
            ui::warn(format!("有没灌上的，第一条失败是:\n{e}"));
        }
        if failed == total {
            bail!("{total} 个包一个都没灌进缓存，预热没起作用");
        }
        if added == 0 {
            ui::ok(format!("{total} 个包缓存里本来就有，这次一个字节都没下。"));
        } else {
            ui::ok(format!(
                "预热完成: {} 个包就绪，缓存涨了 {}{}。",
                total - failed,
                human_size(added),
                if failed > 0 {
                    format!("，{failed} 个没灌上（装的时候会自己再下）")
                } else {
                    String::new()
                }
            ));
        }
        ui::hint(format!(
            "接下来 `fundeploy service paperclip {}` 的下载段就只剩没命中的那几个了。",
            if self.cli_path().is_some() {
                "upgrade"
            } else {
                "install"
            }
        ));
        Ok(())
    }

    /// 通道当前指向的版本。
    fn prefetch_target(&self, ch: Channel, env: &[EnvEdit]) -> Result<String> {
        // 先问官方：`update --check` 给的那个 target，就是 install / upgrade 自己要去
        // 装的那一个。
        if let Some(c) = self.update_check(ch) {
            let v = if c.target.is_empty() {
                c.current
            } else {
                c.target
            };
            if !v.is_empty() {
                ui::step(format!("{} 通道指向 {v}（官方 update --check）", ch.name()));
                return Ok(v);
            }
        }
        // 问不出来（还没装 CLI，或者离线）就退回 dist-tag。猜错的代价有限：差的那几个
        // 是 @paperclipai/* 自己那十来个包，大头（各家 agent CLI 的二进制）相邻两版用
        // 的是同一份，照样命中。
        let spec = format!("paperclipai@{}", ch.dist_tag());
        let out = cmd::try_run_with(
            [
                "npm",
                "view",
                &spec,
                "version",
                "--registry",
                NPM_REGISTRY,
                "--json",
            ],
            env,
        )?;
        if !out.success() {
            bail!(
                "问不出 {} 通道的版本（paperclipai 没装，npm view 也失败了）:\n{}",
                ch.name(),
                last_lines(&out.stderr, 3)
            );
        }
        let v = npm_view_version(&out.stdout).ok_or_else(|| {
            anyhow!(
                "认不出 npm view {spec} version 的输出: {}",
                out.stdout.trim()
            )
        })?;
        ui::step(format!(
            "{} 通道指向 {v}（npm 的 {} dist-tag；paperclipai 还没装，问不到官方那边）",
            ch.name(),
            ch.dist_tag()
        ));
        Ok(v)
    }

    /// 解析出完整依赖图，拿到每个包的下载地址。
    ///
    /// `--package-lock-only` 只写一份 lockfile、不装任何东西，但它要把每个包的
    /// manifest 问一遍（实测 888 个条目约 50 秒）。
    ///
    /// # 那个 allow-remote
    ///
    /// npm 12 新加了 `--allow-remote`，默认 `none`；而 `@paperclipai/server` 带
    /// `bundleDependencies`，arborist 为了读出里面捆了哪些包，必须先把它的 tarball
    /// 取下来解开（`pacote.extract`），于是整条解析被 `EALLOWREMOTE` 顶回来。npm 11
    /// 没有这条策略，所以这件事只在 npm 12 上发作。
    ///
    /// 这里放开它的前提是取回的范围已经被另一道闸门管住：[`prefetch_specs`] 逐条核对
    /// `resolved` 的主机名，不在钉死的 registry 上的一个都不灌。
    fn resolve_tree(&self, version: &str, env: &[EnvEdit]) -> Result<serde_json::Value> {
        let scratch = Scratch::new("paperclip-prefetch")?;
        let mut env = env.to_vec();
        env.push((
            "npm_config_allow_remote".to_string(),
            Some("all".to_string()),
        ));

        let bar = progress_bar();
        bar.set_style(downloading_style(false));
        bar.set_message("解析依赖图（只问元数据，不装东西）".to_string());
        bar.enable_steady_tick(STEADY_TICK);
        let code = cmd::spawn_status_in(
            [
                "npm",
                "install",
                "--prefix",
                &scratch.path().display().to_string(),
                "--package-lock-only",
                &format!("paperclipai@{version}"),
                "--registry",
                NPM_REGISTRY,
                "--no-audit",
                "--no-fund",
                // 自己画进度条，不跟 npm 的抢同一行。
                "--no-progress",
                // npm ≥11.19 会在末尾列出被跳过的 install 脚本。那是装的时候才相关的
                // 事，这一步什么都没装，留着只会让人以为出了问题。
                "--loglevel",
                "error",
            ],
            &env,
            None,
        );
        bar.finish_and_clear();
        match code? {
            Some(0) => {}
            other => bail!(
                "解析 paperclipai@{version} 的依赖图失败（退出码 {}）",
                other.map_or_else(|| "信号终止".to_string(), |c| c.to_string())
            ),
        }

        let path = scratch.path().join("package-lock.json");
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("读不到解析出来的 lockfile: {}", path.display()))?;
        serde_json::from_str(&raw).with_context(|| format!("解析不了 {}", path.display()))
    }

    /// 把预热丢到后台去跑。
    ///
    /// 预热天然适合后台：它只往 npm 的缓存里写，不碰 payload、不碰服务、不改任何
    /// 配置，断了重来也只是重下没命中的那几个。
    fn prefetch_background(&self, ch: Channel) -> Result<()> {
        let exe = std::env::current_exe().context("取不到 fundeploy 自己的路径")?;
        let log = self.run_dir.join("prefetch.log");
        let pid_file = self.run_dir.join("prefetch.pid");
        if let Some(pid) = proc::read_pid_file(&pid_file).filter(|p| proc::is_alive(*p)) {
            bail!(
                "已经有一个预热在后台跑着（PID {pid}）。看进度: tail -f {}",
                log.display()
            );
        }
        let argv = vec![
            exe.display().to_string(),
            "service".to_string(),
            "paperclip".to_string(),
            "prefetch".to_string(),
            ch.name().to_string(),
        ];
        let pid = proc::spawn_detached(&argv, &env::home(), &self.cli_env(&self.config()), &log)?;
        proc::write_pid_file(&pid_file, pid)?;
        ui::ok(format!(
            "{} 通道的预热已在后台开始（PID {pid}）。",
            ch.name()
        ));
        ui::hint(format!("看进度: tail -f {}", log.display()));
        ui::hint(format!("要停掉: kill {pid}"));
        Ok(())
    }

    fn upgrade_channel(&self, ch: Channel) -> Result<()> {
        let npmrc = Npmrc::write(NPM_REGISTRY);
        let npm = self.npm_env(npmrc.as_ref());
        self.print_network_hint(ch);

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
        if self.exec_watching(&argv, npm)? != 0 {
            // 尽力把服务拉起来，但保留 update 本身的失败语义。
            if let Err(e) = self.start() {
                ui::warn(format!("换包失败后想把服务拉起来，也没成功: {e}"));
            }
            bail!("paperclipai update 失败");
        }

        // 上游专门有这条修复命令：检查配置、install store、manifest、current 链接、
        // shim、PATH、目录权限并尝试修好。刻意放在 start 之前——官方 `paperclipai run`
        // 自己也是 onboard + doctor --repair 之后才启动。不让它的返回值挡住启动：服务
        // 停着时「数据库连不上」这类检查必然判失败，真正的闸门是启动后的健康检查。
        if self.exec(&["doctor", "--repair", "--yes"])? != 0 {
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

    fn logs(&self, args: &[&str]) -> Result<()> {
        let sup = self.supervisor();
        if sup.available != Some(false) {
            // 官方 service logs 读的是 journalctl / launchd 的日志。
            return self.must(&prepend(&["service", "logs"], args));
        }
        self.require_fallback(&sup)?;
        self.logs_fallback(args)
    }

    fn onboard(&self, args: &[&str]) -> Result<()> {
        let mut argv = prepend(&["onboard"], args);
        // 无人值守下向导会停在第一个问题上等输入，表现是「卡死」。
        if !ui::interactive() && !args.iter().any(|a| *a == "--yes" || *a == "-y") {
            argv.push("--yes");
        }
        self.must(&argv)
    }

    fn plugin(&self, args: &[&str]) -> Result<()> {
        let (action, rest) = match args.split_first() {
            Some((a, rest)) => (*a, rest),
            None => ("install", &[][..]),
        };
        match action {
            "list" => {
                print_catalog();
                Ok(())
            }
            "installed" => self.must(&prepend(&["plugin", "list"], rest)),
            "install" => self.plugin_install(rest),
            other => bail!("未知的插件命令 {other}（支持 list / install / installed）"),
        }
    }

    fn plugin_install(&self, rest: &[&str]) -> Result<()> {
        let npmrc = Npmrc::write(NPM_REGISTRY);
        let npm = self.npm_env(npmrc.as_ref());
        if let Some((pkg, more)) = rest.split_first() {
            return self.must_with(&prepend(&["plugin", "install", pkg], more), &npm);
        }

        // 没给包名就从清单里挑。清单里有的条目不是 npm 包（装法由各自仓库说明），
        // 挑不了，只在 list 里露面。
        let pickable: Vec<&Plugin> = CATALOG.iter().filter(|p| !p.pkg.is_empty()).collect();
        let labels: Vec<String> = pickable
            .iter()
            .map(|p| format!("{} - {}", p.name, p.about))
            .collect();
        let Some(picked) = ui::select("从 awesome-paperclip 挑一个插件", &labels)? else {
            return Ok(());
        };
        let i = labels
            .iter()
            .position(|l| l == picked)
            .ok_or_else(|| anyhow!("认不出选中的插件"))?;
        ui::step(format!(
            "安装 Paperclip 插件 {}（来源: {}）",
            pickable[i].pkg, pickable[i].url
        ));
        self.must_with(&["plugin", "install", pickable[i].pkg], &npm)
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
        // 默认跟 canary：装了这个项目的机器基本都在跟开发版，stable 落后得多。
        // 要走正式版: PAPERCLIP_CHANNEL=prod。
        self.upgrade_channel(self.channel.unwrap_or(Channel::Canary))
    }

    fn rollback(&self, version: &str) -> Result<()> {
        // 官方只在 install store 里留上一份 payload，没有「回到任意版本」这回事。
        if !matches!(version, "previous" | "prev" | "last" | "-") {
            bail!(
                "paperclip 只能退回 install store 里留着的上一份 payload，退不到指定版本。\n\
                 执行: fundeploy service paperclip rollback previous"
            );
        }
        let npmrc = Npmrc::write(NPM_REGISTRY);
        let npm = self.npm_env(npmrc.as_ref());
        self.apply_update(&["--rollback"], &npm)
    }

    fn start(&self) -> Result<()> {
        let sup = self.supervisor();
        if sup.available != Some(false) {
            // supervisor 可用（含问不出来的情况）就维持官方路径：systemd 的 Type=notify
            // 和 launchd 自己会等到就绪，不必再叠一层健康检查。
            return self.must(&["service", "start"]);
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
            return Ok(());
        }
        // 关掉兜底时也不该假成功：官方 stop 在这种机器上什么都不会停。
        self.require_fallback(&sup)?;
        self.stop_fallback(false).map(|_| ())
    }

    fn restart(&self) -> Result<()> {
        let sup = self.supervisor();
        match sup.available {
            Some(false) => {
                self.require_fallback(&sup)?;
                self.stop_fallback(false)?;
                self.start_fallback(&self.config(), &sup.reason())
            }
            // 问不出状态就交给官方 restart 自己报错，不猜。
            None => self.must(&["service", "restart"]),
            Some(true) => {
                // 官方 service restart 只做热重启：writeHotRestartIntent 第一步就是
                // `if (!status.pid) throw`，服务没在跑时它永远失败——那种场景要的其实
                // 是 start。
                if sup.pid.is_none() {
                    ui::step("服务没在跑（supervisor 里没有 pid），改为 start");
                    return self.must(&["service", "start"]);
                }
                self.must(&["service", "restart"])
            }
        }
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
        self.status_fallback(&cfg, Some(sup.reason()));
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
        self.must(&["uninstall"])?;
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

/// 装包期间顶掉 `~/.npmrc` 的一份最小 npmrc，临时文件随值一起消失。
///
/// 最外层那次 `npx paperclipai@latest` 会读 `~/.npmrc`，并把读到的配置以
/// `npm_config_*` 注入子进程。已知 npm bug（npm/cli#9783、#9912、#9968）会把这样继承
/// 来的 allow-scripts 当成命令行显式传入的 `--allow-scripts`，直接报 EALLOWSCRIPTS；
/// 只清环境变量不够——配置本来就来自文件。
struct Npmrc {
    path: PathBuf,
}

impl Npmrc {
    fn write(registry: &str) -> Option<Self> {
        let path =
            std::env::temp_dir().join(format!("fundeploy-paperclip-npmrc.{}", std::process::id()));
        let registry = registry.trim_end_matches('/');
        std::fs::write(
            &path,
            format!("registry={registry}/\n@paperclipai:registry={registry}/\n"),
        )
        .ok()?;
        Some(Self { path })
    }
}

impl Drop for Npmrc {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// 两次量大小之间至少隔这么久。
const MEASURE_EVERY: Duration = Duration::from_secs(2);

/// 轮询子进程是否退出、以及暂存目录在不在的间隔。
///
/// 暂存目录 rename 成正式 payload 之后，上游紧接着就打那行结果，所以这一问要跟得紧
/// ——慢了就会让进度条盖掉它。一次 `read_dir`（store 里只有两三个条目）的代价可以忽略。
const WATCH_TICK: Duration = Duration::from_millis(500);

/// managed install / update 期间，盯着上游正在写的暂存目录报进度。
///
/// 上游把 npm 的输出整个捕获掉了（`installNpmPayload` 用 `execFile` 系的 runCommand
/// 跑 `npm install --prefix <暂存目录>`，不是继承 stdio），所以从「Installing
/// paperclipai@x」到装完之间，这条命令一个字都不打。那一段实测要写 2.5 GiB、7 万多个
/// 文件，几分钟到几十分钟，看起来跟卡死一模一样。
///
/// 但它正在写的地方是公开的：`<home>/cli/installs/npm/.<版本>.tmp-<pid>-<毫秒>`，装完
/// rename 成正式 payload。盯着那个目录长大就能把那段沉默翻译成进度；分母用 `current`
/// 指向的当前 payload——上游每次都是从零拼一棵新树，新旧两版体积相差一成上下，够估，
/// 估小了就把分母顶上去（见 [`InstallWatch::tick`]）。
///
/// 上游在这一段不打输出，也正好意味着进度条不会和它抢同一片终端：暂存目录在的那段，
/// 就是它闷着的那段。
///
/// 这段沉默里有两件事先后发生，实测 100 秒 + 70 秒，所以分两段报：
///
/// 1. **下载**：npm 先把 tarball 收进 `~/.npm/_cacache`，这期间暂存目录建好了但是空的。
///    只盯暂存目录的话，最难熬的前一半会停在 `0 B` 上——那正是这次要解决的观感。这一段
///    改看缓存的增量，没有分母（不知道总共要下多少），所以不报 ETA。
/// 2. **解包**：往暂存目录里铺 7 万多个文件，有分母可报，ETA 这时才有意义。
struct InstallWatch {
    /// `<home>/cli/installs/npm`，暂存目录和正式 payload 都在这儿。
    store: PathBuf,
    /// `<home>/cli/current`，指向当前 payload，用来估分母。
    current: PathBuf,
    /// npm 的内容寻址缓存。不存在（第一次用 npm）就是 None，那时下载段只报时间。
    cache: Option<PathBuf>,
    bar: Option<indicatif::ProgressBar>,
    /// 下次真正去量大小的时刻。
    next: Instant,
    /// 暂存目录最后一次量到的字节数。只增不减，见 [`InstallWatch::tick`]。
    last: u64,
    /// 开工时缓存的体积，下载段的进度是相对它的增量。
    cache_base: u64,
    started: Option<Instant>,
}

impl InstallWatch {
    fn new(home: &Path) -> Self {
        let cli = home.join("cli");
        let cache = npm_cacache();
        Self {
            store: cli.join("installs").join("npm"),
            current: cli.join("current"),
            cache: cache.is_dir().then_some(cache),
            bar: None,
            next: Instant::now(),
            last: 0,
            cache_base: 0,
            started: None,
        }
    }

    /// 轮询回调，每 [`WATCH_TICK`] 一次。
    fn tick(&mut self) {
        let Some(staging) = staging_dir(&self.store) else {
            // 还没出现（上游在解析版本号），或者已经 rename 走了 / 失败被删了。
            self.finish();
            return;
        };
        if self.bar.is_none() {
            self.open(&staging);
        }
        if Instant::now() < self.next {
            return;
        }
        let t0 = Instant::now();
        let now = dir_size(&staging);
        // 只认涨的那一侧：量到一半目录被 rename 走时这一趟会半途读空，照原样写进去
        // 就是进度条往回跳，收尾那行也会报一个莫名其妙的小数字。
        if now > self.last {
            if self.last == 0 {
                self.unpacking();
            }
            self.last = now;
            if let Some(bar) = &self.bar {
                // 分母是按旧版估的，新版更大就把它顶上去——不然最后那几个百分点会
                // 停在 100% 上，还报着「还要 0s」。
                if bar.length().is_some_and(|len| now > len) {
                    bar.set_length(now);
                }
                bar.set_position(now);
            }
        } else if self.last == 0 {
            // 还在下载段：暂存目录是空的，看缓存涨了多少。
            if let (Some(cache), Some(bar)) = (&self.cache, &self.bar) {
                bar.set_position(dir_size(cache).saturating_sub(self.cache_base));
            }
        }
        // 下一次至少等这一趟实际耗时的 10 倍：依赖树越大走一趟越贵（见 dir_size），
        // 这条让「看进度」这件事本身始终只占个位数的 I/O，不跟安装抢。
        self.next = Instant::now() + MEASURE_EVERY.max(t0.elapsed() * 10);
    }

    fn open(&mut self, staging: &Path) {
        let name = staging.file_name().unwrap_or_default().to_string_lossy();
        let label = staging_version(&name).unwrap_or("依赖树").to_string();
        self.cache_base = self.cache.as_deref().map(dir_size).unwrap_or(0);
        self.started = Some(Instant::now());
        ui::hint("上游装 payload 这一段不打任何输出，下面的进度是 fundeploy 在旁边量的。");
        let bar = progress_bar();
        bar.set_style(downloading_style(self.cache.is_some()));
        bar.set_message(format!("{label} 下载依赖"));
        bar.enable_steady_tick(STEADY_TICK);
        self.bar = Some(bar);
    }

    /// 下载段转解包段：换模板、换分母，并把速率和 ETA 的估算从头算。
    ///
    /// 不 `reset()` 不行：位置要从「缓存增量」跳到「暂存目录大小」，两个量毫无关系，
    /// 留着上一段的历史只会算出一个荒唐的 ETA。
    fn unpacking(&mut self) {
        let Some(bar) = &self.bar else { return };
        // 分母只是个估计值（上游每次从零拼一棵新树，新旧两版体积差几个百分点），
        // 所以说出来它是哪儿来的；没有旧 payload（全新安装）就只报已写入量。
        let total = dir_size(&self.current);
        bar.reset();
        if total > 0 {
            bar.set_length(total);
            bar.set_style(unpacking_style());
            bar.set_message(format!("解包（分母 {} 按当前版本估）", human_size(total)));
        } else {
            bar.set_style(downloading_style(true));
            bar.set_message("解包".to_string());
        }
    }

    /// 收尾。进度条还在就清掉，并留一行说明这一步花了多少。
    fn finish(&mut self) {
        let Some(bar) = self.bar.take() else { return };
        bar.finish_and_clear();
        let used = self.started.take().map(|t| t.elapsed()).unwrap_or_default();
        if self.last == 0 {
            // 一个字节都没量到：暂存目录刚建好就没了（上游在下载段就失败了）。
            return;
        }
        // 措辞刻意中性：失败时暂存目录也会被上游的 finally 删掉，这行不该宣布成功。
        ui::hint(format!(
            "这一步写了约 {}，用了 {}。",
            human_size(self.last),
            human_secs(used)
        ));
    }
}

/// npm 的内容寻址缓存目录。
///
/// npm 默认 `~/.npm/_cacache`，`npm_config_cache` 能改——上游不在命令行上传
/// `--cache`，所以这个环境变量对它那条 `npm install` 同样有效（见
/// [`Paperclip::npm_env`] 里对哪些键透得进去的说明）。
///
/// 刻意不去跑 `npm config get cache`：那是一次几百毫秒的 node 启动，而这里只是为了
/// 量个大小、报个数字，猜错的代价仅仅是少报一行。
fn npm_cacache() -> PathBuf {
    env::opt("npm_config_cache")
        .map(|v| env::expand_tilde(&v).join("_cacache"))
        .unwrap_or_else(|| env::home().join(".npm").join("_cacache"))
}

/// npm 缓存的冷热，以及它对这次下载意味着什么。
///
/// 判据只能是体积：缓存是内容寻址的，里面有多少属于 paperclip 这棵树，不把几万个
/// 索引条目逐个解开是问不出来的。但「比整棵树还小」这一条已经够下结论——装不满。
///
/// `download` 是这次全量要下的估计值（见 [`Paperclip::print_network_hint`]）。
fn cache_state(download: u64) -> String {
    let cache = npm_cacache();
    cache_state_of(dir_size(&cache), &cache, download)
}

fn cache_state_of(size: u64, cache: &Path, download: u64) -> String {
    if size == 0 {
        return format!("npm 缓存 {} 还是空的，这次是全量下载。", cache.display());
    }
    if size < download {
        return format!(
            "npm 缓存现有 {}，装不满这棵树，有一部分还得新下。",
            human_size(size)
        );
    }
    format!(
        "npm 缓存现有 {}，命中的 tarball 不再下载；PAPERCLIP_NPM_PREFER_OFFLINE=1 \
         连「问一遍有没有更新」的那些请求也省掉。",
        human_size(size)
    )
}

/// prefetch 的参数：通道和要不要丢后台。顺序不限，都可以省。
fn prefetch_args(args: &[&str], default: Channel) -> Result<(Channel, bool)> {
    let mut ch = None;
    let mut background = false;
    for a in args {
        match *a {
            "--background" | "-b" => background = true,
            other => ch = Some(Channel::parse(other)?),
        }
    }
    Ok((ch.unwrap_or(default), background))
}

/// 从 lockfile 里挑出来的预热清单。
#[derive(Debug, Default, PartialEq, Eq)]
struct Picked {
    /// 要灌的 `名字@版本`，已去重排序。
    specs: Vec<String>,
    /// 装在父包 tarball 里、不单独下的（`inBundle`）。
    bundled: usize,
    /// 平台不符、npm 装的时候也会跳过的。
    other_platform: usize,
    /// `resolved` 不在钉死的 registry 上的。正常情况下一个都没有。
    foreign: Vec<String>,
}

/// lockfile 里「这台机器真会去下」的那些包。
///
/// 三道筛子，少一道都会多下或者下错：
///
/// 1. **没有 `resolved` 的跳过**。它们是 `inBundle`——装在父包的 tarball 里，没有自己
///    的下载地址。实测 888 个条目里有 348 个是这种，全在 `@paperclipai/server` 肚子里。
/// 2. **`os` / `cpu` / `libc` 不匹配的跳过**。npm 装的时候自己会按这三个字段过滤，
///    预热不跟着过滤就会把另外一百个平台的二进制也灌进缓存（esbuild 一家 25 个平台）。
/// 3. **`resolved` 不在钉死的 registry 上的跳过**。npm 的缓存键里带着完整 URL，所以
///    预热必须跟 install 走同一个源才算命中；顺带也是道闸门，这棵树里本该一个外部
///    URL 都没有。
fn prefetch_specs(lock: &serde_json::Value, plat: &NpmPlatform) -> Picked {
    let mut out = Picked::default();
    let Some(pkgs) = lock["packages"].as_object() else {
        return out;
    };
    let mut specs = std::collections::BTreeSet::new();
    for (path, m) in pkgs {
        // 空路径是根条目，也就是 npm 为这次解析现编的那份 package.json。
        if path.is_empty() {
            continue;
        }
        let Some(resolved) = m["resolved"].as_str() else {
            if m["inBundle"].as_bool() == Some(true) {
                out.bundled += 1;
            }
            continue;
        };
        if !plat.allows(m) {
            out.other_platform += 1;
            continue;
        }
        if !on_pinned_registry(resolved) {
            out.foreign.push(resolved.to_string());
            continue;
        }
        let (Some(name), Some(version)) = (entry_name(path, m), m["version"].as_str()) else {
            continue;
        };
        specs.insert(format!("{name}@{version}"));
    }
    out.specs = specs.into_iter().collect();
    out
}

/// 一个 lockfile 条目的真名。
///
/// 路径上的目录名不一定是包名：别名依赖（`"@openai/codex-linux-x64":
/// "npm:@openai/codex@0.160.0-linux-x64"`）装在以别名命名的目录里，真名在 `name`
/// 字段上。实测这棵树里恰好 12 个条目带 `name`，全是别名——而且正是最大的那几个
/// （codex 一家 141 + 155 MiB），按目录名去灌只会 404。
fn entry_name<'a>(path: &'a str, m: &'a serde_json::Value) -> Option<&'a str> {
    if let Some(n) = m["name"].as_str() {
        return Some(n);
    }
    path.rsplit("node_modules/")
        .next()
        .filter(|s| !s.is_empty())
}

/// 这个下载地址是不是落在钉死的那个 registry 上。
fn on_pinned_registry(url: &str) -> bool {
    url.strip_prefix(NPM_REGISTRY)
        .is_some_and(|rest| rest.starts_with('/'))
}

/// npm 眼里的本机平台：`os` / `cpu` / `libc` 三个字段该拿什么值去比。
#[derive(Debug, Clone, Copy)]
struct NpmPlatform {
    os: &'static str,
    cpu: &'static str,
    /// 只有 linux 的包会声明 libc，别的平台上是 `None`（当没约束处理）。
    libc: Option<&'static str>,
}

impl NpmPlatform {
    fn current() -> Self {
        let os = match std::env::consts::OS {
            "macos" => "darwin",
            other => other,
        };
        let cpu = match std::env::consts::ARCH {
            "x86_64" => "x64",
            "aarch64" => "arm64",
            other => other,
        };
        let libc = (os == "linux").then(|| if musl_system() { "musl" } else { "glibc" });
        Self { os, cpu, libc }
    }

    /// 这个条目适用于本机吗。
    fn allows(&self, m: &serde_json::Value) -> bool {
        constraint_ok(&m["os"], Some(self.os))
            && constraint_ok(&m["cpu"], Some(self.cpu))
            && constraint_ok(&m["libc"], self.libc)
    }
}

/// 一个 `os` / `cpu` / `libc` 约束跟本机的取值合不合。
///
/// 字段缺失就是没约束。写法有两种而且会混用：允许列表（`["linux"]`）和排除
/// （`["!win32"]`）。判定跟 npm 自己一致——命中任一排除项就否，有允许列表而没命中
/// 也否。本机取值未知时（非 linux 机器上的 `libc`）当没约束处理。
fn constraint_ok(v: &serde_json::Value, value: Option<&str>) -> bool {
    let Some(list) = v.as_array() else {
        return true;
    };
    let Some(value) = value else {
        return true;
    };
    let mut has_allow = false;
    let mut matched = false;
    for item in list.iter().filter_map(|i| i.as_str()) {
        match item.strip_prefix('!') {
            Some(neg) => {
                if neg == value {
                    return false;
                }
            }
            None => {
                has_allow = true;
                matched |= item == value;
            }
        }
    }
    !has_allow || matched
}

/// 这台机器的 C 库是不是 musl。
///
/// 判据跟 detect-libc 一个意思：musl 的那个动态链接器在不在（Alpine 上是
/// `/lib/ld-musl-x86_64.so.1`）。刻意不看 fundeploy 自己是怎么编出来的——CI 出的是
/// musl 静态包，而它跑在 glibc 机器上照样是 glibc 的机器，`cfg!(target_env = "musl")`
/// 问的是编译机不是这台机器。
fn musl_system() -> bool {
    std::fs::read_dir("/lib").is_ok_and(|d| {
        d.flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with("ld-musl-"))
    })
}

/// `npm view … version --json` 的输出里那个版本号。
///
/// 通常是一个 JSON 字符串；spec 命中多个版本时 npm 给的是数组，那种情况取最后一个
/// （npm 按版本升序排），跟它自己 `npm view` 的习惯一致。
fn npm_view_version(stdout: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).ok()?;
    let s = match &v {
        serde_json::Value::Array(a) => a.last()?.as_str()?,
        other => other.as_str()?,
    };
    (!s.is_empty()).then(|| s.to_string())
}

/// 一段输出的末尾几行，用来在报错里引上游的原话。
fn last_lines(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// 预热那一段的进度条：分母是包数，顺带报已经灌进去多少字节。
///
/// 刻意不摆 `{eta}`：包的大小差三个数量级（几 KiB 的 shim 和 155 MiB 的 codex 都算
/// 一个），按个数外推出来的「还要多久」没有意义。
fn prefetch_style() -> indicatif::ProgressStyle {
    indicatif::ProgressStyle::with_template(
        "{msg} [{bar:24.cyan/blue}] {pos}/{len} 个包 已过 {elapsed}",
    )
    .unwrap()
    .progress_chars("=>-")
}

/// install store 里正在写的那个暂存目录。
///
/// 上游的命名是 `.<版本>.tmp-<pid>-<毫秒>`，认 `.` 开头加 `.tmp-` 两个特征——正式
/// payload（`<版本>`）和那份临时 npmrc（`.npmrc-<pid>-<毫秒>`，是文件不是目录）都
/// 不会被误认。
///
/// 可能有不止一个：上一次被 kill -9 打断时，上游 `finally` 里的清理没来得及跑。按
/// （mtime, 名字）取最大的那个——名字里带着创建时的毫秒数，两条判据同向，不必依赖
/// 文件系统的 mtime 精度。
fn staging_dir(store: &Path) -> Option<PathBuf> {
    std::fs::read_dir(store)
        .ok()?
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.starts_with('.') && n.contains(".tmp-")
        })
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .max_by_key(|e| {
            (
                e.metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::UNIX_EPOCH),
                e.file_name(),
            )
        })
        .map(|e| e.path())
}

/// 从暂存目录名里取出正在装的版本号。
fn staging_version(name: &str) -> Option<&str> {
    let v = name.strip_prefix('.')?.rsplit_once(".tmp-")?.0;
    (!v.is_empty()).then_some(v)
}

/// 量一次大小要几百毫秒而且限了频，中间这几秒屏幕上得有东西在动，否则看着又像卡住了。
const STEADY_TICK: Duration = Duration::from_millis(200);

/// 一条待设样式的进度条。非终端环境返回隐藏的那种，省掉所有绘制。
fn progress_bar() -> indicatif::ProgressBar {
    if std::io::IsTerminal::is_terminal(&std::io::stderr()) {
        indicatif::ProgressBar::new_spinner()
    } else {
        indicatif::ProgressBar::hidden()
    }
}

/// 没有分母的那一段：报已经过去多久、攒了多少。
///
/// 刻意不摆 `{eta}`：这一段不知道总共要下多少，indicatif 会拿 0 顶上，屏幕上就是
/// 「还要 0s」——比没有数字更糟。`bytes` 为假时连字节数都不报（猜不到 npm 的缓存在哪）。
fn downloading_style(bytes: bool) -> indicatif::ProgressStyle {
    let t = if bytes {
        "{msg} {spinner} {bytes} {bytes_per_sec} 已过 {elapsed}"
    } else {
        "{msg} {spinner} 已过 {elapsed}"
    };
    indicatif::ProgressStyle::with_template(t).unwrap()
}

/// 有分母的那一段。
///
/// 模板比 [`crate::common::download`] 那条多一项 ETA：那边是几秒钟的文件下载，这边
/// 是几分钟到几十分钟，「还要多久」正是这里要回答的问题。
fn unpacking_style() -> indicatif::ProgressStyle {
    indicatif::ProgressStyle::with_template(
        "{msg} [{bar:24.cyan/blue}] {bytes}/{total_bytes} {bytes_per_sec} 还要 {eta}",
    )
    .unwrap()
    .progress_chars("=>-")
}

/// 时长转成人看的。
fn human_secs(d: Duration) -> String {
    let s = d.as_secs();
    if s < 60 {
        format!("{s} 秒")
    } else {
        format!("{} 分 {} 秒", s / 60, s % 60)
    }
}

/// awesome-paperclip 收录的插件。`pkg` 为空表示它不是 npm 包，装法见各自仓库。
struct Plugin {
    name: &'static str,
    pkg: &'static str,
    url: &'static str,
    about: &'static str,
}

const CATALOG: &[Plugin] = &[
    Plugin {
        name: "Agent Pixels",
        pkg: "@agent-pixels/paperclip-plugin",
        url: "https://github.com/gcampton/Agent-Pixels",
        about: "像素风 agent，可自定义行为、模型、房间，还能看监控摄像头",
    },
    Plugin {
        name: "obsidian-paperclip",
        pkg: "",
        url: "https://github.com/istib/obsidian-paperclip",
        about: "Obsidian 集成：在笔记里浏览、评论、指派 Paperclip issue",
    },
    Plugin {
        name: "paperclip-aperture",
        pkg: "@tomismeta/paperclip-aperture",
        url: "https://github.com/tomismeta/paperclip-aperture",
        about: "另一种 Focus 视图，按审批、issue 活动和待人处理的事件排序",
    },
    Plugin {
        name: "paperclip-live-analytics-plugin",
        pkg: "@agent-analytics/paperclip-live-analytics-plugin",
        url: "https://github.com/Agent-Analytics/paperclip-live-analytics-plugin",
        about: "实时访客地图、仪表盘挂件和 Agent Analytics 设置页",
    },
    Plugin {
        name: "paperclip-plugin-acp",
        pkg: "paperclip-plugin-acp",
        url: "https://github.com/mvanhorn/paperclip-plugin-acp",
        about: "在聊天平台里跑 Claude Code、Codex、Gemini CLI 的 ACP 运行时",
    },
    Plugin {
        name: "paperclip-plugin-avp",
        pkg: "paperclip-plugin-avp",
        url: "https://github.com/creatorrmode-lead/paperclip-plugin-avp",
        about: "基于 Agent Veil Protocol 的信任与声誉层",
    },
    Plugin {
        name: "paperclip-plugin-chat",
        pkg: "@paperclipai/plugin-chat",
        url: "https://github.com/webprismdevin/paperclip-plugin-chat",
        about: "面向任务、agent 和工作区的交互式 AI 聊天助手",
    },
    Plugin {
        name: "paperclip-plugin-company-wizard",
        pkg: "@yesterday-ai/paperclip-plugin-company-wizard",
        url: "https://github.com/yesterday-ai/paperclip-plugin-company-wizard",
        about: "带预设的 AI 公司初始化向导",
    },
    Plugin {
        name: "paperclip-plugin-discord",
        pkg: "paperclip-plugin-discord",
        url: "https://github.com/mvanhorn/paperclip-plugin-discord",
        about: "Discord 双向集成",
    },
    Plugin {
        name: "paperclip-plugin-github-issues",
        pkg: "paperclip-plugin-github-issues",
        url: "https://github.com/mvanhorn/paperclip-plugin-github-issues",
        about: "GitHub Issues 双向同步",
    },
    Plugin {
        name: "paperclip-plugin-linear",
        pkg: "@oldharlem/paperclip-plugin-linear",
        url: "https://github.com/Oldharlem/paperclip-linear-plugin",
        about: "Linear 双向同步，带 webhook 和一个 agent 工具",
    },
    Plugin {
        name: "paperclip-plugin-slack",
        pkg: "paperclip-plugin-slack",
        url: "https://github.com/mvanhorn/paperclip-plugin-slack",
        about: "issue 生命周期事件推到 Slack",
    },
    Plugin {
        name: "paperclip-plugin-telegram",
        pkg: "paperclip-plugin-telegram",
        url: "https://github.com/mvanhorn/paperclip-plugin-telegram",
        about: "issue 生命周期事件推到 Telegram",
    },
    Plugin {
        name: "paperclip-plugin-writbase",
        pkg: "paperclip-plugin-writbase",
        url: "https://github.com/Writbase/paperclip-plugin-writbase",
        about: "Paperclip issue 与 WritBase 任务双向同步",
    },
    Plugin {
        name: "paperclip-plugin-hindsight",
        pkg: "paperclip-plugin-hindsight",
        url: "https://github.com/vectorize-io/hindsight/tree/main/hindsight-integrations/paperclip-plugin",
        about: "给 Paperclip agent 的长期记忆",
    },
];

fn print_catalog() {
    for p in CATALOG {
        if p.pkg.is_empty() {
            ui::info(format!("{}（不是 npm 包，装法见仓库）", p.name));
        } else {
            ui::info(format!("{}（{}）", p.name, p.pkg));
        }
        ui::hint(p.about);
        ui::hint(p.url);
    }
}

fn secs(key: &str, default: u64) -> Duration {
    Duration::from_secs(env::num(key, default))
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

/// 认 `-f` / `--follow` 和 `-n N` / `-nN` / `--lines=N`，别的提示后忽略。
///
/// fundeploy 托管的日志就是一个文件，官方 `service logs` 那套 journalctl 参数在这儿
/// 没有对应物，与其假装支持，不如明说忽略了什么。
fn parse_log_args(args: &[&str], default_lines: usize) -> (usize, bool) {
    let mut lines = default_lines;
    let mut follow = false;
    let mut want_lines = false;
    for a in args {
        if want_lines {
            lines = a.parse().unwrap_or(lines);
            want_lines = false;
            continue;
        }
        if *a == "-f" || *a == "--follow" {
            follow = true;
        } else if *a == "-n" || *a == "--lines" {
            want_lines = true;
        } else if let Some(n) = a.strip_prefix("--lines=").or_else(|| a.strip_prefix("-n")) {
            lines = n.parse().unwrap_or(lines);
        } else {
            ui::warn(format!("托管日志是直接读文件，忽略不支持的参数 {a}"));
        }
    }
    (lines, follow)
}

/// 日志末尾的 N 行。
///
/// 只读末尾一段：服务日志能涨到几百 M，为看几十行把整个文件读进内存，会在最需要它的
/// 时候（启动失败、磁盘紧张）把事情变得更糟。
fn tail_lines(path: &Path, lines: usize) -> Vec<String> {
    use std::io::{Read, Seek, SeekFrom};
    const WINDOW: u64 = 256 * 1024;

    let Ok(mut f) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let from = len.saturating_sub(WINDOW);
    if f.seek(SeekFrom::Start(from)).is_err() {
        return Vec::new();
    }
    let mut buf = Vec::new();
    if f.read_to_end(&mut buf).is_err() {
        return Vec::new();
    }

    let text = String::from_utf8_lossy(&buf);
    let mut all: Vec<&str> = text.lines().collect();
    // 从中间截断的第一行多半是半行，丢掉。
    if from > 0 && !all.is_empty() {
        all.remove(0);
    }
    all.iter()
        .skip(all.len().saturating_sub(lines))
        .map(|s| (*s).to_string())
        .collect()
}

fn print_tail(path: &Path, lines: usize) {
    for line in tail_lines(path, lines) {
        eprintln!("  {line}");
    }
}

/// 跟随日志直到 Ctrl+C。
///
/// 自己轮询而不是调 `tail -f`：少一个外部依赖，而且 Ctrl+C 的归属清楚——信号投给的是
/// fundeploy 自己，不会留下一个还在跑的 tail。
fn follow_log(path: &Path) -> Result<()> {
    use std::io::{Read, Seek, SeekFrom, Write};

    let mut f = std::fs::File::open(path)?;
    let mut pos = f.seek(SeekFrom::End(0))?;
    loop {
        std::thread::sleep(Duration::from_millis(300));
        let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(pos);
        if len < pos {
            // 日志被轮转或清空了。旧的文件句柄指向的还是老 inode，必须重开，
            // 否则这条通道从此就哑了。
            f = std::fs::File::open(path)?;
            pos = 0;
            continue;
        }
        if len == pos {
            continue;
        }
        f.seek(SeekFrom::Start(pos))?;
        let mut buf = vec![0u8; (len - pos) as usize];
        let n = f.read(&mut buf)?;
        buf.truncate(n);
        pos += n as u64;
        std::io::stdout().write_all(&buf)?;
        std::io::stdout().flush()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn 日志参数认得出行数和跟随() {
        assert_eq!(parse_log_args(&[], 200), (200, false));
        assert_eq!(parse_log_args(&["-f"], 200), (200, true));
        assert_eq!(parse_log_args(&["-n", "50"], 200), (50, false));
        assert_eq!(parse_log_args(&["-n50"], 200), (50, false));
        assert_eq!(parse_log_args(&["--lines=50", "-f"], 200), (50, true));
        // 不认识的参数只提示，不该让看日志这件事直接失败。
        assert_eq!(parse_log_args(&["--since=1h"], 200), (200, false));
        // 行数不是数字时保持默认，而不是 panic。
        assert_eq!(parse_log_args(&["-n", "很多"], 200), (200, false));
    }

    #[test]
    fn 取日志末尾若干行() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("a.log");
        std::fs::write(&log, "1\n2\n3\n4\n5\n").unwrap();
        assert_eq!(tail_lines(&log, 2), ["4", "5"]);
        // 要的比有的多时给全部，不该 panic。
        assert_eq!(tail_lines(&log, 99).len(), 5);
        // 文件不存在只是「没日志」，不是错误。
        assert!(tail_lines(&dir.path().join("nope.log"), 10).is_empty());
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
    fn 插件清单里的包名不重复() {
        let mut pkgs: Vec<&str> = CATALOG
            .iter()
            .map(|p| p.pkg)
            .filter(|p| !p.is_empty())
            .collect();
        let n = pkgs.len();
        pkgs.sort_unstable();
        pkgs.dedup();
        assert_eq!(pkgs.len(), n, "插件清单里有重复的包名");
        assert!(CATALOG.iter().all(|p| p.url.starts_with("https://")));
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
    fn 暂存目录不会和正式_payload_认错() {
        let store = tempfile::tempdir().unwrap();
        // 正式 payload（装完 rename 出来的）和那份临时 npmrc 都不该被当成暂存目录。
        std::fs::create_dir(store.path().join("2026.1011.0-canary.7")).unwrap();
        std::fs::write(store.path().join(".npmrc-123-456"), "x").unwrap();
        assert_eq!(staging_dir(store.path()), None);

        let s = store.path().join(".2026.1012.0-canary.1.tmp-123-456");
        std::fs::create_dir(&s).unwrap();
        assert_eq!(staging_dir(store.path()), Some(s));
    }

    #[test]
    fn 有多个暂存目录时挑新的那个() {
        // 上一次被 kill -9 打断的话，上游 finally 里的清理没跑完，残骸会留在 store 里。
        let store = tempfile::tempdir().unwrap();
        let old = store.path().join(".1.0.0.tmp-1-100");
        let new = store.path().join(".2.0.0.tmp-1-200");
        std::fs::create_dir(&old).unwrap();
        std::fs::create_dir(&new).unwrap();
        // mtime 和名字两条判据在这儿同向，不依赖文件系统的 mtime 精度。
        assert_eq!(staging_dir(store.path()), Some(new));
    }

    #[test]
    fn 暂存目录名里认得出版本号() {
        assert_eq!(
            staging_version(".2026.1011.0-canary.7.tmp-98765-1760000000000"),
            Some("2026.1011.0-canary.7")
        );
        // 不是暂存目录的名字一个都不认，宁可让进度条显示「依赖树」。
        assert_eq!(staging_version("2026.1011.0"), None);
        assert_eq!(staging_version(".npmrc-123-456"), None);
        assert_eq!(staging_version("..tmp-1-2"), None);
    }

    /// 造一个只有路径、不碰环境的 watch：cache 固定为 None，免得测试去量本机那个
    /// 几 GB 的 npm 缓存。
    fn watch(home: &Path) -> InstallWatch {
        let cli = home.join("cli");
        InstallWatch {
            store: cli.join("installs").join("npm"),
            current: cli.join("current"),
            cache: None,
            bar: None,
            next: Instant::now(),
            last: 0,
            cache_base: 0,
            started: None,
        }
    }

    #[test]
    fn 暂存目录还空着的时候不摆分母() {
        let home = tempfile::tempdir().unwrap();
        let store = home.path().join("cli/installs/npm");
        std::fs::create_dir_all(&store).unwrap();
        std::fs::create_dir_all(home.path().join("cli/current")).unwrap();
        std::fs::write(home.path().join("cli/current/payload"), vec![0u8; 1000]).unwrap();

        let mut w = watch(home.path());
        // 还没建暂存目录：什么都不该发生。
        w.tick();
        assert!(w.bar.is_none(), "上游还在解析版本号，不该先画进度条");

        // 暂存目录建好但是空的——npm 正在往自己的缓存里下东西。这一段没有分母，
        // 摆上去就会算出「还要 0s」。
        std::fs::create_dir(store.join(".1.2.3.tmp-1-1")).unwrap();
        w.tick();
        let bar = w.bar.as_ref().expect("该画进度条了");
        assert_eq!(bar.length(), None, "下载段不该有分母");
        assert_eq!(w.last, 0);
    }

    #[test]
    fn 开始解包之后换成有分母的那一段() {
        let home = tempfile::tempdir().unwrap();
        let store = home.path().join("cli/installs/npm");
        let staging = store.join(".1.2.3.tmp-1-1");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::create_dir_all(home.path().join("cli/current")).unwrap();
        std::fs::write(home.path().join("cli/current/payload"), vec![0u8; 1000]).unwrap();

        let mut w = watch(home.path());
        w.tick();
        std::fs::write(staging.join("a"), vec![0u8; 400]).unwrap();
        w.next = Instant::now(); // 跳过限频，不然这一跳要等 MEASURE_EVERY
        w.tick();
        let bar = w.bar.as_ref().unwrap();
        assert_eq!(bar.length(), Some(1000), "分母该是当前 payload 的体积");
        assert_eq!(bar.position(), 400);

        // 新版比旧版大的话分母要跟着顶上去，否则最后几个百分点会停在 100% 还报
        // 「还要 0s」——实测 2.3 GiB 的估计值对上了 2.5 GiB 的实际值。
        std::fs::write(staging.join("b"), vec![0u8; 900]).unwrap();
        w.next = Instant::now();
        w.tick();
        let bar = w.bar.as_ref().unwrap();
        assert_eq!(bar.length(), Some(1300));
        assert_eq!(bar.position(), 1300);

        // rename 走了（或者失败被删了）：进度条收掉，量到的总量留着报。
        std::fs::remove_dir_all(&staging).unwrap();
        w.tick();
        assert!(
            w.bar.is_none(),
            "暂存目录没了，进度条必须立刻让位给上游的输出"
        );
        assert_eq!(w.last, 1300);
    }

    #[test]
    fn 量到一半被_rename_走不会让进度往回跳() {
        let home = tempfile::tempdir().unwrap();
        let staging = home.path().join("cli/installs/npm/.1.2.3.tmp-1-1");
        std::fs::create_dir_all(&staging).unwrap();
        let mut w = watch(home.path());
        w.tick();
        std::fs::write(staging.join("a"), vec![0u8; 500]).unwrap();
        w.next = Instant::now();
        w.tick();
        assert_eq!(w.last, 500);

        // 模拟「这一趟读到的比上一趟少」：目录被清空了但还在。
        std::fs::remove_file(staging.join("a")).unwrap();
        w.next = Instant::now();
        w.tick();
        assert_eq!(w.last, 500, "只认涨的那一侧");
        assert_eq!(w.bar.as_ref().unwrap().position(), 500);
    }

    #[test]
    fn 缓存冷热三种说法各自说准() {
        let cache = Path::new("/tmp/不存在的缓存/_cacache");
        let full = 1_000_000_000;
        // 空缓存：必须明说这次是全量，并且把路径摆出来（接着那句「别清它」才有着落）。
        let s = cache_state_of(0, cache, full);
        assert!(
            s.contains("空的") && s.contains("全量") && s.contains("_cacache"),
            "实际: {s}"
        );
        // 比整棵树小：只能说「装不满」。说不出更细的——缓存是内容寻址的，里面有多少
        // 属于这棵树，不把几万个索引条目逐个解开是问不出来的。
        let s = cache_state_of(full / 2, cache, full);
        assert!(s.contains("装不满"), "实际: {s}");
        assert!(!s.contains("全量"), "半热的缓存不该说成全量: {s}");
        // 够大：这时候才值得提 prefer-offline，冷缓存下它一点用都没有。
        let s = cache_state_of(full * 2, cache, full);
        assert!(s.contains("PREFER_OFFLINE"), "实际: {s}");
    }

    #[test]
    fn 预热的参数顺序不限也都能省() {
        // 通道省掉时跟 check 一样默认 canary（装了这个项目的机器基本都在跟开发版）。
        assert_eq!(
            prefetch_args(&[], Channel::Canary).unwrap(),
            (Channel::Canary, false)
        );
        assert_eq!(
            prefetch_args(&["prod"], Channel::Canary).unwrap(),
            (Channel::Prod, false)
        );
        // 两个参数谁先谁后都认：命令行上人怎么顺手怎么打。
        for args in [
            vec!["canary", "--background"],
            vec!["--background", "canary"],
            vec!["-b", "canary"],
        ] {
            assert_eq!(
                prefetch_args(&args, Channel::Prod).unwrap(),
                (Channel::Canary, true),
                "args={args:?}"
            );
        }
        // 拼错的通道必须当场报错，不能默默按默认值去灌另一个通道的包。
        let err = prefetch_args(&["canry"], Channel::Canary)
            .unwrap_err()
            .to_string();
        assert!(err.contains("canry"), "实际: {err}");
    }

    /// 造一份跟实测结构一致的 lockfile 片段。字段取名和取值都照 npm 写的来。
    fn lock_fixture() -> serde_json::Value {
        serde_json::json!({
            "packages": {
                // 根条目：npm 为这次解析现编的 package.json，没有包可下。
                "": { "dependencies": { "paperclipai": "^2026.1015.0" } },
                "node_modules/paperclipai": {
                    "version": "2026.1015.0-canary.9",
                    "resolved": "https://registry.npmjs.org/paperclipai/-/paperclipai-2026.1015.0-canary.9.tgz"
                },
                // 别名依赖：目录名是别名，真名在 name 字段上。
                "node_modules/@openai/codex-linux-x64": {
                    "name": "@openai/codex",
                    "version": "0.156.1-linux-x64",
                    "resolved": "https://registry.npmjs.org/@openai/codex/-/codex-0.156.1-linux-x64.tgz",
                    "optional": true, "os": ["linux"], "cpu": ["x64"]
                },
                // 同一个工具的另一个版本，上游版本范围对不上导致两份并存——两份都要下。
                "node_modules/@paperclipai/server/node_modules/@openai/codex-linux-x64": {
                    "name": "@openai/codex",
                    "version": "0.160.0-linux-x64",
                    "resolved": "https://registry.npmjs.org/@openai/codex/-/codex-0.160.0-linux-x64.tgz",
                    "optional": true, "os": ["linux"], "cpu": ["x64"]
                },
                // 别的平台：npm 装的时候也会跳过。
                "node_modules/@openai/codex-win32-x64": {
                    "name": "@openai/codex",
                    "version": "0.156.1-win32-x64",
                    "resolved": "https://registry.npmjs.org/@openai/codex/-/codex-0.156.1-win32-x64.tgz",
                    "optional": true, "os": ["win32"], "cpu": ["x64"]
                },
                // 装在父包 tarball 里的，没有自己的下载地址。
                "node_modules/@paperclipai/server/node_modules/acpx": {
                    "version": "0.8.1", "inBundle": true
                },
                // 外部 URL：这棵树里本该没有，出现就只报不灌。
                "node_modules/somewhere-else": {
                    "version": "1.0.0",
                    "resolved": "https://example.com/somewhere-else-1.0.0.tgz"
                }
            }
        })
    }

    #[test]
    fn 预热清单只留本机真会下的那些() {
        let linux = NpmPlatform {
            os: "linux",
            cpu: "x64",
            libc: Some("glibc"),
        };
        let picked = prefetch_specs(&lock_fixture(), &linux);
        assert_eq!(
            picked.specs,
            vec![
                // 别名条目按真名灌：按目录名（@openai/codex-linux-x64）去灌是 404。
                "@openai/codex@0.156.1-linux-x64".to_string(),
                "@openai/codex@0.160.0-linux-x64".to_string(),
                "paperclipai@2026.1015.0-canary.9".to_string(),
            ]
        );
        assert_eq!(picked.bundled, 1, "inBundle 的不单独下");
        assert_eq!(picked.other_platform, 1, "win32 那份该跳过");
        assert_eq!(
            picked.foreign,
            vec!["https://example.com/somewhere-else-1.0.0.tgz"]
        );

        // 换台 mac 就该挑出另一批；这里的 fixture 里 linux 专属的那两个都得让开。
        let mac = NpmPlatform {
            os: "darwin",
            cpu: "arm64",
            libc: None,
        };
        let picked = prefetch_specs(&lock_fixture(), &mac);
        assert_eq!(
            picked.specs,
            vec!["paperclipai@2026.1015.0-canary.9".to_string()]
        );
        assert_eq!(picked.other_platform, 3);
    }

    #[test]
    fn 平台约束的两种写法都认() {
        let v = |s: &str| serde_json::from_str::<serde_json::Value>(s).unwrap();
        // 字段缺失就是没约束。
        assert!(constraint_ok(&serde_json::Value::Null, Some("linux")));
        // 允许列表。
        assert!(constraint_ok(&v(r#"["linux","darwin"]"#), Some("linux")));
        assert!(!constraint_ok(&v(r#"["darwin"]"#), Some("linux")));
        // 排除写法：npm 里确实有 ["!win32"] 这种。
        assert!(constraint_ok(&v(r#"["!win32"]"#), Some("linux")));
        assert!(!constraint_ok(&v(r#"["!linux"]"#), Some("linux")));
        // 混用时排除优先。
        assert!(!constraint_ok(&v(r#"["linux","!linux"]"#), Some("linux")));
        // 本机取值未知（非 linux 机器上的 libc）当没约束，不能把包全筛掉。
        assert!(constraint_ok(&v(r#"["glibc"]"#), None));
    }

    #[test]
    fn 认得出_npm_view_给的版本号() {
        assert_eq!(
            npm_view_version("\"2026.1015.0-canary.9\"\n").as_deref(),
            Some("2026.1015.0-canary.9")
        );
        // spec 命中多个版本时 npm 给数组，按升序，要的是最后那个。
        assert_eq!(
            npm_view_version(r#"["2026.1014.0","2026.1015.0"]"#).as_deref(),
            Some("2026.1015.0")
        );
        // 认不出来必须是 None，好让调用方报错而不是拿空字符串去拼 spec。
        assert_eq!(npm_view_version(""), None);
        assert_eq!(npm_view_version("npm error code E404"), None);
        assert_eq!(npm_view_version("\"\""), None);
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

    #[test]
    fn 时长说成人话() {
        assert_eq!(human_secs(Duration::from_secs(0)), "0 秒");
        assert_eq!(human_secs(Duration::from_secs(48)), "48 秒");
        assert_eq!(human_secs(Duration::from_secs(60)), "1 分 0 秒");
        assert_eq!(human_secs(Duration::from_secs(372)), "6 分 12 秒");
    }
}
