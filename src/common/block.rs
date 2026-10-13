//! 在别人的配置文件里划一块归自己管的地方。
//!
//! 换源要往 shell 配置和 `~/.cargo/config.toml` 里写东西，这两个文件都是人天天
//! 在编辑的。往里追加容易，难的是**第二次**：怎么把上次写的收回来而不误伤。
//!
//! 办法是用一对标记把自己写的东西框起来，改和删都只认这对标记之间的内容。
//! HomebrewCN 的做法是给每行加个 `#ckbrew` 尾注，然后 `sed -i "/ckbrew/d"`——
//! 那是按**内容**匹配：谁的注释里恰好提到这个词，谁的那行就跟着没了。

use std::path::Path;

use anyhow::{Context, Result};

pub struct Marker {
    begin: String,
    end: String,
}

impl Marker {
    /// `label` 会出现在标记行里，给人看的。
    pub fn new(label: &str) -> Self {
        Self {
            begin: format!("# >>> fundeploy {label} >>>"),
            end: format!("# <<< fundeploy {label} <<<"),
        }
    }

    /// 找出区块在 `lines` 里的范围（含两条标记行）。
    fn find(&self, lines: &[String]) -> Option<std::ops::Range<usize>> {
        let b = lines.iter().position(|l| l.trim() == self.begin)?;
        // 结束标记要在开始标记之后找。缺了结束标记就当区块只有那一行，
        // 这样至少下一次写入能把它替换掉，而不是永远留着一行孤儿标记。
        let e = lines
            .iter()
            .skip(b + 1)
            .position(|l| l.trim() == self.end)
            .map(|i| b + 1 + i)
            .unwrap_or(b);
        Some(b..e + 1)
    }

    /// 区块当前的内容（不含标记行）。没有区块返回 None。
    pub fn read(&self, path: &Path) -> Option<Vec<String>> {
        let lines = read_lines(path).ok()?;
        let r = self.find(&lines)?;
        // 掐掉首尾两条标记行。残块（只有开始标记）掐完是空的，不是负长度。
        let body_start = r.start + 1;
        let body_end = r.end.saturating_sub(1).max(body_start);
        Some(lines[body_start..body_end].to_vec())
    }

    /// 把区块内容换成 `body`。没有区块就追加一个。
    pub fn write(&self, path: &Path, body: &[String]) -> Result<()> {
        let mut lines = read_lines(path)?;
        let mut block = vec![self.begin.clone()];
        block.extend(body.iter().cloned());
        block.push(self.end.clone());

        match self.find(&lines) {
            Some(r) => {
                lines.splice(r, block);
            }
            None => {
                if lines.last().is_some_and(|l| !l.trim().is_empty()) {
                    lines.push(String::new());
                }
                lines.extend(block);
            }
        }
        save(path, &lines)
    }

    /// 删掉区块。返回是否真的删了点什么。
    pub fn remove(&self, path: &Path) -> Result<bool> {
        let mut lines = read_lines(path)?;
        let Some(r) = self.find(&lines) else {
            return Ok(false);
        };
        let start = r.start;
        lines.drain(r);
        // write 追加时为了隔开加过一个空行，删的时候一并收回。不收的话，
        // 每换一轮源就在别人的配置里多留一行空白。
        if start > 0 && lines.get(start - 1).is_some_and(|l| l.trim().is_empty()) {
            lines.remove(start - 1);
        }
        save(path, &lines)?;
        Ok(true)
    }
}

fn read_lines(path: &Path) -> Result<Vec<String>> {
    match std::fs::read_to_string(path) {
        Ok(t) => Ok(t.lines().map(str::to_string).collect()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e).with_context(|| format!("读不了 {}", path.display())),
    }
}

fn save(path: &Path, lines: &[String]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("建目录失败: {}", dir.display()))?;
    }
    let mut text = lines.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    std::fs::write(path, text).with_context(|| format!("写不了 {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> (tempfile::TempDir, std::path::PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("profile");
        (d, p)
    }

    fn read(p: &Path) -> String {
        std::fs::read_to_string(p).unwrap()
    }

    #[test]
    fn 写进不存在的文件() {
        let (_d, p) = tmp();
        Marker::new("x").write(&p, &["export A=1".into()]).unwrap();
        assert_eq!(
            read(&p),
            "# >>> fundeploy x >>>\nexport A=1\n# <<< fundeploy x <<<\n"
        );
    }

    #[test]
    fn 追加时不碰原有内容() {
        let (_d, p) = tmp();
        std::fs::write(&p, "export PATH=/a:$PATH\n").unwrap();
        Marker::new("x").write(&p, &["export A=1".into()]).unwrap();
        assert!(read(&p).starts_with("export PATH=/a:$PATH\n"));
    }

    #[test]
    fn 第二次写是替换不是再追加一份() {
        // 这条是整个模块存在的理由：换了三次源之后，shell 配置里不该有三份
        // 互相覆盖的 export —— 那种文件里到底哪个生效，得看谁在最后面。
        let (_d, p) = tmp();
        let m = Marker::new("x");
        m.write(&p, &["export A=1".into()]).unwrap();
        m.write(&p, &["export A=2".into()]).unwrap();
        let text = read(&p);
        assert_eq!(text.matches("fundeploy x >>>").count(), 1);
        assert!(text.contains("export A=2"));
        assert!(!text.contains("export A=1"));
    }

    #[test]
    fn 删区块只删自己那段() {
        let (_d, p) = tmp();
        std::fs::write(&p, "前面的\n").unwrap();
        let m = Marker::new("x");
        m.write(&p, &["export A=1".into()]).unwrap();
        std::fs::write(&p, format!("{}后面的\n", read(&p))).unwrap();

        assert!(m.remove(&p).unwrap());
        assert_eq!(read(&p), "前面的\n后面的\n");
        // 再删一次什么也没有，但不能报错。
        assert!(!m.remove(&p).unwrap());
    }

    #[test]
    fn 内容里提到标记里的词不会被误删() {
        // HomebrewCN 那种按关键词 sed 删行的做法会把这行一起删掉。
        let (_d, p) = tmp();
        std::fs::write(&p, "# 这行讲的是 fundeploy x 怎么用\n").unwrap();
        let m = Marker::new("x");
        m.write(&p, &["export A=1".into()]).unwrap();
        m.remove(&p).unwrap();
        assert_eq!(read(&p), "# 这行讲的是 fundeploy x 怎么用\n");
    }

    #[test]
    fn 读得回自己写进去的内容() {
        let (_d, p) = tmp();
        let m = Marker::new("x");
        assert_eq!(m.read(&p), None);
        m.write(&p, &["export A=1".into(), "export B=2".into()])
            .unwrap();
        assert_eq!(
            m.read(&p),
            Some(vec!["export A=1".to_string(), "export B=2".to_string()])
        );
    }

    #[test]
    fn 空区块读出来是空的而不是_none() {
        let (_d, p) = tmp();
        let m = Marker::new("x");
        m.write(&p, &[]).unwrap();
        assert_eq!(m.read(&p), Some(vec![]));
    }

    #[test]
    fn 结束标记被手工删掉后还能修回来() {
        // 人手工编辑时把结束标记删了，不能让下一次写入变成「往残块后面再追加一块」。
        let (_d, p) = tmp();
        std::fs::write(&p, "# >>> fundeploy x >>>\nexport A=1\nexport 别人的=2\n").unwrap();
        let m = Marker::new("x");
        m.write(&p, &["export A=9".into()]).unwrap();
        let text = read(&p);
        assert_eq!(text.matches("fundeploy x >>>").count(), 1);
        assert!(text.contains("export A=9"));
        // 残块之后的内容不属于区块，必须留着。
        assert!(text.contains("export 别人的=2"), "实际:\n{text}");
    }

    #[test]
    fn 两个不同标记互不干扰() {
        let (_d, p) = tmp();
        let a = Marker::new("brew");
        let b = Marker::new("cargo");
        a.write(&p, &["A".into()]).unwrap();
        b.write(&p, &["B".into()]).unwrap();
        a.remove(&p).unwrap();
        let text = read(&p);
        assert!(text.contains('B'));
        assert!(!text.contains("fundeploy brew"));
    }
}
