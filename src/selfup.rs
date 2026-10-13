//! 自己升级自己。
//!
//! fundeploy 发布成一个不依赖运行时的静态二进制，所以「升级」就是把磁盘上那个
//! 文件换掉。换的那一步在 Unix 上是安全的：`rename` 到正在执行的路径只解掉旧
//! inode 的目录项，已经映射进内存的那一份不受影响——当前这次执行照常跑完，
//! 新版本从下一次执行开始生效。
//!
//! 下载源固定是 GitHub Releases。网络不通时走 [`crate::common::download`] 里那套
//! 镜像改写，而不是在这儿另开一个「换个仓库」的开关：镜像的事那边已经做了，
//! 两套并存只会让「这次到底从哪儿下的」说不清。

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

    let rel = match version {
        Some(v) => github::by_tag(&repo, &github::tag_of(v)?)?,
        None => github::latest(&repo)?,
    };
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

    download::hint_mirror();
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

    let dir = exe.parent().unwrap_or_else(|| Path::new("."));
    if !writable(dir) {
        bail!(
            "{} 不可写，换不掉 {}。\n\
             用系统包管理器装的就照原渠道升（brew upgrade fundeploy、\
             sudo apt install --only-upgrade fundeploy）；\n\
             想要一份自己说了算的，装到 ~/.local/bin 下。",
            dir.display(),
            exe.display()
        );
    }
    Ok(exe)
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
