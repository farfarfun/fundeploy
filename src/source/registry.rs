//! 所有能换源的东西放在一处。
//!
//! 加一个新的包管理器只需要在 [`all`] 里多一行——命令行、`status --all`、
//! `switch all` 全都是遍历这张表，不用各自再改一遍。

use anyhow::{bail, Result};

use super::Source;

/// 全部可换源的工具，按常用程度排。
pub fn all() -> Vec<Box<dyn Source>> {
    vec![
        Box::new(super::pip::Pip),
        Box::new(super::npm::Npm),
        Box::new(super::brew::Brew),
        Box::new(super::cargo::Cargo),
        Box::new(super::go::Go),
    ]
}

pub fn names() -> Vec<String> {
    all().iter().map(|s| s.name().to_string()).collect()
}

pub fn find(name: &str) -> Result<Box<dyn Source>> {
    let name = name.trim();
    match all().into_iter().find(|s| s.name() == name) {
        Some(s) => Ok(s),
        None => bail!("不认识 {name}。可选: {}", names().join("、")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 名字不重样() {
        let mut n = names();
        let total = n.len();
        n.sort();
        n.dedup();
        assert_eq!(n.len(), total, "有重名: {n:?}");
    }

    #[test]
    fn 每个都能按名字找回来() {
        for name in names() {
            assert_eq!(find(&name).unwrap().name(), name);
        }
    }

    #[test]
    fn 找不到时把可选项列出来() {
        // 报错只说「不认识」等于让人去翻文档。
        let err = find("pypi").err().expect("不该找到").to_string();
        assert!(err.contains("pip"), "实际: {err}");
    }

    #[test]
    fn 每个都写了一句说明() {
        for s in all() {
            assert!(!s.summary().is_empty(), "{} 缺说明", s.name());
        }
    }
}
