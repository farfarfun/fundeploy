//! fundeploy 托管那条路的日志：读末尾若干行、跟随。
//!
//! 只在没有服务管理器的机器上用得到。有 systemd / launchd 的机器上日志归它们管，
//! 走的是官方 `service logs`。

use super::*;

/// 认 `-f` / `--follow` 和 `-n N` / `-nN` / `--lines=N`，别的提示后忽略。
///
/// fundeploy 托管的日志就是一个文件，官方 `service logs` 那套 journalctl 参数在这儿
/// 没有对应物，与其假装支持，不如明说忽略了什么。
pub(super) fn parse_log_args(args: &[&str], default_lines: usize) -> (usize, bool) {
    let mut lines = default_lines;
    let mut follow = false;
    let mut want_lines = false;
    for a in args {
        if want_lines {
            lines = a.parse().unwrap_or(lines);
            want_lines = false;
            continue;
        }
        if *a == "-f" || *a == "--follow" {
            follow = true;
        } else if *a == "-n" || *a == "--lines" {
            want_lines = true;
        } else if let Some(n) = a.strip_prefix("--lines=").or_else(|| a.strip_prefix("-n")) {
            lines = n.parse().unwrap_or(lines);
        } else {
            ui::warn(format!("托管日志是直接读文件，忽略不支持的参数 {a}"));
        }
    }
    (lines, follow)
}

/// 日志末尾的 N 行。
///
/// 只读末尾一段：服务日志能涨到几百 M，为看几十行把整个文件读进内存，会在最需要它的
/// 时候（启动失败、磁盘紧张）把事情变得更糟。
pub(super) fn tail_lines(path: &Path, lines: usize) -> Vec<String> {
    use std::io::{Read, Seek, SeekFrom};
    const WINDOW: u64 = 256 * 1024;

    let Ok(mut f) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let from = len.saturating_sub(WINDOW);
    if f.seek(SeekFrom::Start(from)).is_err() {
        return Vec::new();
    }
    let mut buf = Vec::new();
    if f.read_to_end(&mut buf).is_err() {
        return Vec::new();
    }

    let text = String::from_utf8_lossy(&buf);
    let mut all: Vec<&str> = text.lines().collect();
    // 从中间截断的第一行多半是半行，丢掉。
    if from > 0 && !all.is_empty() {
        all.remove(0);
    }
    all.iter()
        .skip(all.len().saturating_sub(lines))
        .map(|s| (*s).to_string())
        .collect()
}

pub(super) fn print_tail(path: &Path, lines: usize) {
    for line in tail_lines(path, lines) {
        eprintln!("  {line}");
    }
}

/// 跟随日志直到 Ctrl+C。
///
/// 自己轮询而不是调 `tail -f`：少一个外部依赖，而且 Ctrl+C 的归属清楚——信号投给的是
/// fundeploy 自己，不会留下一个还在跑的 tail。
pub(super) fn follow_log(path: &Path) -> Result<()> {
    use std::io::{Read, Seek, SeekFrom, Write};

    let mut f = std::fs::File::open(path)?;
    let mut pos = f.seek(SeekFrom::End(0))?;
    loop {
        std::thread::sleep(Duration::from_millis(300));
        let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(pos);
        if len < pos {
            // 日志被轮转或清空了。旧的文件句柄指向的还是老 inode，必须重开，
            // 否则这条通道从此就哑了。
            f = std::fs::File::open(path)?;
            pos = 0;
            continue;
        }
        if len == pos {
            continue;
        }
        f.seek(SeekFrom::Start(pos))?;
        let mut buf = vec![0u8; (len - pos) as usize];
        let n = f.read(&mut buf)?;
        buf.truncate(n);
        pos += n as u64;
        std::io::stdout().write_all(&buf)?;
        std::io::stdout().flush()?;
    }
}

// Paperclip 身上跟日志有关的那几个动作。单开一个 impl 块放在这儿，而不是留在
// mod.rs 的主 impl 里：那个块本来就近五十个方法，谁都不该为了看一眼 `logs`
// 在里面上下翻。同一个类型在同一个 crate 里可以有多个 impl，分到哪个文件由
// 「这是哪一摊事」决定。
impl Paperclip {
    fn logs_fallback(&self, args: &[&str]) -> Result<()> {
        if !self.log_file.exists() {
            bail!(
                "没有 fundeploy 托管的日志（{} 不存在）；这个实例多半不是用 \
                 `fundeploy service paperclip start` 起的",
                self.log_file.display()
            );
        }
        let (lines, follow) = parse_log_args(args, self.log_lines);
        for line in tail_lines(&self.log_file, lines) {
            println!("{line}");
        }
        if follow {
            follow_log(&self.log_file)?;
        }
        Ok(())
    }

    pub(super) fn logs(&self, args: &[&str]) -> Result<()> {
        let sup = self.supervisor();
        if sup.available != Some(false) {
            // 官方 service logs 读的是 journalctl / launchd 的日志。
            return self.must(&prepend(&["service", "logs"], args));
        }
        self.require_fallback(&sup)?;
        self.logs_fallback(args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 日志参数认得出行数和跟随() {
        assert_eq!(parse_log_args(&[], 200), (200, false));
        assert_eq!(parse_log_args(&["-f"], 200), (200, true));
        assert_eq!(parse_log_args(&["-n", "50"], 200), (50, false));
        assert_eq!(parse_log_args(&["-n50"], 200), (50, false));
        assert_eq!(parse_log_args(&["--lines=50", "-f"], 200), (50, true));
        // 不认识的参数只提示，不该让看日志这件事直接失败。
        assert_eq!(parse_log_args(&["--since=1h"], 200), (200, false));
        // 行数不是数字时保持默认，而不是 panic。
        assert_eq!(parse_log_args(&["-n", "很多"], 200), (200, false));
    }

    #[test]
    fn 取日志末尾若干行() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("a.log");
        std::fs::write(&log, "1\n2\n3\n4\n5\n").unwrap();
        assert_eq!(tail_lines(&log, 2), ["4", "5"]);
        // 要的比有的多时给全部，不该 panic。
        assert_eq!(tail_lines(&log, 99).len(), 5);
        // 文件不存在只是「没日志」，不是错误。
        assert!(tail_lines(&dir.path().join("nope.log"), 10).is_empty());
    }
}
