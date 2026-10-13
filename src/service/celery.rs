//! Celery：worker + beat + Flower。
//!
//! 三个进程里只有 Flower 听端口，另外两个的死活只能看进程本身——这也是这一族
//! 把「端口」做成可选的原因。

use anyhow::Result;
use std::path::PathBuf;

use super::venv::{Proc, Venv, VenvSpec, PROC_EXTRAS};
use super::Extra;
use crate::common::{cmd, env, port};
use crate::ui;

const DEFAULT_BROKER: &str = "redis://localhost:6379/0";
/// 脚手架应用的导入路径。`CELERY_APP` 还是这个值，就说明用户没有自己的应用。
const SCAFFOLD_APP: &str = "celery_app:app";
const DEFAULT_FLOWER_PORT: u16 = 8816;

const SCAFFOLD: &str = r#""""由 fundeploy 生成的最小 Celery 应用。

换成自己的应用：把 CELERY_APP 指到你的模块，例如
CELERY_APP=myproj.tasks:app
"""
import os

from celery import Celery

broker = os.environ.get("CELERY_BROKER_URL", "redis://localhost:6379/0")
backend = os.environ.get("CELERY_RESULT_BACKEND", broker)

app = Celery("celery_app", broker=broker, backend=backend)


@app.task
def add(x, y):
    return x + y
"#;

pub struct Celery {
    spec: VenvSpec,
    broker: String,
    backend: String,
    app: String,
    etc: PathBuf,
    /// 用的是脚手架应用，不是用户自己的。
    scaffold: bool,
    /// Flower 的监听地址和有没有配 basic auth。没起 Flower 时为 None。
    flower: Option<(String, bool)>,
}

impl Celery {
    pub fn from_env() -> Self {
        let home = env::opt("CELERY_HOME")
            .map(|v| env::expand_tilde(&v))
            .unwrap_or_else(|| env::home().join("opt/celery"));
        let etc = home.join("etc");

        let broker = env::str_or("CELERY_BROKER_URL", DEFAULT_BROKER);
        let backend = env::str_or("CELERY_RESULT_BACKEND", &broker);
        let app = env::str_or("CELERY_APP", SCAFFOLD_APP);
        let scaffold = app == SCAFFOLD_APP;
        let loglevel = env::str_or("CELERY_LOGLEVEL", "info");

        // 用脚手架时工作目录必须是 etc/，`celery_app` 才 import 得到。
        let cwd = if scaffold { etc.clone() } else { home.clone() };

        let mut worker_args = vec![
            "-A".to_string(),
            app.clone(),
            "worker".to_string(),
            format!("--loglevel={loglevel}"),
        ];
        if let Some(n) = env::opt("CELERY_CONCURRENCY").filter(|s| !s.is_empty()) {
            worker_args.push(format!("--concurrency={n}"));
        }

        let mut procs =
            vec![Proc::new("worker", "Celery worker", "celery", cwd.clone()).args(worker_args)];

        if env::flag("CELERY_BEAT", true) {
            procs.push(
                Proc::new("beat", "Celery beat", "celery", cwd.clone()).args(vec![
                    "-A".into(),
                    app.clone(),
                    "beat".into(),
                    format!("--loglevel={loglevel}"),
                ]),
            );
        }

        let mut flower = None;
        let fport = env::port("FLOWER_PORT", DEFAULT_FLOWER_PORT);
        if env::flag("CELERY_FLOWER", true) {
            // 默认只听回环。Flower 自己不带任何认证，却把任务参数（里面经常是
            // 凭据）和 revoke/terminate 全摆出来，绑 0.0.0.0 等于把这些交给整个
            // 局域网。
            let addr = env::str_or("FLOWER_ADDRESS", "127.0.0.1");
            let auth = env::opt("FLOWER_BASIC_AUTH").filter(|s| !s.is_empty());

            let mut p = Proc::new("flower", "Flower", "celery", cwd.clone())
                .args(vec![
                    "-A".into(),
                    app.clone(),
                    "flower".into(),
                    format!("--address={addr}"),
                    format!("--port={fport}"),
                ])
                .listen(&addr, fport);

            // 凭据只走环境变量，绝不进命令行：/proc/<pid>/cmdline 对本机所有用户
            // 可读，`--basic-auth=user:pass` 等于 `ps aux` 就能抄走。Flower 原生
            // 支持用 FLOWER_<选项名> 设同名选项。
            p = match &auth {
                Some(v) => p.env("FLOWER_BASIC_AUTH", v.clone()),
                // 空值不能留着：Flower 把空串当成「配了 basic auth」然后解析失败。
                None => p.unset("FLOWER_BASIC_AUTH"),
            };

            flower = Some((addr, auth.is_some()));
            procs.push(p);
        }

        let spec = VenvSpec::new(
            "celery",
            "Celery 分布式任务队列（worker/beat/flower）",
            home,
            "celery",
            procs,
        )
        // 不装裸包：flower 2.0 前后一批选项名换过，celery 5.3 以下对 Python 3.11+
        // 的支持也不完整，解析到远古版本不会报错，只会在运行时跑偏。
        .floor("5.3")
        .deps(vec![
            env::str_or("CELERY_REDIS_SPEC", "redis>=5.0"),
            env::str_or("CELERY_FLOWER_SPEC", "flower>=2.0"),
        ])
        .python(env::opt("CELERY_PYTHON_BIN"))
        .python_range((3, 9), None)
        .dirs(&["etc", "data"])
        // 前台跑整套时 worker 的输出才是人要盯的那一份。
        .foreground("worker");

        let spec = match env::opt("CELERY_VERSION").filter(|s| !s.is_empty()) {
            Some(v) => spec.version(v),
            None => spec,
        };
        let spec = match &flower {
            Some((addr, _)) => spec.entry(format!("http://{}:{fport}", port::display_host(addr))),
            None => spec,
        };
        // 注意是脱敏过的：notes 每次 start 都会打一遍。
        let spec = spec.note(format!("broker: {}", redact_url(&broker)));
        let spec = if scaffold {
            spec.note(format!(
                "用的是脚手架应用 {}。换成自己的: CELERY_APP=你的模块:app",
                etc.join("celery_app.py").display()
            ))
        } else {
            spec
        };

        Self {
            spec,
            broker,
            backend,
            app,
            etc,
            scaffold,
            flower,
        }
    }

    /// 用脚手架时要让 `celery_app` 能被 import 到。
    fn pythonpath(&self) -> Option<String> {
        if !self.scaffold {
            return None;
        }
        let etc = self.etc.to_string_lossy().into_owned();
        // 追加而不是覆盖：操作者可能已经把自己的任务包放在 PYTHONPATH 上了。
        Some(match env::opt("PYTHONPATH").filter(|s| !s.is_empty()) {
            Some(p) => format!("{etc}:{p}"),
            None => etc,
        })
    }

    fn write_scaffold(&self) -> Result<()> {
        let file = self.etc.join("celery_app.py");
        if file.exists() {
            return Ok(());
        }
        std::fs::create_dir_all(&self.etc)?;
        std::fs::write(&file, SCAFFOLD)?;
        ui::info(format!("已生成脚手架 {}", file.display()));
        Ok(())
    }
}

impl Venv for Celery {
    fn spec(&self) -> &VenvSpec {
        &self.spec
    }

    fn env(&self) -> Vec<cmd::EnvEdit> {
        let mut e = vec![
            ("CELERY_BROKER_URL".to_string(), Some(self.broker.clone())),
            (
                "CELERY_RESULT_BACKEND".to_string(),
                Some(self.backend.clone()),
            ),
            ("CELERY_APP".to_string(), Some(self.app.clone())),
        ];
        if let Some(p) = self.pythonpath() {
            e.push(("PYTHONPATH".to_string(), Some(p)));
        }
        e
    }

    fn ensure_config(&self) -> Result<()> {
        if self.scaffold {
            self.write_scaffold()?;
        }

        // broker 不通只是警告：worker 起来之后会自己重连，而且 broker 常常是另一台
        // 机器上刚要起的服务，这里拦住反而挡了正常流程。
        match broker_endpoint(&self.broker) {
            Some((host, port)) if !port::is_listening(&host, port) => ui::warn(format!(
                "broker {} 连不上；worker 会起来但一直在重连。",
                redact_url(&self.broker)
            )),
            _ => {}
        }

        if let Some((addr, has_auth)) = &self.flower {
            if port::is_public(addr) && !has_auth {
                ui::warn(format!(
                    "Flower 监听 {addr} 且没设 FLOWER_BASIC_AUTH：任务参数和 revoke/terminate 对这个网络完全开放。"
                ));
                ui::hint("加上认证: FLOWER_BASIC_AUTH=user:pass fundeploy service celery start");
            }
        }
        Ok(())
    }

    fn extra_status(&self) -> Result<()> {
        ui::blank();
        ui::heading("连接");
        ui::kv("broker", Some(&redact_url(&self.broker)));
        ui::kv("backend", Some(&redact_url(&self.backend)));
        ui::kv("app", Some(&self.app));
        Ok(())
    }

    fn extras(&self) -> &'static [Extra] {
        PROC_EXTRAS
    }

    fn extra(&self, name: &str, args: &[String]) -> Result<()> {
        match self.proc_extra(name, args) {
            Some(r) => r,
            None => anyhow::bail!("celery 不认识动作 {name}"),
        }
    }
}

/// 只留 `scheme://host:port`。
///
/// broker 和 backend 的地址里常常带口令（`redis://:pass@host/0`），而 status 的
/// 输出经常被整段贴进工单和聊天窗口。
fn redact_url(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return "（已配置）".into();
    };
    // 先砍掉 path/query/fragment，再砍掉 userinfo。
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit('@').next().unwrap_or("");
    if host_port.is_empty() {
        return "（已配置）".into();
    }
    format!("{scheme}://{host_port}")
}

/// 从 broker 地址里抠出 host 和 port，抠不出来就不探测。
fn broker_endpoint(url: &str) -> Option<(String, u16)> {
    let (scheme, rest) = url.split_once("://")?;
    let default_port = match scheme {
        "redis" | "rediss" => 6379,
        "amqp" => 5672,
        "amqps" => 5671,
        _ => return None,
    };
    let authority = rest.split(['/', '?', '#']).next()?;
    let host_port = authority.rsplit('@').next()?;

    // IPv6 字面量写成 [::1]:6379。
    if let Some(tail) = host_port.strip_prefix('[') {
        let (host, after) = tail.split_once(']')?;
        let port = after
            .strip_prefix(':')
            .and_then(|p| p.parse().ok())
            .unwrap_or(default_port);
        return (!host.is_empty()).then(|| (host.to_string(), port));
    }
    let (host, port) = match host_port.rsplit_once(':') {
        Some((h, p)) => (h, p.parse().ok()?),
        None => (host_port, default_port),
    };
    (!host.is_empty()).then(|| (host.to_string(), port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 地址里的口令不会被打出来() {
        // 这条守的是「status 的输出被贴进工单」这个常规动作。
        assert_eq!(
            redact_url("redis://:s3cret@10.0.0.2:6379/0"),
            "redis://10.0.0.2:6379"
        );
        assert_eq!(
            redact_url("amqp://user:pw@rabbit:5672/vhost"),
            "amqp://rabbit:5672"
        );
        assert_eq!(
            redact_url("redis://localhost:6379/0"),
            "redis://localhost:6379"
        );
        assert_eq!(redact_url("什么都不是"), "（已配置）");
        assert_eq!(redact_url("redis://"), "（已配置）");
    }

    #[test]
    fn 从_broker_地址里抠出主机和端口() {
        assert_eq!(
            broker_endpoint("redis://localhost:6379/0"),
            Some(("localhost".into(), 6379))
        );
        // 没写端口就按协议的默认端口探。
        assert_eq!(
            broker_endpoint("redis://localhost/0"),
            Some(("localhost".into(), 6379))
        );
        assert_eq!(
            broker_endpoint("amqp://u:p@rabbit/vhost"),
            Some(("rabbit".into(), 5672))
        );
        assert_eq!(
            broker_endpoint("redis://[::1]:6380/0"),
            Some(("::1".into(), 6380))
        );
        // 不认识的协议不探测，总比对着错的地址报「连不上」强。
        assert_eq!(broker_endpoint("sqs://whatever"), None);
        assert_eq!(broker_endpoint("redis://"), None);
    }

    #[test]
    fn 只有_flower_占端口() {
        let c = Celery::from_env();
        let with_port: Vec<&str> = c
            .spec
            .procs
            .iter()
            .filter(|p| p.port().is_some())
            .map(|p| p.key.as_str())
            .collect();
        assert_eq!(with_port, vec!["flower"]);
    }

    #[test]
    fn 前台跑的是_worker() {
        // 不指定的话默认是最后一个声明的进程，那会变成 flower——前台跑整套时人要
        // 盯的是 worker 的输出。
        let c = Celery::from_env();
        assert_eq!(c.spec.foreground.as_deref(), Some("worker"));
    }
}
