//! 自己升级自己。
//!
//! 一条命令、两条路，分岔点是「这份二进制归谁管」（[`route`]）：
//!
//! - 自己装的（install.sh、手解 tar.gz）：fundeploy 发布成一个不依赖运行时的静态
//!   二进制，所以「升级」就是把磁盘上那个文件换掉。换的那一步在 Unix 上是安全的：
//!   `rename` 到正在执行的路径只解掉旧 inode 的目录项，已经映射进内存的那一份不受
//!   影响——当前这次执行照常跑完，新版本从下一次执行开始生效。
//! - 包管理器装的（cargo / Homebrew / apt）：不原地换，调它自己的升级命令（见本文件
//!   末尾「交回原渠道」那一节）。原地换那边是不知情的，下一次 `brew upgrade` 会把
//!   换上去的盖回去。
//!
//! 下载源是 GitHub Releases，连不上则换 Gitee 上那份镜像（见
//! [`crate::common::github::Source`]）。第三方加速前缀是另一回事，走
//! [`crate::common::download`] 里那套改写。
//!
//! 两者并存而不至于让「这次到底从哪儿下的」说不清，靠的是：换了源一定出声，
//! 而且 `FUNDEPLOY_RELEASE_SOURCE` 点了名就绝不背着人再换。

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::common::platform::{Arch, Os, Platform};
use crate::common::pm::Pm;
use crate::common::{archive, asset, cmd, download, env, fs, github, version};
use crate::ui;

/// 发布仓库。
const REPO: &str = "farfarfun/fundeploy";

/// 本机对应的 target 三元组，也就是附件名里的那一段。
///
/// Linux 侧一律是 musl。glibc 动态链接的二进制带着一条「运行机的 glibc 不能比
/// 构建机的旧」的硬约束，CI 上用新发行版构建出来的东西拿到 Debian 11 这种机器
/// 上就是 `GLIBC_2.34 not found`——而那台机器正是最需要一个自带一切的二进制的
/// 地方。静态链接的 musl 没有这条地板。
fn target(plat: Platform) -> &'static str {
    match (plat.os, plat.arch) {
        (Os::Linux, Arch::X86_64) => "x86_64-unknown-linux-musl",
        (Os::Linux, Arch::Aarch64) => "aarch64-unknown-linux-musl",
        (Os::MacOs, Arch::X86_64) => "x86_64-apple-darwin",
        (Os::MacOs, Arch::Aarch64) => "aarch64-apple-darwin",
    }
}

/// 本机该下哪个附件。
fn asset_name(plat: Platform) -> String {
    format!("fundeploy-{}.tar.gz", target(plat))
}

/// `fundeploy upgrade`。
///
/// `version` 点了名就装那一版（降级也照做，这是回退的用法）；没点名只往新了走。
pub fn run(version: Option<&str>, check: bool) -> Result<()> {
    if check {
        return check_only(version);
    }
    match route()? {
        Route::Delegate(m, exe) => delegate(m, &exe, version),
        Route::Swap(exe) => from_release(version, &exe),
    }
}

/// `--check`：只看有没有新版本，什么都不动。
fn check_only(version: Option<&str>) -> Result<()> {
    let rel = fetch(version)?;
    let newest = rel.version();

    ui::kv("当前版本", Some(crate::VERSION));
    ui::kv("远端版本", Some(newest));

    // 认得出渠道就说一声：同一条 `fundeploy upgrade` 在不同渠道下背后调的东西不同，
    // 而「它会动哪儿」是升级之前最该知道的一件事。
    let managed = current_exe().ok().as_deref().and_then(managed_by);
    if let Some(m) = managed {
        ui::kv("安装渠道", Some(m.who()));
        ui::hint(format!(
            "这种安装下 `fundeploy upgrade` 会替你跑 {}。",
            m.upgrade_cmd()
        ));
    }

    if version::is_newer(newest, crate::VERSION) {
        ui::info("有新版本，升级: fundeploy upgrade");
        if managed.is_some() {
            ui::hint(LAG);
        }
    } else {
        ui::ok("已经是最新的了。");
    }
    Ok(())
}

/// 取发布信息，顺带把「这次是从哪儿取的」说清楚。
fn fetch(version: Option<&str>) -> Result<github::Release> {
    Ok(fetch_from(version)?.0)
}

/// 同 [`fetch`]，但把源也带出来——下载那条路要靠它决定还提不提加速前缀。
fn fetch_from(version: Option<&str>) -> Result<(github::Release, github::Source)> {
    let repo = env::str_or("FUNDEPLOY_RELEASE_REPO", REPO);
    let tag = version.map(github::tag_of).transpose()?;
    let got = github::fetch_release(&repo, tag.as_deref())?;
    // 不在 GitHub 上就说一声，而且要说清是哪一种。被动回落是警告（网络有问题）；
    // 主动点名只是陈述——但也不能不说：这变量多半写在某个 shell 配置里，设完就
    // 忘，几个月后「怎么升不上新版」的答案就在这一行。
    match (got.source, got.fell_back) {
        (github::Source::GitHub, _) => {}
        (s, true) => ui::warn(format!("GitHub 连不上，改用 {} 上那份镜像。", s.label())),
        (s, false) => ui::info(format!("按 FUNDEPLOY_RELEASE_SOURCE 从 {} 取。", s.label())),
    }
    Ok((got.release, got.source))
}

/// 自己管的那种安装：下 Release 里的 tar.gz，换掉 `exe`。
fn from_release(version: Option<&str>, exe: &Path) -> Result<()> {
    let plat = Platform::current()?;
    let want = asset_name(plat);

    let (rel, source) = fetch_from(version)?;
    let newest = rel.version();

    ui::kv("当前版本", Some(crate::VERSION));
    ui::kv("远端版本", Some(newest));

    if version.is_none() && !version::is_newer(newest, crate::VERSION) {
        ui::ok("已经是最新的了。");
        return Ok(());
    }

    let Some(a) = rel.asset(&want) else {
        bail!(
            "{} 这一版里没有 {want}。\n有的是: {}",
            rel.tag_name,
            asset::names(&rel)
        );
    };

    // 已经落到镜像上了就别再提加速前缀，那条提示是给直连 GitHub 的人看的。
    if source == github::Source::GitHub {
        download::hint_mirror();
    }
    let scratch = fs::Scratch::new("selfup")?;
    let pkg = scratch.path().join(&want);
    download::to_file(
        &a.browser_download_url,
        &pkg,
        &format!("fundeploy {newest}"),
    )?;

    let sums = rel.checksums_url().and_then(|u| download::get_text(u).ok());
    download::verify(&pkg, sums.as_deref(), &want)?;
    if !archive::looks_gzip(&pkg) {
        bail!("下下来的 {want} 不是 gzip，多半是中间有个什么东西回了一张错误页。");
    }

    let unpacked = scratch.path().join("unpacked");
    archive::untar_gz(&pkg, &unpacked, 0)?;
    let fresh = archive::find_file(&unpacked, "fundeploy")
        .ok_or_else(|| anyhow::anyhow!("{want} 里没有 fundeploy 这个文件"))?;

    swap(&fresh, exe, newest)
}

/// 把 `fresh` 换到 `exe` 上。
fn swap(fresh: &Path, exe: &Path, ver: &str) -> Result<()> {
    let dir = exe.parent().unwrap_or_else(|| Path::new("."));
    // 先落到 `exe` 的同目录再 rename：rename 跨不了文件系统，而解包是在缓存目录
    // 里做的，`~/.cache` 和 `~/.local` 分属两个挂载点并不罕见。
    let staged = dir.join(format!(".fundeploy-new-{}", std::process::id()));
    std::fs::copy(fresh, &staged).with_context(|| format!("写不了 {}", staged.display()))?;
    archive::make_executable(&staged)?;

    // 换之前先让新的跑一下。换完才发现它起不来（架构下错了、附件传坏了），旧的
    // 那份已经没了，人手上就只剩一个跑不起来的 fundeploy——而修它要用的正是
    // fundeploy。
    let probe = cmd::try_run([&staged.display().to_string(), "--version"]);
    let bad = match &probe {
        Err(e) => Some(e.to_string()),
        Ok(o) if !o.success() => Some(o.stderr.trim().to_string()),
        Ok(_) => None,
    };
    if let Some(why) = bad {
        let _ = std::fs::remove_file(&staged);
        bail!("下下来的那份 fundeploy 跑不起来，已经丢掉，原来那份没动过。\n  {why}");
    }

    std::fs::rename(&staged, exe).with_context(|| format!("换不掉 {}", exe.display()))?;
    ui::ok(format!("{} 现在是 {ver}。", exe.display()));
    Ok(())
}

/// 升级这件事该谁来做。
enum Route {
    /// 自己管的安装：原地把文件换掉。
    Swap(PathBuf),
    /// 包管理器装的：调它自己的升级命令。
    Delegate(Managed, PathBuf),
}

/// 这次执行的是哪个文件。
///
/// Linux 上 current_exe 走 /proc/self/exe，本来就是解析过的；别的平台上拿到的
/// 可能还是 PATH 里那条符号链接。要认的是它指向的真文件，不是链接本身。
fn current_exe() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("问不到当前执行的是哪个文件")?;
    Ok(exe.canonicalize().unwrap_or(exe))
}

/// 看这份二进制归谁管，定下走哪条路。
fn route() -> Result<Route> {
    let exe = current_exe()?;

    if is_build_artifact(&exe) {
        bail!(
            "{} 是 cargo 的构建产物，换掉它下一次 cargo build 又会盖回来。\n\
             要试升级流程，先把二进制装到 PATH 上的位置；只想知道有没有新版本，加 --check。",
            exe.display()
        );
    }

    // 包管理器装的那份不原地换——那边是不知道的，下一次 `brew upgrade` / `apt upgrade`
    // 会把换上去的盖回去。交给它自己升。
    if let Some(m) = managed_by(&exe) {
        return Ok(Route::Delegate(m, exe));
    }

    // 先确认换得掉再去下载。顺序反过来的话，十几兆下完才说「这个目录不可写」。
    let dir = exe.parent().unwrap_or_else(|| Path::new("."));
    if !writable(dir) {
        bail!(
            "{} 不可写，换不掉 {}。\n\
             要么用 sudo，要么装一份到 ~/.local/bin 下自己管。",
            dir.display(),
            exe.display()
        );
    }
    Ok(Route::Swap(exe))
}

/// 这份二进制归谁管。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Managed {
    Cargo,
    Homebrew,
    Dpkg,
}

impl Managed {
    fn who(self) -> &'static str {
        match self {
            Managed::Cargo => "cargo",
            Managed::Homebrew => "Homebrew",
            Managed::Dpkg => "apt/dpkg",
        }
    }

    /// 这条渠道升级要跑的命令，照实说给人听。
    fn upgrade_cmd(self) -> &'static str {
        match self {
            Managed::Cargo => "cargo install fundeploy --force --locked",
            Managed::Homebrew => "git -C <tap> pull 然后 brew upgrade fundeploy",
            Managed::Dpkg => "sudo apt-get update && sudo apt-get install fundeploy",
        }
    }
}

/// 认出这份二进制是不是某个包管理器装的。
///
/// 光看目录可不可写不够。`~/.cargo/bin` 和 Apple Silicon 上的 `/opt/homebrew/bin`
/// 都是当前用户可写的——于是 rename 会成功，而包管理器那边毫不知情：
/// `cargo install --list` / `brew list --versions` 报的还是它自己装的那个版本，
/// 下一次 `cargo install` 或 `brew upgrade` 又把我们换上去的盖回去。两边来回覆盖，
/// 全程没有一句报错。所以这种安装不原地换，改调它自己的升级命令（[`delegate`]）。
fn managed_by(exe: &Path) -> Option<Managed> {
    if in_cargo_bin(exe) {
        return Some(Managed::Cargo);
    }
    if in_cellar(exe) {
        return Some(Managed::Homebrew);
    }
    if dpkg_owns(Path::new("/var/lib/dpkg/info/fundeploy.list"), exe) {
        return Some(Managed::Dpkg);
    }
    None
}

/// 在不在 `$CARGO_HOME/bin`（默认 `~/.cargo/bin`）下。
fn in_cargo_bin(exe: &Path) -> bool {
    let home = match crate::common::env::opt("CARGO_HOME") {
        Some(h) => PathBuf::from(h),
        None => crate::common::env::home().join(".cargo"),
    };
    let bin = home.join("bin");
    // 两边都 canonicalize：~/.cargo 常常是指向别处的符号链接，比字符串就漏了。
    let real = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    exe.parent().map(real) == Some(real(&bin))
}

/// 路径里有没有 `Cellar` 这一段。
///
/// brew 装出来的 `<prefix>/bin/fundeploy` 是指向 `<prefix>/Cellar/fundeploy/<版本>/bin/`
/// 的符号链接，而上面已经 canonicalize 过了，所以这儿看到的是 Cellar 里的真路径。
/// 认这一段而不是写死 `/opt/homebrew`、`/usr/local`、`/home/linuxbrew/.linuxbrew`：
/// brew 的前缀是可以自己改的，枚举前缀总会漏。
fn in_cellar(exe: &Path) -> bool {
    exe.components().any(|c| c.as_os_str() == "Cellar")
}

/// dpkg 的文件清单里有没有这个路径。
///
/// 问 `dpkg -S` 要起一个进程、还得 dpkg 在 PATH 上；这份清单就是它的数据源，
/// 直接读更快也更可靠。
fn dpkg_owns(list: &Path, exe: &Path) -> bool {
    let Ok(s) = std::fs::read_to_string(list) else {
        return false;
    };
    let real = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let exe = real(exe);
    s.lines().any(|l| real(Path::new(l.trim())) == exe)
}

/// 是不是 `target/{debug,release}/` 下的构建产物。
fn is_build_artifact(exe: &Path) -> bool {
    let named = |p: &Path, n: &str| p.file_name().is_some_and(|f| f == n);
    let Some(profile) = exe.parent() else {
        return false;
    };
    (named(profile, "debug") || named(profile, "release"))
        && profile.parent().is_some_and(|t| named(t, "target"))
}

/// 目录能不能写。
///
/// 真去建一个文件，而不是读权限位算：权限位之外还有 ACL、只读挂载、不可变属性，
/// 算出来的结论和 rename 的实际结果对不上时，错会报在最不该报的那一步上。
fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".fundeploy-write-test-{}", std::process::id()));
    let ok = std::fs::File::create(&probe).is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

// ---------------------------------------------------------------------------
// 交回原渠道
// ---------------------------------------------------------------------------

/// apt / brew / crates.io 这几条渠道的新版本不是发版那一刻就有的。
const LAG: &str = "包管理器那边比 Release 晚一会儿：apt 索引和 brew formula 要等 \
                   farfarfun/packages 重建，crates.io 要等发布流程跑到最后一步，通常都是\
                   几分钟。刚在 GitHub 上看到新版就来升的话，过一会儿再试。";

/// 交给 `m` 这条渠道去升。
fn delegate(m: Managed, exe: &Path, version: Option<&str>) -> Result<()> {
    ui::kv("安装渠道", Some(m.who()));
    ui::kv("当前版本", Some(crate::VERSION));
    // 这儿故意不查 Release。包管理器升到哪一版由它自己的索引说了算，不是 Release
    // 说了算；多查一次只是给一条本来能成的 `brew upgrade` 添一个连不上 GitHub 的
    // 失败点——而这正是国内最常见的那一种失败。
    match m {
        Managed::Homebrew => brew_upgrade(exe, version),
        Managed::Cargo => cargo_upgrade(exe, version),
        Managed::Dpkg => apt_upgrade(exe, version),
    }
}

/// Homebrew：先把 tap 刷到最新，再 `brew upgrade`。
///
/// 两步缺一不可。tap 是个普通的 git clone，不刷新的话里面的 formula 还写着旧版本，
/// `brew upgrade` 自然「无事可做」而且退出 0——看着像升过了，其实没动。
fn brew_upgrade(exe: &Path, version: Option<&str>) -> Result<()> {
    if let Some(v) = version {
        bail!("{}", no_pinning(Managed::Homebrew, v));
    }
    let brew = cmd::require("brew", "Homebrew 装的那份要靠 brew 升。")?;

    let r = receipt_of(exe);
    let fresh_tap = match tap_dir(&brew, &r) {
        Some(d) => pull_tap(&d),
        None => {
            ui::info("找不到对应的 tap 仓库，刷新交给 brew 自己做。");
            false
        }
    };

    // formula 用全名（`<tap>/fundeploy`）：别的 tap 里也可能有同名 formula，
    // 裸名字在那种机器上会让 brew 停下来问是哪一个。
    let formula = r
        .tap
        .as_deref()
        .map_or_else(|| "fundeploy".to_string(), |t| format!("{t}/fundeploy"));
    // tap 刚刷过就别让 brew 再 auto-update 一遍：那一步要去 GitHub 把所有 tap 都拉一轮，
    // 国内常常是干等几分钟，而我们要的那个 formula 已经是最新的了。formula 装的是
    // Release 里那个静态二进制、没有依赖，跳过这一步不会牵连别的包。
    let env: Vec<cmd::EnvEdit> = if fresh_tap {
        vec![("HOMEBREW_NO_AUTO_UPDATE".to_string(), Some("1".to_string()))]
    } else {
        vec![]
    };

    let argv = vec![brew.clone(), "upgrade".into(), "--formula".into(), formula];
    // 报命令时用 `brew` 这个名字，不用解析出来的绝对路径：前者是人自己也能敲的那一条。
    ui::step(format!("brew {}", argv[1..].join(" ")));
    // 失败先收着，不立刻 bail。`brew upgrade` 的退出码盖的事比我们关心的多：它还会
    // 顺带升别的 formula、去取 API 元数据，那些失败和「fundeploy 升没升上去」是两回
    // 事。实测过一次：fundeploy 自己那步早就 🍺 完、0.3.8 也清理掉了，brew 接着去下
    // 另一个 formula 的 .rb 撞上 raw.githubusercontent.com 连不通（国内常事），整条
    // 命令退出 1，于是升级成功却报了失败。
    let failed = cmd::run_with(argv, &env).err();
    brew_confirm(&brew, failed, r.tap.as_deref())
}

/// 刷新 tap 仓库。返回它现在是不是最新的。
///
/// 失败不算升级失败：brew 自己的 auto-update 也会刷 tap，让它去就是了。
/// 用 `--ff-only` 是为了碰上本地改过 formula 时明着停下来，而不是悄悄 merge 一下。
fn pull_tap(dir: &Path) -> bool {
    ui::step(format!("刷新 tap: {}", dir.display()));
    let argv = vec![
        "git".to_string(),
        "-C".into(),
        dir.display().to_string(),
        "pull".into(),
        "--ff-only".into(),
    ];
    match cmd::run(argv) {
        Ok(()) => true,
        Err(e) => {
            ui::warn(format!("tap 没刷新成，刷新改交给 brew 自己做：\n  {e}"));
            false
        }
    }
}

/// `INSTALL_RECEIPT.json` 里关于「这个 formula 从哪儿来」的线索。
#[derive(Debug, Default, PartialEq, Eq)]
struct Receipt {
    /// tap 名，形如 `farfarfun/packages`。
    tap: Option<String>,
    /// formula 那个 .rb 文件的路径。
    path: Option<PathBuf>,
}

/// 读出 `exe` 所属 keg 的 receipt。读不到、或者解不出来，就当什么线索都没有。
fn receipt_of(exe: &Path) -> Receipt {
    keg_root(exe)
        .and_then(|k| std::fs::read_to_string(k.join("INSTALL_RECEIPT.json")).ok())
        .map(|s| parse_receipt(&s))
        .unwrap_or_default()
}

/// keg 的根目录，也就是放着 `INSTALL_RECEIPT.json` 的那一层。
///
/// brew 装出来的是 `<prefix>/Cellar/fundeploy/<版本>/bin/fundeploy`，receipt 在
/// `<版本>` 那一层。往上找而不是写死「上两级」：keg 里的布局（`bin/` 还是
/// `libexec/bin/`）是 formula 自己定的。
fn keg_root(exe: &Path) -> Option<PathBuf> {
    exe.ancestors()
        .skip(1)
        .take(4)
        .find(|d| d.join("INSTALL_RECEIPT.json").is_file())
        .map(Path::to_path_buf)
}

/// 解 receipt。
///
/// `source.tap` 是 Homebrew 一直写的字段（见上游 `tab.rb`）；`source.path`（formula
/// 的 .rb 路径）新版的 `Tab.create` 里已经不写了，所以只当兜底。两个字段都可能是
/// `null`——本地 .rb 直接装的就没有 tap。
fn parse_receipt(json: &str) -> Receipt {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return Receipt::default();
    };
    Receipt {
        tap: v["source"]["tap"].as_str().map(str::to_string),
        path: v["source"]["path"].as_str().map(PathBuf::from),
    }
}

/// tap 的 git 仓库在哪儿。
fn tap_dir(brew: &str, r: &Receipt) -> Option<PathBuf> {
    // brew 自己说的最准：tap 的目录名是按 tap 名拼的（`Library/Taps/<用户>/homebrew-<仓库>`），
    // 跟 clone 时给的 URL 没关系，自己拼容易拼错。
    if let Some(t) = &r.tap {
        if let Ok(out) = cmd::capture([brew, "--repository", t]) {
            let d = PathBuf::from(out.trim());
            if is_git_repo(&d) {
                return Some(d);
            }
        }
    }
    // 退一步：从 formula 那个 .rb 往上找。
    r.path.as_deref().and_then(git_root)
}

/// 从 `start` 往上找第一个 git 仓库。
fn git_root(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .skip(1)
        .find(|d| is_git_repo(d))
        .map(Path::to_path_buf)
}

fn is_git_repo(dir: &Path) -> bool {
    dir.join(".git").exists()
}

/// brew 那边装着 `got`、当前跑的是 `current` 时，这次升级算不算落地了。
///
/// 同版也算：那说明本来就已经是最新的，`brew upgrade` 后面报的错是别的 formula
/// 的事，不该把它记在这次升级账上。
fn landed(got: &str, current: &str) -> bool {
    !version::is_newer(current, got)
}

/// brew 那边现在装着哪一版，据此下结论。
///
/// 退出码两头都不可信：`brew upgrade` 在「没有更新可升」时退出 0，而它顺带升别的
/// formula 失败时又会退出非零——两种情况下真正的答案都只有一个来源，就是
/// `brew list --versions`。所以 `failed` 只用来决定「怎么说」，不用来决定「成没成」。
fn brew_confirm(brew: &str, failed: Option<anyhow::Error>, tap: Option<&str>) -> Result<()> {
    let got = cmd::capture([brew, "list", "--versions", "fundeploy"])
        .ok()
        .and_then(|o| newest_version(&o));

    match (got, failed) {
        // 落地了。brew 后面那些抱怨是别的事，说一声就行，不能判成失败。
        (Some(v), e) if landed(&v, crate::VERSION) => {
            if let Some(e) = e {
                ui::warn(format!(
                    "brew upgrade 最后退出码非零，但 fundeploy 这边已经是 {v} 了——\
                     那条错是别的 formula 的事（常见的是取 formula 元数据时连不上 GitHub）：\n  {e}"
                ));
            }
            report("Homebrew 里", &v, "Homebrew", None);
            Ok(())
        }
        // brew 失败了，而 fundeploy 也确实没升上去：这才是真失败。
        (got, Some(e)) => {
            let now = got
                .map(|v| format!("，Homebrew 现在装着 {v}"))
                .unwrap_or_default();
            bail!(
                "{e}\n\
                 fundeploy 没升上去{now}。\n\
                 brew 要是嫌 tap 不可信（Homebrew 6.0 起第三方 tap 默认不可信）：brew trust {}",
                tap.unwrap_or("farfarfun/packages")
            );
        }
        // brew 说成功了，版本也问出来了，但比当前还旧——交给 report 说清楚。
        (Some(v), None) => {
            report("Homebrew 里", &v, "Homebrew", None);
            Ok(())
        }
        // 问不出来就不替它下结论。
        (None, None) => {
            ui::warn("brew upgrade 跑完了，但问不出 Homebrew 现在装着哪一版，自己核一下：");
            ui::hint("brew list --versions fundeploy");
            Ok(())
        }
    }
}

/// cargo：`cargo install --force`。
fn cargo_upgrade(exe: &Path, version: Option<&str>) -> Result<()> {
    let cargo = cargo_bin(exe)?;
    let argv = cargo_argv(&cargo, version);
    ui::info("cargo 装的只能从源码编，比别的渠道慢得多（好几分钟）。");
    ui::step(format!("cargo {}", argv[1..].join(" ")));
    cmd::run(argv)?;
    exe_confirm(exe, "crates.io", version)
}

/// cargo 在哪儿。
///
/// PATH 上没有不等于没装：`exe` 就在 `$CARGO_HOME/bin` 里（`in_cargo_bin` 就是这么
/// 认出来的），cargo 本身通常也在同一个目录，只是那个目录没进 PATH——这种机器不少见。
fn cargo_bin(exe: &Path) -> Result<String> {
    if let Some(p) = cmd::find("cargo") {
        return Ok(p);
    }
    let sibling = exe.with_file_name("cargo");
    if sibling.is_file() {
        return Ok(sibling.display().to_string());
    }
    bail!(
        "{} 是 cargo 装的，升级要用 cargo，可 PATH 上没有，{} 里也没有。\n\
         装一套工具链（fundeploy dev rust install），或者换成别的安装方式（见 install.sh）。",
        exe.display(),
        sibling.display()
    );
}

/// `cargo install` 的 argv。
///
/// `--locked` 用发布时那份 Cargo.lock：不临时解依赖，省掉「某个依赖发了新版于是编不过」
/// 这类跟升级本身无关的失败。
fn cargo_argv(cargo: &str, version: Option<&str>) -> Vec<String> {
    let mut v: Vec<String> = ["install", "fundeploy", "--force", "--locked"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    if let Some(ver) = version {
        v.push("--version".into());
        v.push(ver.to_string());
    }
    v.insert(0, cargo.to_string());
    v
}

/// apt：刷索引 + 装最新。
fn apt_upgrade(exe: &Path, version: Option<&str>) -> Result<()> {
    if let Some(v) = version {
        bail!("{}", no_pinning(Managed::Dpkg, v));
    }
    ui::step("apt-get update && apt-get install fundeploy");
    // 刷索引、索引刷失败只警告不中断、按需加 sudo，这些 Pm 都做过。
    Pm::Apt.install(&["fundeploy"])?;
    exe_confirm(exe, "apt 源", None)
}

/// 升完之后 `exe` 自己报的版本。
///
/// cargo / apt 装的路径升级前后不变（`~/.cargo/bin/fundeploy`、`/usr/bin/fundeploy`），
/// 所以直接问那个文件最准：它回的就是下一次执行会跑的那一版。（brew 不走这条——它
/// 的路径带着版本号，升完之后 `exe` 指的还是旧 keg。）
fn exe_confirm(exe: &Path, who: &str, pinned: Option<&str>) -> Result<()> {
    let argv = vec![exe.display().to_string(), "--version".to_string()];
    let got = cmd::capture(argv).ok().and_then(|o| newest_version(&o));
    let Some(v) = got else {
        ui::warn(format!(
            "升级命令跑完了，但问不出 {} 现在是哪一版，自己核一下：{} --version",
            exe.display(),
            exe.display()
        ));
        return Ok(());
    };
    report(&format!("{} 现在", exe.display()), &v, who, pinned);
    Ok(())
}

/// 把核实出来的版本说清楚：真升上去了，还是那边暂时就没有更新的。
///
/// `pinned` 是 `--version` 点的那一版（只有 cargo 那条路点得了）。
fn report(subject: &str, got: &str, who: &str, pinned: Option<&str>) {
    // 点了名就只对一件事：落地的是不是那一版。比当前旧是正常的——回退本来就是
    // `--version` 的用法；落地的是别的版本则要说出来，不然人会以为回退成了。
    if let Some(p) = pinned {
        if version::is_same(p, got) {
            ui::ok(format!(
                "{subject}是 {got}，下一次执行 fundeploy 就是它了。"
            ));
        } else {
            ui::warn(format!("{subject}是 {got}，不是点名的 {p}。"));
        }
        return;
    }

    if version::is_newer(got, crate::VERSION) {
        ui::ok(format!(
            "{subject}是 {got}，下一次执行 fundeploy 就是它了。"
        ));
    } else if version::is_same(got, crate::VERSION) {
        ui::ok(format!(
            "{subject}还是 {got}：{who} 那边暂时没有更新的版本。"
        ));
        ui::hint(LAG);
    } else {
        // 没点名却装出个旧的：说出来，别让人以为升过了。
        ui::warn(format!(
            "{subject}是 {got}，比当前这份（{}）还旧。",
            crate::VERSION
        ));
    }
}

/// brew / apt 点不了版本。
///
/// 这两家的源里都只放最新一版，也没有带版本号的 formula/包名，所以 `--version`
/// 在这两种安装下无从下手。拒绝得说清出路：回退是真需求，别让人卡在这儿。
fn no_pinning(m: Managed, v: &str) -> String {
    let why = match m {
        Managed::Homebrew => format!(
            "tap 里只有一个 fundeploy.rb，跟着最新版走，没有带版本号的 formula。\n\
             想试试那一版还在不在：brew install fundeploy@{v}（一般不存在）"
        ),
        _ => format!(
            "apt 源里只放最新一版。\n\
             想试试源里还留着没有：sudo apt-get install fundeploy={v}"
        ),
    };
    let url = "https://raw.githubusercontent.com/farfarfun/fundeploy/master/install.sh";
    format!(
        "{} 装的升不到点名的 {v}：{why}\n\
         要一份能自由点版本的，按 install.sh 装到 ~/.local/bin 下\
         （那种安装下 `fundeploy upgrade --version` 一直好使）：\n  \
         curl -fsSL {url} | sh -s -- --version {v}",
        m.who()
    )
}

/// 一段输出里最新的那个版本号。
///
/// 两种输入都吃得下：`brew list --versions fundeploy` 回的是
/// `fundeploy 0.3.3 0.3.4`（brew 允许多个 keg 并存，真正链到 PATH 上的是最新那个），
/// `fundeploy --version` 回的是 `fundeploy 0.3.4`。头一段是包名，跳过。
fn newest_version(out: &str) -> Option<String> {
    out.split_whitespace()
        .skip(1)
        .filter(|s| s.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .fold(None::<&str>, |best, v| match best {
            Some(b) if !version::is_newer(v, b) => Some(b),
            _ => Some(v),
        })
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLATS: [(Os, Arch); 4] = [
        (Os::Linux, Arch::X86_64),
        (Os::Linux, Arch::Aarch64),
        (Os::MacOs, Arch::X86_64),
        (Os::MacOs, Arch::Aarch64),
    ];

    #[test]
    fn brew_升上去了就算成功_哪怕_brew_自己退出非零() {
        // 实际踩过的一次：fundeploy 已经 🍺 到 0.3.9、0.3.8 也清理掉了，brew 接着去
        // 下另一个 formula 的 .rb 撞上 raw.githubusercontent.com 连不通，整条命令
        // 退出 1，于是升级成功却报了失败。判定只能看 brew 装着哪一版。
        assert!(landed("0.3.9", "0.3.8"), "装上新版就是升上去了");
        // 同版也算落地：本来就是最新的，brew 后面报的错是别的 formula 的事。
        assert!(landed("0.3.9", "0.3.9"));
        // 真没升上去：brew 那边还是旧的。
        assert!(!landed("0.3.8", "0.3.9"));
    }

    #[test]
    fn 四个平台的附件名各不相同() {
        let names: Vec<String> = PLATS
            .iter()
            .map(|&(os, arch)| asset_name(Platform { os, arch }))
            .collect();
        let mut uniq = names.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(uniq.len(), names.len(), "有重名: {names:?}");
    }

    #[test]
    fn linux_发的是_musl() {
        // 动态链接 glibc 的那份在老发行版上根本起不来，而那正是主要的落地场景。
        for arch in [Arch::X86_64, Arch::Aarch64] {
            let t = target(Platform {
                os: Os::Linux,
                arch,
            });
            assert!(t.ends_with("-musl"), "{t}");
        }
    }

    /// 装机脚本和自升级必须拼出同一个附件名。
    ///
    /// 这两边是各写各的（一边 shell 一边 Rust），名字对不上的表现很隐蔽：装得上、
    /// 升级时 404，或者反过来。这里直接拿 install.sh 的内容来对。
    #[test]
    fn 装机脚本认得全部四个附件名() {
        let script = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/install.sh"))
            .expect("读不到 install.sh");
        for &(os, arch) in &PLATS {
            let t = target(Platform { os, arch });
            assert!(script.contains(t), "install.sh 里没有 {t}");
        }
        assert!(script.contains("fundeploy-"), "附件名前缀变了");
    }

    #[test]
    fn 构建产物认得出来() {
        assert!(is_build_artifact(Path::new(
            "/w/proj/target/debug/fundeploy"
        )));
        assert!(is_build_artifact(Path::new(
            "/w/proj/target/release/fundeploy"
        )));
        // 装出来的那些都不算。
        assert!(!is_build_artifact(Path::new(
            "/home/me/.local/bin/fundeploy"
        )));
        assert!(!is_build_artifact(Path::new("/usr/bin/fundeploy")));
        // 名字里带 release 但不在 target 下的不算。
        assert!(!is_build_artifact(Path::new("/opt/release/fundeploy")));
    }

    #[test]
    fn brew_装的认得出来() {
        // Apple Silicon 的 /opt/homebrew/bin 是用户可写的，光靠可写性检查拦不住。
        assert!(in_cellar(Path::new(
            "/opt/homebrew/Cellar/fundeploy/0.3.1/bin/fundeploy"
        )));
        assert!(in_cellar(Path::new(
            "/usr/local/Cellar/fundeploy/0.3.1/bin/fundeploy"
        )));
        // 改过前缀的也认，因为认的是 Cellar 这一段而不是前缀。
        assert!(in_cellar(Path::new("/home/me/brew/Cellar/f/1/bin/f")));
        assert!(!in_cellar(Path::new("/home/me/.local/bin/fundeploy")));
        // 要整段相等，不是子串。
        assert!(!in_cellar(Path::new("/opt/CellarX/bin/fundeploy")));
    }

    #[test]
    fn cargo_装的认得出来() {
        let d = tempfile::tempdir().unwrap();
        let bin = d.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let _锁 = 排队();
        std::env::set_var("CARGO_HOME", d.path());

        assert!(in_cargo_bin(&bin.join("fundeploy")));
        assert!(!in_cargo_bin(&d.path().join("fundeploy")));
        assert!(!in_cargo_bin(Path::new("/home/me/.local/bin/fundeploy")));

        std::env::remove_var("CARGO_HOME");
    }

    #[test]
    fn dpkg_清单里的算_dpkg_装的() {
        let d = tempfile::tempdir().unwrap();
        let exe = d.path().join("fundeploy");
        std::fs::write(&exe, "x").unwrap();
        let list = d.path().join("fundeploy.list");
        std::fs::write(
            &list,
            format!("/usr/share/doc/fundeploy\n{}\n", exe.display()),
        )
        .unwrap();

        assert!(dpkg_owns(&list, &exe));
        assert!(!dpkg_owns(&list, &d.path().join("别的")));
        // 清单不存在就是没装过，不该当成 dpkg 的。
        assert!(!dpkg_owns(&d.path().join("没有这个"), &exe));
    }

    #[test]
    fn 自己装到_local_bin_的不算谁管() {
        let d = tempfile::tempdir().unwrap();
        let _锁 = 排队();
        std::env::set_var("CARGO_HOME", d.path());
        assert_eq!(managed_by(&d.path().join("fundeploy")), None);
        std::env::remove_var("CARGO_HOME");
    }

    /// 这几个测试要动 CARGO_HOME，而环境变量是进程级的，得排队。
    fn 排队() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn 不可写的目录判得出来() {
        let d = tempfile::tempdir().unwrap();
        assert!(writable(d.path()));
        // 不存在的目录也是写不了的。
        assert!(!writable(&d.path().join("没有这个目录")));
    }

    #[test]
    fn receipt_里取得到_tap_名和_formula_路径() {
        let r = parse_receipt(
            r#"{"source":{"tap":"farfarfun/packages",
                "path":"/opt/homebrew/Library/Taps/farfarfun/homebrew-packages/Formula/fundeploy.rb",
                "tap_git_head":"abc"}}"#,
        );
        assert_eq!(r.tap.as_deref(), Some("farfarfun/packages"));
        assert!(r.path.unwrap().ends_with("Formula/fundeploy.rb"));

        // 新版 brew 的 Tab.create 不写 path，只有 tap——这是最常见的那一种。
        let r = parse_receipt(r#"{"source":{"tap":"farfarfun/packages"}}"#);
        assert_eq!(r.tap.as_deref(), Some("farfarfun/packages"));
        assert_eq!(r.path, None);

        // 本地 .rb 直接装的，tap 是 null。
        assert_eq!(
            parse_receipt(r#"{"source":{"tap":null}}"#),
            Receipt::default()
        );
        // 解不出来也不该 panic，当成没线索。
        assert_eq!(parse_receipt("不是 json"), Receipt::default());
        assert_eq!(parse_receipt(""), Receipt::default());
    }

    #[test]
    fn keg_根是放_install_receipt_的那一层() {
        let d = tempfile::tempdir().unwrap();
        let keg = d.path().join("Cellar/fundeploy/0.3.3");
        let bin = keg.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let exe = bin.join("fundeploy");
        std::fs::write(&exe, "x").unwrap();

        // 没有 receipt 时不乱认一层。
        assert_eq!(keg_root(&exe), None);
        std::fs::write(keg.join("INSTALL_RECEIPT.json"), "{}").unwrap();
        assert_eq!(keg_root(&exe), Some(keg));
    }

    #[test]
    fn 从_formula_路径往上找得到_tap_仓库() {
        let d = tempfile::tempdir().unwrap();
        let tap = d.path().join("Library/Taps/farfarfun/homebrew-packages");
        let formula = tap.join("Formula/fundeploy.rb");
        std::fs::create_dir_all(formula.parent().unwrap()).unwrap();
        std::fs::write(&formula, "class Fundeploy").unwrap();

        // 还不是 git 仓库的时候不认——不是仓库就 pull 不了。
        assert_eq!(git_root(&formula), None);
        std::fs::create_dir_all(tap.join(".git")).unwrap();
        assert_eq!(git_root(&formula), Some(tap));
    }

    #[test]
    fn 版本号从_brew_和_version_的输出里都读得出来() {
        // brew 允许多个 keg 并存，链到 PATH 上的是最新那个。
        assert_eq!(
            newest_version("fundeploy 0.3.3 0.3.10 0.3.4").as_deref(),
            Some("0.3.10")
        );
        assert_eq!(newest_version("fundeploy 0.3.4").as_deref(), Some("0.3.4"));
        // clap 的 --version 输出。
        assert_eq!(
            newest_version("fundeploy 0.3.4\n").as_deref(),
            Some("0.3.4")
        );
        // 没装、或者命令回了句别的，都不能瞎认一个版本出来。
        assert_eq!(newest_version(""), None);
        assert_eq!(newest_version("fundeploy"), None);
        assert_eq!(newest_version("Error: No such keg"), None);
    }

    #[test]
    fn cargo_的_argv_点不点版本都对() {
        let plain = cargo_argv("/home/me/.cargo/bin/cargo", None).join(" ");
        assert_eq!(
            plain,
            "/home/me/.cargo/bin/cargo install fundeploy --force --locked"
        );
        // --force 不能漏：少了它 cargo 看见已经装过就什么都不做，还退出 0。
        assert!(plain.contains("--force"));
        let pinned = cargo_argv("cargo", Some("0.3.2")).join(" ");
        assert_eq!(
            pinned,
            "cargo install fundeploy --force --locked --version 0.3.2"
        );
    }

    #[test]
    fn brew_和_apt_下点版本要明着拒绝并给出路() {
        for m in [Managed::Homebrew, Managed::Dpkg] {
            let msg = no_pinning(m, "0.2.0");
            assert!(msg.contains("0.2.0"), "{msg}");
            assert!(msg.contains(m.who()), "{msg}");
            // 拒绝了就得给一条走得通的路，否则回退这件事就卡死在这儿。
            assert!(msg.contains("install.sh"), "{msg}");
        }
        assert!(no_pinning(Managed::Dpkg, "0.2.0").contains("fundeploy=0.2.0"));
    }

    /// 造一个「二进制」。`body` 是 shell 脚本内容，`exec` 决定加不加执行位。
    fn fake(dir: &Path, name: &str, body: &str, exec: bool) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        if exec {
            archive::make_executable(&p).unwrap();
        }
        p
    }

    /// 换完之后，目录里不该剩下中转文件。
    fn 中转文件(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(".fundeploy-new-"))
            .collect()
    }

    #[test]
    fn 换掉自己之后留下的是新的那份() {
        let d = tempfile::tempdir().unwrap();
        let exe = fake(d.path(), "fundeploy", "#!/bin/sh\necho 旧\n", true);
        let src = tempfile::tempdir().unwrap();
        let fresh = fake(src.path(), "fundeploy", "#!/bin/sh\necho 新\n", true);

        swap(&fresh, &exe, "9.9.9").unwrap();
        assert!(std::fs::read_to_string(&exe).unwrap().contains('新'));
        assert!(中转文件(d.path()).is_empty());
        // 执行位要跟过来，否则换完之后 fundeploy 这个命令就没了。
        assert!(cmd::try_run([&exe.display().to_string(), "--version"])
            .unwrap()
            .success());
    }

    #[test]
    fn 新的跑不起来就不换() {
        // 这条守的是最坏的结果：换完才发现新的起不来，而修它要用的正是 fundeploy。
        let d = tempfile::tempdir().unwrap();
        let exe = fake(d.path(), "fundeploy", "#!/bin/sh\necho 旧\n", true);
        let src = tempfile::tempdir().unwrap();

        for body in [
            "#!/bin/sh\nexit 1\n",                 // 能跑但报错
            "<!DOCTYPE html><title>502</title>\n", // 镜像回了张错误页
        ] {
            let fresh = fake(src.path(), "fundeploy", body, true);
            let err = swap(&fresh, &exe, "9.9.9").unwrap_err().to_string();
            assert!(err.contains("没动过"), "实际: {err}");
            assert!(std::fs::read_to_string(&exe).unwrap().contains('旧'));
            // 试坏了的那份要收走，不然 ~/.local/bin 里会越积越多。
            assert!(中转文件(d.path()).is_empty());
        }
    }
}
