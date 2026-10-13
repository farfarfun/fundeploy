//! 自己升级自己。
//!
//! fundeploy 发布成一个不依赖运行时的静态二进制，所以「升级」就是把磁盘上那个
//! 文件换掉。换的那一步在 Unix 上是安全的：`rename` 到正在执行的路径只解掉旧
//! inode 的目录项，已经映射进内存的那一份不受影响——当前这次执行照常跑完，
//! 新版本从下一次执行开始生效。
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
    let plat = Platform::current()?;
    let want = asset_name(plat);
    let repo = env::str_or("FUNDEPLOY_RELEASE_REPO", REPO);

    // 先确认换得掉再去下载。顺序反过来的话，十几兆下完才说「这个目录不可写」。
    let exe = (!check).then(replaceable).transpose()?;

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
    let (rel, source) = (got.release, got.source);
    let newest = rel.version();

    ui::kv("当前版本", Some(crate::VERSION));
    ui::kv("远端版本", Some(newest));

    let Some(exe) = exe else {
        if version::is_newer(newest, crate::VERSION) {
            ui::info("有新版本，升级: fundeploy upgrade");
        } else {
            ui::ok("已经是最新的了。");
        }
        return Ok(());
    };

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

    swap(&fresh, &exe, newest)
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

/// 这次执行的是哪个文件，顺带确认它换得掉。
fn replaceable() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("问不到当前执行的是哪个文件")?;
    // Linux 上 current_exe 走 /proc/self/exe，本来就是解析过的；别的平台上拿到的
    // 可能还是 PATH 里那条符号链接。要换的是它指向的真文件，不是链接本身。
    let exe = exe.canonicalize().unwrap_or(exe);

    if is_build_artifact(&exe) {
        bail!(
            "{} 是 cargo 的构建产物，换掉它下一次 cargo build 又会盖回来。\n\
             要试升级流程，先把二进制装到 PATH 上的位置；只想知道有没有新版本，加 --check。",
            exe.display()
        );
    }

    if let Some(m) = managed_by(&exe) {
        bail!(
            "{} 是{}装的，这儿不接手。\n\
             原地换掉文件那边是不知道的：它记着的还是自己装的那个版本，\
             下一次 `{}` 会把我们换上去的盖回去。\n\
             照原渠道升：{}\n\
             想要一份自己说了算的，装到 ~/.local/bin 下（见 install.sh）。",
            exe.display(),
            m.who(),
            m.clobbers(),
            m.upgrade_cmd(),
        );
    }

    let dir = exe.parent().unwrap_or_else(|| Path::new("."));
    if !writable(dir) {
        bail!(
            "{} 不可写，换不掉 {}。\n\
             要么用 sudo，要么装一份到 ~/.local/bin 下自己管。",
            dir.display(),
            exe.display()
        );
    }
    Ok(exe)
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

    /// 那边下一次会把我们换上去的东西盖回去的命令。
    fn clobbers(self) -> &'static str {
        match self {
            Managed::Cargo => "cargo install fundeploy",
            Managed::Homebrew => "brew upgrade",
            Managed::Dpkg => "apt upgrade",
        }
    }

    fn upgrade_cmd(self) -> &'static str {
        match self {
            Managed::Cargo => "cargo install fundeploy --force",
            Managed::Homebrew => "brew upgrade fundeploy",
            Managed::Dpkg => "sudo apt update && sudo apt install --only-upgrade fundeploy",
        }
    }
}

/// 认出这份二进制是不是某个包管理器装的。
///
/// 光看目录可不可写不够。`~/.cargo/bin` 和 Apple Silicon 上的 `/opt/homebrew/bin`
/// 都是当前用户可写的——于是 rename 会成功，而包管理器那边毫不知情：
/// `cargo install --list` / `brew list --versions` 报的还是它自己装的那个版本，
/// 下一次 `cargo install` 或 `brew upgrade` 又把我们换上去的盖回去。两边来回覆盖，
/// 全程没有一句报错。这种情况下宁可不升，把该敲的命令告诉人。
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
