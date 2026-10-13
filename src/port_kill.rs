//! 腾端口：查出谁占着某个端口，然后把它停掉。
//!
//! 「占着」的判定见 [`crate::common::port::holders`]——只认监听中的 TCP 和已绑定的
//! UDP，不认出站连接。

use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{anyhow, bail, Result};

use crate::common::port::{self, Holder};
use crate::common::proc;
use crate::ui;

/// SIGTERM 之后等多久再上 SIGKILL。
///
/// 占着端口的多半是 web 服务，正常退出要等在途请求收尾；几秒钟就 KILL 会把它们
/// 一起掐断，而这条命令的典型场景（腾个端口重启自己的服务）完全等得起。
const GRACE: Duration = Duration::from_secs(15);

/// 无人值守下跳过确认的环境变量。
const ENV_YES: &str = "FUNDEPLOY_PORT_KILL_YES";

/// 只看不动。
pub fn list(ports: &[u16]) -> Result<()> {
    show(&scan(ports));
    Ok(())
}

/// 停掉占着这些端口的进程。
pub fn kill(ports: &[u16], yes: bool) -> Result<()> {
    let found = scan(ports);
    show(&found);

    let mut targets: Vec<(u16, &Holder)> = Vec::new();
    for (&port, hs) in &found {
        for h in hs {
            match protected(h.pid) {
                Some(why) => ui::warn(format!("跳过 {} ({})：{why}。", h.pid, name(h))),
                None => targets.push((port, h)),
            }
        }
    }

    if !targets.is_empty() {
        ui::blank();
        if !yes {
            ui::confirm_destructive(&format!("要结束上面这 {} 个进程吗", targets.len()), ENV_YES)?;
        }
        for (port, h) in &targets {
            ui::step(format!("停 {} ({})，占着 {port}", h.pid, name(h)));
            // 绝不按进程组发信号：这些 PID 是从端口反查出来的，它们的进程组里
            // 很可能坐着调用者自己的 shell。
            proc::stop_pid(h.pid, GRACE, false)?;
        }
    }

    // 复查端口而不是复查 PID：进程没了不等于端口腾出来了。同一个端口上还可能压着
    // 别的用户的进程，非 root 时上面那张表根本看不见它。
    let stuck: Vec<String> = found
        .keys()
        .filter(|&&p| port::is_taken(p))
        .map(u16::to_string)
        .collect();
    if !stuck.is_empty() {
        bail!(
            "端口 {} 还占着。占用者多半不属于当前用户，加 sudo 再跑一次。",
            stuck.join("、")
        );
    }
    ui::ok(format!("端口 {} 都空着了。", joined(&found)));
    Ok(())
}

/// 把手输的一行端口拆成数字。空格、逗号（半角全角）、顿号都算分隔符。
pub fn parse_ports(line: &str) -> Result<Vec<u16>> {
    let mut out = Vec::new();
    for tok in line.split([' ', '\t', ',', '，', '、']) {
        let tok = tok.trim();
        if tok.is_empty() {
            continue;
        }
        let p: u16 = tok
            .parse()
            .map_err(|_| anyhow!("{tok} 不是合法端口号（1-65535）"))?;
        if p == 0 {
            bail!("端口号不能是 0");
        }
        out.push(p);
    }
    if out.is_empty() {
        bail!("没给端口号");
    }
    Ok(out)
}

/// 查一遍每个端口上有谁。
///
/// 用 BTreeMap 顺带把 `port-kill 8080 8080` 这种重复参数收成一份，否则第二遍会
/// 对着已经杀掉的 PID 再走一轮超时等待。
fn scan(ports: &[u16]) -> BTreeMap<u16, Vec<Holder>> {
    ports.iter().map(|&p| (p, port::holders(p))).collect()
}

fn show(found: &BTreeMap<u16, Vec<Holder>>) {
    println!(
        "{}{}{}协议",
        ui::pad("端口", 8),
        ui::pad("PID", 9),
        ui::pad("进程", 22)
    );
    for (port, hs) in found {
        if hs.is_empty() {
            // 看不见占用者不等于没人占：非 root 时读不到别人的 /proc/<pid>/fd。
            let what = if port::is_taken(*port) {
                "有人占着，但看不出是谁（试试 sudo）"
            } else {
                "没人占"
            };
            println!("{}{what}", ui::pad(&port.to_string(), 8));
            continue;
        }
        for (i, h) in hs.iter().enumerate() {
            // 同一个端口的多个进程只在头一行写端口号，剩下的留空，一眼能看出是一组。
            let head = if i == 0 {
                port.to_string()
            } else {
                String::new()
            };
            println!(
                "{}{}{}{}",
                ui::pad(&head, 8),
                ui::pad(&h.pid.to_string(), 9),
                ui::pad(&name(h), 22),
                h.protos.join("/")
            );
        }
    }
}

/// 不能动的 PID。
fn protected(pid: i32) -> Option<&'static str> {
    if pid <= 1 {
        // 1 号进程是 init，容器里把它停了整个容器就没了。
        return Some("是 init");
    }
    if pid == std::process::id() as i32 {
        // fundeploy 自己占着这个端口确实少见，但真撞上时信号发出去，后面几个端口
        // 就无声无息地没处理了，连「失败了」都报不出来。
        return Some("就是 fundeploy 自己");
    }
    None
}

fn name(h: &Holder) -> String {
    if h.comm.is_empty() {
        "?".to_string()
    } else {
        h.comm.clone()
    }
}

fn joined(found: &BTreeMap<u16, Vec<Holder>>) -> String {
    found
        .keys()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join("、")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn 拆端口号() {
        assert_eq!(parse_ports("8080").unwrap(), [8080]);
        assert_eq!(parse_ports(" 8080  8081 ").unwrap(), [8080, 8081]);
        assert_eq!(parse_ports("8080,8081").unwrap(), [8080, 8081]);
        // 中文输入法下顺手打出来的全角标点也认，不然报一句「不是合法端口号」
        // 让人盯着看起来一模一样的字符发呆。
        assert_eq!(parse_ports("8080，8081、8082").unwrap(), [8080, 8081, 8082]);
    }

    #[test]
    fn 不是端口号的东西要报错() {
        assert!(parse_ports("").is_err());
        assert!(parse_ports("  ").is_err());
        assert!(parse_ports("abc").is_err());
        assert!(parse_ports("0").is_err());
        assert!(parse_ports("70000").is_err());
        // 一个坏的就整条拒掉，不能默默只处理前半截。
        assert!(parse_ports("8080 abc").is_err());
    }

    #[test]
    fn 报错里带上那个看不懂的词() {
        let err = parse_ports("8080 八千").unwrap_err().to_string();
        assert!(err.contains("八千"), "实际: {err}");
    }

    #[test]
    fn 重复的端口只算一次() {
        let found = scan(&[8080, 8080, 9000]);
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn 自己占的端口不会被自己杀掉() {
        // 没有 protected 这道闸，这条测试会把测试进程本身干掉。
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let err = kill(&[port], true).unwrap_err().to_string();
        assert!(err.contains(&port.to_string()), "实际: {err}");
        assert!(port::is_taken(port), "端口不该被腾掉");
        drop(listener);
    }

    #[test]
    fn init_和自己都在保护名单里() {
        assert!(protected(1).is_some());
        assert!(protected(0).is_some());
        assert!(protected(std::process::id() as i32).is_some());
        assert!(protected(2).is_none());
    }

    #[test]
    fn 空闲端口不用动也不报错() {
        let port = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        list(&[port]).unwrap();
        kill(&[port], true).unwrap();
    }
}
