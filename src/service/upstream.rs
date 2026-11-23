//! 上游自带守护的那一族。
//!
//! 这些服务的 CLI 自己有 `server start/stop/status`，PID 文件、后台化、日志都归
//! 它管。fundeploy 在这里只做四件事：用对的包管理器装包、按依赖顺序调上游命令、
//! 等端口真的起来、停完之后核对端口确实放开了。
//!
//! 生命周期写在 [`Upstream`] 的默认方法里，服务自己只给一份 [`ServiceSpec`]；
//! 有个性的地方（例如启动前要生成配置文件）覆盖对应的钩子。

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};

use super::{
    force_free_port, uninstall_env_var, Extra, Service, ENV_KILL_PORT_YES, FORCE_GRACE,
    START_TIMEOUT,
};
use crate::common::{cmd, npm::Npm, par, pip, pip::PipPkg, port, proc};
use crate::service::spec::{Component, Pkg, ServiceSpec};
use crate::ui;

// ---------------------------------------------------------------- 包管理

/// 一个组件的包管理操作。
enum Installer {
    Pip(PipPkg),
    Npm {
        npm: Npm,
        pkg: String,
        installed: Option<String>,
        shadow_warning: Option<String>,
    },
}

fn installer(c: &Component) -> Result<Installer> {
    match &c.pkg {
        Pkg::Pip { name, cli } => Ok(Installer::Pip(PipPkg::new(name, cli))),
        // 一次并发探测选工具、取已装版本并查双份遮挡，免得同一批 Node 进程起三四遍。
        Pkg::Npm {
            name,
            registry_scope,
            ..
        } => {
            let lookup = Npm::lookup_package(name, None)?;
            Ok(Installer::Npm {
                npm: lookup.tool.with_registry_scope(registry_scope.as_deref()),
                pkg: name.clone(),
                installed: lookup.installed,
                shadow_warning: lookup.shadow_warning,
            })
        }
    }
}

impl Installer {
    fn install(&self, version: Option<&str>) -> Result<()> {
        match self {
            Installer::Pip(p) => p.install(version),
            Installer::Npm { npm, pkg, .. } => npm.install_global(&npm.spec(pkg, version)),
        }
    }

    fn uninstall(&self) -> bool {
        match self {
            Installer::Pip(p) => p.uninstall(),
            Installer::Npm { npm, pkg, .. } => npm.uninstall_global(pkg),
        }
    }

    /// 装完后重新查询最新版本（此时不能用装前缓存的 `installed`）。
    fn fresh_version(&self) -> Option<String> {
        match self {
            Installer::Pip(p) => p.managed_version(),
            Installer::Npm { npm, pkg, .. } => npm.installed_version(pkg),
        }
    }

    /// 装的那份是不是就是 PATH 上会跑到的那份。
    fn shadow_warning(&self) -> Option<String> {
        match self {
            Installer::Pip(p) => p.shadow_warning(),
            Installer::Npm { shadow_warning, .. } => shadow_warning.clone(),
        }
    }

    /// 一次探出状态卡片要展示的（版本、安装来源），避免重复起子进程。
    fn status_probe(&self) -> (Option<String>, String) {
        match self {
            Installer::Pip(p) => {
                let (ver, mode) = p.probe_status();
                let src = match mode {
                    pip::Mode::UvTool => "uv tool".into(),
                    pip::Mode::Pip => "pip".into(),
                };
                (ver, src)
            }
            Installer::Npm { npm, installed, .. } => (installed.clone(), npm.name()),
        }
    }
}

/// 已安装版本。查不出来（没装、或者 npm 本身不可用）都是 None。
fn installed_version(c: &Component) -> Option<String> {
    match installer(c).ok()? {
        Installer::Pip(p) => p.managed_version(),
        Installer::Npm { installed, .. } => installed,
    }
}

/// CLI 在 PATH 上吗。这是「能不能启动」的判据——包装没装进哪个环境无所谓，
/// 关键是敲下去那个名字能不能跑起来。
fn cli_available(c: &Component) -> bool {
    cmd::has(c.pkg.cli())
}

// ---------------------------------------------------------------- 共用动作

fn require_cli_available(spec: &ServiceSpec) -> Result<()> {
    let missing: Vec<String> = by_package(spec)
        .into_iter()
        .filter(|c| !cli_available(c))
        .map(|c| format!("{}（来自 {}）", c.pkg.cli(), c.pkg.name()))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    bail!(
        "这些命令不在 PATH 上: {}\n先执行 `fundeploy service {} install`",
        missing.join("、"),
        spec.name
    )
}

fn any_listening(spec: &ServiceSpec) -> bool {
    spec.components
        .iter()
        .any(|c| port::is_listening(&c.host, c.port))
}

/// 启动一个组件并等到它真的在监听。
fn start_component(spec: &ServiceSpec, c: &Component) -> Result<()> {
    if c.cli.start.is_empty() {
        bail!("{} 没有声明 start 的调用方式", c.label);
    }
    if c.is_public() {
        ui::warn(format!(
            "{} 监听 {}，本机之外也能访问；确认这是你要的。",
            c.label, c.host
        ));
    }
    ui::step(format!("启动 {}，监听 {}:{}", c.label, c.host, c.port));

    let argv = c.render(&c.cli.start, &spec.components)?;
    let argv = with_program(c, argv);
    match cmd::spawn_status(&argv, &c.env)? {
        Some(0) => {}
        other => bail!(
            "{} 启动失败（退出码 {}）",
            c.label,
            other.map_or("信号终止".to_string(), |x| x.to_string())
        ),
    }

    let sp = ui::Spinner::new(format!("等 {} 监听 {}:{}", c.label, c.host, c.port));
    let listening = port::wait_for_listener_ticking(&c.host, c.port, START_TIMEOUT, || sp.tick());
    if listening {
        sp.done(format!("{} 起来了，{}:{} 在听", c.label, c.host, c.port));
    } else {
        sp.give_up();
    }
    if !listening {
        bail!(never_listened(spec, c));
    }
    Ok(())
}

/// 上游组件停止参数。供 [`Upstream`] 与 [`super::funmill::Funmill`] 的单组件停止共用。
pub(super) struct UpstreamStop<'a> {
    pub label: &'a str,
    pub host: &'a str,
    pub port: u16,
    pub stop_argv: &'a [String],
    pub status_argv: Option<&'a [String]>,
    pub env: &'a [(String, String)],
    pub match_tokens: &'a [String],
}

struct LingeringProc {
    pid: i32,
    snap: proc::StopTarget,
    owned: bool,
}

fn format_pid(pid: i32, snap: &proc::StopTarget) -> String {
    let name = snap.name();
    if name.is_empty() {
        format!("PID {pid}")
    } else {
        format!("PID {pid}（{name}）")
    }
}

/// 从上游 CLI 输出里提取明确标为 `pid` 的进程号（保留出现顺序并去重）。
///
/// 兼容常见写法：`（pid 3123645）`、`pid=1234`、`PID: 1234`、`PID：1234`、`pid 1234`。
/// 要求 `pid` 前面不是 ASCII 字母/数字/下划线，且后面必须紧跟分隔符，避免把端口号、
/// 超时秒数或 `rapid 10`、`pid2` 误当成 PID。
pub(super) fn extract_pids(text: &str) -> Vec<i32> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let is_pid = bytes[i..i + 3].eq_ignore_ascii_case(b"pid");
        let boundary_before =
            i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
        if is_pid && boundary_before {
            let rest = &text[i + 3..];
            let mut chars = rest.chars().peekable();
            let mut saw_sep = false;
            let mut saw_delim = false;
            while let Some(&ch) = chars.peek() {
                if ch.is_ascii_whitespace() {
                    saw_sep = true;
                    chars.next();
                } else if !saw_delim && matches!(ch, '=' | ':' | '：') {
                    saw_sep = true;
                    saw_delim = true;
                    chars.next();
                } else {
                    break;
                }
            }
            if saw_sep {
                let mut digits = String::new();
                while let Some(&ch) = chars.peek() {
                    if ch.is_ascii_digit() {
                        digits.push(ch);
                        chars.next();
                    } else {
                        break;
                    }
                }
                let boundary_after = chars
                    .peek()
                    .is_none_or(|ch| !(ch.is_ascii_alphabetic() || *ch == '_'));
                if boundary_after {
                    if let Ok(pid) = digits.parse::<i32>() {
                        if pid > 0 && !out.contains(&pid) {
                            out.push(pid);
                        }
                    }
                }
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    out
}

fn component_match_tokens(c: &Component) -> Vec<String> {
    let mut out = Vec::new();
    let mut push = |raw: &str| {
        let base = raw.rsplit('/').next().unwrap_or(raw).to_ascii_lowercase();
        if base.len() < 3 {
            return;
        }
        if !out.contains(&base) {
            out.push(base.clone());
        }
        if base.contains('-') {
            let under = base.replace('-', "_");
            if !out.contains(&under) {
                out.push(under);
            }
        }
    };
    push(c.pkg.cli());
    push(c.pkg.name());
    out
}

fn wait_released_listeners(
    pre_listeners: &[(i32, proc::StopTarget)],
    port: u16,
    timeout: Duration,
) {
    let deadline = Instant::now() + timeout;
    loop {
        let cur: BTreeSet<i32> = port::holders(port)
            .into_iter()
            .filter(|h| h.protos.contains(&"tcp"))
            .map(|h| h.pid)
            .collect();
        let any_draining = pre_listeners.iter().any(|(pid, snap)| {
            !cur.contains(pid) && proc::is_alive(*pid) && snap.recheck(*pid).is_ok()
        });
        if !any_draining || Instant::now() >= deadline {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn stop_lingering_procs(label: &str, port: u16, lingering: &[LingeringProc]) -> Result<()> {
    let all_desc: Vec<String> = lingering
        .iter()
        .map(|p| format_pid(p.pid, &p.snap))
        .collect();
    if port::is_taken(port) {
        ui::warn(format!("{label} 的进程 {} 仍在运行。", all_desc.join("、")));
    } else {
        ui::warn(format!(
            "{label} 的端口 {port} 已释放，但进程 {} 仍在运行。",
            all_desc.join("、")
        ));
    }

    let mut targets = Vec::with_capacity(lingering.len());
    let mut owned_desc = Vec::new();
    for p in lingering {
        if let Some(reason) = proc::protected(p.pid) {
            bail!(
                "{label} 的残留进程 {} {reason}，已中止",
                format_pid(p.pid, &p.snap)
            );
        }
        let consent = if p.owned {
            owned_desc.push(format_pid(p.pid, &p.snap));
            None
        } else {
            Some(proc::StopTarget::confirm(
                p.pid,
                &format!("{label} 上游报告的残留进程"),
            )?)
        };
        targets.push((p, consent));
    }

    if !owned_desc.is_empty()
        && !ui::confirm_default_yes(
            &format!("强制停止 {label} 的残留进程 {} 吗", owned_desc.join("、")),
            ENV_KILL_PORT_YES,
        )
    {
        bail!("已取消，{label} 的残留进程仍在运行");
    }

    for (p, consent) in targets {
        if !proc::is_alive(p.pid) {
            continue;
        }
        match consent {
            Some(target) => target.recheck(p.pid)?,
            None => p.snap.recheck(p.pid)?,
        }
        ui::step(format!(
            "强制停止 {label} 残留进程 {}",
            format_pid(p.pid, &p.snap)
        ));
        proc::stop_pid(p.pid, FORCE_GRACE, p.owned)?;
        if proc::is_alive(p.pid) {
            bail!(
                "{label} 的残留进程 {} 仍在运行，停止失败",
                format_pid(p.pid, &p.snap)
            );
        }
    }
    Ok(())
}

/// 停一个上游组件并收尾：既兜住「仍占着端口」，也兜住「已释放端口但进程卡在退出里」。
///
/// Web 服务收到 SIGTERM 后常常第一时间关掉监听 socket、但进程本身还在排空连接或
/// 卡在清理钩子里；上游 `stop` 只等一轮 SIGTERM 超时就报错返回，留下活着的 PID 和
/// PID 文件。只看端口是否释放会把这种半死不活的进程当成「已停止」，紧接着的 `start`
/// 立刻撞上「已在运行（pid ...）」。
pub(super) fn stop_upstream_component(req: &UpstreamStop<'_>) -> Result<()> {
    let pre_listeners: Vec<(i32, proc::StopTarget)> = port::holders(req.port)
        .into_iter()
        .filter(|h| h.protos.contains(&"tcp"))
        .filter_map(|h| proc::StopTarget::snapshot(h.pid).map(|snap| (h.pid, snap)))
        .collect();

    let out = cmd::spawn_tee(req.stop_argv, req.env);
    let stop_ok = out.as_ref().is_ok_and(cmd::Output::success);

    let mut reported_pids = Vec::new();
    let mut status_says_running = false;
    if !stop_ok {
        if let Ok(o) = &out {
            for pid in extract_pids(&o.stdout)
                .into_iter()
                .chain(extract_pids(&o.stderr))
            {
                if !reported_pids.contains(&pid) {
                    reported_pids.push(pid);
                }
            }
        }
        let has_alive_reported = reported_pids.iter().any(|&pid| proc::is_alive(pid));
        if !has_alive_reported {
            if let Some(status_argv) = req.status_argv {
                let env_edits: Vec<cmd::EnvEdit> = req
                    .env
                    .iter()
                    .map(|(k, v)| (k.clone(), Some(v.clone())))
                    .collect();
                if let Ok(Some(st)) =
                    cmd::try_run_timeout_with(status_argv, &env_edits, Duration::from_secs(5))
                {
                    if st.success() {
                        status_says_running = true;
                        for pid in extract_pids(&st.stdout)
                            .into_iter()
                            .chain(extract_pids(&st.stderr))
                        {
                            if !reported_pids.contains(&pid) {
                                reported_pids.push(pid);
                            }
                        }
                    }
                }
            }
        }
    }

    if stop_ok && !pre_listeners.is_empty() {
        wait_released_listeners(&pre_listeners, req.port, Duration::from_secs(2));
    }

    let cur_listener_pids: BTreeSet<i32> = port::holders(req.port)
        .into_iter()
        .filter(|h| h.protos.contains(&"tcp"))
        .map(|h| h.pid)
        .collect();

    let mut lingering: Vec<LingeringProc> = Vec::new();
    for (pid, snap) in &pre_listeners {
        if cur_listener_pids.contains(pid) || !proc::is_alive(*pid) || snap.recheck(*pid).is_err() {
            continue;
        }
        lingering.push(LingeringProc {
            pid: *pid,
            snap: snap.clone(),
            owned: true,
        });
    }
    for &pid in &reported_pids {
        if cur_listener_pids.contains(&pid)
            || lingering.iter().any(|p| p.pid == pid)
            || !proc::is_alive(pid)
        {
            continue;
        }
        let Some(snap) = proc::StopTarget::snapshot(pid) else {
            continue;
        };
        let owned = proc::process_matches_tokens(pid, req.match_tokens);
        lingering.push(LingeringProc { pid, snap, owned });
    }

    if !lingering.is_empty() {
        stop_lingering_procs(req.label, req.port, &lingering)?;
    } else if !stop_ok {
        if port::is_taken(req.port) {
            ui::warn(format!(
                "{} 的停止命令失败，端口 {} 仍被占用，继续按端口检查。",
                req.label, req.port
            ));
        } else if status_says_running {
            bail!(
                "{} 的停止命令失败，且上游 status 仍报告在运行（未解析到残留 PID）",
                req.label
            );
        } else {
            ui::info(format!(
                "{} 的停止命令报错，但端口 {} 已释放。",
                req.label, req.port
            ));
        }
    }

    force_free_port(req.label, req.host, req.port, |pid| {
        let same_pre = pre_listeners
            .iter()
            .find(|(p, _)| *p == pid)
            .is_some_and(|(_, snap)| snap.recheck(pid).is_ok());
        (same_pre && reported_pids.contains(&pid))
            || proc::process_matches_tokens(pid, req.match_tokens)
    })
}

/// 停一个组件。未安装、没在运行都不算错；但残留进程或占着的端口没清掉必须报错。
fn stop_component(spec: &ServiceSpec, c: &Component) -> Result<()> {
    if !cli_available(c) {
        ui::info(format!("{} 未安装，跳过。", c.label));
        return force_free_port(&c.label, &c.host, c.port, |_| false);
    }
    if c.cli.stop.is_empty() {
        ui::warn(format!("{} 没有声明 stop 的调用方式，跳过。", c.label));
        return force_free_port(&c.label, &c.host, c.port, |_| false);
    }
    ui::step(format!("停止 {}", c.label));

    let Ok(argv) = c.render(&c.cli.stop, &spec.components) else {
        ui::warn(format!("{} 的 stop 参数渲染失败，跳过。", c.label));
        return force_free_port(&c.label, &c.host, c.port, |_| false);
    };
    let stop_argv = with_program(c, argv);
    let status_argv = if c.cli.status.is_empty() {
        None
    } else {
        c.render(&c.cli.status, &spec.components)
            .ok()
            .map(|a| with_program(c, a))
    };
    let tokens = component_match_tokens(c);
    stop_upstream_component(&UpstreamStop {
        label: &c.label,
        host: &c.host,
        port: c.port,
        stop_argv: &stop_argv,
        status_argv: status_argv.as_deref(),
        env: &c.env,
        match_tokens: &tokens,
    })
}

/// 把程序名拼到渲染好的参数前面。
fn with_program(c: &Component, mut argv: Vec<String>) -> Vec<String> {
    argv.insert(0, c.pkg.cli().to_string());
    argv
}

fn never_listened(spec: &ServiceSpec, c: &Component) -> String {
    let mut msg = format!(
        "{} 的启动命令已经返回，但端口 {} 迟迟没被监听（多半是起来之后崩了）。",
        c.label, c.port
    );
    if !c.cli.run.is_empty() {
        if let Ok(argv) = c.render(&c.cli.run, &spec.components) {
            msg.push_str(&format!(
                "\n跑这条看前台报错: {} {}",
                c.pkg.cli(),
                argv.join(" ")
            ));
        }
    }
    msg
}

/// 按包名去重后的组件，顺序不变。
///
/// 一个包提供多个要分别起停的进程是常事（funmill 的 API 和编排后端都来自
/// funmill-api）。进程是两个，包只有一个：装、卸、报版本都只该对包做一次，
/// 否则同一个包会被装两遍，卸载时第二遍还会报一句「卸载失败」。
fn by_package(spec: &ServiceSpec) -> Vec<&Component> {
    let mut seen: Vec<&str> = Vec::new();
    let mut out = Vec::new();
    for c in &spec.components {
        if seen.contains(&c.pkg.name()) {
            continue;
        }
        seen.push(c.pkg.name());
        out.push(c);
    }
    out
}

fn versions(spec: &ServiceSpec) -> Vec<Option<String>> {
    let pkgs = by_package(spec);
    par::map(&pkgs, par::JOBS, |c| installed_version(c))
}

/// 组件在版本清单里的抬头。
///
/// label 里多半已经带了包名（`后端 funlesson-api` 对 `funlesson-api`），再打一遍
/// 只是噪音；但带 scope 的包名跟 label 对不上（`前端 funread-web` 对
/// `@farfarfun/funread-web`），那就得写出来，否则看不出装的是哪个包。
fn display_name(c: &Component) -> String {
    let pkg = c.pkg.name();
    if c.label.contains(pkg) {
        c.label.clone()
    } else {
        format!("{} {pkg}", c.label)
    }
}

/// 版本清单里的抬头。
///
/// 一个包带多个组件时，拿其中任意一个组件的 label 当抬头是错的：
/// 「编排后端 dagu funmill-api: 1.0.21」里的 dagu 跟这个版本号毫无关系。
/// 这种情况只打包名。
fn package_label(spec: &ServiceSpec, c: &Component) -> String {
    let shared = spec
        .components
        .iter()
        .filter(|o| o.pkg.name() == c.pkg.name())
        .count()
        > 1;
    if shared {
        c.pkg.name().to_string()
    } else {
        display_name(c)
    }
}

fn report_entry(spec: &ServiceSpec) {
    ui::ok("已启动。");
    if let Some(url) = spec.entry_url() {
        ui::hint(format!("入口: {url}"));
    }
    for n in &spec.notes {
        ui::hint(n);
    }
}

// ---------------------------------------------------------------- Upstream

pub trait Upstream {
    fn spec(&self) -> &ServiceSpec;

    // ------------------------------------------------------------ 钩子

    /// 启动前要就位的东西（配置文件、数据目录）。默认什么都不做。
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

    /// 服务声明的额外动作。绝大多数上游服务没有。
    fn extras(&self) -> &'static [Extra] {
        &[]
    }

    /// 执行一个额外动作。
    fn extra(&self, name: &str, args: &[String]) -> Result<()> {
        let _ = args;
        bail!("{} 声明了动作 {name} 却没有实现它", self.spec().name)
    }

    // ------------------------------------------------------------ 生命周期

    fn install(&self, version: Option<&str>) -> Result<()> {
        let _ = self.install_and_report(version)?;
        Ok(())
    }

    fn install_and_report(&self, version: Option<&str>) -> Result<Vec<Option<String>>> {
        let spec = self.spec();
        spec.validate()?;

        if spec
            .components
            .iter()
            .any(|c| matches!(c.pkg, Pkg::Pip { .. }))
        {
            pip::require_any_installer()?;
        }

        let pkgs = by_package(spec);
        let mut installers = Vec::with_capacity(pkgs.len());
        for c in &pkgs {
            let want = version.or(c.version.as_deref());
            let inst = installer(c)?;
            let what = package_label(spec, c);
            ui::step(match want {
                Some(v) => format!("安装 {what} {v}"),
                None => format!("安装 {what}（最新版）"),
            });
            inst.install(want)
                .with_context(|| format!("{} {} 安装失败", c.label, c.pkg.name()))?;
            if let Some(w) = inst.shadow_warning() {
                ui::warn(w);
            }
            installers.push(inst);
        }

        self.post_install()?;

        // 直接复用刚才挑出来的 installer 查装后版本，免得为 npm 包再跑一遍双工具探针。
        let after: Vec<Option<String>> = par::map(&installers, par::JOBS, Installer::fresh_version);
        ui::blank();
        ui::heading("已安装版本");
        for (c, ver) in pkgs.iter().zip(&after) {
            ui::kv(package_label(spec, c), ver.as_deref());
        }
        Ok(after)
    }

    fn upgrade(&self) -> Result<()> {
        let spec = self.spec();
        spec.validate()?;

        let before = versions(spec);
        let was_running = any_listening(spec);

        if was_running {
            ui::step("升级前服务在运行，先停服");
            self.stop()?;
            ui::blank();
        }

        // 停了服又没换成包，服务此刻是停的。不说清楚的话，操作者看到的是一条
        // 安装报错，会以为「失败了所以什么都没变」，而实际上服务已经下线了。
        let after = self.install_and_report(None).map_err(|e| {
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
        for (i, c) in by_package(spec).into_iter().enumerate() {
            ui::info(format!(
                "{}: {} -> {}",
                package_label(spec, c),
                before[i].as_deref().unwrap_or("未安装"),
                after[i].as_deref().unwrap_or("未知"),
            ));
        }

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
        spec.validate()?;

        let version = version.trim().trim_start_matches(['v', 'V']);
        if version.is_empty() {
            bail!(
                "rollback 要给版本号，例如: fundeploy service {} rollback 1.1.109",
                spec.name
            );
        }

        // 先确认源上真有这个版本再动手。装是逐个组件装的，第二个组件装不上而第一个
        // 已经降级，会停在组件之间版本不一致的中间态上——那比没降级更难收拾。
        for c in by_package(spec) {
            if let Installer::Npm { npm, pkg: name, .. } = installer(c)? {
                if npm.latest_version(&format!("{name}@{version}")).is_none() {
                    bail!("npm 源上没有 {name}@{version}，已中止（没有改动任何安装）。",);
                }
            }
        }

        let was_running = any_listening(spec);
        if was_running {
            ui::step("回退前服务在运行，先停服");
            self.stop()?;
            ui::blank();
        }

        self.install(Some(version)).map_err(|e| {
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
                "已回退到 {version}（回退前服务没在运行，没有自动启动）。"
            ));
            Ok(())
        }
    }

    fn start(&self) -> Result<()> {
        let spec = self.spec();
        spec.validate()?;
        require_cli_available(spec)?;
        self.ensure_config()?;

        for (i, c) in spec.components.iter().enumerate() {
            // 上游的 start 在目标已经在跑时返回非零码。先看端口，已经在跑就跳过，
            // 不要当成启动失败。
            if port::is_listening(&c.host, c.port) {
                ui::info(format!(
                    "{} {}:{} 端口已监听，跳过启动。",
                    c.label, c.host, c.port
                ));
                continue;
            }
            start_component(spec, c).with_context(|| {
                let rest: Vec<&str> = spec.components[i + 1..]
                    .iter()
                    .map(|x| x.label.as_str())
                    .collect();
                if rest.is_empty() {
                    format!("启动 {} 失败", spec.name)
                } else {
                    format!(
                        "启动 {} 失败，已中止（{} 没有启动；要退回干净状态执行 `fundeploy service {} stop`）",
                        c.label,
                        rest.join("、"),
                        spec.name
                    )
                }
            })?;
        }

        report_entry(spec);
        Ok(())
    }

    /// 前台跑。最后一个组件占住终端，前面的组件后台起；前台退出时把后台那些一起停掉。
    ///
    /// 不用 `exec` 顶替自己，就是为了留着这个收尾——前台退出后留一个孤儿后端在那儿，
    /// 比压根没启动更糟：下次 start 会因为端口被占而跳过它。
    fn run(&self) -> Result<()> {
        let spec = self.spec();
        spec.validate()?;
        require_cli_available(spec)?;

        // run 起的是全新一套。接管已有进程的话，前台一退就会把别人起的服务停掉。
        for c in &spec.components {
            if port::is_listening(&c.host, c.port) {
                bail!(
                    "{} 的端口 {} 已被占用；run 不接管已有进程。\n\
                     要前台跑，先 `fundeploy service {} stop`。",
                    c.label,
                    c.port,
                    spec.name
                );
            }
        }
        self.ensure_config()?;

        let (last, rest) = spec
            .components
            .split_last()
            .ok_or_else(|| anyhow!("服务 {} 没有组件", spec.name))?;
        if last.cli.run.is_empty() {
            bail!("{} 没有声明前台运行的调用方式", last.label);
        }

        let mut started: Vec<&Component> = Vec::new();
        let mut result = Ok(());

        for c in rest {
            if let Err(e) = start_component(spec, c) {
                result = Err(e);
                break;
            }
            started.push(c);
        }

        if result.is_ok() {
            ui::step(format!(
                "前台启动 {}（Ctrl+C 退出，退出时自动停掉后台组件）",
                last.label
            ));
            let argv = match last.render(&last.cli.run, &spec.components) {
                Ok(a) => with_program(last, a),
                Err(e) => {
                    for c in started.iter().rev() {
                        if let Err(stop_err) = stop_component(spec, c) {
                            ui::warn(format!("{}: {stop_err}", c.label));
                        }
                    }
                    return Err(e);
                }
            };

            // 前台子进程和自己在同一个进程组里，Ctrl+C 两边都收到。自己要是按默认
            // 行为当场死掉，就没机会回来停上面那些后台组件了。
            let guard = proc::SignalGuard::install();
            let code = cmd::spawn_status(&argv, &last.env);
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
            for c in started.iter().rev() {
                if let Err(stop_err) = stop_component(spec, c) {
                    ui::warn(format!("{}: {stop_err}", c.label));
                }
            }
        }
        result
    }

    fn stop(&self) -> Result<()> {
        let spec = self.spec();
        // 逆序停：先停依赖别人的那个，它才不会在被依赖方消失后报一堆连接错误。
        let mut result = Ok(());
        for c in spec.components.iter().rev() {
            if let Err(e) = stop_component(spec, c) {
                ui::warn(format!("{}: {e}", c.label));
                result = result.and(Err(e));
            }
        }
        result?;
        ui::info("已停止（best effort）。");
        Ok(())
    }

    fn restart(&self) -> Result<()> {
        self.stop()?;
        ui::blank();
        self.start()
    }

    /// 一张卡片装下版本和各组件状态。
    ///
    /// 有一处卡片装不下：上游 CLI 自己提供了 `status` 时，那段输出的格式完全由
    /// 它说了算。这种组件在卡片里只占一行「见下方」，真正的输出紧跟在卡片后面
    /// 原样转出去——替上游的输出套个框只会把它的排版挤坏。
    fn status(&self) -> Result<()> {
        let spec = self.spec();
        let mut card = ui::Card::new(&spec.name).section("包");

        let pkgs = by_package(spec);
        let probed: Vec<Result<(Option<String>, String)>> = par::map(&pkgs, par::JOBS, |c| {
            let inst = installer(c)?;
            Ok(inst.status_probe())
        });
        for (c, res) in pkgs.iter().zip(probed) {
            let name = package_label(spec, c);
            card = match res {
                Ok((Some(v), src)) => card.row(&name, format!("{v} · {src}")),
                Ok((None, _)) => card.row_state(&name, ui::State::Idle, "未安装"),
                Err(e) => card.row_state(&name, ui::State::Down, format!("查不到（{e}）")),
            };
        }

        // 分节：包名和组件名长得几乎一样（都是「后端 funfluid-api」这种），
        // 不分节看起来就是同一组东西重复了两遍。
        card = card.section("运行");
        let avail: Vec<bool> = spec.components.iter().map(cli_available).collect();
        let need_ports: Vec<u16> = spec
            .components
            .iter()
            .zip(&avail)
            .filter(|(c, &ok)| ok && c.cli.status.is_empty())
            .map(|(c, _)| c.port)
            .collect();
        let holders_map = port::holders_many(&need_ports);

        let mut up = 0usize;
        let mut delegated: Vec<&Component> = Vec::new();
        for (c, &ok) in spec.components.iter().zip(&avail) {
            let at = format!("{}:{}", c.host, c.port);
            if !ok {
                card = card.row_state(&c.label, ui::State::Idle, format!("未安装 · {at}"));
                continue;
            }
            if !c.cli.status.is_empty() {
                delegated.push(c);
                card = card.row(&c.label, format!("{at} · 见下方上游输出"));
                continue;
            }
            // 上游没给 status，就报端口实况——这也正是别人关心的那件事。
            let listener = holders_map
                .get(&c.port)
                .and_then(|hs| hs.iter().find(|h| h.protos.contains(&"tcp")))
                .map(|h| h.pid);
            card = match listener {
                Some(pid) => {
                    up += 1;
                    card.row_state(
                        &c.label,
                        ui::State::Up,
                        format!("端口已监听 · PID {pid} · {at}"),
                    )
                }
                None if port::is_listening(&c.host, c.port) => {
                    up += 1;
                    card.row_state(&c.label, ui::State::Up, format!("端口已监听 · {at}"))
                }
                None => card.row_state(&c.label, ui::State::Idle, format!("没在运行 · {at}")),
            };
        }

        // 一个组件都没装上时才提示怎么装；装了一半的情况留给上面那几行自己说。
        if avail.iter().all(|&ok| !ok) {
            card = card.note(format!("装上: fundeploy service {} install", spec.name));
        }
        // 交给上游的那几个不算进分子分母：它们的状态这会儿还不知道。
        let judged = spec.components.len() - delegated.len();
        let (bst, blabel) = match (up, judged) {
            (_, 0) => (ui::State::Idle, "见下方".to_string()),
            (0, _) => (ui::State::Idle, "未监听".to_string()),
            (u, n) if u == n => (ui::State::Up, "端口已监听".to_string()),
            (u, n) => (ui::State::Partial, format!("部分 {u}/{n}")),
        };
        card.badge(bst, blabel).print();

        for c in delegated {
            ui::blank();
            ui::heading(format!("{}（上游 status）", c.label));
            let argv = with_program(c, c.render(&c.cli.status, &spec.components)?);
            // 上游用非零退出码表达「没在运行」，那不是故障。
            let _ = cmd::spawn_status(&argv, &c.env);
        }

        self.extra_status()
    }

    fn uninstall(&self) -> Result<()> {
        let spec = self.spec();
        let list: Vec<&str> = by_package(spec).iter().map(|c| c.pkg.name()).collect();

        ui::confirm_destructive(
            &format!("将停止并卸载 {}，确认？", list.join("、")),
            &uninstall_env_var(&spec.name),
        )?;

        // 先停再卸，绝不把包从活着的进程底下抽掉。
        self.stop()?;
        ui::blank();

        for c in by_package(spec).into_iter().rev() {
            ui::step(format!("卸载 {}", c.pkg.name()));
            match installer(c) {
                Ok(i) if i.uninstall() => {}
                Ok(_) => ui::warn(format!("卸载 {} 失败", c.pkg.name())),
                Err(e) => ui::warn(format!("卸载 {} 跳过: {e}", c.pkg.name())),
            }
        }

        ui::ok("已卸载。各 CLI 自己的配置和日志没有动。");
        Ok(())
    }
}

/// 没有任何特殊性的服务：行为全部由声明决定。
pub struct Declared(pub ServiceSpec);

impl Upstream for Declared {
    fn spec(&self) -> &ServiceSpec {
        &self.0
    }
}

/// 把 [`Upstream`] 接到统一的 [`Service`] 上。
///
/// 两族各写一个 `impl<T: 族 trait> Service for T` 会撞 coherence（编译器无法排除
/// 将来有类型同时属于两族），所以各自套一层壳。
pub struct Adapter<T>(pub T);

impl<T: Upstream> Service for Adapter<T> {
    fn name(&self) -> &str {
        &self.0.spec().name
    }
    fn summary(&self) -> &str {
        &self.0.spec().summary
    }
    fn ports(&self) -> Vec<u16> {
        self.0.spec().components.iter().map(|c| c.port).collect()
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

pub fn boxed<T: Upstream + 'static>(t: T) -> Box<dyn Service> {
    Box::new(Adapter(t))
}

/// 纯声明式服务的快捷入口。
pub fn declared(spec: ServiceSpec) -> Box<dyn Service> {
    boxed(Declared(spec))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args;
    use crate::service::spec::{Arg, CliShape};

    #[test]
    fn 抬头不重复打包名但带_scope_的要打出来() {
        let unscoped = Component::new(
            "backend",
            "后端 demo-api",
            Pkg::pip("demo-api", "demo-api"),
            "127.0.0.1",
            1,
        );
        assert_eq!(display_name(&unscoped), "后端 demo-api");

        let scoped = Component::new(
            "frontend",
            "前端 demo-web",
            Pkg::npm("@demo/demo-web", "demo-web"),
            "127.0.0.1",
            2,
        );
        assert_eq!(display_name(&scoped), "前端 demo-web @demo/demo-web");
    }

    #[test]
    fn 共用一个包时版本抬头只打包名() {
        // 「编排后端 dagu funmill-api: 1.0.21」里的 dagu 跟这个版本号没关系，
        // 拿任何一个组件的 label 当抬头都在误导人。
        let p = || Pkg::pip("shared-pkg", "sh");
        let shared = ServiceSpec::new(
            "s",
            "",
            vec![
                Component::new("a", "编排后端 dagu", p(), "127.0.0.1", 1),
                Component::new("b", "API", p(), "127.0.0.1", 2),
            ],
        );
        assert_eq!(package_label(&shared, &shared.components[0]), "shared-pkg");

        // 独占一个包的还是照常带 label。
        let solo = ServiceSpec::new(
            "s",
            "",
            vec![Component::new(
                "a",
                "前端 demo-web",
                Pkg::npm("@demo/demo-web", "demo-web"),
                "127.0.0.1",
                1,
            )],
        );
        assert_eq!(
            package_label(&solo, &solo.components[0]),
            "前端 demo-web @demo/demo-web"
        );
    }

    #[test]
    fn 同一个包提供的多个组件在包操作时只算一次() {
        // 进程是两个，包只有一个。不去重的话同一个包会被装两遍，
        // 卸载时第二遍还会多报一句「卸载失败」。
        let p = || Pkg::pip("one-pkg", "one");
        let spec = ServiceSpec::new(
            "s",
            "",
            vec![
                Component::new("a", "A", p(), "127.0.0.1", 1),
                Component::new("b", "B", p(), "127.0.0.1", 2),
                Component::new("c", "C", Pkg::npm("other", "other"), "127.0.0.1", 3),
            ],
        );
        let keys: Vec<&str> = by_package(&spec).iter().map(|c| c.key.as_str()).collect();
        // 保留先声明的那个，顺序不变。
        assert_eq!(keys, ["a", "c"]);
    }

    fn two_component_spec() -> ServiceSpec {
        let backend = Component::new(
            "backend",
            "后端",
            Pkg::pip("demo-api", "demo-api"),
            "127.0.0.1",
            18811,
        )
        .cli_shape(
            CliShape::new()
                .start(args![
                    "server",
                    "start",
                    "--host",
                    Arg::Host,
                    "--port",
                    Arg::Port
                ])
                .run(args!["server", "run", "--port", Arg::Port])
                .stop(args!["server", "stop", "--port", Arg::Port])
                .status(args!["server", "status", "--port", Arg::Port]),
        );
        let frontend = Component::new(
            "frontend",
            "前端",
            Pkg::npm("@demo/web", "demo-web"),
            "127.0.0.1",
            8811,
        )
        .cli_shape(CliShape::new().start(args![
            "server",
            "start",
            "--port",
            Arg::Port,
            "--backend",
            Arg::SiblingUrl("backend".into())
        ]));

        ServiceSpec::new("demo", "演示", vec![backend, frontend]).entry("frontend")
    }

    #[test]
    fn 渲染出的_argv_第一项是命令名() {
        let spec = two_component_spec();
        let c = spec.component("backend").unwrap();
        let argv = with_program(c, c.render(&c.cli.start, &spec.components).unwrap());
        assert_eq!(
            argv,
            [
                "demo-api",
                "server",
                "start",
                "--host",
                "127.0.0.1",
                "--port",
                "18811"
            ]
        );
    }

    #[test]
    fn 前端的启动参数里带上后端地址() {
        let spec = two_component_spec();
        let c = spec.component("frontend").unwrap();
        let argv = with_program(c, c.render(&c.cli.start, &spec.components).unwrap());
        assert_eq!(
            argv,
            [
                "demo-web",
                "server",
                "start",
                "--port",
                "8811",
                "--backend",
                "http://127.0.0.1:18811"
            ]
        );
    }

    #[test]
    fn 没装的_cli_会被挡在启动之前() {
        // 这条守的是报错时机：CLI 不在 PATH 上就该在启动前一次性报齐，
        // 而不是让第一个组件起好、第二个才发现没装。
        let spec = two_component_spec();
        let err = require_cli_available(&spec).unwrap_err().to_string();
        assert!(err.contains("demo-api"), "实际: {err}");
        assert!(err.contains("demo-web"), "实际: {err}");
        assert!(err.contains("install"), "实际: {err}");
    }

    #[test]
    fn 端口没监听起来的报错里带前台复跑命令() {
        let spec = two_component_spec();
        let c = spec.component("backend").unwrap();
        let msg = never_listened(&spec, c);
        assert!(msg.contains("18811"), "实际: {msg}");
        assert!(
            msg.contains("demo-api server run --port 18811"),
            "没给出前台复跑的命令: {msg}"
        );
    }

    #[test]
    fn 从上游输出里提取_pid_且不误抓端口号和无关词() {
        assert_eq!(
            extract_pids("funflix-api 在 10s 内未退出（pid 3123645）"),
            [3123645]
        );
        assert_eq!(extract_pids("api is running: pid=4200\n"), [4200]);
        assert_eq!(extract_pids("running, PID: 8899\nPID：9900"), [8899, 9900]);
        assert_eq!(extract_pids("pid 1234 and pid=1234"), [1234]);
        assert!(extract_pids("未在运行").is_empty());
        assert!(extract_pids("rapid 10s 端口 18810 pid2 99 pid 10ms").is_empty());
    }

    #[test]
    fn 组件标识词包含命令名包名及下划线形式() {
        let c = Component::new(
            "frontend",
            "前端 funflix-web",
            Pkg::npm("@farfarfun/funflix-web", "funflix-web"),
            "127.0.0.1",
            8810,
        );
        assert_eq!(component_match_tokens(&c), ["funflix-web", "funflix_web"]);
    }

    #[test]
    fn 没有_run_形态的组件报错里就不编一条命令出来() {
        let spec = two_component_spec();
        let c = spec.component("frontend").unwrap();
        let msg = never_listened(&spec, c);
        assert!(msg.contains("8811"), "实际: {msg}");
        assert!(!msg.contains("跑这条"), "编了一条上游不支持的命令: {msg}");
    }
}
