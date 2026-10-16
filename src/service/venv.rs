//! 装在独占虚拟环境里、进程由 fundeploy 自己守的那一族。
//!
//! 和 [`super::daemon`] 的分歧只在包从哪来：那边是 GitHub release 的附件，这边是
//! PyPI 装进 `<home>/venv`。和 [`super::upstream`] 的分歧在谁管进程：那边上游 CLI
//! 自带 `server start/stop`，这边上游只给一条前台命令。
//!
//! ```text
//! <home>/venv/  独占的虚拟环境
//! <home>/run/   PID 文件
//! <home>/log/   fundeploy 接管下来的 stdout/stderr
//! ```
//!
//! 没有并到 [`super::daemon`] 里，是因为那边几条承重假设在这儿都不成立：
//! 可执行文件在 `venv/bin/` 而不是 `app/`；认进程靠 `/proc/<pid>/exe` 比对二进制，
//! 而 venv 里的控制台脚本跑起来 `exe` 是 python 解释器，比对必然失败；还有「每个
//! 进程都有端口」这条——celery 的 worker 和 beat 一个端口都不听。

use anyhow::{anyhow, bail, Context, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{force_free_port, uninstall_env_var, Extra, Service, START_TIMEOUT};
use crate::common::fs::guard_removable;
use crate::common::{cmd, env, port, proc};
use crate::ui;

/// 约束文件的 URL：`(主包版本, Python 的 (主, 次)) -> URL`。
///
/// Airflow 这类把一整棵依赖树钉死在一张清单里的项目需要它；不给就按普通依赖解析。
pub type ConstraintsFn = Box<dyn Fn(&str, (u32, u32)) -> String>;

/// 停一个进程最多等多久。
const STOP_TIMEOUT: Duration = Duration::from_secs(30);

/// 按单个进程操作的三个动作。
///
/// 多进程服务排查问题时「只把 worker 放前台跑一遍看它到底报什么」是常规操作，
/// 九个标准动作粒度太粗。服务把它们并进自己的 [`Venv::extras`] 即可，实现由
/// [`Venv::proc_extra`] 统一提供。
pub const PROC_EXTRAS: &[Extra] = &[
    Extra {
        name: "start-one",
        about: "只后台起一个进程: start-one <进程名>",
    },
    Extra {
        name: "run-one",
        about: "只前台跑一个进程: run-one <进程名>",
    },
    Extra {
        name: "stop-one",
        about: "只停一个进程: stop-one <进程名>",
    },
];

/// 不监听端口的进程，起完等多久再看它还在不在。
///
/// 这类进程没有「端口开了」这种明确的就绪信号，只能退而求其次。起不来的典型原因
/// （broker 连不上、app 模块 import 失败）都在头一两秒里就暴露了。
const SETTLE: Duration = Duration::from_secs(2);

/// 一个要守着的进程。
pub struct Proc {
    pub key: String,
    pub label: String,
    /// `venv/bin/` 下的可执行文件名。
    pub bin: String,
    pub args: Vec<String>,
    /// 监听地址。一个端口都不听的进程给 `None`，它的死活只能看进程本身。
    pub listen: Option<(String, u16)>,
    pub cwd: PathBuf,
    pub env: Vec<cmd::EnvEdit>,
}

impl Proc {
    pub fn new(key: &str, label: &str, bin: &str, cwd: PathBuf) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            bin: bin.into(),
            args: Vec::new(),
            listen: None,
            cwd,
            env: Vec::new(),
        }
    }

    pub fn args(mut self, args: Vec<String>) -> Self {
        self.args = args;
        self
    }

    pub fn listen(mut self, host: &str, port: u16) -> Self {
        self.listen = Some((host.into(), port));
        self
    }

    pub fn env(mut self, k: &str, v: impl Into<String>) -> Self {
        self.env.push((k.into(), Some(v.into())));
        self
    }

    /// 确保这个变量不出现在子进程的环境里。见 [`cmd::EnvEdit`]。
    pub fn unset(mut self, k: &str) -> Self {
        self.env.push((k.into(), None));
        self
    }

    pub fn port(&self) -> Option<u16> {
        self.listen.as_ref().map(|(_, p)| *p)
    }
}

pub struct VenvSpec {
    pub name: String,
    pub summary: String,
    pub home: PathBuf,
    pub venv: PathBuf,
    /// 指定建 venv 用哪个解释器。`None` 时自己挑。
    pub python: Option<String>,
    /// 解释器版本的下限和上限（含）。
    pub min_python: (u32, u32),
    pub max_python: Option<(u32, u32)>,
    /// 主包的分发名。
    pub dist: String,
    /// 把主包钉死在这个版本。
    pub version: Option<String>,
    /// 没钉版本时的版本下限。
    ///
    /// 不装裸包：上游跨大版本时选项名和默认值都会变，解析到一个远古版本不会报错，
    /// 只会在运行时以奇怪的方式跑偏。
    pub floor: Option<String>,
    /// 和主包一起装的其它依赖，每个都是完整的 requirement 串。
    pub deps: Vec<String>,
    pub constraints: Option<ConstraintsFn>,
    pub procs: Vec<Proc>,
    /// `run` 时放前台的那个进程的 key。`None` 表示最后一个。
    pub foreground: Option<String>,
    /// `home` 下还要建出来的目录。
    pub dirs: Vec<String>,
    /// 等端口监听起来的上限。
    pub start_timeout: Duration,
    pub entry: Option<String>,
    pub notes: Vec<String>,
}

impl VenvSpec {
    pub fn new(name: &str, summary: &str, home: PathBuf, dist: &str, procs: Vec<Proc>) -> Self {
        Self {
            name: name.into(),
            summary: summary.into(),
            venv: home.join("venv"),
            home,
            python: None,
            min_python: (3, 9),
            max_python: None,
            dist: dist.into(),
            version: None,
            floor: None,
            deps: Vec::new(),
            constraints: None,
            procs,
            foreground: None,
            dirs: Vec::new(),
            start_timeout: START_TIMEOUT,
            entry: None,
            notes: Vec::new(),
        }
    }

    pub fn python(mut self, py: Option<String>) -> Self {
        self.python = py;
        self
    }

    pub fn python_range(mut self, min: (u32, u32), max: Option<(u32, u32)>) -> Self {
        self.min_python = min;
        self.max_python = max;
        self
    }

    pub fn version(mut self, v: impl Into<String>) -> Self {
        self.version = Some(v.into());
        self
    }

    pub fn floor(mut self, v: impl Into<String>) -> Self {
        self.floor = Some(v.into());
        self
    }

    pub fn deps(mut self, deps: Vec<String>) -> Self {
        self.deps = deps;
        self
    }

    pub fn constraints(mut self, f: ConstraintsFn) -> Self {
        self.constraints = Some(f);
        self
    }

    pub fn foreground(mut self, key: &str) -> Self {
        self.foreground = Some(key.into());
        self
    }

    pub fn dirs(mut self, dirs: &[&str]) -> Self {
        self.dirs = dirs.iter().map(|d| d.to_string()).collect();
        self
    }

    pub fn start_timeout(mut self, d: Duration) -> Self {
        self.start_timeout = d;
        self
    }

    pub fn entry(mut self, url: impl Into<String>) -> Self {
        self.entry = Some(url.into());
        self
    }

    pub fn note(mut self, n: impl Into<String>) -> Self {
        self.notes.push(n.into());
        self
    }

    /// venv 自带的解释器。它在不在，就是「装没装」。
    pub fn python_path(&self) -> PathBuf {
        self.venv.join("bin").join("python")
    }

    pub fn bin(&self, name: &str) -> PathBuf {
        self.venv.join("bin").join(name)
    }

    fn pid_file(&self, p: &Proc) -> PathBuf {
        self.home.join("run").join(format!("{}.pid", p.key))
    }

    fn log_file(&self, p: &Proc) -> PathBuf {
        self.home.join("log").join(format!("{}.log", p.key))
    }

    /// 这次要装的主包写成 requirement。
    pub fn requirement(&self, version: Option<&str>) -> String {
        match self.pinned(version) {
            Some(v) => format!("{}=={v}", self.dist),
            None => match &self.floor {
                Some(f) => format!("{}>={f}", self.dist),
                None => self.dist.clone(),
            },
        }
    }

    /// 本次实际钉住的版本：命令行给的优先，其次是声明里的。
    fn pinned(&self, version: Option<&str>) -> Option<String> {
        version
            .map(|v| v.to_string())
            .or_else(|| self.version.clone())
    }

    /// 装进去的主包是哪个版本。问 venv 自己，不另外记账。
    pub fn installed_version(&self) -> Option<String> {
        let py = self.python_path();
        if !py.is_file() {
            return None;
        }
        let code = format!(
            "import importlib.metadata as m; print(m.version({:?}))",
            self.dist
        );
        let v = cmd::capture([py.to_string_lossy().as_ref(), "-c", &code]).ok()?;
        let v = v.trim().to_string();
        (!v.is_empty()).then_some(v)
    }

    /// `run` 时放前台的那个进程的下标。
    fn fg_index(&self) -> usize {
        match &self.foreground {
            Some(key) => self
                .procs
                .iter()
                .position(|p| &p.key == key)
                .unwrap_or(self.procs.len() - 1),
            None => self.procs.len() - 1,
        }
    }

    fn validate(&self) -> Result<()> {
        if self.procs.is_empty() {
            bail!("服务 {} 没有进程声明", self.name);
        }
        let mut keys: Vec<&str> = Vec::new();
        let mut ports: Vec<u16> = Vec::new();
        for p in &self.procs {
            if keys.contains(&p.key.as_str()) {
                bail!("服务 {} 里有重复的进程 key: {}", self.name, p.key);
            }
            keys.push(&p.key);
            if let Some(port) = p.port() {
                if ports.contains(&port) {
                    bail!("服务 {} 里有两个进程都要听 {port}", self.name);
                }
                ports.push(port);
            }
        }
        if let Some(key) = &self.foreground {
            if !keys.contains(&key.as_str()) {
                bail!("服务 {} 的 foreground 指向不存在的进程 {key}", self.name);
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- Venv

pub trait Venv {
    fn spec(&self) -> &VenvSpec;

    // ------------------------------------------------------------ 钩子

    /// 在这个 venv 里跑任何东西都要带上的环境变量。
    ///
    /// 这类服务的行为基本全由环境变量决定（`AIRFLOW_HOME`、`CELERY_BROKER_URL`），
    /// 而且 start 和各种诊断命令必须看到同一套，不然问的是两个库。
    fn env(&self) -> Vec<cmd::EnvEdit> {
        Vec::new()
    }

    /// 启动前要就位的东西（配置文件、脚手架）。
    fn ensure_config(&self) -> Result<()> {
        Ok(())
    }

    /// 装完之后的收尾（建库、迁移）。
    fn post_install(&self) -> Result<()> {
        Ok(())
    }

    /// status 末尾追加的服务专属信息。
    fn extra_status(&self) -> Result<()> {
        Ok(())
    }

    fn extras(&self) -> &'static [Extra] {
        &[]
    }

    fn extra(&self, name: &str, args: &[String]) -> Result<()> {
        let _ = args;
        bail!("{} 声明了动作 {name} 却没有实现它", self.spec().name)
    }

    // ------------------------------------------------------------ 生命周期

    fn install(&self, version: Option<&str>) -> Result<()> {
        self.provision(version, false)?;
        let spec = self.spec();
        ui::blank();
        ui::ok(format!(
            "{} {} 已装在 {}。",
            spec.name,
            spec.installed_version().as_deref().unwrap_or("?"),
            spec.venv.display()
        ));
        ui::hint(format!("启动: fundeploy service {} start", spec.name));
        Ok(())
    }

    fn upgrade(&self) -> Result<()> {
        let spec = self.spec();
        self.require_installed()?;
        let before = spec.installed_version();

        // 先停服。Python 包是一堆散文件，在解释器跑着的时候整体换掉，那些还没被
        // import 到的模块就会和已经 import 的那批对不上，表现成运行中途莫名其妙的
        // ImportError。何况紧接着的 post_install 往往要迁库。
        let was_running = self.running();
        if was_running {
            ui::step("升级前服务在运行，先停服");
            self.stop()?;
            ui::blank();
        }

        self.provision(None, true).map_err(|e| {
            if was_running {
                e.context(format!(
                    "升级失败，而服务已经被停掉了——它现在是停的。\n\
                     修好之后执行 `fundeploy service {} start` 把它拉起来。",
                    spec.name
                ))
            } else {
                e
            }
        })?;

        ui::blank();
        ui::heading("版本变化");
        ui::info(format!(
            "{}: {} -> {}",
            spec.dist,
            before.as_deref().unwrap_or("未安装"),
            spec.installed_version().as_deref().unwrap_or("未知"),
        ));

        ui::blank();
        if was_running {
            ui::step("升级前服务在运行，重新启动");
            self.start()
        } else {
            ui::ok("升级完成（升级前服务没在运行，没有自动启动）。");
            Ok(())
        }
    }

    fn rollback(&self, version: &str) -> Result<()> {
        let spec = self.spec();
        let v = version.trim().trim_start_matches(['v', 'V']);
        if v.is_empty() {
            bail!(
                "rollback 要给版本号，例如: fundeploy service {} rollback 3.1.8",
                spec.name
            );
        }

        let was_running = self.running();
        if was_running {
            ui::step("回退前服务在运行，先停服");
            self.stop()?;
            ui::blank();
        }

        // 不带 --upgrade：带上之后 `dist==旧版本` 也能装，但 pip 会顺手把其它依赖
        // 往新了升，而回退的本意是回到那套能跑的组合。
        self.provision(Some(v), false).map_err(|e| {
            if was_running {
                e.context(format!(
                    "回退失败，而服务已经被停掉了——它现在是停的。\n\
                     修好之后执行 `fundeploy service {} start` 把它拉起来。",
                    spec.name
                ))
            } else {
                e
            }
        })?;

        ui::blank();
        if was_running {
            ui::step("回退前服务在运行，重新启动");
            self.start()
        } else {
            ui::ok(format!(
                "已回退到 {v}（回退前服务没在运行，没有自动启动）。"
            ));
            Ok(())
        }
    }

    fn start(&self) -> Result<()> {
        let spec = self.spec();
        spec.validate()?;
        self.require_installed()?;
        ensure_dirs(spec)?;
        self.ensure_config()?;

        for p in &spec.procs {
            if let Some((host, port)) = &p.listen {
                if port::is_listening(host, *port) {
                    ui::info(format!("{} {host}:{port} 已在运行，跳过。", p.label));
                    continue;
                }
            } else if proc::read_pid_file(&spec.pid_file(p)).is_some_and(proc::is_alive) {
                ui::info(format!("{} 已在运行，跳过。", p.label));
                continue;
            }
            self.spawn(p)?;
        }

        ui::ok("已启动。");
        if let Some(url) = &spec.entry {
            ui::hint(format!("入口: {url}"));
        }
        for n in &spec.notes {
            ui::hint(n);
        }
        Ok(())
    }

    /// 前台跑一个进程，其余后台起；前台退出时把后台那些一起停掉。
    fn run(&self) -> Result<()> {
        let spec = self.spec();
        spec.validate()?;
        self.require_installed()?;
        ensure_dirs(spec)?;

        // run 起的是全新一套。接管已有进程的话，前台一退就会把别人起的服务停掉。
        for p in &spec.procs {
            if let Some((host, port)) = &p.listen {
                if port::is_listening(host, *port) {
                    bail!(
                        "{} 的端口 {port} 已被占用；run 不接管已有进程。\n\
                         要前台跑，先 `fundeploy service {} stop`。",
                        p.label,
                        spec.name
                    );
                }
            }
            if let Some(pid) = proc::read_pid_file(&spec.pid_file(p)).filter(|&x| proc::is_alive(x))
            {
                bail!(
                    "{} 已经在跑（PID {pid}）；run 不接管已有进程。\n\
                     要前台跑，先 `fundeploy service {} stop`。",
                    p.label,
                    spec.name
                );
            }
        }
        self.ensure_config()?;

        let fg = spec.fg_index();
        let mut started: Vec<&Proc> = Vec::new();
        let mut result = Ok(());
        for (i, p) in spec.procs.iter().enumerate() {
            if i == fg {
                continue;
            }
            if let Err(e) = self.spawn(p) {
                result = Err(e);
                break;
            }
            started.push(p);
        }

        if result.is_ok() {
            let p = &spec.procs[fg];
            std::fs::create_dir_all(&p.cwd)
                .with_context(|| format!("建工作目录失败: {}", p.cwd.display()))?;
            ui::step(format!(
                "前台启动 {}（Ctrl+C 退出，退出时自动停掉后台进程）",
                p.label
            ));
            let argv = self.argv(p);
            // 前台子进程和自己在同一个进程组里，Ctrl+C 两边都收到。自己要是按默认
            // 行为当场死掉，就没机会回来停上面那些后台进程了。
            let guard = proc::SignalGuard::install();
            let code = cmd::spawn_status_in(&argv, &self.proc_env(p), Some(&p.cwd));
            drop(guard);

            result = match code {
                // 被信号终止（Ctrl+C）是正常的退出方式，不是失败。
                Ok(Some(0)) | Ok(None) => Ok(()),
                Ok(Some(c)) => Err(anyhow!("{} 退出，退出码 {c}", p.label)),
                Err(e) => Err(e),
            };
        }

        if !started.is_empty() {
            ui::blank();
            for p in started.iter().rev() {
                self.kill(p);
            }
        }
        result
    }

    fn stop(&self) -> Result<()> {
        let spec = self.spec();
        // 逆序停：先停依赖别人的那个，它才不会在被依赖方消失后报一堆连接错误。
        for p in spec.procs.iter().rev() {
            self.kill(p);
        }
        for p in spec.procs.iter().rev() {
            if let Some((host, port)) = &p.listen {
                force_free_port(&p.label, host, *port);
            }
        }
        ui::info("已停止（best effort）。");
        Ok(())
    }

    fn restart(&self) -> Result<()> {
        self.stop()?;
        ui::blank();
        self.start()
    }

    /// 一张卡片说完：版本、每个进程的状态、要排查时去哪看。
    ///
    /// 这里不用 [`ui::warn`]——「进程在但端口没监听」是状态本身，不是操作中途出
    /// 的意外。分到 stderr 会让一次 status 的输出一半进文件、一半留在屏幕，顺序
    /// 还会因为两个流各自缓冲而错乱。
    fn status(&self) -> Result<()> {
        let spec = self.spec();
        let mut card = ui::Card::new(&spec.name);

        match spec.installed_version() {
            Some(v) => card = card.row(&spec.dist, format!("{v} · {}", spec.venv.display())),
            None => {
                card = card.row_state(&spec.dist, ui::State::Idle, "未安装");
                card = card.note(format!("装上: fundeploy service {} install", spec.name));
            }
        }

        let mut up = 0usize;
        for p in &spec.procs {
            let pid = proc::read_pid_file(&spec.pid_file(p)).filter(|&x| proc::is_alive(x));
            let (st, text) = match (&p.listen, pid) {
                // 没有端口的进程（celery worker、beat）只能看 PID 文件。
                (None, Some(pid)) => {
                    up += 1;
                    (ui::State::Up, format!("在运行 · PID {pid}"))
                }
                (None, None) => (ui::State::Idle, "没在运行".to_string()),
                (Some((host, port)), pid) => {
                    let at = format!("{host}:{port}");
                    let listening = port::is_listening(host, *port);
                    match (pid, listening) {
                        (Some(pid), true) => {
                            up += 1;
                            (ui::State::Up, format!("在运行 · PID {pid} · {at}"))
                        }
                        // PID 还活着但端口没开：多半是起来之后业务初始化就挂了，
                        // 这跟「没在运行」是两件事，排查方向也不一样。
                        (Some(pid), false) => {
                            (ui::State::Down, format!("进程在 PID {pid}，但 {at} 没监听"))
                        }
                        // 端口有人但 PID 文件对不上。是手工起的本服务，还是别的程序
                        // 正好占了这个端口，差别很大。
                        (None, true) => match port::listener_pid(*port) {
                            Some(other) => match owned_by(&spec.venv, other) {
                                Some(true) => {
                                    up += 1;
                                    (
                                        ui::State::Up,
                                        format!(
                                            "在运行 · PID {other}（不是 fundeploy 起的）· {at}"
                                        ),
                                    )
                                }
                                Some(false) => (
                                    ui::State::Down,
                                    format!("{at} 被 PID {other} 占着，不是这个 venv 里的东西"),
                                ),
                                None => (
                                    ui::State::Down,
                                    format!("{at} 被 PID {other} 占着，认不出是不是本服务"),
                                ),
                            },
                            None => {
                                up += 1;
                                (
                                    ui::State::Up,
                                    format!("在运行（不是 fundeploy 起的）· {at}"),
                                )
                            }
                        },
                        (None, false) => (ui::State::Idle, format!("没在运行 · {at}")),
                    }
                }
            };
            card = card.row_state(&p.label, st, text);
        }

        let (bst, blabel) = match (up, spec.procs.len()) {
            (_, 0) => (ui::State::Idle, "没有进程".to_string()),
            (0, _) => (ui::State::Idle, "未运行".to_string()),
            (u, n) if u == n => (ui::State::Up, "在运行".to_string()),
            (u, n) => (ui::State::Partial, format!("部分 {u}/{n}")),
        };
        card.badge(bst, blabel).print();

        // 日志路径摆框外：它常常比框里所有字段都长。
        for p in &spec.procs {
            ui::hint(format!("日志 {} {}", p.label, spec.log_file(p).display()));
        }

        self.extra_status()
    }

    fn uninstall(&self) -> Result<()> {
        let spec = self.spec();
        guard_removable(&spec.home)?;
        guard_removable(&spec.venv)?;

        ui::confirm_destructive(
            &format!(
                "将停止服务并删除整个 {}（含里面的数据和数据库，不可逆），确认？",
                spec.home.display()
            ),
            &uninstall_env_var(&spec.name),
        )?;

        self.stop()?;
        ui::blank();

        // venv 一般就在 home 里面，配置成别处时得单独删。
        if !spec.venv.starts_with(&spec.home) && spec.venv.exists() {
            std::fs::remove_dir_all(&spec.venv)
                .with_context(|| format!("删除失败: {}", spec.venv.display()))?;
            ui::ok(format!("已删除 {}。", spec.venv.display()));
        }
        if spec.home.exists() {
            std::fs::remove_dir_all(&spec.home)
                .with_context(|| format!("删除失败: {}", spec.home.display()))?;
        }
        ui::ok(format!("已删除 {}。", spec.home.display()));
        Ok(())
    }

    // ------------------------------------------------------------ 单进程动作

    /// 处理 [`PROC_EXTRAS`] 里的动作。不是这三个就返回 `None`，留给服务自己的
    /// `extra` 分支。
    fn proc_extra(&self, name: &str, args: &[String]) -> Option<Result<()>> {
        match name {
            "start-one" | "run-one" | "stop-one" => Some(self.one(name, args)),
            _ => None,
        }
    }

    fn one(&self, action: &str, args: &[String]) -> Result<()> {
        let spec = self.spec();
        let keys = spec
            .procs
            .iter()
            .map(|p| p.key.as_str())
            .collect::<Vec<_>>()
            .join("、");

        let Some(key) = args.first() else {
            bail!(
                "用法: fundeploy service {} {action} <进程名>\n可选: {keys}",
                spec.name
            );
        };
        let Some(p) = spec.procs.iter().find(|p| &p.key == key) else {
            bail!("{} 没有叫 {key} 的进程。可选: {keys}", spec.name);
        };

        if action == "stop-one" {
            self.kill(p);
            // 跟整体 stop 一样收尾，不然单停一个进程会是唯一一条「端口没腾出来也不
            // 吭声」的路径，紧接着的 start-one 照样起不来。
            if let Some((host, port)) = &p.listen {
                force_free_port(&p.label, host, *port);
            }
            return Ok(());
        }

        self.require_installed()?;
        ensure_dirs(spec)?;
        self.ensure_config()?;

        if action == "start-one" {
            return self.spawn(p);
        }

        // run-one：和 run 一样不接管已有进程，不然 Ctrl+C 会把别人起的那份带走。
        if let Some(pid) = proc::read_pid_file(&spec.pid_file(p)).filter(|&x| proc::is_alive(x)) {
            bail!(
                "{} 已经在跑（PID {pid}）。要前台跑先: fundeploy service {} stop-one {key}",
                p.label,
                spec.name
            );
        }
        std::fs::create_dir_all(&p.cwd)
            .with_context(|| format!("建工作目录失败: {}", p.cwd.display()))?;
        ui::step(format!("前台启动 {}（Ctrl+C 退出）", p.label));
        match cmd::spawn_status_in(self.argv(p), &self.proc_env(p), Some(&p.cwd))? {
            Some(0) | None => Ok(()),
            Some(c) => bail!("{} 退出，退出码 {c}", p.label),
        }
    }

    // ------------------------------------------------------------ 在 venv 里跑东西

    /// 跑 `venv/bin/<bin>`，要求成功。
    fn exec(&self, bin: &str, args: &[&str]) -> Result<()> {
        let mut argv = vec![self.spec().bin(bin).to_string_lossy().into_owned()];
        argv.extend(args.iter().map(|a| a.to_string()));
        cmd::run_with(&argv, &self.env())
    }

    /// 同 [`Venv::exec`]，但允许失败，返回退出码。
    fn try_exec(&self, bin: &str, args: &[&str]) -> Result<Option<i32>> {
        let mut argv = vec![self.spec().bin(bin).to_string_lossy().into_owned()];
        argv.extend(args.iter().map(|a| a.to_string()));
        cmd::spawn_status_in(&argv, &self.env(), None)
    }

    /// 跑 `venv/bin/<bin>` 并取回 stdout。
    fn capture(&self, bin: &str, args: &[&str]) -> Result<String> {
        let mut argv = vec![self.spec().bin(bin).to_string_lossy().into_owned()];
        argv.extend(args.iter().map(|a| a.to_string()));
        cmd::capture_with(&argv, &self.env())
    }

    // ------------------------------------------------------------ 内部

    /// 建 venv、装包、收尾。install / upgrade / rollback 共用的那一段。
    fn provision(&self, version: Option<&str>, upgrade: bool) -> Result<()> {
        let spec = self.spec();
        spec.validate()?;
        ensure_dirs(spec)?;

        if !spec.python_path().is_file() {
            let py = pick_python(spec)?;
            ui::step(format!("用 {py} 建虚拟环境 {}", spec.venv.display()));
            create_venv(&py, &spec.venv)?;
        }
        let py_ver = python_version(&spec.python_path())?;
        ensure_pip(spec);

        let mut pkgs = vec![spec.requirement(version)];
        pkgs.extend(spec.deps.iter().cloned());

        // 没钉住主包版本就没有约束文件可言——约束清单是按版本发的。
        let constraint = match (&spec.constraints, spec.pinned(version)) {
            (Some(f), Some(v)) => Some(f(&v, py_ver)),
            _ => None,
        };
        if let Some(url) = &constraint {
            ui::info(format!("约束: {url}"));
        }

        ui::step(format!("安装 {}", pkgs.join(" ")));
        pip_install(spec, &pkgs, constraint.as_deref(), upgrade)?;

        ui::blank();
        self.post_install()
    }

    /// 我们起的那份在跑吗。
    ///
    /// 跟「端口上有东西」不是一回事。别人的进程占着同一个端口时这里要答「没在跑」，
    /// 否则 upgrade/rollback 会先去停一个不归自己管的进程，再在换完包之后把一个
    /// 从没停过的服务「恢复」起来。
    fn running(&self) -> bool {
        let spec = self.spec();
        spec.procs.iter().any(|p| {
            if proc::read_pid_file(&spec.pid_file(p)).is_some_and(proc::is_alive) {
                return true;
            }
            // PID 文件没了也可能真是我们的进程（手工删过），认 venv。
            p.port()
                .and_then(port::listener_pid)
                .and_then(|pid| owned_by(&spec.venv, pid))
                .unwrap_or(false)
        })
    }

    fn require_installed(&self) -> Result<()> {
        let spec = self.spec();
        if !spec.python_path().is_file() {
            bail!(
                "{} 还没装（{} 不存在）\n先执行 `fundeploy service {} install`",
                spec.name,
                spec.python_path().display(),
                spec.name
            );
        }
        for p in &spec.procs {
            if !spec.bin(&p.bin).is_file() {
                bail!(
                    "{} 用的 {} 不在 venv 里。\n重新装一次: fundeploy service {} install",
                    p.label,
                    spec.bin(&p.bin).display(),
                    spec.name
                );
            }
        }
        Ok(())
    }

    fn argv(&self, p: &Proc) -> Vec<String> {
        let mut argv = vec![self.spec().bin(&p.bin).to_string_lossy().into_owned()];
        argv.extend(p.args.iter().cloned());
        argv
    }

    /// 服务级环境叠上这个进程自己的。顺序要紧：后面的覆盖前面的。
    fn proc_env(&self, p: &Proc) -> Vec<cmd::EnvEdit> {
        let mut env = self.env();
        env.extend(p.env.iter().cloned());
        env
    }

    /// 后台起一个进程并确认它真的活下来了。
    fn spawn(&self, p: &Proc) -> Result<()> {
        let spec = self.spec();
        if let Some((host, _)) = &p.listen {
            if port::is_public(host) {
                ui::warn(format!(
                    "{} 监听 {host}，本机之外也能访问；确认这是你要的。",
                    p.label
                ));
            }
        }
        std::fs::create_dir_all(&p.cwd)
            .with_context(|| format!("建工作目录失败: {}", p.cwd.display()))?;
        match &p.listen {
            Some((host, port)) => ui::step(format!("启动 {}，监听 {host}:{port}", p.label)),
            None => ui::step(format!("启动 {}", p.label)),
        }

        let log = spec.log_file(p);
        let pid = proc::spawn_detached(&self.argv(p), &p.cwd, &self.proc_env(p), &log)?;
        proc::write_pid_file(&spec.pid_file(p), pid)?;

        match &p.listen {
            Some((host, port)) => {
                let sp = ui::Spinner::new(format!("等 {} 监听 {host}:{port}", p.label));
                let listening =
                    port::wait_for_listener_ticking(host, *port, spec.start_timeout, || sp.tick());
                if listening {
                    sp.done(format!("{} 起来了，{host}:{port} 在听", p.label));
                } else {
                    sp.give_up();
                }
                if listening {
                    return Ok(());
                }
                let why = if proc::is_alive(pid) {
                    format!(
                        "{} 起来了（PID {pid}）但端口 {port} 迟迟没被监听。",
                        p.label
                    )
                } else {
                    format!("{} 起来之后就退了（PID {pid} 已经没了）。", p.label)
                };
                bail!("{why}\n日志 {}:\n{}", log.display(), tail(&log, 20));
            }
            None => {
                std::thread::sleep(SETTLE);
                if proc::is_alive(pid) {
                    return Ok(());
                }
                bail!(
                    "{} 起来之后就退了（PID {pid} 已经没了）。\n日志 {}:\n{}",
                    p.label,
                    log.display(),
                    tail(&log, 20)
                );
            }
        }
    }

    /// 停一个进程。best effort：没在跑不算错。
    fn kill(&self, p: &Proc) {
        let spec = self.spec();
        ui::step(format!("停止 {}", p.label));

        // 自成进程组起的，整组停——celery worker 的子进程池、airflow standalone
        // 派生的 scheduler/triggerer 不跟着走的话会变成孤儿。
        match proc::stop_by_pid_file(&spec.pid_file(p), STOP_TIMEOUT, true) {
            Ok(true) => return,
            Ok(false) => {}
            Err(e) => ui::warn(format!("{} 停止失败: {e}", p.label)),
        }

        // PID 文件丢了，按端口兜底。没有端口的进程到这儿就只能作罢——拿名字去
        // 全机扫进程然后动手，误伤的代价比留个孤儿大得多。
        let Some((host, port)) = &p.listen else {
            ui::info(format!("{} 本来就没在运行。", p.label));
            return;
        };
        if !port::is_listening(host, *port) {
            ui::info(format!("{} 本来就没在运行。", p.label));
            return;
        }
        let Some(pid) = port::listener_pid(*port) else {
            ui::warn(format!(
                "{} 的端口 {port} 有人在听，但既没有 PID 文件也查不到 PID。",
                p.label
            ));
            return;
        };

        // 只报不动：真要动手是 stop 收尾那一步（[`force_free_port`]）的事，强停要问
        // 一声，而这一问全流程只该出现一次。这里先把归属交代清楚——那一问只给得出
        // 进程名，「它是不是这个 venv 里的东西」只有这儿答得上来。
        match owned_by(&spec.venv, pid) {
            Some(true) => ui::info(format!("PID 文件没了，按端口 {port} 找到 PID {pid}")),
            Some(false) => ui::warn(format!(
                "端口 {port} 被 PID {pid} 占着，它跑的不是 {} 里的东西。",
                spec.venv.display()
            )),
            None => ui::warn(format!(
                "端口 {port} 被 PID {pid} 占着，确认不了那是不是 {}（多半不是当前用户起的）。",
                p.label
            )),
        }
    }
}

// ---------------------------------------------------------------- 进程归属

/// `pid` 跑的是不是 `venv` 里的东西。
///
/// 这里是 `starts_with` 而不是比对某个具体文件：venv 里的控制台脚本（`bin/airflow`）
/// 跑起来，`/proc/<pid>/exe` 指的是 `venv/bin/python`；而真正监听端口的往往还是它
/// 再 fork 出来的子进程，同样是那个解释器。认「属于这个 venv」才覆盖得住。
///
/// `None` 表示答不上来——非 Linux 没有 `/proc`，进程不属于当前用户时读
/// `/proc/<pid>/exe` 也会被拒。「答不上来」和「确认不是」得分开。
fn owned_by(venv: &Path, pid: i32) -> Option<bool> {
    let exe = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    Some(exe.starts_with(venv.canonicalize().ok()?))
}

// ---------------------------------------------------------------- 装包

fn ensure_dirs(spec: &VenvSpec) -> Result<()> {
    let mut dirs: Vec<String> = vec!["run".into(), "log".into()];
    dirs.extend(spec.dirs.iter().cloned());
    for d in dirs {
        let p = spec.home.join(d);
        std::fs::create_dir_all(&p).with_context(|| format!("建目录失败: {}", p.display()))?;
    }
    Ok(())
}

/// 挑一个够格的解释器。
fn pick_python(spec: &VenvSpec) -> Result<String> {
    let want = describe_range(spec);

    // 显式指定了就必须是它。悄悄退回别的解释器，人会以为跑在自己指定的那个上。
    if let Some(py) = &spec.python {
        let v = version_of(py)
            .ok_or_else(|| anyhow!("指定的解释器 {py} 用不了（找不到，或者跑不起来）"))?;
        if !in_range(v, spec) {
            bail!("指定的解释器 {py} 是 {}.{}，{want}", v.0, v.1);
        }
        return Ok(py.clone());
    }

    let mut seen: Vec<String> = Vec::new();
    for c in python_candidates() {
        match version_of(&c) {
            Some(v) if in_range(v, spec) => return Ok(c),
            Some(v) => seen.push(format!("{c}（{}.{}）", v.0, v.1)),
            None => {}
        }
    }

    let found = if seen.is_empty() {
        "一个解释器都没找到".to_string()
    } else {
        format!("只找到: {}", seen.join("、"))
    };
    bail!(
        "{want}，但{found}。\n\
         装一个之后用 {}_PYTHON_BIN 指过去，例如 {}_PYTHON_BIN=/usr/bin/python3.12",
        spec.name.to_ascii_uppercase().replace('-', "_"),
        spec.name.to_ascii_uppercase().replace('-', "_"),
    )
}

fn describe_range(spec: &VenvSpec) -> String {
    let (lo_a, lo_b) = spec.min_python;
    match spec.max_python {
        Some((hi_a, hi_b)) => format!("需要 Python {lo_a}.{lo_b} ~ {hi_a}.{hi_b}"),
        None => format!("需要 Python {lo_a}.{lo_b} 及以上"),
    }
}

fn in_range(v: (u32, u32), spec: &VenvSpec) -> bool {
    if v < spec.min_python {
        return false;
    }
    match spec.max_python {
        Some(max) => v <= max,
        None => true,
    }
}

/// 按优先级排的候选解释器。系统默认的排最前，够用就不折腾。
fn python_candidates() -> Vec<String> {
    let mut v = vec!["python3".to_string()];
    for minor in (10..=14).rev() {
        v.push(format!("python3.{minor}"));
    }
    let home = env::home();
    for minor in (10..=14).rev() {
        v.push(
            home.join(format!("opt/py3{minor}/bin/python3"))
                .to_string_lossy()
                .into_owned(),
        );
    }
    v
}

fn version_of(py: &str) -> Option<(u32, u32)> {
    python_version(Path::new(py)).ok()
}

fn python_version(py: &Path) -> Result<(u32, u32)> {
    let out = cmd::capture([
        py.to_string_lossy().as_ref(),
        "-c",
        "import sys; print('%d.%d' % sys.version_info[:2])",
    ])
    .with_context(|| format!("问不出 {} 的版本", py.display()))?;
    parse_py_version(&out).ok_or_else(|| anyhow!("{} 报的版本看不懂: {out}", py.display()))
}

fn parse_py_version(s: &str) -> Option<(u32, u32)> {
    let s = s.trim();
    let (a, b) = s.split_once('.')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

fn create_venv(py: &str, venv: &Path) -> Result<()> {
    if let Some(parent) = venv.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("建目录失败: {}", parent.display()))?;
    }
    if cmd::has("uv") {
        return cmd::run(["uv", "venv", "--python", py, &venv.to_string_lossy()]);
    }
    cmd::run([py, "-m", "venv", &venv.to_string_lossy()])
}

/// venv 里有没有 pip。
///
/// `uv venv` 默认不装 pip。补一个：往这个 venv 里加个 provider、临时装个包是常事，
/// 一个没有 pip 的 venv 对拿到它的人是意外。装不上不致命。
fn ensure_pip(spec: &VenvSpec) {
    if spec.bin("pip").is_file() {
        return;
    }
    if let Err(e) = pip_install(spec, &["pip".to_string()], None, false) {
        ui::warn(format!("往 venv 里装 pip 失败（不影响服务本身）: {e}"));
    }
}

fn pip_install(
    spec: &VenvSpec,
    pkgs: &[String],
    constraint: Option<&str>,
    upgrade: bool,
) -> Result<()> {
    let py = spec.python_path().to_string_lossy().into_owned();
    let mut argv: Vec<String> = if cmd::has("uv") {
        vec![
            "uv".into(),
            "pip".into(),
            "install".into(),
            "--python".into(),
            py,
        ]
    } else {
        vec![py, "-m".into(), "pip".into(), "install".into()]
    };
    if upgrade {
        argv.push("--upgrade".into());
    }
    if let Some(c) = constraint {
        argv.push("--constraint".into());
        argv.push(c.into());
    }
    argv.extend(pkgs.iter().cloned());
    cmd::run(&argv)
}

/// 日志末尾若干行。启动失败时贴出来，省得人再去找日志路径。
fn tail(path: &Path, lines: usize) -> String {
    let Ok(text) = std::fs::read_to_string(path) else {
        return "（读不到日志）".into();
    };
    let all: Vec<&str> = text.lines().collect();
    if all.is_empty() {
        return "（日志是空的）".into();
    }
    all[all.len().saturating_sub(lines)..].join("\n")
}

// ---------------------------------------------------------------- 适配

/// 把 [`Venv`] 接到统一的 [`Service`] 上。
pub struct Adapter<T>(pub T);

impl<T: Venv> Service for Adapter<T> {
    fn name(&self) -> &str {
        &self.0.spec().name
    }
    fn summary(&self) -> &str {
        &self.0.spec().summary
    }
    fn ports(&self) -> Vec<u16> {
        self.0
            .spec()
            .procs
            .iter()
            .filter_map(|p| p.port())
            .collect()
    }
    fn install(&self, version: Option<&str>) -> Result<()> {
        self.0.install(version)
    }
    fn upgrade(&self) -> Result<()> {
        self.0.upgrade()
    }
    fn rollback(&self, version: &str) -> Result<()> {
        self.0.rollback(version)
    }
    fn start(&self) -> Result<()> {
        self.0.start()
    }
    fn run(&self) -> Result<()> {
        self.0.run()
    }
    fn stop(&self) -> Result<()> {
        self.0.stop()
    }
    fn restart(&self) -> Result<()> {
        self.0.restart()
    }
    fn status(&self) -> Result<()> {
        self.0.status()
    }
    fn uninstall(&self) -> Result<()> {
        self.0.uninstall()
    }
    fn extras(&self) -> &'static [Extra] {
        self.0.extras()
    }
    fn extra(&self, name: &str, args: &[String]) -> Result<()> {
        self.0.extra(name, args)
    }
}

pub fn boxed<T: Venv + 'static>(t: T) -> Box<dyn Service> {
    Box::new(Adapter(t))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn 样例(procs: Vec<Proc>) -> VenvSpec {
        VenvSpec::new(
            "demo",
            "测试用",
            PathBuf::from("/tmp/fd-demo"),
            "demo-dist",
            procs,
        )
    }

    fn 进程(key: &str) -> Proc {
        Proc::new(key, key, key, PathBuf::from("/tmp"))
    }

    #[test]
    fn 钉了版本就用等号没钉就用下限() {
        let mut s = 样例(vec![进程("a")]);
        assert_eq!(s.requirement(None), "demo-dist");

        s.floor = Some("5.3".into());
        assert_eq!(s.requirement(None), "demo-dist>=5.3");

        s.version = Some("6.0".into());
        assert_eq!(s.requirement(None), "demo-dist==6.0");

        // 命令行给的版本压过声明里的。
        assert_eq!(s.requirement(Some("5.9")), "demo-dist==5.9");
    }

    #[test]
    fn 前台进程默认是最后一个也可以指名() {
        let s = 样例(vec![进程("a"), 进程("b"), 进程("c")]);
        assert_eq!(s.fg_index(), 2);
        assert_eq!(s.foreground("a").fg_index(), 0);
    }

    #[test]
    fn 进程_key_重复要报错() {
        let s = 样例(vec![进程("a"), 进程("a")]);
        let err = s.validate().unwrap_err().to_string();
        assert!(err.contains("重复的进程 key"), "实际: {err}");
    }

    #[test]
    fn 同一个服务里两个进程抢一个端口要报错() {
        // 这比端口被外人占走更难排查：两个进程一个起来一个起不来，还都是自己的。
        let s = 样例(vec![
            进程("a").listen("127.0.0.1", 9001),
            进程("b").listen("127.0.0.1", 9001),
        ]);
        let err = s.validate().unwrap_err().to_string();
        assert!(err.contains("9001"), "实际: {err}");
    }

    #[test]
    fn 前台指向不存在的进程要报错() {
        let s = 样例(vec![进程("a")]).foreground("b");
        let err = s.validate().unwrap_err().to_string();
        assert!(err.contains("foreground"), "实际: {err}");
    }

    #[test]
    fn 没有端口的进程不进端口列表() {
        let s = 样例(vec![
            进程("worker"),
            进程("flower").listen("127.0.0.1", 9002),
        ]);
        let ports: Vec<u16> = s.procs.iter().filter_map(|p| p.port()).collect();
        assert_eq!(ports, vec![9002]);
    }

    #[test]
    fn 解析_python_版本() {
        assert_eq!(parse_py_version("3.12\n"), Some((3, 12)));
        assert_eq!(parse_py_version(" 3.9 "), Some((3, 9)));
        assert_eq!(parse_py_version("什么"), None);
        assert_eq!(parse_py_version("3"), None);
    }

    #[test]
    fn 版本区间两头都卡() {
        let s = 样例(vec![进程("a")]).python_range((3, 10), Some((3, 13)));
        assert!(!in_range((3, 9), &s));
        assert!(in_range((3, 10), &s));
        assert!(in_range((3, 13), &s));
        // 上限存在的意义就在这儿：系统 python 跑到上游还不支持的版本时要跳过它，
        // 而不是装到一半报一堆编译错误。
        assert!(!in_range((3, 14), &s));
    }

    #[test]
    fn 没有上限时只卡下限() {
        let s = 样例(vec![进程("a")]).python_range((3, 9), None);
        assert!(!in_range((3, 8), &s));
        assert!(in_range((3, 20), &s));
    }

    #[test]
    fn 候选解释器里系统默认排第一() {
        let c = python_candidates();
        assert_eq!(c[0], "python3");
        assert!(c.contains(&"python3.12".to_string()));
    }
}
