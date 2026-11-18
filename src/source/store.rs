//! 自定义源的落盘。
//!
//! 内置那张表覆盖不了两种情况：公司内网的私有仓库，以及镜像站改了路径而我们
//! 还没发版。两种都不该逼人去改代码，所以 `source add` 写在这里，和内置表一起
//! 参与列表与测速。

use std::collections::BTreeMap;
use std::io::{ErrorKind, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use super::Mirror;
use crate::common::env;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    name: String,
    #[serde(default)]
    label: String,
    url: String,
}

/// 整个文件：`{ 包管理器名 -> [源] }`。
type File = BTreeMap<String, Vec<Entry>>;

pub fn path() -> PathBuf {
    if let Some(p) = env::opt("FUNDEPLOY_SOURCES_FILE") {
        return env::expand_tilde(&p);
    }
    dirs::config_dir()
        .unwrap_or_else(|| env::home().join(".config"))
        .join("fundeploy")
        .join("sources.toml")
}

fn load(p: &Path) -> Result<File> {
    let text = match std::fs::read_to_string(p) {
        Ok(text) => text,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(File::new()),
        Err(e) => return Err(e).with_context(|| format!("读不了 {}", p.display())),
    };
    toml::from_str(&text).with_context(|| format!("自定义源配置格式有误: {}", p.display()))
}

/// 锁住目录直到替换完成：两个同时启动的 add 都必须读到上一次写入。
fn edit(change: impl FnOnce(&mut File) -> bool) -> Result<bool> {
    let path = path();
    let path = if path.is_symlink() {
        path.canonicalize()
            .with_context(|| format!("无法解析 {}", path.display()))?
    } else {
        path
    };
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let lock = std::fs::File::open(dir)?;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(std::io::Error::last_os_error()).context("无法锁定自定义源配置目录");
    }
    let mut file = load(&path)?;
    if !change(&mut file) {
        return Ok(false);
    }
    let mut staged = tempfile::NamedTempFile::new_in(dir)?;
    if let Ok(meta) = path.metadata() {
        staged.as_file().set_permissions(meta.permissions())?;
    }
    staged.write_all(toml::to_string_pretty(&file)?.as_bytes())?;
    staged
        .persist(&path)
        .with_context(|| format!("写不了 {}", path.display()))?;
    Ok(true)
}

/// 某个包管理器下，用户自己加的源。
pub fn custom(source: &str) -> Vec<Mirror> {
    load(&path())
        .unwrap_or_default()
        .remove(source)
        .unwrap_or_default()
        .into_iter()
        .map(|e| Mirror {
            label: if e.label.is_empty() {
                "自定义".to_string()
            } else {
                e.label
            },
            name: e.name,
            url: e.url,
            extras: Vec::new(),
            custom: true,
            official: false,
        })
        .collect()
}

pub fn add(source: &str, name: &str, url: &str, label: Option<&str>) -> Result<()> {
    let name = name.trim();
    let url = url.trim();
    if name.is_empty() {
        bail!("源名是空的");
    }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        bail!("{url} 不是 http(s) 地址");
    }
    let entry = Entry {
        name: name.to_string(),
        label: label.unwrap_or("自定义").to_string(),
        url: url.to_string(),
    };

    edit(|f| {
        let list = f.entry(source.to_string()).or_default();
        match list.iter().position(|e| e.name == name) {
            // 同名改成覆盖而不是报错：改一个写错的地址，重跑一遍 add 是最顺手的做法。
            Some(i) => list[i] = entry,
            None => list.push(entry),
        }
        true
    })?;
    Ok(())
}

pub fn remove(source: &str, name: &str) -> Result<bool> {
    edit(|f| {
        let Some(list) = f.get_mut(source) else {
            return false;
        };
        let before = list.len();
        list.retain(|e| e.name != name);
        if list.len() == before {
            return false;
        }
        if list.is_empty() {
            f.remove(source);
        }
        true
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::排队;

    fn 临时文件() -> (tempfile::TempDir, std::sync::MutexGuard<'static, ()>) {
        let g = 排队();
        let d = tempfile::tempdir().unwrap();
        std::env::set_var("FUNDEPLOY_SOURCES_FILE", d.path().join("sources.toml"));
        (d, g)
    }

    #[test]
    fn 加了能读回来() {
        let (_d, _g) = 临时文件();
        add("pip", "corp", "https://pypi.corp/simple/", Some("公司内网")).unwrap();
        let all = custom("pip");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "corp");
        assert_eq!(all[0].label, "公司内网");
        assert!(all[0].custom, "自定义源必须标出来，否则 rm 的时候分不清");
        // 别的包管理器看不到它。
        assert!(custom("npm").is_empty());
    }

    #[test]
    fn 同名是覆盖不是加第二份() {
        let (_d, _g) = 临时文件();
        add("npm", "corp", "https://a/", None).unwrap();
        add("npm", "corp", "https://b/", None).unwrap();
        let all = custom("npm");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].url, "https://b/");
    }

    #[test]
    fn 删得掉也报得出没删着() {
        let (_d, _g) = 临时文件();
        add("pip", "x", "https://a/", None).unwrap();
        assert!(remove("pip", "x").unwrap());
        assert!(custom("pip").is_empty());
        assert!(!remove("pip", "x").unwrap());
        assert!(!remove("没这个包管理器", "x").unwrap());
    }

    #[test]
    fn 不是地址的东西加不进去() {
        let (_d, _g) = 临时文件();
        assert!(add("pip", "x", "pypi.corp/simple", None).is_err());
        assert!(add("pip", "", "https://a/", None).is_err());
    }

    #[test]
    fn 文件是坏的也只是当成没有自定义源() {
        let (d, _g) = 临时文件();
        std::fs::write(d.path().join("sources.toml"), "这不是 toml {{{").unwrap();
        // 坏文件让 `source list` 整个失败的话，人连内置源都看不到了。
        assert!(custom("pip").is_empty());
        assert!(add("npm", "new", "https://example.org", None).is_err());
        assert!(remove("npm", "old").is_err());
        assert_eq!(std::fs::read_to_string(path()).unwrap(), "这不是 toml {{{");
    }

    #[test]
    fn 多个包管理器各存各的() {
        let (_d, _g) = 临时文件();
        add("pip", "a", "https://pip/", None).unwrap();
        add("npm", "a", "https://npm/", None).unwrap();
        assert_eq!(custom("pip")[0].url, "https://pip/");
        assert_eq!(custom("npm")[0].url, "https://npm/");
    }
}
