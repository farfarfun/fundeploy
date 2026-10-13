//! 逐行改 `键 = 值` 配置文件，不动别的内容。
//!
//! pip.conf、`.npmrc`、`go env` 的文件都是这个形状（pip.conf 带 `[global]` 这类
//! 节，另两个没有节）。换源只需要动其中一两个键，但这些文件是**人也在编辑的**：
//! 里面有注释、有超时设置、有代理配置。
//!
//! 所以不走「解析成结构体再序列化回去」那条路——那样写回去的是一份语义等价、
//! 外观全变的文件：注释没了、顺序变了、`git diff` 一片红。这里只改目标那一行，
//! 其余字节原样留着。

use std::path::Path;

use anyhow::{Context, Result};

pub struct Ini {
    lines: Vec<String>,
}

/// 把一行拆成 `(节名)` / `(键, 值)` / 其它。
enum Line<'a> {
    Section(&'a str),
    Pair(&'a str, &'a str),
    Other,
}

fn classify(line: &str) -> Line<'_> {
    let t = line.trim();
    if t.is_empty() || t.starts_with('#') || t.starts_with(';') {
        return Line::Other;
    }
    if let Some(inner) = t.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        return Line::Section(inner.trim());
    }
    match t.split_once('=') {
        Some((k, v)) => Line::Pair(k.trim(), v.trim()),
        None => Line::Other,
    }
}

impl Ini {
    /// 读一个文件。不存在当成空文件——换源常常是这台机器上第一次写它。
    pub fn load(path: &Path) -> Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e).with_context(|| format!("读不了 {}", path.display())),
        };
        Ok(Self {
            lines: text.lines().map(str::to_string).collect(),
        })
    }

    /// 扫一遍，对每一行给出 `(当前所处的节, 行号, 这一行)`。节外的行拿到 None。
    fn scan<'a>(&'a self) -> impl Iterator<Item = (Option<&'a str>, usize, Line<'a>)> + 'a {
        let mut section: Option<&'a str> = None;
        self.lines.iter().enumerate().map(move |(i, line)| {
            let parsed = classify(line);
            if let Line::Section(name) = parsed {
                section = Some(name);
                // 节标题行自己不属于任何节，否则 `[a]` 会被当成 a 里的内容。
                return (None, i, Line::Section(name));
            }
            (section, i, parsed)
        })
    }

    fn locate(&self, section: Option<&str>, key: &str) -> Option<usize> {
        self.scan().find_map(|(sec, i, parsed)| match parsed {
            Line::Pair(k, _) if sec == section && k == key => Some(i),
            _ => None,
        })
    }

    pub fn get(&self, section: Option<&str>, key: &str) -> Option<String> {
        let i = self.locate(section, key)?;
        match classify(&self.lines[i]) {
            Line::Pair(_, v) => Some(v.to_string()),
            _ => None,
        }
    }

    /// 设一个键。已有就改那一行，没有就插进对应的节末尾。
    pub fn set(&mut self, section: Option<&str>, key: &str, value: &str) {
        let entry = format!("{key} = {value}");
        if let Some(i) = self.locate(section, key) {
            self.lines[i] = entry;
            return;
        }
        let Some(section) = section else {
            self.lines.push(entry);
            return;
        };
        match self.section_end(section) {
            Some(end) => self.lines.insert(end, entry),
            None => {
                // 新建一节。前面空一行，免得跟上一节的最后一行黏在一起。
                if self.lines.last().is_some_and(|l| !l.trim().is_empty()) {
                    self.lines.push(String::new());
                }
                self.lines.push(format!("[{section}]"));
                self.lines.push(entry);
            }
        }
    }

    /// 某一节内容的结束位置（下一节标题的下标，或文件末尾）。节不存在返回 None。
    fn section_end(&self, section: &str) -> Option<usize> {
        let mut start = None;
        for (i, line) in self.lines.iter().enumerate() {
            if let Line::Section(name) = classify(line) {
                if name == section {
                    start = Some(i);
                } else if start.is_some() {
                    return Some(i);
                }
            }
        }
        start.map(|_| self.lines.len())
    }

    /// 删一个键。不存在就什么都不做。
    pub fn unset(&mut self, section: Option<&str>, key: &str) {
        while let Some(i) = self.locate(section, key) {
            self.lines.remove(i);
        }
    }

    /// 这个文件里还有没有实质内容（除了节标题、空行、注释）。
    ///
    /// 用来决定 reset 之后是留下一个空壳还是把文件删掉。
    pub fn is_trivial(&self) -> bool {
        !self
            .lines
            .iter()
            .any(|l| matches!(classify(l), Line::Pair(..)))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("建目录失败: {}", dir.display()))?;
        }
        let mut text = self.lines.join("\n");
        if !text.is_empty() {
            text.push('\n');
        }
        std::fs::write(path, text).with_context(|| format!("写不了 {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ini(text: &str) -> Ini {
        Ini {
            lines: text.lines().map(str::to_string).collect(),
        }
    }

    fn text(i: &Ini) -> String {
        i.lines.join("\n")
    }

    #[test]
    fn 读得出带节的键() {
        let i = ini("[global]\nindex-url = https://a/\ntimeout = 60\n");
        assert_eq!(
            i.get(Some("global"), "index-url").as_deref(),
            Some("https://a/")
        );
        assert_eq!(i.get(Some("global"), "没这个"), None);
        // 节名对不上就不该读到。
        assert_eq!(i.get(Some("install"), "index-url"), None);
        assert_eq!(i.get(None, "index-url"), None);
    }

    #[test]
    fn 读得出不带节的键() {
        let i = ini("registry=https://registry.npmjs.org/\n//r/:_authToken=x\n");
        assert_eq!(
            i.get(None, "registry").as_deref(),
            Some("https://registry.npmjs.org/")
        );
    }

    #[test]
    fn 改键只动那一行() {
        // 守的是「别把人家的文件重排一遍」：注释、顺序、无关键都必须原样。
        let mut i = ini("# 公司代理\n[global]\ntimeout = 60\nindex-url = https://old/\n[install]\nuser = true\n");
        i.set(Some("global"), "index-url", "https://new/");
        assert_eq!(
            text(&i),
            "# 公司代理\n[global]\ntimeout = 60\nindex-url = https://new/\n[install]\nuser = true"
        );
    }

    #[test]
    fn 新键插在本节末尾而不是文件末尾() {
        // 插到文件末尾的话，它会落进最后那一节里，对 pip 来说就是写进了
        // [install] —— 看起来写了，实际不生效。
        let mut i = ini("[global]\ntimeout = 60\n[install]\nuser = true\n");
        i.set(Some("global"), "index-url", "https://new/");
        assert_eq!(
            text(&i),
            "[global]\ntimeout = 60\nindex-url = https://new/\n[install]\nuser = true"
        );
    }

    #[test]
    fn 节不存在时新建一节() {
        let mut i = ini("[install]\nuser = true\n");
        i.set(Some("global"), "index-url", "https://new/");
        assert_eq!(
            text(&i),
            "[install]\nuser = true\n\n[global]\nindex-url = https://new/"
        );
    }

    #[test]
    fn 空文件也能写() {
        let mut i = ini("");
        i.set(Some("global"), "index-url", "https://new/");
        assert_eq!(text(&i), "[global]\nindex-url = https://new/");

        let mut i = ini("");
        i.set(None, "registry", "https://r/");
        assert_eq!(text(&i), "registry = https://r/");
    }

    #[test]
    fn 删键不碰同名但不同节的键() {
        let mut i = ini("[global]\nindex-url = https://a/\n[install]\nindex-url = https://b/\n");
        i.unset(Some("global"), "index-url");
        assert_eq!(text(&i), "[global]\n[install]\nindex-url = https://b/");
    }

    #[test]
    fn 重复的键会被删干净() {
        // 手工编辑留下两行同名键是常事，只删第一行等于「删了还在」。
        let mut i = ini("[global]\nindex-url = https://a/\nindex-url = https://b/\n");
        i.unset(Some("global"), "index-url");
        assert!(i.get(Some("global"), "index-url").is_none());
    }

    #[test]
    fn 注释行不被当成键值() {
        let i = ini("# index-url = https://a/\n; registry = https://b/\n");
        assert_eq!(i.get(None, "# index-url"), None);
        assert!(i.is_trivial());
    }

    #[test]
    fn 只剩节标题和注释算空壳() {
        assert!(ini("[global]\n# 没了\n").is_trivial());
        assert!(!ini("[global]\nx = 1\n").is_trivial());
    }

    #[test]
    fn 值里有等号也能读全() {
        // 带 token 的 npmrc、带 query 的 URL 都会出现第二个等号。
        let i = ini("registry=https://r/?a=1&b=2\n");
        assert_eq!(
            i.get(None, "registry").as_deref(),
            Some("https://r/?a=1&b=2")
        );
    }

    #[test]
    fn 写盘读回来还是同一份() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("sub/pip.conf"); // 顺带验证父目录会被建出来
        let mut i = Ini::load(&p).unwrap();
        i.set(Some("global"), "index-url", "https://a/");
        i.save(&p).unwrap();
        assert_eq!(
            Ini::load(&p).unwrap().get(Some("global"), "index-url"),
            Some("https://a/".into())
        );
    }
}
