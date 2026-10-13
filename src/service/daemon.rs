//! fundeploy 自己当进程管理器的那一族。
//!
//! 这些服务上游只发一个前台二进制（GitHub release 的附件），没有 `server start`
//! 之类的东西。于是下载、解包、起后台进程、写 PID、停的时候先 TERM 后 KILL，
//! 全都落在这儿。
//!
//! 目录是固定的四格，`home` 由各服务自己给：
//!
//! ```text
//! <home>/app/   上游解出来的东西，每次装都整个换掉
//! <home>/data/  服务自己的数据，装和升都不碰
//! <home>/run/   PID 文件
//! <home>/log/   日志
//! ```
//!
//! `app/` 整体替换是刻意的：原地覆盖会把上个版本多出来的文件留在那里，上游删掉
//! 某个插件之后它还在，表现为「升级了但老行为还在」。

use anyhow::{anyhow, bail, Context, Result};
use std::path::{Path, PathBuf};

use super::{uninstall_env_var, warn_if_still_listening, Service, START_TIMEOUT};
use crate::common::platform::Platform;
use crate::common::{archive, cmd, download, github, port, proc};
use crate::ui;

/// 按 tag 和平台算出附件名。
///
/// 收 tag 而不是版本号，因为附件名里嵌的到底带不带 `v` 各家不一样
/// （`new-api-v0.12.6` 对 `code-server-4.112.0-linux-amd64.tar.gz`）。
pub type AssetFn = Box<dyn Fn(&str, Platform) -> String>;

/// 上游在 GitHub release 里发二进制。
pub struct Github {
    pub repo: String,
    pub asset: AssetFn,
    /// 归档要剥掉的前导目录层数，对应 `tar --strip-components`。
    pub strip: usize,
    /// 附件是裸二进制时，放进 `app/` 后叫什么名字。
    pub binary_name: String,
    /// 往回翻几条 release 找带附件的那条。
    pub scan: usize,
}

/// 一个要守着的进程。
pub struct Proc {
    pub key: String,
    pub label: String,
    /// 可执行文件，相对 `app/`。
    pub bin: String,
    pub args: Vec<String>,
    pub host: String,
    pub port: u16,
    /// 在哪个目录下跑。
    pub cwd: PathBuf,
    pub env: Vec<cmd::EnvEdit>,
}

impl Proc {
    pub fn new(key: &str, label: &str, bin: &str, host: &str, port: u16, cwd: PathBuf) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            bin: bin.into(),
            args: Vec::new(),
            host: host.into(),
            port,
            cwd,
            env: Vec::new(),
        }
    }

    pub fn args(mut self, args: Vec<String>) -> Self {
        self.args = args;
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

    /// 监听地址在本机之外也能到吗。
    fn is_public(&self) -> bool {
        !matches!(self.host.as_str(), "127.0.0.1" | "localhost" | "::1")
    }
}

pub struct DaemonSpec {
    pub name: String,
    pub summary: String,
    pub home: PathBuf,
    pub source: Github,
    pub procs: Vec<Proc>,
    /// 装完/起完给人看的入口地址。
    pub entry: Option<String>,
    pub notes: Vec<String>,
}

impl DaemonSpec {
    pub fn new(name: &str, summary: &str, home: PathBuf, source: Github, procs: Vec<Proc>) -> Self {
        Self {
            name: name.into(),
            summary: summary.into(),
            home,
            source,
            procs,
            entry: None,
            notes: Vec::new(),
        }
    }

    pub fn entry(mut self, url: impl Into<String>) -> Self {
        self.entry = Some(url.into());
        self
    }

    pub fn note(mut self, n: impl Into<String>) -> Self {
        self.notes.push(n.into());
        self
    }

    pub fn app_dir(&self) -> PathBuf {
        self.home.join("app")
    }
    pub fn data_dir(&self) -> PathBuf {
        self.home.join("data")
    }
    fn pid_file(&self, p: &Proc) -> PathBuf {
        self.home.join("run").join(format!("{}.pid", p.key))
    }
    fn log_file(&self, p: &Proc) -> PathBuf {
        self.home.join("log").join(format!("{}.log", p.key))
    }
    /// 装进去的是哪个版本。上游二进制的 `--version` 未必和 release tag 对得上，
    /// 自己记一笔最靠得住。
    fn version_file(&self) -> PathBuf {
        self.home.join("version")
    }

    pub fn installed_version(&self) -> Option<String> {
        let v = std::fs::read_to_string(self.version_file()).ok()?;
        let v = v.trim().to_string();
        (!v.is_empty()).then_some(v)
    }

    fn validate(&self) -> Result<()> {
        if self.procs.is_empty() {
            bail!("服务 {} 没有进程声明", self.name);
        }
        let mut seen: Vec<&str> = Vec::new();
        for p in &self.procs {
            if seen.contains(&p.key.as_str()) {
                bail!("服务 {} 里有重复的进程 key: {}", self.name, p.key);
            }
            seen.push(&p.key);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- Daemon

pub trait Daemon {
    fn spec(&self) -> &DaemonSpec;

    // ------------------------------------------------------------ 钩子

    /// 启动前要就位的东西（配置文件、数据目录、密钥）。
    fn ensure_config(&self) -> Result<()> {
        Ok(())
    }

    /// 装完之后的收尾。
    fn post_install(&self) -> Result<()> {
        Ok(())
    }

    /// status 末尾追加的服务专属信息。
    fn extra_status(&self) -> Result<()> {
        Ok(())
    }

    /// 实际生效的端口。
    ///
    /// 有些服务的端口最终由它自己的配置文件说了算（new-api 读 `.env` 里的 PORT），
    /// 声明里的值只是默认。看状态、等监听都得用这个。
    fn effective_port(&self, p: &Proc) -> u16 {
        p.port
    }

    // ------------------------------------------------------------ 生命周期

    fn install(&self, version: Option<&str>) -> Result<()> {
        let spec = self.spec();
        spec.validate()?;
        let src = &spec.source;
        let plat = Platform::current()?;
        download::hint_mirror();

        let (release, asset_name) = match version {
            Some(v) => {
                let tag = github::tag_of(v)?;
                let r = github::by_tag(&src.repo, &tag)?;
                let name = (src.asset)(&r.tag_name, plat);
                if r.asset(&name).is_none() {
                    bail!(
                        "{} 的 {tag} 里没有 {name}（这个版本没发当前平台的包）",
                        src.repo
                    );
                }
                (r, name)
            }
            // latest 可能刚打完 tag、CI 还没传附件，照着它装只会 404。
            None => github::latest_with_asset(&src.repo, src.scan, |t| (src.asset)(t, plat))?,
        };
        let url = release
            .asset(&asset_name)
            .map(|a| a.browser_download_url.clone())
            .unwrap_or_else(|| github::asset_url(&src.repo, &release.tag_name, &asset_name));

        ui::step(format!(
            "安装 {} {}（{asset_name}）",
            spec.name,
            release.version()
        ));

        // 临时目录放在 home 下而不是 /tmp：同一个文件系统，最后那步换目录才是
        // 原子的 rename，而不是跨设备拷贝拷到一半断电。
        let work = spec.home.join(".tmp");
        let _ = std::fs::remove_dir_all(&work);
        std::fs::create_dir_all(&work)
            .with_context(|| format!("建临时目录失败: {}", work.display()))?;

        let downloaded = work.join(&asset_name);
        download::to_file(&url, &downloaded, &asset_name)?;

        let checksums = release
            .checksums_url()
            .and_then(|u| download::get_text(u).ok());
        download::verify(&downloaded, checksums.as_deref(), &asset_name)?;

        let staging = work.join("app");
        unpack(&downloaded, &asset_name, &staging, src)?;

        // 解出来没有要跑的那个文件，多半是 strip 剥多了或者上游换了布局。
        // 这里不拦住的话，install 报成功、start 才说找不到二进制。
        for p in &spec.procs {
            let bin = staging.join(&p.bin);
            if !bin.is_file() {
                bail!(
                    "{asset_name} 里没有 {}（解包后的布局和预期不符，strip={} 可能不对）",
                    p.bin,
                    src.strip
                );
            }
            archive::make_executable(&bin)?;
        }

        swap_in(&spec.app_dir(), &staging, &work)?;
        std::fs::write(spec.version_file(), format!("{}\n", release.version()))
            .with_context(|| format!("写版本记录失败: {}", spec.version_file().display()))?;
        let _ = std::fs::remove_dir_all(&work);

        self.post_install()?;

        ui::blank();
        ui::heading("已安装版本");
        ui::kv(&spec.name, Some(release.version()));
        ui::hint(format!("装在 {}", spec.home.display()));
        Ok(())
    }

    fn upgrade(&self) -> Result<()> {
        let spec = self.spec();
        spec.validate()?;

        let before = spec.installed_version();
        let was_running = self.running();

        if was_running {
            ui::step("升级前服务在运行，先停服");
            self.stop()?;
            ui::blank();
        }

        // 停了服又没换成包，服务此刻是停的。不说清楚的话，操作者看到的是一条
        // 安装报错，会以为「失败了所以什么都没变」，而实际上服务已经下线了。
        self.install(None).map_err(|e| {
            if was_running {
                e.context(format!(
                    "换包失败，而服务已经被停掉了——它现在是停的。\n\
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
            spec.name,
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
                "rollback 要给版本号，例如: fundeploy service {} rollback 0.12.6",
                spec.name
            );
        }

        let was_running = self.running();
        if was_running {
            ui::step("回退前服务在运行，先停服");
            self.stop()?;
            ui::blank();
        }

        self.install(Some(v)).map_err(|e| {
            if was_running {
                e.context(format!(
                    "换包失败，而服务已经被停掉了——它现在是停的。\n\
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
            let port = self.effective_port(p);
            if port::is_listening(&p.host, port) {
                ui::info(format!("{} {}:{port} 已在运行，跳过。", p.label, p.host));
                continue;
            }
            self.spawn(p, port)?;
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

    /// 前台跑最后一个进程，前面的后台起；前台退出时把后台那些一起停掉。
    fn run(&self) -> Result<()> {
        let spec = self.spec();
        spec.validate()?;
        self.require_installed()?;
        ensure_dirs(spec)?;

        // run 起的是全新一套。接管已有进程的话，前台一退就会把别人起的服务停掉。
        for p in &spec.procs {
            let port = self.effective_port(p);
            if port::is_listening(&p.host, port) {
                bail!(
                    "{} 的端口 {port} 已被占用；run 不接管已有进程。\n\
                     要前台跑，先 `fundeploy service {} stop`。",
                    p.label,
                    spec.name
                );
            }
        }
        self.ensure_config()?;

        let (last, rest) = spec
            .procs
            .split_last()
            .ok_or_else(|| anyhow!("服务 {} 没有进程", spec.name))?;

        let mut started: Vec<&Proc> = Vec::new();
        let mut result = Ok(());
        for p in rest {
            let port = self.effective_port(p);
            if let Err(e) = self.spawn(p, port) {
                result = Err(e);
                break;
            }
            started.push(p);
        }

        if result.is_ok() {
            ui::step(format!(
                "前台启动 {}（Ctrl+C 退出，退出时自动停掉后台进程）",
                last.label
            ));
            let argv = self.argv(last)?;
            // 前台子进程和自己在同一个进程组里，Ctrl+C 两边都收到。自己要是按默认
            // 行为当场死掉，就没机会回来停上面那些后台进程了。
            let guard = proc::SignalGuard::install();
            let code = cmd::spawn_status_in(&argv, &last.env, Some(&last.cwd));
            drop(guard);

            result = match code {
                // 被信号终止（Ctrl+C）是正常的退出方式，不是失败。
                Ok(Some(0)) | Ok(None) => Ok(()),
                Ok(Some(c)) => Err(anyhow!("{} 退出，退出码 {c}", last.label)),
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
            warn_if_still_listening(&p.label, &p.host, self.effective_port(p));
        }
        ui::info("已停止（best effort）。");
        Ok(())
    }

    fn restart(&self) -> Result<()> {
        self.stop()?;
        ui::blank();
        self.start()
    }

    fn status(&self) -> Result<()> {
        let spec = self.spec();

        ui::heading("已安装版本");
        match spec.installed_version() {
            Some(v) => ui::info(format!("{}: {v}（{}）", spec.name, spec.home.display())),
            None => ui::info(format!(
                "{}: 未安装（fundeploy service {} install）",
                spec.name, spec.name
            )),
        }

        for p in &spec.procs {
            let port = self.effective_port(p);
            ui::blank();
            ui::heading(format!("{}（{}:{port}）", p.label, p.host));

            let pid = proc::read_pid_file(&spec.pid_file(p)).filter(|&x| proc::is_alive(x));
            let listening = port::is_listening(&p.host, port);
            match (pid, listening) {
                (Some(pid), true) => ui::info(format!("在运行（PID {pid}）")),
                // PID 还活着但端口没开：多半是起来之后业务初始化就挂了，
                // 这跟「没在运行」是两件事，排查方向也不一样。
                (Some(pid), false) => ui::warn(format!(
                    "进程在（PID {pid}）但端口没监听，看日志: {}",
                    spec.log_file(p).display()
                )),
                // 端口有人但 PID 文件对不上。是手工起的本服务，还是别的程序正好占了
                // 这个端口，差别很大——后者意味着 start 会起不来、stop 不会去碰它。
                (None, true) => match port::listener_pid(port) {
                    Some(other) => match same_binary(other, &spec.app_dir().join(&p.bin)) {
                        Some(true) => {
                            ui::info(format!("在运行（PID {other}，不是 fundeploy 起的）"))
                        }
                        Some(false) => {
                            ui::warn(format!("端口被 PID {other} 占着，跑的不是本服务的二进制"))
                        }
                        None => ui::warn(format!(
                            "端口被 PID {other} 占着，确认不了是不是本服务（多半不是当前用户起的）"
                        )),
                    },
                    None => ui::info("在运行（不是 fundeploy 起的）"),
                },
                (None, false) => ui::info("没在运行"),
            }
            ui::hint(format!("日志: {}", spec.log_file(p).display()));
        }

        self.extra_status()
    }

    fn uninstall(&self) -> Result<()> {
        let spec = self.spec();
        ui::confirm_destructive(
            &format!(
                "将停止服务并删除整个 {}（含 data/ 下的数据，不可逆），确认？",
                spec.home.display()
            ),
            &uninstall_env_var(&spec.name),
        )?;

        self.stop()?;
        ui::blank();

        if spec.home.exists() {
            std::fs::remove_dir_all(&spec.home)
                .with_context(|| format!("删除失败: {}", spec.home.display()))?;
        }
        ui::ok(format!("已删除 {}。", spec.home.display()));
        Ok(())
    }

    // ------------------------------------------------------------ 内部

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
            // PID 文件没了也可能真是我们的进程（手工删过），认二进制。
            port::listener_pid(self.effective_port(p))
                .and_then(|pid| same_binary(pid, &spec.app_dir().join(&p.bin)))
                .unwrap_or(false)
        })
    }

    fn require_installed(&self) -> Result<()> {
        let spec = self.spec();
        for p in &spec.procs {
            if !spec.app_dir().join(&p.bin).is_file() {
                bail!(
                    "{} 还没装（{} 不存在）\n先执行 `fundeploy service {} install`",
                    p.label,
                    spec.app_dir().join(&p.bin).display(),
                    spec.name
                );
            }
        }
        Ok(())
    }

    fn argv(&self, p: &Proc) -> Result<Vec<String>> {
        let bin = self.spec().app_dir().join(&p.bin);
        let mut argv = vec![bin.to_string_lossy().into_owned()];
        argv.extend(p.args.iter().cloned());
        Ok(argv)
    }

    /// 后台起一个进程并等到端口真的在听。
    fn spawn(&self, p: &Proc, port: u16) -> Result<()> {
        let spec = self.spec();
        if p.is_public() {
            ui::warn(format!(
                "{} 监听 {}，本机之外也能访问；确认这是你要的。",
                p.label, p.host
            ));
        }
        std::fs::create_dir_all(&p.cwd)
            .with_context(|| format!("建工作目录失败: {}", p.cwd.display()))?;
        ui::step(format!("启动 {}，监听 {}:{port}", p.label, p.host));

        let log = spec.log_file(p);
        let argv = self.argv(p)?;
        let pid = proc::spawn_detached(&argv, &p.cwd, &p.env, &log)?;
        proc::write_pid_file(&spec.pid_file(p), pid)?;

        if port::wait_for_listener(&p.host, port, START_TIMEOUT) {
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

    /// 停一个进程。best effort：没在跑不算错。
    fn kill(&self, p: &Proc) {
        let spec = self.spec();
        let pid_file = spec.pid_file(p);
        ui::step(format!("停止 {}", p.label));

        // 自成进程组起的，整组停——服务派生的子进程（worker、浏览器实例）
        // 不跟着走的话会继续占着端口。
        match proc::stop_by_pid_file(&pid_file, START_TIMEOUT, true) {
            Ok(true) => return,
            Ok(false) => {}
            Err(e) => ui::warn(format!("{} 停止失败: {e}", p.label)),
        }

        // PID 文件丢了（手工删过、或者上一次是别的方式起的），按端口兜底。
        let port = self.effective_port(p);
        if !port::is_listening(&p.host, port) {
            ui::info(format!("{} 本来就没在运行。", p.label));
            return;
        }
        let Some(pid) = port::listener_pid(port) else {
            ui::warn(format!(
                "{} 的端口 {port} 有人在听，但既没有 PID 文件也查不到 PID。",
                p.label
            ));
            return;
        };

        // 认准二进制再动手。端口是全机共享的，别人的进程完全可能正好坐在这个
        // 端口上（同一台机器上另装了一份、或者端口号撞了），把它停掉是不可挽回
        // 的误伤；少停一次只是留个提示。
        let bin = spec.app_dir().join(&p.bin);
        match same_binary(pid, &bin) {
            Some(true) => {
                ui::info(format!("PID 文件没了，按端口 {port} 找到 PID {pid}"));
                if let Err(e) = proc::stop_pid(pid, START_TIMEOUT, false) {
                    ui::warn(format!("{} 停止失败: {e}", p.label));
                }
            }
            Some(false) => ui::warn(format!(
                "端口 {port} 被 PID {pid} 占着，但它跑的不是 {}，没有动它。\n\
                 确认该停的话: kill -TERM {pid}",
                bin.display()
            )),
            None => ui::warn(format!(
                "端口 {port} 被 PID {pid} 占着，但确认不了那是不是 {}（多半不是当前用户起的），没有动它。\n\
                 确认该停的话: kill -TERM {pid}",
                p.label
            )),
        }
    }
}

/// `pid` 跑的是不是 `bin` 这个文件。
///
/// `None` 表示答不上来——非 Linux 没有 `/proc`，进程不属于当前用户时读
/// `/proc/<pid>/exe` 也会被拒。「答不上来」和「确认不是」得分开：前者只能保守
/// 处理，后者可以直说。
fn same_binary(pid: i32, bin: &Path) -> Option<bool> {
    let exe = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    Some(exe == bin.canonicalize().ok()?)
}

// ---------------------------------------------------------------- 安装细节

fn ensure_dirs(spec: &DaemonSpec) -> Result<()> {
    for d in ["run", "log", "data"] {
        let p = spec.home.join(d);
        std::fs::create_dir_all(&p).with_context(|| format!("建目录失败: {}", p.display()))?;
    }
    Ok(())
}

/// 把下载来的附件铺到 `staging`。
fn unpack(downloaded: &Path, asset_name: &str, staging: &Path, src: &Github) -> Result<()> {
    match archive::Kind::of(asset_name) {
        archive::Kind::Raw => {
            std::fs::create_dir_all(staging)
                .with_context(|| format!("建目录失败: {}", staging.display()))?;
            let to = staging.join(&src.binary_name);
            std::fs::rename(downloaded, &to)
                .with_context(|| format!("放置二进制失败: {}", to.display()))?;
            archive::make_executable(&to)
        }
        archive::Kind::TarGz => {
            // 镜像站出错时常常回一个 HTML 错误页，但 HTTP 状态是 200、文件名还是
            // .tar.gz。不看魔数的话，报错会是「不是有效的 gzip 流」，离真因太远。
            if !archive::looks_gzip(downloaded) {
                bail!(
                    "{asset_name} 下下来不是 gzip（多半是镜像或代理回了错误页）。\n\
                     换个源再试: FUNDEPLOY_GITHUB_DOWNLOAD_MODE=hub_proxy"
                );
            }
            archive::extract(downloaded, asset_name, staging, src.strip)
        }
        archive::Kind::Zip => archive::extract(downloaded, asset_name, staging, src.strip),
    }
}

/// 用新解出来的目录换掉 `app/`。
///
/// 先把旧的挪开再把新的改名过去，两步都是同一文件系统上的 rename。中间那一瞬
/// `app/` 不存在，但服务此刻已经停了，没人会读到。
fn swap_in(app: &Path, staging: &Path, work: &Path) -> Result<()> {
    let old = work.join("app.old");
    if app.exists() {
        std::fs::rename(app, &old).with_context(|| format!("挪开旧版本失败: {}", app.display()))?;
    }
    if let Some(parent) = app.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("建目录失败: {}", parent.display()))?;
    }
    if let Err(e) = std::fs::rename(staging, app) {
        // 新的没铺上去，把旧的放回来——总比两头落空好。
        if old.exists() {
            let _ = std::fs::rename(&old, app);
        }
        return Err(anyhow!("启用新版本失败: {e}（已退回原来那份）"));
    }
    let _ = std::fs::remove_dir_all(&old);
    Ok(())
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

/// 把 [`Daemon`] 接到统一的 [`Service`] 上。另一族也有一个这样的壳，
/// 见 [`super::upstream::Adapter`] 处的说明。
pub struct Adapter<T>(pub T);

impl<T: Daemon> Service for Adapter<T> {
    fn name(&self) -> &str {
        &self.0.spec().name
    }
    fn summary(&self) -> &str {
        &self.0.spec().summary
    }
    fn ports(&self) -> Vec<u16> {
        self.0.spec().procs.iter().map(|p| p.port).collect()
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
}

pub fn boxed<T: Daemon + 'static>(t: T) -> Box<dyn Service> {
    Box::new(Adapter(t))
}

/// 没有任何特殊性的后台服务。
pub struct Declared(pub DaemonSpec);

impl Daemon for Declared {
    fn spec(&self) -> &DaemonSpec {
        &self.0
    }
}

pub fn declared(spec: DaemonSpec) -> Box<dyn Service> {
    boxed(Declared(spec))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(procs: Vec<Proc>) -> DaemonSpec {
        DaemonSpec::new(
            "demo",
            "演示",
            PathBuf::from("/tmp/__fd_demo"),
            Github {
                repo: "o/r".into(),
                asset: Box::new(|tag, p| format!("demo-{tag}-{}-{}", p.os.go(), p.arch.go())),
                strip: 1,
                binary_name: "demo".into(),
                scan: 10,
            },
            procs,
        )
    }

    fn one(key: &str, port: u16) -> Proc {
        Proc::new(key, key, "demo", "127.0.0.1", port, PathBuf::from("/tmp"))
    }

    #[test]
    fn 目录布局是固定的四格() {
        let s = spec(vec![one("main", 1)]);
        assert!(s.app_dir().ends_with("app"));
        assert!(s.data_dir().ends_with("data"));
        assert!(s.pid_file(&s.procs[0]).ends_with("run/main.pid"));
        assert!(s.log_file(&s.procs[0]).ends_with("log/main.log"));
    }

    #[test]
    fn 认得出端口上那个进程跑的是不是指定的二进制() {
        // stop 的端口兜底要靠它来避免停掉别人的进程。自己的 PID 一定查得出来，
        // 拿 PID 1 当反例——它一定不是测试二进制，而且就算读不到也只会得到 None。
        let me = std::process::id() as i32;
        let exe = std::env::current_exe().unwrap();
        assert_eq!(same_binary(me, &exe), Some(true));
        assert_ne!(same_binary(1, &exe), Some(true), "不能把别人的进程认成自己");
        assert_eq!(same_binary(me, Path::new("/bin/sh")), Some(false));
        // 不存在的路径 canonicalize 不了，只能答「不知道」，不能答「是」。
        assert_eq!(same_binary(me, Path::new("/nope/nothing")), None);
    }

    #[test]
    fn 重复的进程_key_在声明阶段就被挡住() {
        // key 决定 PID 文件名，重了会让两个进程抢同一个文件：
        // 停的时候只停得掉后写进去的那个，另一个变成孤儿。
        let s = spec(vec![one("main", 1), one("main", 2)]);
        assert!(s.validate().unwrap_err().to_string().contains("重复"));
        assert!(spec(vec![]).validate().is_err());
    }

    #[test]
    fn 附件名按_tag_和平台拼出来() {
        let s = spec(vec![one("main", 1)]);
        let p = Platform {
            os: crate::common::platform::Os::Linux,
            arch: crate::common::platform::Arch::Aarch64,
        };
        assert_eq!((s.source.asset)("v1.2.3", p), "demo-v1.2.3-linux-arm64");
    }

    #[test]
    fn 换目录失败时旧版本会被放回来() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("app");
        let work = dir.path().join("work");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join("old"), b"v1").unwrap();
        std::fs::create_dir_all(&work).unwrap();

        // staging 不存在 —— 第二步的 rename 必然失败。
        let err = swap_in(&app, &work.join("app"), &work).unwrap_err();
        assert!(err.to_string().contains("退回"), "实际: {err}");
        assert_eq!(std::fs::read(app.join("old")).unwrap(), b"v1");
    }

    #[test]
    fn 换目录会整个替换而不是叠加() {
        // 原地覆盖会把上个版本多出来的文件留下，表现为「升级了但老行为还在」。
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("app");
        let work = dir.path().join("work");
        let staging = work.join("app");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join("只有老版本有"), b"x").unwrap();
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("新文件"), b"y").unwrap();

        swap_in(&app, &staging, &work).unwrap();
        assert!(app.join("新文件").exists());
        assert!(!app.join("只有老版本有").exists());
    }

    #[test]
    fn 日志尾巴只取最后几行() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("a.log");
        std::fs::write(
            &log,
            (1..=50).fold(String::new(), |a, i| a + &format!("{i}\n")),
        )
        .unwrap();
        let t = tail(&log, 3);
        assert_eq!(t, "48\n49\n50");
        assert!(tail(&dir.path().join("没有"), 3).contains("读不到"));
    }

    #[test]
    fn 非本机地址会被认出来() {
        let mut p = one("main", 1);
        assert!(!p.is_public());
        p.host = "0.0.0.0".into();
        assert!(p.is_public());
    }
}
