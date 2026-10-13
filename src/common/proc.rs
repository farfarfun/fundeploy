//! 进程生命周期：PID 文件、存活判定、优雅停止。

use anyhow::{Context, Result};
use std::path::Path;
use std::time::{Duration, Instant};

/// 读 PID 文件。文件不存在、内容不是正整数，都当成「没有记录在跑的进程」返回 None，
/// 而不是报错——PID 文件缺失是 stop/status 的正常情况，不是故障。
pub fn read_pid_file(path: &Path) -> Option<i32> {
    let raw = std::fs::read_to_string(path).ok()?;
    let trimmed: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    match trimmed.parse::<i32>() {
        Ok(pid) if pid > 0 => Some(pid),
        _ => None,
    }
}

pub fn write_pid_file(path: &Path, pid: i32) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("建不了 PID 文件的目录: {}", dir.display()))?;
    }
    std::fs::write(path, format!("{pid}\n"))
        .with_context(|| format!("写不了 PID 文件: {}", path.display()))
}

/// 进程是否活着。等价于 `kill -0 <pid>`。
///
/// 注意这回答的是「这个 PID 当前有进程」，不是「还是原来那个进程」：PID 会被系统回收，
/// 一个停了很久的服务留下的 PID 文件可能指向一个毫不相干的新进程。调用方在要据此
/// 发信号时，应当同时核对端口是否真的被这个 PID 监听（见 port::listener_pid）。
pub fn is_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    // signal 0 不投递信号，只做权限与存在性检查。
    unsafe { libc::kill(pid, 0) == 0 }
}

/// 取 pid 的进程组 ID。拿不到（进程已退出、/proc 不可读）返回 None。
fn pgid_of(pid: i32) -> Option<i32> {
    let r = unsafe { libc::getpgid(pid) };
    if r < 0 {
        None
    } else {
        Some(r)
    }
}

/// 停止一个进程：先 SIGTERM，等到 `timeout` 还活着再 SIGKILL。
///
/// `process_group = true` 时连同整个进程组一起停（服务常会派生子进程，只杀父进程会留下孤儿）。
///
/// # 只有组 leader 才能整组停
///
/// **只有当 pid 本身就是进程组 leader 时，才可以对 `-pgid` 发信号。** 以
/// `nohup cmd &` 方式起的子进程会**继承调用者的进程组**——此时 pgid 指向的是
/// fundeploy 自己（或用户的 shell）所在的组，对 `-pgid` 发 SIGKILL 等于把调用者
/// 连同它的终端会话一起杀掉。无条件 `kill(-pgid)` 的症状是「停个服务顺手把 ssh
/// 会话踢了」，且只在服务由 nohup 起的机器上复现。
pub fn stop_pid(pid: i32, timeout: Duration, process_group: bool) -> Result<()> {
    if !is_alive(pid) {
        return Ok(());
    }

    // 决定信号投给谁：进程组（负数）还是单个进程。
    let target = if process_group {
        match pgid_of(pid) {
            Some(pgid) if pgid == pid => -pgid, // pid 是组 leader，整组停是安全的
            _ => pid,                           // 否则退回只停这一个，绝不碰别人的组
        }
    } else {
        pid
    };

    signal(target, libc::SIGTERM);

    let deadline = Instant::now() + timeout;
    while is_alive(pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(200));
    }

    if is_alive(pid) {
        signal(target, libc::SIGKILL);
        // KILL 之后再给内核一点时间回收，否则紧随其后的 status 会报「还在跑」。
        let deadline = Instant::now() + Duration::from_secs(3);
        while is_alive(pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    Ok(())
}

/// 投信号。失败一律忽略：进程正好在这一刻退出（ESRCH）是竞态而非错误，
/// 而权限不足（EPERM）在 stop 的语义下也只能继续往下走到超时。
fn signal(target: i32, sig: libc::c_int) {
    unsafe {
        libc::kill(target, sig);
    }
}

/// 按 PID 文件停止，并在停成功后删掉那个文件。
pub fn stop_by_pid_file(path: &Path, timeout: Duration, process_group: bool) -> Result<bool> {
    let Some(pid) = read_pid_file(path) else {
        // 没有 PID 文件：可能从没起过，也可能上次是干净退出的。都不算错。
        let _ = std::fs::remove_file(path);
        return Ok(false);
    };
    if !is_alive(pid) {
        // 陈旧的 PID 文件。清掉它，让下次 status 不再误报「在跑」。
        let _ = std::fs::remove_file(path);
        return Ok(false);
    }
    stop_pid(pid, timeout, process_group)?;
    let _ = std::fs::remove_file(path);
    Ok(true)
}

/// 在作用域内把 SIGINT/SIGTERM 从「默认杀掉自己」改成「记下来」。
///
/// 给前台子进程用：Ctrl+C 会投给整个前台进程组，父子都收到。父进程若按默认行为
/// 当场死掉，就没机会去停它先前起好的后台组件，留下孤儿进程。
///
/// 装的必须是**处理函数**而不是 `SIG_IGN`：`exec` 会把已装处理函数的信号重置为
/// 默认行为，但会保留 `SIG_IGN`——用 `SIG_IGN` 的话子进程也跟着忽略 Ctrl+C，
/// 表现为「按了没反应」。
pub struct SignalGuard {
    saved: Vec<(libc::c_int, libc::sighandler_t)>,
}

static CAUGHT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

extern "C" fn on_signal(_sig: libc::c_int) {
    CAUGHT.store(true, std::sync::atomic::Ordering::SeqCst);
}

impl SignalGuard {
    pub fn install() -> Self {
        CAUGHT.store(false, std::sync::atomic::Ordering::SeqCst);
        let handler = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
        let mut saved = Vec::new();
        for sig in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
            let prev = unsafe { libc::signal(sig, handler) };
            saved.push((sig, prev));
        }
        Self { saved }
    }

    /// 作用域内有没有收到过信号。
    pub fn caught(&self) -> bool {
        CAUGHT.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl Drop for SignalGuard {
    fn drop(&mut self) {
        for (sig, prev) in self.saved.drain(..) {
            unsafe {
                libc::signal(sig, prev);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn 自己一定活着() {
        let me = unsafe { libc::getpid() };
        assert!(is_alive(me));
    }

    #[test]
    fn 非法_pid_一律判死() {
        assert!(!is_alive(0));
        assert!(!is_alive(-1));
    }

    #[test]
    fn 读不存在的_pid_文件返回_none() {
        let d = tmp();
        assert_eq!(read_pid_file(&d.path().join("nope.pid")), None);
    }

    #[test]
    fn pid_文件里的空白被忽略() {
        let d = tmp();
        let p = d.path().join("a.pid");
        std::fs::write(&p, "  12345 \n").unwrap();
        assert_eq!(read_pid_file(&p), Some(12345));
    }

    #[test]
    fn 垃圾内容的_pid_文件返回_none() {
        let d = tmp();
        let p = d.path().join("b.pid");
        // 服务崩在写 PID 文件的半路上，或磁盘满了只写进半行，都会得到这种内容。
        // 这里必须返回 None 而不是 panic，否则 status 会直接崩掉而不是报「没在跑」。
        for junk in ["", "abc", "0", "-5", "12x34", "\n\n"] {
            std::fs::write(&p, junk).unwrap();
            assert_eq!(read_pid_file(&p), None, "junk={junk:?}");
        }
    }

    #[test]
    fn 写了能读回来() {
        let d = tmp();
        let p = d.path().join("c/d/e.pid"); // 顺带验证父目录会被建出来
        write_pid_file(&p, 4242).unwrap();
        assert_eq!(read_pid_file(&p), Some(4242));
    }

    #[test]
    fn 停一个陈旧_pid_文件不报错且会清掉文件() {
        let d = tmp();
        let p = d.path().join("stale.pid");
        // 取一个几乎不可能存在的 PID。
        let mut f = std::fs::File::create(&p).unwrap();
        writeln!(f, "4194303").unwrap();
        drop(f);
        let stopped = stop_by_pid_file(&p, Duration::from_secs(1), false).unwrap();
        assert!(!stopped, "不该报告「停掉了」——那个进程本来就不在");
        assert!(!p.exists(), "陈旧 PID 文件必须被清掉");
    }

    #[test]
    fn 真的能停掉一个子进程() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("起不了 sleep");
        let pid = child.id() as i32;
        assert!(is_alive(pid));

        stop_pid(pid, Duration::from_secs(5), false).unwrap();
        // 回收僵尸，否则 is_alive 仍然看得到它。
        let _ = child.wait();
        assert!(!is_alive(pid));
    }

    #[test]
    fn 停不是组_leader_的进程时绝不整组停() {
        // 这条守的是上面那条「血换来的规则」：测试进程自己不是组 leader 时
        // （cargo test 跑在 shell 的进程组里），stop_pid 必须退化成只停单个进程。
        // 若实现回退成无条件 kill(-pgid)，这个测试会把 cargo test 自己杀掉——
        // 表现为整个测试二进制被 SIGKILL，而不是一条断言失败。
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let pid = child.id() as i32;
        let me = unsafe { libc::getpid() };

        // 子进程继承了我们的组，所以它的 pgid != 它自己的 pid。
        let child_pgid = pgid_of(pid).unwrap();
        assert_ne!(
            child_pgid, pid,
            "前提不成立：子进程竟是组 leader，这个测试就测不到那条规则了"
        );

        stop_pid(pid, Duration::from_secs(5), true).unwrap();
        let _ = child.wait();

        assert!(!is_alive(pid), "子进程该被停掉");
        assert!(is_alive(me), "调用者必须还活着——整组停会把自己也杀了");
    }
}
