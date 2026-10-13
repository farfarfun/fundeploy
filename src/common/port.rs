//! 端口探测：谁在监听、等它监听起来。
//!
//! Linux 上监听表直接读 /proc，不依赖 lsof/ss——容器镜像里这两个常常都没装，
//! 而 `ss` 的输出格式在不同 iproute2 版本间也变过。macOS 没有 /proc，退回 lsof。

use std::collections::HashSet;
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

/// 轮询等端口起来。返回是否在 `timeout` 内等到了。
///
/// 服务刚 spawn 出来时端口还没 bind，调用方不能立刻判定失败——这个间隔在装了
/// 一堆 Python 依赖的后端上能有十几秒（import 时间）。
pub fn wait_for_listener(host: &str, port: u16, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if is_listening(host, port) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

/// 监听指定端口的进程 PID。查不到返回 None。
///
/// 查不到有两种情况，调用方通常不需要区分：没人监听，或者监听者属于别的用户
/// （非 root 时读不到别人的 /proc/<pid>/fd，`ss -p` 在同样情况下也打不出 pid）。
pub fn listener_pid(port: u16) -> Option<i32> {
    #[cfg(target_os = "linux")]
    {
        linux::listener_pid(port)
    }
    #[cfg(not(target_os = "linux"))]
    {
        lsof::listener_pid(port)
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;

    /// TCP_LISTEN 在 /proc/net/tcp 的 `st` 列里是 0A。
    const TCP_LISTEN: &str = "0A";

    /// 从 /proc/net/tcp 与 /proc/net/tcp6 里收集所有在 `port` 上 LISTEN 的 socket inode。
    fn listening_inodes(port: u16) -> HashSet<u64> {
        let mut out = HashSet::new();
        for path in ["/proc/net/tcp", "/proc/net/tcp6"] {
            let Ok(content) = std::fs::read_to_string(path) else {
                continue;
            };
            for line in content.lines().skip(1) {
                // 格式（空格分隔，列数固定）：
                //   sl  local_address rem_address st ... uid timeout inode ...
                //    0: 0100007F:1F90 00000000:0000 0A  ... 1000      0  12345
                let f: Vec<&str> = line.split_whitespace().collect();
                if f.len() < 10 {
                    continue;
                }
                if f[3] != TCP_LISTEN {
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
                if let Ok(inode) = f[9].parse::<u64>() {
                    out.insert(inode);
                }
            }
        }
        out
    }

    pub fn listener_pid(port: u16) -> Option<i32> {
        let inodes = listening_inodes(port);
        if inodes.is_empty() {
            return None;
        }

        // 反查哪个进程持有这些 inode：扫 /proc/<pid>/fd/*，符号链接指向 "socket:[12345]"。
        let entries = std::fs::read_dir("/proc").ok()?;
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
                let Some(t) = target.to_str() else { continue };
                let Some(rest) = t.strip_prefix("socket:[") else {
                    continue;
                };
                let Some(num) = rest.strip_suffix(']') else {
                    continue;
                };
                if let Ok(inode) = num.parse::<u64>() {
                    if inodes.contains(&inode) {
                        return Some(pid);
                    }
                }
            }
        }
        None
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::net::TcpListener;

        #[test]
        fn 能查到自己监听的端口对应的_pid() {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let me = unsafe { libc::getpid() };

            assert_eq!(
                listener_pid(port),
                Some(me),
                "自己持有的监听 socket 必须能从 /proc 反查到"
            );
        }

        #[test]
        fn 收集到的_inode_非空且端口匹配() {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            assert!(!listening_inodes(port).is_empty());
        }

        #[test]
        fn 没人监听的端口查不到() {
            // 先 bind 再 drop，拿一个几乎确定空闲的端口号。
            let port = {
                let l = TcpListener::bind("127.0.0.1:0").unwrap();
                l.local_addr().unwrap().port()
            };
            assert_eq!(listener_pid(port), None);
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod lsof {
    /// macOS 没有 /proc，退回 lsof。这是唯一还需要外部命令的平台分支。
    pub fn listener_pid(port: u16) -> Option<i32> {
        let out = std::process::Command::new("lsof")
            .args([&format!("-tiTCP:{port}"), "-sTCP:LISTEN", "-n", "-P"])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .next()?
            .trim()
            .parse()
            .ok()
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
}
