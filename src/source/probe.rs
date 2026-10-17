//! 并发量一批地址的响应延迟。
//!
//! 量的是**首字节时间**，不是下完整个响应的时间：镜像索引动辄几十 MB
//! （Homebrew 的 formula.jws.json 是 34 MB），下完再比等于把带宽测试和
//! 可达性测试混成一件事，还得等上半分钟。收到响应头就够判断「这个站活着、
//! 到它有多远」了。
//!
//! 并发是必须的：十来个源串行量，每个最坏等满超时，一次 `test` 要一分钟。

use std::time::{Duration, Instant};

use super::Mirror;
use crate::common::par;

/// 一次探测的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// 回了 2xx/3xx，用了这么久。
    Ok(Duration),
    /// 连上了但回的不是成功状态。常见是镜像站把这个仓库下线了（404）。
    Status(u16),
    /// 连不上：DNS 解析不了、拒绝连接、超时。
    Unreachable,
}

impl Outcome {
    /// 排序用的键。不可达的返回 None。
    pub fn millis(&self) -> Option<u128> {
        match self {
            Outcome::Ok(d) => Some(d.as_millis()),
            _ => None,
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Outcome::Ok(d) => format!("{} ms", d.as_millis()),
            Outcome::Status(c) => format!("HTTP {c}"),
            Outcome::Unreachable => "连不上".to_string(),
        }
    }
}

pub struct Timing {
    pub mirror: Mirror,
    pub outcome: Outcome,
}

/// 量一遍。返回的顺序和 `urls` 一一对应。
pub fn race(urls: &[String], timeout: Duration, jobs: usize) -> Vec<Outcome> {
    // Agent 建一次就够，内部是 Arc。每个 URL 新建一个等于把 rustls 的 ClientConfig
    // （含根证书集）重新装配十几遍——量到的那个数字里就掺进了这部分本机开销，而这条
    // 命令的全部意义是比较各个源之间的差别。
    let agent = build_agent(timeout);
    par::map(urls, jobs, |url| one(&agent, url))
}

fn build_agent(timeout: Duration) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(timeout)
        .timeout(timeout)
        // 不跟重定向：跟过去量到的是另一个站的延迟，而配置里写的还是这一个。
        .redirects(0)
        .user_agent(concat!("fundeploy/", env!("CARGO_PKG_VERSION")))
        .build()
}

fn one(agent: &ureq::Agent, url: &str) -> Outcome {
    let t0 = Instant::now();
    match agent.get(url).call() {
        // 响应体不读：`call()` 拿到响应头就返回，这里直接丢掉连接。
        Ok(_) => Outcome::Ok(t0.elapsed()),
        // 3xx 在 redirects(0) 下也走这个分支，它同样说明站是活的。
        Err(ureq::Error::Status(code, _)) if (300..400).contains(&code) => {
            Outcome::Ok(t0.elapsed())
        }
        Err(ureq::Error::Status(code, _)) => Outcome::Status(code),
        Err(ureq::Error::Transport(_)) => Outcome::Unreachable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::{Shutdown, TcpListener, TcpStream};

    /// 起一个只会回固定响应的极简 HTTP 服务，返回它的地址。
    ///
    /// 不打真实镜像站：测试得能在断网的机器和 CI 里跑过，而且量到的数字
    /// 不该随别人的机房状况变。
    fn 假服务(响应: &'static str, 条数: usize) -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for _ in 0..条数 {
                let Ok((mut s, _)) = l.accept() else { break };
                吃掉请求(&s);
                let _ = s.write_all(响应.as_bytes());
                let _ = s.flush();
                // 显式半关闭，让客户端看到一个正常收尾的响应。
                let _ = s.shutdown(Shutdown::Write);
            }
        });
        format!("http://127.0.0.1:{port}/")
    }

    /// 回响应之前必须先把请求读完。
    ///
    /// 收缓冲区里还压着没读的数据时，close() 在 macOS 上发的是 RST 而不是 FIN，
    /// 连已经写出去的响应一起作废：客户端看到的是「连接被重置」，而不是我们想让
    /// 它看到的那个状态码。Linux 的回环上同样的写法碰巧能过，于是这类问题只在
    /// Mac 上现形。
    fn 吃掉请求(s: &TcpStream) {
        let mut r = BufReader::new(s);
        let mut line = String::new();
        // 读到空行为止——测试里的请求都没有 body。
        while r.read_line(&mut line).is_ok_and(|n| n > 0) {
            if line == "\r\n" || line == "\n" {
                break;
            }
            line.clear();
        }
    }

    #[test]
    fn 二百算可达且量到了时间() {
        let url = 假服务(
            "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 2\r\n\r\nhi",
            1,
        );
        let r = race(&[url], Duration::from_secs(5), 4);
        assert!(matches!(r[0], Outcome::Ok(_)), "实际: {:?}", r[0]);
    }

    #[test]
    fn 三百零一算可达() {
        // 镜像站用 301 把 /simple 指到 /simple/ 很常见，判成不可达的话
        // 一个好源会被排到最后。
        let url = 假服务(
            "HTTP/1.1 301 Moved Permanently\r\nConnection: close\r\nLocation: /x\r\nContent-Length: 0\r\n\r\n",
            1,
        );
        let r = race(&[url], Duration::from_secs(5), 4);
        assert!(matches!(r[0], Outcome::Ok(_)), "实际: {:?}", r[0]);
    }

    #[test]
    fn 四百零四算不可用并带上状态码() {
        let url = 假服务(
            "HTTP/1.1 404 Not Found\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
            1,
        );
        let r = race(&[url], Duration::from_secs(5), 4);
        assert_eq!(r[0], Outcome::Status(404));
        assert_eq!(r[0].describe(), "HTTP 404");
        assert_eq!(r[0].millis(), None);
    }

    #[test]
    fn 没人监听的端口算连不上() {
        let port = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let r = race(
            &[format!("http://127.0.0.1:{port}/")],
            Duration::from_secs(2),
            4,
        );
        assert_eq!(r[0], Outcome::Unreachable);
    }

    #[test]
    fn 结果顺序和输入一一对应() {
        // 并发跑完按完成顺序收结果的话，这里会错位——于是「最快的那个」
        // 配出去的是另一个源的地址。
        let ok = 假服务(
            "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
            2,
        );
        let bad = 假服务(
            "HTTP/1.1 404 Not Found\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
            2,
        );
        let urls = vec![bad.clone(), ok.clone(), bad, ok];
        let r = race(&urls, Duration::from_secs(5), 4);
        assert_eq!(r[0], Outcome::Status(404));
        assert!(matches!(r[1], Outcome::Ok(_)));
        assert_eq!(r[2], Outcome::Status(404));
        assert!(matches!(r[3], Outcome::Ok(_)));
    }

    #[test]
    fn 并发数比地址多也不会卡住() {
        let url = 假服务(
            "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
            1,
        );
        let r = race(&[url], Duration::from_secs(5), 32);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn 空列表直接返回() {
        assert!(race(&[], Duration::from_secs(1), 8).is_empty());
    }

    #[test]
    fn 可达的排在不可达前面() {
        let a = Outcome::Ok(Duration::from_millis(900));
        let b = Outcome::Status(404);
        let c = Outcome::Unreachable;
        let mut v = [&c, &b, &a];
        v.sort_by_key(|o| o.millis().unwrap_or(u128::MAX));
        assert_eq!(v[0], &a);
    }
}
