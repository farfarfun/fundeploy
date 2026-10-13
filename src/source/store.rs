//! 自定义源的落盘。
//!
//! 内置那张表覆盖不了两种情况：公司内网的私有仓库，以及镜像站改了路径而我们
//! 还没发版。两种都不该逼人去改代码，所以 `source add` 写在这里，和内置表一起
//! 参与列表与测速。

use std::collections::BTreeMap;
use std::path::PathBuf;

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

fn load() -> File {
    // 读不出来就当没有：这个文件丢了顶多少几个自定义源，不该让 `source list`
    // 整个失败。真正要报错的是写不进去的时候。
    std::fs::read_to_string(path())
        .ok()
        .and_then(|t| toml::from_str(&t).ok())
        .unwrap_or_default()
}

fn save(f: &File) -> Result<()> {
    let p = path();
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("建目录失败: {}", dir.display()))?;
    }
    let text = toml::to_string_pretty(f).context("序列化自定义源失败")?;
    std::fs::write(&p, text).with_context(|| format!("写不了 {}", p.display()))
}

/// 某个包管理器下，用户自己加的源。
pub fn custom(source: &str) -> Vec<Mirror> {
    load()
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

    let mut f = load();
    let list = f.entry(source.to_string()).or_default();
    match list.iter().position(|e| e.name == name) {
        // 同名改成覆盖而不是报错：改一个写错的地址，重跑一遍 add 是最顺手的做法。
        Some(i) => list[i] = entry,
        None => list.push(entry),
    }
    save(&f)
}

pub fn remove(source: &str, name: &str) -> Result<bool> {
    let mut f = load();
    let Some(list) = f.get_mut(source) else {
        return Ok(false);
    };
    let before = list.len();
    list.retain(|e| e.name != name);
    if list.len() == before {
        return Ok(false);
    }
    if list.is_empty() {
        f.remove(source);
    }
    save(&f)?;
    Ok(true)
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
