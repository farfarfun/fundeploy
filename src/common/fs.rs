//! 文件系统上那几件需要先想一下再做的事。

use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::env;

/// 还原 macOS 的 firmlink（`/System/Volumes/Data/<原路径>`）。
pub(crate) fn unfirmlink(p: PathBuf) -> PathBuf {
    match p.strip_prefix("/System/Volumes/Data") {
        Ok(rest) => Path::new("/").join(rest),
        Err(_) => p,
    }
}

/// 拒绝路径里的 `..`，并对最近已存在的祖先目录解析符号链接与 firmlink。
///
/// 词法 `starts_with` 挡不住 `~/opt/../../etc` 或指向 `/` 的符号链接；在真正删目录
/// 前先把真实路径落下来再判定。
pub(crate) fn resolve_no_parent_dir(p: &Path) -> Result<PathBuf> {
    if p.components().any(|c| matches!(c, Component::ParentDir)) {
        bail!("路径不允许包含 `..`: {}", p.display());
    }
    let mut existing = p;
    let mut tail: Vec<&std::ffi::OsStr> = Vec::new();
    while !existing.exists() {
        let Some(name) = existing.file_name() else {
            break;
        };
        tail.push(name);
        let Some(parent) = existing.parent() else {
            break;
        };
        existing = parent;
    }
    let mut base = if existing.exists() {
        unfirmlink(
            existing
                .canonicalize()
                .unwrap_or_else(|_| existing.to_path_buf()),
        )
    } else {
        existing.to_path_buf()
    };
    for part in tail.into_iter().rev() {
        base.push(part);
    }
    Ok(base)
}

/// 拦住明显不该被递归删除的路径。
///
/// 要删的目录是从环境变量拼出来的，变量空掉时拼出来的可能就是 `/` 或者家目录本身。
/// `rm -rf $HOME` 这种事只会发生一次，所以在真正动手之前先问一句这像不像一个
/// 安装目录。
pub fn guard_removable(p: &Path) -> Result<()> {
    let real = resolve_no_parent_dir(p)?;
    let home_raw = env::home();
    let home = unfirmlink(home_raw.canonicalize().unwrap_or(home_raw));
    if real == Path::new("/") || real == home || real == Path::new("/opt") {
        bail!("拒绝删除 {}：这不像是安装目录。", p.display());
    }
    if !real.starts_with(&home) && !real.starts_with("/opt") {
        bail!(
            "{} 既不在 {} 下也不在 /opt 下，拒绝自动删除。确认无误的话手工处理。",
            p.display(),
            home.display()
        );
    }
    Ok(())
}

/// 原子写文件（先写同目录临时文件再 rename），避免半写入破坏配置文件。
///
/// 若 `path` 是符号链接，沿链接指向的真实目标原子替换，不破坏用户的 dotfiles 链接。
/// `mode` 为 `Some(m)` 时在写入内容前就设为该权限（适合含密钥的配置）；为 `None` 时
/// 若目标文件已存在则保留其原权限，否则按默认 `0o644`（受进程 umask 约束的常规预期）。
pub fn write_atomic_mode(path: &Path, content: &str, mode: Option<u32>) -> Result<()> {
    let display = path.display();
    let target = if path.is_symlink() {
        path.canonicalize()
            .with_context(|| format!("无法解析符号链接 {display}"))?
    } else {
        path.to_path_buf()
    };
    let dir = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir).with_context(|| format!("建目录失败: {}", dir.display()))?;

    let keep_mode = match mode {
        Some(m) => Some(m),
        None => std::fs::metadata(&target)
            .ok()
            .map(|m| m.permissions().mode() & 0o777),
    };
    let mut staged = tempfile::NamedTempFile::new_in(dir)
        .with_context(|| format!("建临时文件失败: {}", dir.display()))?;
    // NamedTempFile 默认 0600；写入前就按目标权限设定，含密钥的文件不会经过 0644 窗口。
    if let Some(m) = keep_mode {
        staged
            .as_file()
            .set_permissions(std::fs::Permissions::from_mode(m))
            .with_context(|| format!("设置权限失败: {display}"))?;
    } else {
        let _ = staged
            .as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o644));
    }
    staged
        .write_all(content.as_bytes())
        .with_context(|| format!("写不了 {display}"))?;
    staged
        .as_file()
        .flush()
        .with_context(|| format!("刷盘失败: {display}"))?;
    staged
        .persist(&target)
        .map_err(|e| e.error)
        .with_context(|| format!("写不了 {display}"))?;
    Ok(())
}

/// 原子写常规配置文件。
pub fn write_atomic(path: &Path, content: &str) -> Result<()> {
    write_atomic_mode(path, content, None)
}

/// 从文件末尾读取最后 `max` 行（最多读最后 256 KiB，遇到非 UTF-8 字节按 lossy 降级）。
///
/// 服务日志长期运行可能涨到数 GB，整文件 `read_to_string` 会直接 OOM 或被单字节乱码
/// 拦住；固定窗口既省内存也能稳定打印出报错上下文。
pub fn tail_lines(path: &Path, max: usize) -> Option<String> {
    const WINDOW: u64 = 256 * 1024;
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(WINDOW);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::with_capacity((len - start) as usize);
    f.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    // 窗口从文件中间切进去时，首行可能是截断的半行，丢掉以免打出残片。
    let text = if start > 0 {
        text.split_once('\n')
            .map_or(text.as_ref(), |(_, rest)| rest)
    } else {
        text.as_ref()
    };
    let mut kept: Vec<&str> = text.lines().rev().take(max).collect();
    kept.reverse();
    Some(kept.join("\n"))
}

/// 一次性的工作目录，用完即删。
///
/// 放在用户缓存目录下而不是 `/tmp`：`/tmp` 是所有人都能写的，往里放一个名字可
/// 预测的脚本文件再执行它，等于把「谁先建出这个路径谁说了算」交给了别人。缓存
/// 目录只有本人能写，这条路就堵死了。
pub struct Scratch {
    dir: tempfile::TempDir,
}

impl Scratch {
    pub fn new(label: &str) -> Result<Self> {
        let base = dirs::cache_dir()
            .unwrap_or_else(|| env::home().join(".cache"))
            .join("fundeploy");
        std::fs::create_dir_all(&base)
            .with_context(|| format!("建目录失败: {}", base.display()))?;
        let dir = tempfile::Builder::new()
            .prefix(&format!("{label}-"))
            .tempdir_in(&base)
            .with_context(|| format!("建临时目录失败: {}", base.display()))?;
        Ok(Self { dir })
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }
}

/// 目录里有什么，按名字排序，每行带上大小。目录不存在返回 None。
pub fn listing(dir: &Path) -> Option<Vec<String>> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| {
            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
            format!("{}  {}", e.file_name().to_string_lossy(), human_size(size))
        })
        .collect();
    out.sort();
    Some(out)
}

/// 目录里所有普通文件的字节数之和。目录不存在、读不动都算 0。
///
/// 用来给「正在被别的进程写的目录」量进度，所以路上任何一步出错都跳过而不是中断：
/// npm 一边装一边删临时文件，`read_dir` 列出的条目到 `metadata` 时已经没了是常态。
///
/// 不跟符号链接。npm 的 `.bin/` 全是指回包里的链接，跟过去就会把同一份内容数两遍；
/// 更糟的是 node_modules 里出现指向上层的链接时会绕不出来。
///
/// 代价随文件数走，实测 2.1 GiB / 73042 个文件约 0.3 秒（dentry 热的时候）——调用方
/// 要是在循环里量同一个目录，自己限频。
pub fn dir_size(dir: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            // file_type() 在 Linux 上取的是 readdir 返回的 d_type，不额外 stat；
            // 只有普通文件才需要再问一次大小。
            let Ok(t) = e.file_type() else { continue };
            if t.is_dir() {
                stack.push(e.path());
            } else if t.is_file() {
                total += e.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    total
}

/// 字节数转成人看的单位。
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 拒绝删除根目录和家目录() {
        assert!(guard_removable(Path::new("/")).is_err());
        assert!(guard_removable(&env::home()).is_err());
        assert!(guard_removable(Path::new("/opt")).is_err());
    }

    #[test]
    fn 拒绝包含路径穿越的目录() {
        assert!(guard_removable(&env::home().join("opt/../../etc")).is_err());
        assert!(guard_removable(&env::home().join("opt/..")).is_err());
        assert!(guard_removable(Path::new("/opt/../etc")).is_err());
    }

    #[test]
    fn 原子写支持权限与符号链接目标() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("real.conf");
        let link = dir.path().join("link.conf");
        std::fs::write(&target, "old\n").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        write_atomic_mode(&link, "secret=1\n", Some(0o600)).unwrap();
        assert!(link.is_symlink());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "secret=1\n");
        let mode = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn tail_lines_只取末尾行且容忍非_utf8() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("app.log");
        std::fs::write(&log, b"a\n\xffbad\nc\nd\n").unwrap();
        assert_eq!(tail_lines(&log, 2).as_deref(), Some("c\nd"));
        assert!(tail_lines(&dir.path().join("missing.log"), 5).is_none());
    }

    #[test]
    fn 拒绝删除家目录和_opt_之外的路径() {
        assert!(guard_removable(Path::new("/etc")).is_err());
        assert!(guard_removable(Path::new("/var/lib/x")).is_err());
    }

    #[test]
    fn 放行家目录和_opt_下的安装目录() {
        assert!(guard_removable(&env::home().join("opt/airflow")).is_ok());
        assert!(guard_removable(Path::new("/opt/airflow")).is_ok());
    }

    #[test]
    fn 临时目录用完就没了() {
        let path;
        {
            let s = Scratch::new("test").unwrap();
            path = s.path().to_path_buf();
            assert!(path.is_dir());
            std::fs::write(path.join("x"), "y").unwrap();
        }
        assert!(!path.exists(), "{} 没被删掉", path.display());
    }

    #[test]
    fn 同时存在的两个临时目录不打架() {
        let a = Scratch::new("test").unwrap();
        let b = Scratch::new("test").unwrap();
        assert_ne!(a.path(), b.path());
    }

    #[test]
    fn 目录大小是递归求和的() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a"), vec![0u8; 100]).unwrap();
        let sub = dir.path().join("x/y");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("b"), vec![0u8; 23]).unwrap();
        assert_eq!(dir_size(dir.path()), 123);
    }

    #[test]
    fn 目录大小不跟符号链接() {
        // 守的是「同一份内容不数两遍」：npm 的 .bin/ 全是链接，跟过去的话量出来的
        // 进度会虚高，分母和分子还会虚高得不一样多。
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir(&real).unwrap();
        std::fs::write(real.join("f"), vec![0u8; 50]).unwrap();
        std::os::unix::fs::symlink(&real, dir.path().join("链到目录")).unwrap();
        std::os::unix::fs::symlink(real.join("f"), dir.path().join("链到文件")).unwrap();
        assert_eq!(dir_size(dir.path()), 50);
    }

    #[test]
    fn 目录不存在算零而不是报错() {
        assert_eq!(dir_size(Path::new("/definitely/not/here")), 0);
    }

    #[test]
    fn 字节数转成人看的单位() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1024), "1.0 KiB");
        assert_eq!(human_size(1024 * 1024 * 3 / 2), "1.5 MiB");
    }
}
