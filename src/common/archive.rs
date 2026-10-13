//! 解包 tar.gz / zip。
//!
//! 自己解而不是调 `tar`：macOS 自带的是 bsdtar，`--strip-components` 的行为和
//! GNU tar 在边界情况上对不齐，而布局差一层目录，后面找二进制就全错了。

use anyhow::{bail, Context, Result};
use flate2::read::GzDecoder;
use std::fs::File;
use std::io::{BufReader, Seek};
use std::path::{Component, Path, PathBuf};

/// 解 tar.gz 到 `dest`。
///
/// `strip` 是要剥掉的前导路径层数，对应 `tar --strip-components`。上游打包时
/// 常常把所有东西塞进一个带版本号的顶层目录（`code-server-4.112.0-linux-amd64/`），
/// 剥掉这一层才能让安装路径不随版本变化。
pub fn untar_gz(archive: &Path, dest: &Path, strip: usize) -> Result<()> {
    let f = File::open(archive).with_context(|| format!("打开失败: {}", archive.display()))?;
    let mut tar = tar::Archive::new(GzDecoder::new(BufReader::new(f)));
    std::fs::create_dir_all(dest).with_context(|| format!("建目录失败: {}", dest.display()))?;

    let mut count = 0usize;
    for entry in tar.entries().context("读取 tar 条目失败")? {
        let mut entry = entry.context("读取 tar 条目失败")?;
        let path = entry.path().context("tar 条目路径无效")?.into_owned();
        let Some(rel) = strip_safe(&path, strip)? else {
            continue;
        };
        let out = dest.join(&rel);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("建目录失败: {}", parent.display()))?;
        }
        entry
            .unpack(&out)
            .with_context(|| format!("解压失败: {}", out.display()))?;
        count += 1;
    }
    if count == 0 {
        bail!(
            "{} 解出来是空的（strip={strip} 可能剥多了）",
            archive.display()
        );
    }
    Ok(())
}

/// 解 zip 到 `dest`。
pub fn unzip(archive: &Path, dest: &Path, strip: usize) -> Result<()> {
    let f = File::open(archive).with_context(|| format!("打开失败: {}", archive.display()))?;
    let mut zip = zip::ZipArchive::new(BufReader::new(f)).context("不是有效的 zip")?;
    std::fs::create_dir_all(dest).with_context(|| format!("建目录失败: {}", dest.display()))?;

    let mut count = 0usize;
    for i in 0..zip.len() {
        let mut item = zip.by_index(i).context("读取 zip 条目失败")?;
        // enclosed_name 会挡掉 `..` 和绝对路径，自己拼会把文件写到解压目录外面。
        let Some(path) = item.enclosed_name() else {
            bail!("zip 里有不安全的路径: {}", item.name());
        };
        let Some(rel) = strip_safe(&path, strip)? else {
            continue;
        };
        let out = dest.join(&rel);
        if item.is_dir() {
            std::fs::create_dir_all(&out)
                .with_context(|| format!("建目录失败: {}", out.display()))?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("建目录失败: {}", parent.display()))?;
        }
        let mut dst = File::create(&out).with_context(|| format!("写入失败: {}", out.display()))?;
        std::io::copy(&mut item, &mut dst)
            .with_context(|| format!("写入失败: {}", out.display()))?;
        #[cfg(unix)]
        if let Some(mode) = item.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode)).ok();
        }
        count += 1;
    }
    if count == 0 {
        bail!(
            "{} 解出来是空的（strip={strip} 可能剥多了）",
            archive.display()
        );
    }
    Ok(())
}

/// 剥掉前 `strip` 层，同时挡住会跳出目标目录的路径。
///
/// 返回 None 表示这个条目层数不够，应当跳过（`tar --strip-components` 的行为）。
fn strip_safe(path: &Path, strip: usize) -> Result<Option<PathBuf>> {
    let mut parts: Vec<&std::ffi::OsStr> = Vec::new();
    for c in path.components() {
        match c {
            Component::Normal(p) => parts.push(p),
            Component::CurDir => {}
            // 归档里带 `..` 或绝对路径是典型的穿越攻击：解压就能覆盖目标目录外的文件。
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                bail!("归档里有不安全的路径: {}", path.display())
            }
        }
    }
    if parts.len() <= strip {
        return Ok(None);
    }
    Ok(Some(parts[strip..].iter().collect()))
}

/// 归档格式。按文件名猜。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    TarGz,
    Zip,
    /// 不是归档，就是一个裸二进制。
    Raw,
}

impl Kind {
    pub fn of(filename: &str) -> Self {
        let lower = filename.to_ascii_lowercase();
        if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
            Kind::TarGz
        } else if lower.ends_with(".zip") {
            Kind::Zip
        } else {
            Kind::Raw
        }
    }
}

/// 按格式解包；裸二进制则原样复制过去并加执行位。
pub fn extract(archive: &Path, filename: &str, dest: &Path, strip: usize) -> Result<()> {
    match Kind::of(filename) {
        Kind::TarGz => untar_gz(archive, dest, strip),
        Kind::Zip => unzip(archive, dest, strip),
        Kind::Raw => bail!("{filename} 不是归档，应当直接安装而不是解包"),
    }
}

/// 在目录树里找一个名字叫 `name` 的可执行文件。
///
/// 上游把二进制放在哪一层不固定（有的在根，有的在 `bin/`），装完要能定位到它。
pub fn find_file(root: &Path, name: &str) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.file_name().and_then(|s| s.to_str()) == Some(name) {
                return Some(p);
            }
        }
    }
    None
}

/// 给文件加上执行位（0755）。
pub fn make_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .with_context(|| format!("设置执行位失败: {}", path.display()))?;
    }
    let _ = path;
    Ok(())
}

/// 判断一个已下载的文件是不是 gzip（魔数 1f 8b）。
///
/// 用来兜一种常见故障：镜像站或代理在出错时回一个 HTML 错误页，但 HTTP 状态是
/// 200、文件名还是 `.tar.gz`。不看魔数的话，报错会是「不是有效的 gzip 流」，
/// 和「网络被劫持了」差得很远。
pub fn looks_gzip(path: &Path) -> bool {
    let Ok(mut f) = File::open(path) else {
        return false;
    };
    let mut magic = [0u8; 2];
    use std::io::Read;
    let ok = f.read_exact(&mut magic).is_ok();
    let _ = f.rewind();
    ok && magic == [0x1f, 0x8b]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tgz(entries: &[(&str, &[u8])]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.tar.gz");
        let f = File::create(&path).unwrap();
        let enc = flate2::write::GzEncoder::new(f, flate2::Compression::fast());
        let mut tar = tar::Builder::new(enc);
        for (name, data) in entries {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            tar.append_data(&mut h, name, *data).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap();
        (dir, path)
    }

    #[test]
    fn 剥掉顶层目录() {
        let (_d, a) = tgz(&[("pkg-1.0/bin/x", b"hi"), ("pkg-1.0/README", b"doc")]);
        let out = tempfile::tempdir().unwrap();
        untar_gz(&a, out.path(), 1).unwrap();
        assert_eq!(std::fs::read(out.path().join("bin/x")).unwrap(), b"hi");
        assert!(out.path().join("README").exists());
        // 顶层目录本身不该再出现。
        assert!(!out.path().join("pkg-1.0").exists());
    }

    #[test]
    fn 不剥时保留原布局() {
        let (_d, a) = tgz(&[("pkg/bin/x", b"hi")]);
        let out = tempfile::tempdir().unwrap();
        untar_gz(&a, out.path(), 0).unwrap();
        assert!(out.path().join("pkg/bin/x").exists());
    }

    #[test]
    fn 剥过头时报错而不是静默解出空目录() {
        // 静默成功是最坏的结果：install 报成功，start 却说找不到二进制。
        let (_d, a) = tgz(&[("x", b"hi")]);
        let out = tempfile::tempdir().unwrap();
        let err = untar_gz(&a, out.path(), 3).unwrap_err().to_string();
        assert!(err.contains("空"), "实际: {err}");
    }

    #[test]
    fn 跳出目标目录的条目被拒() {
        // 守的是路径穿越：归档里的 `../` 或绝对路径能覆盖解压目录外的文件。
        // 这里直接测 strip_safe 而不是造一个恶意 tar——tar::Builder 自己就不肯
        // 写这种条目，但真实世界里的归档是别人打的，没有这层约束。
        for bad in ["../evil", "a/../../evil", "/etc/evil"] {
            assert!(strip_safe(Path::new(bad), 0).is_err(), "{bad} 应当被拒绝");
        }
    }

    #[test]
    fn strip_的层数语义() {
        assert_eq!(
            strip_safe(Path::new("a/b/c"), 1).unwrap(),
            Some(PathBuf::from("b/c"))
        );
        assert_eq!(strip_safe(Path::new("a/b"), 2).unwrap(), None);
        assert_eq!(
            strip_safe(Path::new("./a/b"), 0).unwrap(),
            Some(PathBuf::from("a/b"))
        );
    }

    #[test]
    fn 按扩展名认格式() {
        assert_eq!(Kind::of("x.tar.gz"), Kind::TarGz);
        assert_eq!(Kind::of("x.TGZ"), Kind::TarGz);
        assert_eq!(Kind::of("x.zip"), Kind::Zip);
        assert_eq!(Kind::of("new-api-v0.12.6"), Kind::Raw);
    }

    #[test]
    fn 魔数能认出不是_gzip_的东西() {
        let dir = tempfile::tempdir().unwrap();
        let html = dir.path().join("fake.tar.gz");
        let mut f = File::create(&html).unwrap();
        f.write_all(b"<!DOCTYPE html><title>502</title>").unwrap();
        drop(f);
        assert!(!looks_gzip(&html));

        let (_d, real) = tgz(&[("x", b"hi")]);
        assert!(looks_gzip(&real));
    }

    #[test]
    fn 能在目录树里找到二进制() {
        let out = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(out.path().join("a/b")).unwrap();
        std::fs::write(out.path().join("a/b/sub2api"), b"x").unwrap();
        let found = find_file(out.path(), "sub2api").unwrap();
        assert!(found.ends_with("a/b/sub2api"));
        assert!(find_file(out.path(), "没有这个").is_none());
    }
}
