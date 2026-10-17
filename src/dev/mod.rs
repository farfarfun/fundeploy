//! 开发工具链：Go、Node.js、pnpm、Rust、uv、Python 环境。
//!
//! 除 Python 外，每样东西都有两条装法，由 `--method` 选：
//!
//! - **`pkg`**（默认）：交给系统包管理器。版本跟着发行版走，可能偏旧，但升级和
//!   卸载都归包管理器管。
//! - **`source`**：官方预编译包或官方安装脚本，装进 `~/opt` 下各自的目录。不要
//!   root，版本也新，代价是 PATH 得自己配。
//!
//! 两条路可以同时存在于一台机器上，这是「升级完版本号没变」的头号原因——所以
//! `status` 一定把两边都列出来，并指出 PATH 实际用的是哪一个。
//!
//! Python 不在这个模子里，它装的不是一个工具而是一批按版本号分开的虚拟环境，
//! 见 [`python`]。

pub mod go;
pub mod nodejs;
pub mod pnpm;
pub mod python;
pub mod registry;
pub mod rust;
pub mod uv;

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use clap::ValueEnum;

use crate::common::pm::Pm;
use crate::common::{cmd, download, env, fs, par};
use crate::ui;

/// 装法。
#[derive(ValueEnum, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Method {
    /// 系统包管理器（apt / dnf / brew …）
    Pkg,
    /// 官方预编译包或官方安装脚本，装进 ~/opt
    Source,
}

impl Method {
    fn label(self) -> &'static str {
        match self {
            Method::Pkg => "包管理器",
            Method::Source => "官方包",
        }
    }
}

/// 一样开发工具。
///
/// `Sync` 是为了 [`status_all`] 能并发探：所有实现都是无状态的单元结构体，
/// 这个约束不要求任何人多做什么。
pub trait Dev: Sync {
    fn name(&self) -> &str;
    fn summary(&self) -> &str;

    /// 这样东西在某家包管理器下的包名。空表示那家装不了它。
    fn pkgs(&self, pm: Pm) -> &'static [&'static str];

    /// source 方式的安装根目录。
    fn root(&self) -> PathBuf;

    /// 用官方包装。`version` 来自 `--version`，不给就装上游的最新版。
    fn install_source(&self, version: Option<&str>) -> Result<()>;

    /// 升级 source 装的那份。默认就是重装一遍最新版。
    fn upgrade_source(&self) -> Result<()> {
        self.install_source(None)
    }

    /// 卸掉 source 装的那份。
    fn uninstall_source(&self) -> Result<()> {
        wipe(&self.root())
    }

    /// 主程序叫什么、拿什么参数问它版本。
    fn probe(&self) -> (&'static str, &'static [&'static str]);

    /// source 装完之后主程序在哪个目录里。
    ///
    /// 默认 `<root>/bin`。上游安装器自己决定布局的（uv、pnpm）覆盖它，去实际
    /// 找一遍——猜错了提示出来的 PATH 就是错的，而人照着配完发现命令还是找不到。
    fn bin_dir(&self) -> Option<PathBuf> {
        let d = self.root().join("bin");
        d.is_dir().then_some(d)
    }
}

/// 删 source 安装目录。
pub fn wipe(root: &Path) -> Result<()> {
    if !root.exists() {
        ui::info(format!("目录不存在，跳过: {}", root.display()));
        return Ok(());
    }
    fs::guard_removable(root)?;
    std::fs::remove_dir_all(root).map_err(|e| anyhow::anyhow!("删不掉 {}: {e}", root.display()))?;
    ui::ok(format!("已删除 {}", root.display()));
    Ok(())
}

/// 定下这次用哪条路。
///
/// 优先级：`--method` > `FUNDEPLOY_INSTALL_METHOD` > 默认 `pkg`。选了 pkg 但这台
/// 机器上压根没有包管理器时退回 source，并说一声——直接报错也行，但那等于让人
/// 多打一遍命令去得到同一个结论。
fn resolve(explicit: Option<Method>) -> Method {
    let m = explicit.or_else(|| {
        let raw = env::opt("FUNDEPLOY_INSTALL_METHOD")?;
        match raw.trim().to_ascii_lowercase().as_str() {
            "source" | "src" | "official" | "tarball" => Some(Method::Source),
            "pkg" | "package" | "system" => Some(Method::Pkg),
            other => {
                ui::warn(format!(
                    "FUNDEPLOY_INSTALL_METHOD={other} 不认识，当 pkg 处理。"
                ));
                Some(Method::Pkg)
            }
        }
    });
    let m = m.unwrap_or(Method::Pkg);
    if m == Method::Pkg && Pm::detect().is_none() {
        ui::warn("这台机器上没探测到包管理器，改用官方包装到 ~/opt。");
        return Method::Source;
    }
    m
}

/// pkg 方式要用的包管理器和包名。
fn pkg_plan(d: &dyn Dev) -> Result<(Pm, &'static [&'static str])> {
    let pm =
        Pm::detect().ok_or_else(|| anyhow::anyhow!("没探测到包管理器，改用 --method source。"))?;
    let pkgs = d.pkgs(pm);
    if pkgs.is_empty() {
        bail!(
            "{} 在 {} 下没有现成的包，改用 --method source。",
            d.name(),
            pm.name()
        );
    }
    Ok((pm, pkgs))
}

pub fn install(d: &dyn Dev, method: Option<Method>, version: Option<&str>) -> Result<()> {
    let m = resolve(method);
    ui::heading(format!("安装 {}（{}）", d.name(), m.label()));
    match m {
        Method::Pkg => {
            if let Some(v) = version {
                // 装了别的版本而不吭声，比「这条路钉不了版本」难查得多。
                bail!("包管理器装的版本跟着发行版走，钉不到 {v}。要钉版本就用 --method source。");
            }
            let (pm, pkgs) = pkg_plan(d)?;
            ui::step(format!("{} install {}", pm.name(), pkgs.join(" ")));
            pm.install(pkgs)?;
            ui::ok(format!("{} 装好了。", d.name()));
            Ok(())
        }
        Method::Source => {
            d.install_source(version)?;
            report_path(d);
            Ok(())
        }
    }
}

pub fn upgrade(d: &dyn Dev, method: Option<Method>) -> Result<()> {
    let m = resolve(method);
    ui::heading(format!("升级 {}（{}）", d.name(), m.label()));
    match m {
        // 包管理器那边「升级」就是再装一遍：装过了它自己会升，没装过就装上。
        Method::Pkg => {
            let (pm, pkgs) = pkg_plan(d)?;
            pm.install(pkgs)?;
            ui::ok(format!("{} 已是 {} 源里的最新版。", d.name(), pm.name()));
            Ok(())
        }
        Method::Source => {
            d.upgrade_source()?;
            report_path(d);
            Ok(())
        }
    }
}

/// 卸载。
///
/// 不点名装法时两边都清：人装的时候可能走的是 pkg，也可能是 source，甚至两次都
/// 装过。只清一边的结果是 `uninstall` 跑完命令还在，而且看不出为什么。
pub fn uninstall(d: &dyn Dev, method: Option<Method>, yes: bool) -> Result<()> {
    let both = method.is_none();
    ui::heading(format!(
        "卸载 {}（{}）",
        d.name(),
        method.map(Method::label).unwrap_or("包管理器 + 官方包")
    ));
    if !yes {
        ui::confirm_destructive(
            &format!("要卸掉 {} 吗", d.name()),
            &format!(
                "FUNDEPLOY_{}_UNINSTALL_YES",
                d.name().to_ascii_uppercase().replace(['-', '.'], "_")
            ),
        )?;
    }

    let mut hit = false;
    if both || method == Some(Method::Pkg) {
        match pkg_plan(d) {
            Ok((pm, pkgs)) => {
                ui::step(format!("{} remove {}", pm.name(), pkgs.join(" ")));
                // 本来就不是这么装的很正常，失败只说一声。
                match pm.uninstall(pkgs) {
                    Ok(()) => hit = true,
                    Err(e) => ui::info(format!(
                        "{} 那边没卸掉（多半本就不是它装的）：{e}",
                        pm.name()
                    )),
                }
            }
            Err(e) if !both => return Err(e),
            Err(_) => {}
        }
    }
    if both || method == Some(Method::Source) {
        // 看 bin_dir 而不只是看 root 在不在：有的工具装完不落在自己的 root 下
        // （pnpm 走 corepack 时落在 Node 的 bin 里），只看目录会把刚卸掉的东西
        // 报成「本来就没装」。
        if d.bin_dir().is_some() || d.root().exists() {
            hit = true;
        }
        d.uninstall_source()?;
    }
    if !hit {
        ui::info(format!("{} 本来就没装。", d.name()));
    }
    Ok(())
}

/// 一样工具探出来的状态。探和打分开，是为了能并发探。
///
/// 每样工具要起一到两个子进程问版本，而有些上游问一次版本很贵——`pnpm --version`
/// 在装了 corepack 的机器上是 corepack → pnpm-native 三层嵌套。五样串行探实测
/// 一百五十多毫秒，而它们彼此毫无关系。
struct Probed {
    name: String,
    bin: &'static str,
    root: PathBuf,
    source_bin: Option<PathBuf>,
    source_ver: Option<String>,
    on_path: Option<String>,
    path_ver: Option<String>,
}

/// 探一样工具。只问，不打印。
fn probe_status(d: &dyn Dev) -> Probed {
    let (bin, args) = d.probe();
    let source_bin = d.bin_dir().map(|b| b.join(bin)).filter(|p| p.is_file());
    let source_ver = source_bin.as_ref().map(|p| ask_version(p, args));
    let on_path = cmd::find(bin);
    let path_ver = on_path.as_ref().map(|p| ask_version(Path::new(p), args));
    Probed {
        name: d.name().to_string(),
        bin,
        root: d.root(),
        source_bin,
        source_ver,
        on_path,
        path_ver,
    }
}

/// 打印一样工具的状态。只打印，不问。
fn print_status(p: &Probed) {
    ui::heading(&p.name);
    ui::kv(
        format!("官方包   {}", p.root.display()),
        Some(p.source_ver.as_deref().unwrap_or("未安装")),
    );
    match &p.on_path {
        Some(path) => ui::kv(format!("PATH     {path}"), p.path_ver.as_deref()),
        None => ui::kv(format!("PATH     {}", p.bin), Some("找不到")),
    }

    // 两边都装了、版本还对不上，就是「升级完版本号没变」的那个经典场面：
    // 升的是 ~/opt 里这份，用的是 PATH 上另一份。
    //
    // 只在版本真的不一样时才说。两份同版本是装机时很常见的局面（包管理器一份、
    // 我们一份），此时提醒没有任何可操作的内容，次次都报反而把真出问题时的那次
    // 淹掉了。
    if let (Some(s), Some(sv), Some(pv)) = (&p.source_bin, &p.source_ver, &p.path_ver) {
        if sv != pv {
            ui::warn(format!(
                "PATH 上的 {} 是 {pv}，{} 里是 {sv}。要用后者，把 {} 排到 PATH 前面。",
                p.bin,
                p.root.display(),
                s.parent().unwrap_or(&p.root).display()
            ));
        }
    }
}

pub fn status(d: &dyn Dev) -> Result<()> {
    print_status(&probe_status(d));
    Ok(())
}

/// 全部工具链的状态：并发探，按给进来的顺序打。
///
/// 顺序必须稳：这张表每次打出来的样子应该一样，否则人没法一眼扫过去对比，
/// 把输出贴进 issue 做 diff 也会全是噪音。
pub fn status_all(all: &[Box<dyn Dev>]) -> Result<()> {
    for p in par::map(all, par::JOBS, |d| probe_status(d.as_ref())) {
        print_status(&p);
        ui::blank();
    }
    Ok(())
}

fn ask_version(bin: &Path, args: &[&str]) -> String {
    let mut argv: Vec<String> = vec![bin.to_string_lossy().into_owned()];
    argv.extend(args.iter().map(|s| s.to_string()));
    match cmd::try_run(&argv) {
        Ok(o) if o.success() => {
            let out = if o.stdout.trim().is_empty() {
                o.stderr
            } else {
                o.stdout
            };
            out.lines().next().unwrap_or("").trim().to_string()
        }
        _ => "问不出版本".to_string(),
    }
}

/// 装完告诉人 PATH 怎么配。
fn report_path(d: &dyn Dev) {
    let Some(dir) = d.bin_dir() else { return };
    let (bin, _) = d.probe();
    // 已经能从 PATH 上找到同一个文件就别再念一遍。
    if let Some(found) = cmd::find(bin) {
        if Path::new(&found).canonicalize().ok() == dir.join(bin).canonicalize().ok() {
            ui::ok(format!("{bin} 已在 PATH 上: {found}"));
            return;
        }
    }
    ui::hint(format!(
        "把这行加进 shell 配置（~/.bashrc 或 ~/.zshrc）:\n  export PATH=\"{}:$PATH\"",
        dir.display()
    ));
}

/// 下载一个上游的安装脚本并执行。
///
/// 先下到文件再跑，不用 `curl … | sh`：管道执行是边下边跑的，连接在中途断掉
/// 时脚本已经执行了前半截——而「跑了一半的安装脚本」留下的是一堆半成品，既不算
/// 装上也不算没装，下一次重跑还可能踩到它。
fn run_installer(url: &str, label: &str, args: &[&str], env: &[cmd::EnvEdit]) -> Result<()> {
    let scratch = fs::Scratch::new(label)?;
    let script = scratch.path().join("install.sh");
    download::to_file(url, &script, &format!("{label} 安装脚本"))?;
    let mut argv: Vec<String> = vec!["sh".into(), script.to_string_lossy().into_owned()];
    argv.extend(args.iter().map(|s| s.to_string()));
    ui::step(format!("执行 {label} 官方安装脚本"));
    cmd::run_with(&argv, env)
}

fn set(k: &str, v: impl Into<String>) -> cmd::EnvEdit {
    (k.to_string(), Some(v.into()))
}

/// `~/opt/<名字>`，可用环境变量覆盖。
fn install_root(env_key: &str, leaf: &str) -> PathBuf {
    match env::opt(env_key) {
        Some(d) => env::expand_tilde(&d),
        None => env::home().join("opt").join(leaf),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 这些测试会改进程级的环境变量，必须排队。
    fn 排队() -> std::sync::MutexGuard<'static, ()> {
        static M: std::sync::Mutex<()> = std::sync::Mutex::new(());
        M.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn 命令行上的装法压过环境变量() {
        let _g = 排队();
        std::env::set_var("FUNDEPLOY_INSTALL_METHOD", "source");
        assert_eq!(resolve(Some(Method::Pkg)), Method::Pkg);
        assert_eq!(resolve(None), Method::Source);
        std::env::remove_var("FUNDEPLOY_INSTALL_METHOD");
    }

    #[test]
    fn 环境变量的几种写法都认() {
        let _g = 排队();
        for v in ["source", "SRC", " official ", "tarball"] {
            std::env::set_var("FUNDEPLOY_INSTALL_METHOD", v);
            assert_eq!(resolve(None), Method::Source, "{v}");
        }
        std::env::remove_var("FUNDEPLOY_INSTALL_METHOD");
    }

    #[test]
    fn 认不出来的装法当_pkg_处理而不是报错() {
        let _g = 排队();
        std::env::set_var("FUNDEPLOY_INSTALL_METHOD", "rpm");
        // 这台机器上有没有包管理器不一定，所以只断言它没当成 source。
        let got = resolve(None);
        let expect = if Pm::detect().is_some() {
            Method::Pkg
        } else {
            Method::Source
        };
        assert_eq!(got, expect);
        std::env::remove_var("FUNDEPLOY_INSTALL_METHOD");
    }

    #[test]
    fn 安装根目录能被环境变量改掉() {
        let _g = 排队();
        std::env::remove_var("TESTROOT");
        assert_eq!(install_root("TESTROOT", "go"), env::home().join("opt/go"));
        std::env::set_var("TESTROOT", "~/elsewhere");
        assert_eq!(
            install_root("TESTROOT", "go"),
            env::home().join("elsewhere")
        );
        std::env::remove_var("TESTROOT");
    }

    #[test]
    fn 不肯删安装目录之外的东西() {
        // 环境变量空掉时拼出来的路径可能是 / 或者家目录本身。
        assert!(wipe(Path::new("/")).is_err());
        assert!(wipe(&env::home()).is_err());
        assert!(wipe(Path::new("/usr/local")).is_err());
    }

    #[test]
    fn 每样工具至少在一家包管理器下装得了() {
        for d in registry::all() {
            let any = Pm::ALL.iter().any(|pm| !d.pkgs(*pm).is_empty());
            assert!(any, "{} 哪家都装不了", d.name());
        }
    }

    #[test]
    fn 工具名不跟别的域撞车() {
        // 三个域各有一张表，重名的话 `fundeploy dev x` 和 `fundeploy tool x`
        // 指的是两样东西，而人多半以为是同一样。
        let tools = crate::tool::registry::names();
        let services = crate::service::registry::names();
        for d in registry::all() {
            let n = d.name().to_string();
            assert!(!tools.contains(&n), "{n} 既是 dev 又是 tool");
            assert!(!services.contains(&n), "{n} 既是 dev 又是 service");
        }
    }

    #[test]
    fn 注册表里没有重名() {
        let names = registry::names();
        let mut s = names.clone();
        s.sort();
        s.dedup();
        assert_eq!(s.len(), names.len(), "有重名: {names:?}");
    }

    #[test]
    fn 按名字找得到() {
        for n in registry::names() {
            assert!(registry::find(&n).is_some(), "找不到 {n}");
        }
        assert!(registry::find("__不存在__").is_none());
    }
}
