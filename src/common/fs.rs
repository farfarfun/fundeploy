//! 文件系统上那几件需要先想一下再做的事。

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::env;

/// 拦住明显不该被递归删除的路径。
///
/// 要删的目录是从环境变量拼出来的，变量空掉时拼出来的可能就是 `/` 或者家目录本身。
/// `rm -rf $HOME` 这种事只会发生一次，所以在真正动手之前先问一句这像不像一个
/// 安装目录。
pub fn guard_removable(p: &Path) -> Result<()> {
    let home = env::home();
    if p == Path::new("/") || p == home {
        bail!("拒绝删除 {}：这不像是安装目录。", p.display());
    }
    if !p.starts_with(&home) && !p.starts_with("/opt") {
        bail!(
            "{} 既不在 {} 下也不在 /opt 下，拒绝自动删除。确认无误的话手工处理。",
            p.display(),
            home.display()
        );
    }
    Ok(())
}

/// 一次性的工作目录，用完即删。
///
/// 放在用户缓存目录下而不是 `/tmp`：`/tmp` 是所有人都能写的，往里放一个名字可
/// 预测的脚本文件再执行它，等于把「谁先建出这个路径谁说了算」交给了别人。缓存
/// 目录只有本人能写，这条路就堵死了。
pub struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    pub fn new(label: &str) -> Result<Self> {
        let base = dirs::cache_dir()
            .unwrap_or_else(|| env::home().join(".cache"))
            .join("fundeploy");
        std::fs::create_dir_all(&base)
            .with_context(|| format!("建目录失败: {}", base.display()))?;

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        // 进程号防别人、纳秒防上一次运行、计数器防同一个进程里连着建两个——
        // 时钟精度粗的机器上，连续两次取到的纳秒数可能一模一样。
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = base.join(format!("{label}-{}-{nanos}-{n}", std::process::id()));
        // create_dir 而不是 create_dir_all：路径已存在就失败，不复用别人留下的目录。
        std::fs::create_dir(&dir).with_context(|| format!("建临时目录失败: {}", dir.display()))?;
        Ok(Self { dir })
    }

    pub fn path(&self) -> &Path {
        &self.dir
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
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
