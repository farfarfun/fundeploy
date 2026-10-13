//! 版本号比较。
//!
//! 上游的版本号**不保证是 semver**，要伺候的形态至少有：
//!
//! ```text
//! 1.1.116                  三段式
//! 2026.1008.0-canary.4     日期式 + 预发布通道
//! v1.2.0                   GitHub tag 的 v 前缀
//! 1.2                      缺段
//! ```
//!
//! semver crate 对缺段和四段式会直接解析失败，所以这里实现一个宽松比较器：
//! 能按数字比的按数字比，比不了的退到字典序。

use std::cmp::Ordering;

/// 一个宽松解析的版本号。`release` 是 `-` 之前按 `.` 切开的段，
/// `pre` 是 `-` 之后的段（没有预发布时为空）。
///
/// `PartialEq`/`Eq` **不能 derive**：derive 出来的实现会把 `raw` 也比进去，于是
/// `parse("v1.2.0") == parse("1.2.0")` 为假，而手写的 `Ord` 对这两个返回 `Equal`。
/// `Ord` 与 `Eq` 不一致会让 `sort`、`dedup`、`BTreeMap` 行为未定义，表面症状则是
/// `is_same` 认不出同一版本的两种写法——upgrade 会把 `v1.2.0` 和 `1.2.0` 当成
/// 两个版本来回装。所以下面用 `cmp` 定义 `eq`。
#[derive(Debug, Clone)]
pub struct Version {
    release: Vec<Segment>,
    pre: Vec<Segment>,
    raw: String,
}

impl PartialEq for Version {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Version {}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Num(u64),
    Text(String),
}

impl Segment {
    fn parse(s: &str) -> Self {
        // 纯数字才当数字。`1008` → Num(1008)，`canary` → Text。
        // 前导零的 `007` 也会变成 Num(7)：版本号里 007 和 7 指同一个版本。
        match s.parse::<u64>() {
            Ok(n) => Segment::Num(n),
            Err(_) => Segment::Text(s.to_ascii_lowercase()),
        }
    }
}

impl Ord for Segment {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Segment::Num(a), Segment::Num(b)) => a.cmp(b),
            (Segment::Text(a), Segment::Text(b)) => a.cmp(b),
            // 数字段与文本段比较时数字在前。semver 的规则正好相反（文本 > 数字），
            // 但那条规则只在预发布标识符里成立，而这里要同时伺候 release 段。
            // release 段里出现文本本身就是异常输入（比如 `1.2.x`），把它排在数字后面
            // 更符合直觉：`1.2.x` 不该被当成比 `1.2.0` 新的版本去触发升级。
            (Segment::Num(_), Segment::Text(_)) => Ordering::Less,
            (Segment::Text(_), Segment::Num(_)) => Ordering::Greater,
        }
    }
}

impl PartialOrd for Segment {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Version {
    pub fn parse(raw: &str) -> Self {
        let s = raw.trim();
        // 去掉 tag 的 v 前缀，但只在后面紧跟数字时去——`vnext` 这种不是版本号前缀。
        let s = s
            .strip_prefix('v')
            .or_else(|| s.strip_prefix('V'))
            .filter(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
            .unwrap_or(s);

        // 构建号（`+` 之后）按 semver 的规定不参与比较，直接丢掉。
        let s = s.split('+').next().unwrap_or(s);

        let (rel, pre) = match s.split_once('-') {
            Some((r, p)) => (r, p),
            None => (s, ""),
        };

        let split = |part: &str| -> Vec<Segment> {
            if part.is_empty() {
                Vec::new()
            } else {
                part.split('.').map(Segment::parse).collect()
            }
        };

        Version {
            release: split(rel),
            pre: split(pre),
            raw: raw.trim().to_string(),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.raw
    }

    pub fn is_prerelease(&self) -> bool {
        !self.pre.is_empty()
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        // 段数不等时，缺的段按 0 补齐，这样 `1.2` == `1.2.0` 而不是 `1.2` < `1.2.0`。
        // 上游偶尔会把 `1.2.0` 写成 `1.2`，若判成「更旧」，upgrade 会一直认为有新版可装。
        let n = self.release.len().max(other.release.len());
        for i in 0..n {
            let a = self.release.get(i).unwrap_or(&Segment::Num(0));
            let b = other.release.get(i).unwrap_or(&Segment::Num(0));
            match a.cmp(b) {
                Ordering::Equal => continue,
                ord => return ord,
            }
        }

        // release 相同时：有预发布的更旧（1.0.0 > 1.0.0-canary.4）。
        match (self.pre.is_empty(), other.pre.is_empty()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => {
                // 两边都是预发布：逐段比。段数少的更旧（canary.4 > canary）。
                let n = self.pre.len().max(other.pre.len());
                for i in 0..n {
                    match (self.pre.get(i), other.pre.get(i)) {
                        (Some(a), Some(b)) => match a.cmp(b) {
                            Ordering::Equal => continue,
                            ord => return ord,
                        },
                        (Some(_), None) => return Ordering::Greater,
                        (None, Some(_)) => return Ordering::Less,
                        (None, None) => break,
                    }
                }
                Ordering::Equal
            }
        }
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.raw)
    }
}

/// `a` 是否严格比 `b` 新。给 upgrade 判断「要不要装」用。
pub fn is_newer(a: &str, b: &str) -> bool {
    Version::parse(a) > Version::parse(b)
}

/// 两个版本是否指同一个版本（吸收 `v` 前缀、补齐缺段、忽略构建号的差异）。
pub fn is_same(a: &str, b: &str) -> bool {
    Version::parse(a) == Version::parse(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s)
    }

    #[test]
    fn 数字段按数值比而非字典序() {
        // 这是 `sort` 不加 -V 时最经典的错：字典序下 "9" > "10"。
        assert!(v("1.10.0") > v("1.9.0"));
        assert!(v("1.0.100") > v("1.0.99"));
        // paperclip 的日期式版本号同理：1008 > 1001。
        assert!(v("2026.1008.0") > v("2026.1001.0"));
    }

    #[test]
    fn 缺少的段按零补齐() {
        assert_eq!(v("1.2"), v("1.2.0"));
        assert_eq!(v("1"), v("1.0.0"));
        assert!(v("1.2.1") > v("1.2"));
    }

    #[test]
    fn v_前缀被吸收但不误伤非版本号() {
        assert_eq!(v("v1.2.0"), v("1.2.0"));
        assert_eq!(v("V1.2.0"), v("1.2.0"));
        // `vnext` 后面不是数字，不该当成带前缀的版本号去剥。
        assert_eq!(v("vnext").release, vec![Segment::Text("vnext".into())]);
    }

    #[test]
    fn 预发布比正式版旧() {
        assert!(v("1.0.0") > v("1.0.0-canary.4"));
        assert!(v("1.0.0") > v("1.0.0-rc.1"));
        // 但预发布仍然比上一个正式版新。
        assert!(v("1.0.0-canary.1") > v("0.9.9"));
    }

    #[test]
    fn 预发布之间逐段比较() {
        assert!(v("1.0.0-canary.4") > v("1.0.0-canary.3"));
        // 段数多的更新：canary.4 > canary。
        assert!(v("1.0.0-canary.4") > v("1.0.0-canary"));
        assert!(v("1.0.0-rc.1") > v("1.0.0-beta.9"));
    }

    #[test]
    fn 构建号不参与比较() {
        assert_eq!(v("1.0.0+build.5"), v("1.0.0+build.9"));
        assert_eq!(v("1.0.0+build.5"), v("1.0.0"));
    }

    #[test]
    fn 降级能被识别出来() {
        // canary 通道上 2026.1001.0 比 2026.1008.0-canary.4 旧，装它是一次降级。
        assert!(!is_newer("2026.1001.0", "2026.1008.0-canary.4"));
        assert!(is_newer("2026.1008.0-canary.4", "2026.1001.0"));
    }

    #[test]
    fn 三段式版本序() {
        assert!(is_newer("1.1.116", "1.1.113"));
        assert!(is_newer("1.1.116", "1.1.11"));
        assert!(!is_newer("1.1.113", "1.1.116"));
    }

    #[test]
    fn 相同判定吸收表面差异() {
        assert!(is_same("v1.2.0", "1.2.0"));
        assert!(is_same("1.2", "1.2.0"));
        assert!(!is_same("1.2.0", "1.2.1"));
    }

    #[test]
    fn 前导零不影响相等() {
        assert!(is_same("1.02.0", "1.2.0"));
    }

    #[test]
    fn 排序稳定() {
        let mut xs: Vec<Version> = ["1.10.0", "1.2.0", "1.0.0-rc.1", "1.0.0", "2.0.0"]
            .iter()
            .map(|s| v(s))
            .collect();
        xs.sort();
        let got: Vec<&str> = xs.iter().map(|x| x.as_str()).collect();
        assert_eq!(got, ["1.0.0-rc.1", "1.0.0", "1.2.0", "1.10.0", "2.0.0"]);
    }

    #[test]
    fn 预发布标记可识别() {
        assert!(v("2026.1008.0-canary.4").is_prerelease());
        assert!(!v("2026.1008.0").is_prerelease());
    }
}
