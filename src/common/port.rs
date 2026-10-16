//! 端口探测：谁在监听、等它监听起来。
//!
//! Linux 上监听表直接读 /proc，不依赖 lsof/ss——容器镜像里这两个常常都没装，
//! 而 `ss` 的输出格式在不同 iproute2 版本间也变过。macOS 没有 /proc，退回 lsof。

use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

/// 端口上是否有人在监听。
///
/// 用「连一下试试」而不是查监听表：这正是调用方真正关心的问题（服务起来了吗），
/// 而且跨平台、不需要权限。代价是对只监听 127.0.0.1 的服务必须传对 host——
/// 传 0.0.0.0 去连是连不上的，所以 host 一律用服务配置里那个值。
pub fn is_listening(host: &str, port: u16) -> bool {
    // 0.0.0.0 / :: 是「监听所有网卡」的写法，不是可连接的目的地址。
    // 真去连它在部分平台上会直接 ECONNREFUSED，所以换成回环再连。
    let host = match host {
        "0.0.0.0" | "*" | "" => "127.0.0.1",
        "::" | "[::]" => "::1",
        h => h,
    };

    let Ok(addrs) = (host, port).to_socket_addrs() else {
        return false;
    };
    for addr in addrs {
        if TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok() {
            return true;
        }
    }
    false
}

/// 这个监听地址在本机之外也能到吗。
///
/// 用来决定要不要就「这东西暴露在网上了」提醒一句。
pub fn is_public(host: &str) -> bool {
    !matches!(host, "127.0.0.1" | "localhost" | "::1")
}

/// 给人看的主机名。
///
/// `0.0.0.0` 是绑定时的写法，贴进浏览器地址栏打不开；入口地址要给回环。
pub fn display_host(host: &str) -> &str {
    match host {
        "0.0.0.0" | "*" | "" => "127.0.0.1",
        "::" | "[::]" => "::1",
        h => h,
    }
}

/// 轮询等端口起来。返回是否在 `timeout` 内等到了。
///
/// 服务刚 spawn 出来时端口还没 bind，调用方不能立刻判定失败——这个间隔在装了
/// 一堆 Python 依赖的后端上能有十几秒（import 时间）。
pub fn wait_for_listener(host: &str, port: u16, timeout: Duration) -> bool {
    wait_for_listener_ticking(host, port, timeout, || {})
}

/// 同上，但每轮询一次就喊一下 `on_tick`。
///
/// 这一等最长可以到半分钟（装了一堆依赖的 Python 后端 import 完就要这么久），
/// 屏幕上一个字都不动的话跟卡死没有区别。调用方拿这个回调去转一个 spinner。
pub fn wait_for_listener_ticking(
    host: &str,
    port: u16,
    timeout: Duration,
    mut on_tick: impl FnMut(),
) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if is_listening(host, port) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        on_tick();
        std::thread::sleep(Duration::from_millis(300));
    }
}

/// 占着某个端口的一个进程。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holder {
    pub pid: i32,
    /// 进程名。读不到时是空串（进程刚退出，或者属于别的用户）。
    pub comm: String,
    /// 占的是哪些协议，`tcp` / `udp`，按字母序。
    pub protos: Vec<&'static str>,
}

/// 占着 `port` 的所有进程，按 PID 升序。
///
/// 「占着」只算**监听中**的 TCP 和**已绑定**的 UDP。`lsof -i :8080` 那种匹配法还会
/// 把本地临时端口恰好是 8080 的出站连接算进来，照它的结果去 kill，杀掉的是一个跟
/// 这个端口毫无关系的进程。
///
/// 同一个端口会有多个进程：`fork` 出来的 worker 继承着同一个监听 fd（gunicorn、
/// nginx 都是这样），必须全列出来——只停 master，worker 还攥着端口不放。
///
/// 非 root 时看不见别的用户的进程（读不到他们的 /proc/<pid>/fd，`ss -p` 同样打不出
/// pid）。那种情况下这里返回空，但端口是实打实占着的，用 [`is_taken`] 判。
pub fn holders(port: u16) -> Vec<Holder> {
    #[cfg(target_os = "linux")]
    {
        linux::holders(port)
    }
    #[cfg(not(target_os = "linux"))]
    {
        lsof::holders(port)
    }
}

/// 端口上还有没有被占着的 socket——不要求能看出占用者是谁。
///
/// 杀完进程之后用它复查：[`holders`] 返回空只说明「我看不见占用者」，
/// 把这个当成「腾出来了」会在非 root 下报假的成功。
pub fn is_taken(port: u16) -> bool {
    #[cfg(target_os = "linux")]
    {
        // /proc/net/tcp 是全局可读的，别人的监听也在里面。
        !linux::bound_inodes(port).is_empty()
    }
    #[cfg(not(target_os = "linux"))]
    {
        !holders(port).is_empty()
    }
}

/// 监听指定端口的进程 PID。查不到返回 None。
///
/// 多个进程共用一个监听 fd 时给 PID 最小的那个——`fork` 模型下父进程先于子进程
/// 创建，最小的 PID 通常就是 master，也正是该对它发信号的那个。
pub fn listener_pid(port: u16) -> Option<i32> {
    holders(port)
        .into_iter()
        .find(|h| h.protos.contains(&"tcp"))
        .map(|h| h.pid)
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet, HashMap};

    /// TCP_LISTEN 在 /proc/net/tcp 的 `st` 列里是 0A。
    const TCP_LISTEN: &str = "0A";

    /// 端口上所有被占着的 socket inode，连带它属于哪个协议。
    pub fn bound_inodes(port: u16) -> HashMap<u64, &'static str> {
        let mut out = HashMap::new();
        // UDP 不筛状态：UDP 没有 LISTEN 一说，bind 上了就占着这个端口。
        for (path, proto, listen_only) in [
            ("/proc/net/tcp", "tcp", true),
            ("/proc/net/tcp6", "tcp", true),
            ("/proc/net/udp", "udp", false),
            ("/proc/net/udp6", "udp", false),
        ] {
            let Ok(content) = std::fs::read_to_string(path) else {
                continue;
            };
            for line in content.lines().skip(1) {
                // 格式（空格分隔，前十列的布局 tcp 与 udp 一致）：
                //   sl  local_address rem_address st ... uid timeout inode ...
                //    0: 0100007F:1F90 00000000:0000 0A  ... 1000      0  12345
                let f: Vec<&str> = line.split_whitespace().collect();
                if f.len() < 10 {
                    continue;
                }
                if listen_only && f[3] != TCP_LISTEN {
                    continue;
                }
                // local_address 是 HEX_IP:HEX_PORT，只取端口。IP 不比对：
                // 同一个端口可能被 0.0.0.0 或某个具体网卡监听，调用方问的是「端口被谁占了」。
                let Some((_, hex_port)) = f[1].rsplit_once(':') else {
                    continue;
                };
                let Ok(p) = u16::from_str_radix(hex_port, 16) else {
                    continue;
                };
                if p != port {
                    continue;
                }
                // inode 0 是没有属主的 socket（TIME_WAIT 之类），既反查不到进程，
                // 也不该让 is_taken 报出一个谁都腾不掉的占用。
                match f[9].parse::<u64>() {
                    Ok(inode) if inode != 0 => {
                        out.insert(inode, proto);
                    }
                    _ => {}
                }
            }
        }
        out
    }

    pub fn holders(port: u16) -> Vec<Holder> {
        let inodes = bound_inodes(port);
        if inodes.is_empty() {
            return Vec::new();
        }

        // 反查哪个进程持有这些 inode：扫 /proc/<pid>/fd/*，符号链接指向 "socket:[12345]"。
        let Ok(entries) = std::fs::read_dir("/proc") else {
            return Vec::new();
        };
        let mut found: BTreeMap<i32, BTreeSet<&'static str>> = BTreeMap::new();
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Ok(pid) = name.parse::<i32>() else {
                continue; // /proc 下还有 self、net、meminfo 这些非数字项
            };

            let Ok(fds) = std::fs::read_dir(format!("/proc/{pid}/fd")) else {
                continue; // 别人的进程（非 root 时读不到），或刚好退出了
            };
            for fd in fds.flatten() {
                let Ok(target) = std::fs::read_link(fd.path()) else {
                    continue;
                };
                let Some(inode) = socket_inode(target.to_str().unwrap_or_default()) else {
                    continue;
                };
                if let Some(proto) = inodes.get(&inode) {
                    found.entry(pid).or_default().insert(proto);
                }
            }
        }

        // BTreeMap / BTreeSet 顺带把顺序定死了：PID 升序、协议字母序。
        found
            .into_iter()
            .map(|(pid, protos)| Holder {
                pid,
                comm: comm_of(pid),
                protos: protos.into_iter().collect(),
            })
            .collect()
    }

    /// `socket:[12345]` → 12345。指向别的东西的 fd 返回 None。
    fn socket_inode(target: &str) -> Option<u64> {
        target
            .strip_prefix("socket:[")?
            .strip_suffix(']')?
            .parse()
            .ok()
    }

    /// 进程名。读不到就给空串——进程可能正好在这一瞬退出了。
    fn comm_of(pid: i32) -> String {
        std::fs::read_to_string(format!("/proc/{pid}/comm"))
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::common::port::{is_taken, listener_pid};
        use std::net::{TcpListener, UdpSocket};

        fn 我() -> i32 {
            unsafe { libc::getpid() }
        }

        /// 只挑出自己那条。TCP 和 UDP 的端口空间是分开的，同一个端口号上可能还压着
        /// 别人的 socket，断言整张表的长度会偶发地挂。
        fn 自己那条(port: u16) -> Holder {
            holders(port)
                .into_iter()
                .find(|h| h.pid == 我())
                .expect("自己持有的 socket 必须能从 /proc 反查到")
        }

        #[test]
        fn 能查到自己监听的端口对应的_pid() {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            assert_eq!(listener_pid(port), Some(我()));
        }

        #[test]
        fn 占用者带着进程名和协议() {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let h = 自己那条(port);
            assert_eq!(h.protos, ["tcp"]);
            // 进程名是 port-kill 列表里唯一能让人认出「这是什么」的一列，
            // 空着的话这条命令就只剩一串数字。
            assert!(!h.comm.is_empty(), "进程名不该是空的");
        }

        #[test]
        fn udp_绑着也算占了端口() {
            let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
            let port = sock.local_addr().unwrap().port();
            assert_eq!(自己那条(port).protos, ["udp"]);
            // UDP 没有「监听」一说，listener_pid 不该把它算进来。
            assert_ne!(listener_pid(port), Some(我()));
        }

        #[test]
        fn 没人监听的端口查不到() {
            // 先 bind 再 drop，拿一个几乎确定空闲的端口号。
            let port = {
                let l = TcpListener::bind("127.0.0.1:0").unwrap();
                l.local_addr().unwrap().port()
            };
            assert_eq!(listener_pid(port), None);
            assert!(holders(port).is_empty());
            assert!(!is_taken(port));
        }

        #[test]
        fn 占着的端口判得出来() {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            assert!(is_taken(port));
        }

        #[test]
        fn 只认_socket_类型的_fd() {
            assert_eq!(socket_inode("socket:[12345]"), Some(12345));
            assert_eq!(socket_inode("/dev/null"), None);
            assert_eq!(socket_inode("anon_inode:[eventpoll]"), None);
            assert_eq!(socket_inode("socket:[不是数字]"), None);
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod lsof {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};
    use std::process::Command;

    /// macOS 没有 /proc，退回 lsof。这是唯一还需要外部命令的平台分支。
    pub fn holders(port: u16) -> Vec<Holder> {
        let mut found: BTreeMap<i32, (String, BTreeSet<&'static str>)> = BTreeMap::new();
        scan(
            &[format!("-iTCP:{port}"), "-sTCP:LISTEN".to_string()],
            "tcp",
            &mut found,
        );
        scan(&[format!("-iUDP:{port}")], "udp", &mut found);
        found
            .into_iter()
            .map(|(pid, (comm, protos))| Holder {
                pid,
                comm,
                protos: protos.into_iter().collect(),
            })
            .collect()
    }

    /// `-F pc` 让 lsof 一行输出一个字段：`p<pid>`、`c<进程名>`。
    ///
    /// 不用 `-t`（那个只给 pid）：进程名得从这儿拿，列表里没有进程名就只剩一串数字。
    fn scan(
        args: &[String],
        proto: &'static str,
        out: &mut BTreeMap<i32, (String, BTreeSet<&'static str>)>,
    ) {
        // 什么都没匹配到时 lsof 的退出码是 1，不能拿它当失败看，读 stdout 就行。
        let Ok(o) = Command::new("lsof")
            .args(["-nP", "-Fpc"])
            .args(args)
            .output()
        else {
            return;
        };
        let text = String::from_utf8_lossy(&o.stdout);
        let mut pid = None;
        for line in text.lines() {
            if let Some(v) = line.strip_prefix('p') {
                pid = v.trim().parse::<i32>().ok();
                if let Some(p) = pid {
                    out.entry(p).or_default().1.insert(proto);
                }
            } else if let Some(v) = line.strip_prefix('c') {
                if let Some(p) = pid {
                    out.entry(p).or_default().0 = v.trim().to_string();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn 监听中的端口被判为在监听() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(is_listening("127.0.0.1", port));
    }

    #[test]
    fn 空闲端口被判为未监听() {
        let port = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        assert!(!is_listening("127.0.0.1", port));
    }

    #[test]
    fn 通配地址被换成回环再连() {
        // 服务配置里写 0.0.0.0 很常见。直接拿它当目的地址去 connect 在部分平台上
        // 必然失败，于是 start 之后的「等端口起来」会永远超时——服务明明起来了，
        // fundeploy 却报启动失败。这里验证 0.0.0.0 被正确换成了 127.0.0.1。
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(is_listening("0.0.0.0", port));
        assert!(is_listening("", port));
    }

    #[test]
    fn 等待已在监听的端口立刻返回() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let t0 = Instant::now();
        assert!(wait_for_listener("127.0.0.1", port, Duration::from_secs(5)));
        assert!(t0.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn 等待空闲端口会超时() {
        let port = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        assert!(!wait_for_listener(
            "127.0.0.1",
            port,
            Duration::from_millis(600)
        ));
    }

    #[test]
    fn 干等的时候会按时招呼调用方() {
        // 守的是「屏幕上有东西在动」: 这一等最长到半分钟, 回调不被调用就等于
        // spinner 不转, 跟卡死没有区别。
        let port = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let mut ticks = 0usize;
        let ok =
            wait_for_listener_ticking("127.0.0.1", port, Duration::from_millis(700), || ticks += 1);
        assert!(!ok);
        assert!(ticks >= 1, "一次都没招呼，spinner 不会转: {ticks}");
    }

    #[test]
    fn 端口已经在听就不必招呼() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut ticks = 0usize;
        assert!(wait_for_listener_ticking(
            "127.0.0.1",
            port,
            Duration::from_secs(5),
            || ticks += 1
        ));
        // 立刻就返回了，没有「等」这回事，也就不该闪一下 spinner。
        assert_eq!(ticks, 0);
    }
}
