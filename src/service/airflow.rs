//! Apache Airflow。
//!
//! 只对 Airflow 3.x 负责。3 和 2 之间 web 层、认证层、DAG 的写法全换过一遍，
//! 想同时伺候两代的话每个动作都得先分支一次，不如把界限划清楚。

use anyhow::{anyhow, bail, Context, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::venv::{Proc, Venv, VenvSpec};
use super::Extra;
use crate::common::{cmd, env, port};
use crate::ui;

const DEFAULT_VERSION: &str = "3.1.8";
const DEFAULT_PORT: u16 = 8815;

/// Airflow 3 把 web 层从 webserver 换成了 api-server，端口也跟着从
/// `[webserver] web_server_port` 挪到了 `[api] port`。老名字在 3.x 里不再被读取——
/// 设了不会报错，服务只是照样听在默认的 8080 上，而外面还以为改成功了。
const PORT_VAR: &str = "AIRFLOW__API__PORT";
const HOST_VAR: &str = "AIRFLOW__API__HOST";

/// Airflow 3 默认用 SimpleAuthManager，在它底下 `airflow users` 这组子命令根本没有
/// 注册，报的是「没有这个命令」而不是「功能未启用」。要管用户就得切到 FAB。
const FAB_AUTH_MANAGER: &str = "airflow.providers.fab.auth_manager.fab_auth_manager.FabAuthManager";

const EXTRAS: &[Extra] = &[
    Extra {
        name: "dag-scaffold",
        about: "在 dags/ 下写一个最小可跑的示例 DAG",
    },
    Extra {
        name: "dags-list",
        about: "列出所有 DAG",
    },
    Extra {
        name: "trigger",
        about: "触发一次 DAG: trigger <dag_id> [airflow 的其它参数]",
    },
    Extra {
        name: "task-test",
        about: "单测一个 task: task-test <dag_id> <task_id> <logical_date>",
    },
    Extra {
        name: "users-create",
        about: "创建用户。不给参数就逐项询问，口令不回显",
    },
    Extra {
        name: "users-list",
        about: "列出用户",
    },
    Extra {
        name: "users-reset-password",
        about: "重置口令: users-reset-password <username>",
    },
    Extra {
        name: "http-trigger",
        about: "走 REST API 触发 DAG: http-trigger <dag_id> [payload.json]",
    },
];

const SCAFFOLD: &str = r#"# 由 fundeploy 生成的最小示例，确认调度链路通了就可以删掉。
from datetime import datetime

from airflow.sdk import DAG
from airflow.providers.standard.operators.bash import BashOperator

with DAG(
    dag_id="example_minimal",
    start_date=datetime(2024, 1, 1),
    schedule=None,
    catchup=False,
    tags=["fundeploy"],
) as dag:
    BashOperator(task_id="hello", bash_command="echo hello from airflow")
"#;

pub struct Airflow {
    spec: VenvSpec,
    api_base: String,
}

impl Airflow {
    pub fn from_env() -> Self {
        let home = env::opt("AIRFLOW_HOME")
            .map(|v| env::expand_tilde(&v))
            .unwrap_or_else(|| env::home().join("opt/airflow"));

        // 端口有两个来源：Airflow 自己的配置变量和我们的简写。先认前者，不然操作者
        // 明明 export 了 AIRFLOW__API__PORT，我们却守着另一个端口等它监听。
        let port = env::opt(PORT_VAR)
            .and_then(|v| v.parse::<u16>().ok())
            .unwrap_or_else(|| env::port("AIRFLOW_PORT", DEFAULT_PORT));
        let host = env::opt(HOST_VAR).unwrap_or_else(|| env::str_or("AIRFLOW_HOST", "127.0.0.1"));
        let version = env::str_or("AIRFLOW_VERSION", DEFAULT_VERSION);

        let standalone = Proc::new("standalone", "Airflow standalone", "airflow", home.clone())
            .args(vec!["standalone".into()])
            .listen(&host, port)
            .env(HOST_VAR, host.clone())
            .env(PORT_VAR, port.to_string())
            // 示例 DAG 会在列表里盖掉自己的，默认不要。
            .env(
                "AIRFLOW__CORE__LOAD_EXAMPLES",
                env::str_or("AIRFLOW__CORE__LOAD_EXAMPLES", "False"),
            )
            // standalone 是「一台机器上跑整套」的形态，默认并发按单机调小，
            // 免得一个 DAG 把机器打满。
            .env(
                "AIRFLOW__CORE__PARALLELISM",
                env::str_or("AIRFLOW__CORE__PARALLELISM", "4"),
            )
            .env(
                "AIRFLOW__CORE__MAX_ACTIVE_TASKS_PER_DAG",
                env::str_or("AIRFLOW__CORE__MAX_ACTIVE_TASKS_PER_DAG", "4"),
            )
            .env(
                "AIRFLOW__CORE__MAX_ACTIVE_RUNS_PER_DAG",
                env::str_or("AIRFLOW__CORE__MAX_ACTIVE_RUNS_PER_DAG", "2"),
            )
            .env(
                "AIRFLOW__API__WORKERS",
                env::str_or("AIRFLOW__API__WORKERS", "2"),
            );

        let spec = VenvSpec::new(
            "airflow",
            "Apache Airflow 工作流调度（standalone）",
            home,
            "apache-airflow",
            vec![standalone],
        )
        .version(version)
        .python(env::opt("AIRFLOW_PYTHON_BIN"))
        // 下限是 Airflow 3.1 自己要求的；上限是因为比支持范围新的解释器装不上
        // （轮子没发、依赖编译不过），跳过它去找个旧的比半路报错强。
        .python_range((3, 10), Some((3, 13)))
        .deps(vec![env::str_or(
            "AIRFLOW_FAB_SPEC",
            "apache-airflow-providers-fab",
        )])
        // Airflow 的依赖树很深，不照着官方约束清单装几乎必然解析出一组跑不起来的
        // 组合。清单是按 (airflow 版本, python 版本) 发的。
        .constraints(Box::new(|v, (major, minor)| {
            format!(
                "https://raw.githubusercontent.com/apache/airflow/constraints-{v}/constraints-{major}.{minor}.txt"
            )
        }))
        .dirs(&["dags", "logs", "data", "etc"])
        // 第一次启动要初始化元数据库再把 api-server、scheduler、triggerer、
        // dag-processor 都拉起来，30 秒远远不够。
        .start_timeout(Duration::from_secs(env::num("AIRFLOW_START_TIMEOUT_SEC", 180)))
        .entry(format!("http://{}:{port}", port::display_host(&host)));

        let dags = spec.home.join("dags");
        let spec = spec
            .note(format!("DAG 放在 {}", dags.display()))
            .note("还没有用户的话: fundeploy service airflow users-create");

        let api_base = env::str_or(
            "AIRFLOW_API_BASE",
            &format!("http://{}:{port}", port::display_host(&host)),
        );

        Self { spec, api_base }
    }

    /// 实际要用的 auth manager。
    fn auth_manager(&self) -> Option<String> {
        // 人自己指定了就听他的，别拿我们的推断去盖。
        if let Some(v) = env::opt("AIRFLOW__CORE__AUTH_MANAGER") {
            return Some(v);
        }
        self.fab_installed().then(|| FAB_AUTH_MANAGER.to_string())
    }

    /// FAB provider 装了没。
    ///
    /// 看目录在不在，不用 `python -c "import airflow.providers.fab"`：后者会把整个
    /// airflow 连带它的依赖 import 一遍，好几秒；而这个判断每跑一条 airflow 命令
    /// 之前都要做一次。
    fn fab_installed(&self) -> bool {
        site_packages(&self.spec.venv)
            .is_some_and(|sp| sp.join("airflow").join("providers").join("fab").is_dir())
    }

    fn require_fab(&self) -> Result<()> {
        if self.fab_installed() {
            return Ok(());
        }
        bail!(
            "用户管理要 apache-airflow-providers-fab，当前 venv 里没有。\n\
             重新装一次: fundeploy service airflow install"
        )
    }

    /// 跑 `airflow <head...> <rest...>`。
    fn cli(&self, head: &[&str], rest: &[String]) -> Result<()> {
        self.require_installed()?;
        let mut argv: Vec<&str> = head.to_vec();
        argv.extend(rest.iter().map(|s| s.as_str()));
        self.exec("airflow", &argv)
    }

    fn dag_scaffold(&self) -> Result<()> {
        let dir = self.spec.home.join("dags");
        std::fs::create_dir_all(&dir).with_context(|| format!("建目录失败: {}", dir.display()))?;
        let file = dir.join("example_minimal.py");
        if file.exists() {
            ui::info(format!("{} 已存在，没有覆盖。", file.display()));
            return Ok(());
        }
        std::fs::write(&file, SCAFFOLD).with_context(|| format!("写不了 {}", file.display()))?;
        ui::ok(format!("已生成 {}", file.display()));
        ui::hint("等调度器扫到（默认几十秒）之后: fundeploy service airflow dags-list");
        Ok(())
    }

    fn users_create(&self, args: &[String]) -> Result<()> {
        self.require_installed()?;
        self.require_fab()?;
        if !args.is_empty() {
            return self.cli(&["users", "create"], args);
        }

        let username = ui::input("用户名", "admin")?;
        if username.is_empty() {
            bail!("用户名不能为空。");
        }
        let firstname = ui::input("名", "Admin")?;
        let lastname = ui::input("姓", "User")?;
        let email = ui::input("邮箱", &format!("{username}@example.com"))?;
        let role = ui::input("角色", "Admin")?;
        let password = ui::password("口令")?;
        if password.is_empty() {
            bail!("口令不能为空。");
        }

        // 口令最终还是会出现在 airflow 进程的 argv 里——上游这条命令没有从 stdin
        // 读口令的口子。交互至少把它挡在了 shell 历史之外。
        self.exec(
            "airflow",
            &[
                "users",
                "create",
                "--username",
                &username,
                "--firstname",
                &firstname,
                "--lastname",
                &lastname,
                "--email",
                &email,
                "--role",
                &role,
                "--password",
                &password,
            ],
        )
    }

    fn http_trigger(&self, args: &[String]) -> Result<()> {
        let dag = args.first().filter(|s| !s.is_empty()).ok_or_else(|| {
            anyhow!("用法: fundeploy service airflow http-trigger <dag_id> [payload.json]")
        })?;
        let username = env::opt("AIRFLOW_API_USERNAME")
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("要先设 AIRFLOW_API_USERNAME 和 AIRFLOW_API_PASSWORD。"))?;
        let password = env::opt("AIRFLOW_API_PASSWORD")
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("要先设 AIRFLOW_API_PASSWORD。"))?;

        let body: serde_json::Value = match args.get(1) {
            Some(path) => {
                let text =
                    std::fs::read_to_string(path).with_context(|| format!("读不了 {path}"))?;
                serde_json::from_str(&text).with_context(|| format!("{path} 不是合法 JSON"))?
            }
            None => serde_json::json!({ "conf": {} }),
        };

        let base = self.api_base.trim_end_matches('/');
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(30))
            .build();

        ui::step(format!("向 {base} 取令牌"));
        let tok = post_json(
            &agent,
            &format!("{base}/auth/token"),
            None,
            serde_json::json!({ "username": username, "password": password }),
        )?;
        let token = tok["access_token"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("{base}/auth/token 没回 access_token"))?;

        ui::step(format!("触发 {dag}"));
        let resp = post_json(
            &agent,
            &format!("{base}/api/v2/dags/{}/dagRuns", encode_segment(dag)),
            Some(token),
            body,
        )?;
        ui::info(serde_json::to_string_pretty(&resp).unwrap_or_else(|_| resp.to_string()));
        Ok(())
    }
}

impl Venv for Airflow {
    fn spec(&self) -> &VenvSpec {
        &self.spec
    }

    fn env(&self) -> Vec<cmd::EnvEdit> {
        let mut e = vec![(
            "AIRFLOW_HOME".to_string(),
            Some(self.spec.home.to_string_lossy().into_owned()),
        )];
        if let Some(am) = self.auth_manager() {
            e.push(("AIRFLOW__CORE__AUTH_MANAGER".to_string(), Some(am)));
        }
        e
    }

    fn post_install(&self) -> Result<()> {
        ui::step("初始化/迁移元数据库");
        self.exec("airflow", &["db", "migrate"])?;

        // FAB 的表不在主库里，3.x 要单独迁一次。迁不动只影响用户管理那一块，
        // 把整个安装判为失败不合适。
        if self.fab_installed() {
            ui::step("迁移 FAB 认证库");
            if !matches!(self.try_exec("airflow", &["fab-db", "migrate"])?, Some(0)) {
                ui::warn("airflow fab-db migrate 没成功，用户管理可能用不了。");
            }
        }

        // 三个都给齐才建。少一个就当没打算在安装阶段建用户——这时候半自动地去问，
        // 会把一次无人值守的安装卡在提示符上。
        if let (Some(user), Some(pass), Some(mail)) = (
            env::opt("AIRFLOW_ADMIN_USER"),
            env::opt("AIRFLOW_ADMIN_PASSWORD"),
            env::opt("AIRFLOW_ADMIN_EMAIL"),
        ) {
            self.require_fab()?;
            ui::step(format!("创建管理员 {user}"));
            let code = self.try_exec(
                "airflow",
                &[
                    "users",
                    "create",
                    "--role",
                    "Admin",
                    "--username",
                    &user,
                    "--password",
                    &pass,
                    "--firstname",
                    "Admin",
                    "--lastname",
                    "User",
                    "--email",
                    &mail,
                ],
            )?;
            if !matches!(code, Some(0)) {
                ui::warn("创建管理员没成功（多半是这个用户名已经存在）。");
            }
        }
        Ok(())
    }

    fn extra_status(&self) -> Result<()> {
        let bin = self.spec.bin("airflow");
        if !bin.is_file() {
            return Ok(());
        }
        ui::blank();
        ui::heading("调度器");

        let bin = bin.to_string_lossy().into_owned();
        let vars = self.env();
        let mut out = cmd::try_run_with(
            [
                bin.as_str(),
                "jobs",
                "check",
                "--job-type",
                "SchedulerJob",
                "--local",
            ],
            &vars,
        )?;
        if !out.success() {
            // --local 只认本机起的那份，换了 executor 这个参数会直接被拒。
            out = cmd::try_run_with([bin.as_str(), "jobs", "check"], &vars)?;
        }

        let text = if out.stdout.trim().is_empty() {
            out.stderr.trim()
        } else {
            out.stdout.trim()
        };
        let last = text.lines().last().unwrap_or("（没有输出）");
        if out.success() {
            ui::info(last);
        } else {
            ui::warn(format!("调度器没通过检查: {last}"));
        }
        Ok(())
    }

    fn extras(&self) -> &'static [Extra] {
        EXTRAS
    }

    fn extra(&self, name: &str, args: &[String]) -> Result<()> {
        match name {
            "dag-scaffold" => self.dag_scaffold(),
            "dags-list" => self.cli(&["dags", "list"], args),
            "trigger" => {
                if args.is_empty() {
                    bail!("用法: fundeploy service airflow trigger <dag_id> [...]");
                }
                self.cli(&["dags", "trigger"], args)
            }
            "task-test" => {
                if args.len() < 3 {
                    bail!("用法: fundeploy service airflow task-test <dag_id> <task_id> <logical_date>");
                }
                self.cli(&["tasks", "test"], args)
            }
            "users-create" => self.users_create(args),
            "users-list" => {
                self.require_fab()?;
                self.cli(&["users", "list"], args)
            }
            "users-reset-password" => {
                self.require_fab()?;
                if args.is_empty() {
                    bail!("用法: fundeploy service airflow users-reset-password <username>");
                }
                self.cli(&["users", "reset-password", "--username"], args)
            }
            "http-trigger" => self.http_trigger(args),
            _ => bail!("airflow 不认识动作 {name}"),
        }
    }
}

/// venv 里的 site-packages。
fn site_packages(venv: &Path) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(venv.join("lib"))
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("python"))
        })
        .collect();
    dirs.sort();
    dirs.pop().map(|p| p.join("site-packages"))
}

/// URL 路径片段的百分号编码。DAG id 里出现空格、斜杠、中文都是合法的。
fn encode_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn post_json(
    agent: &ureq::Agent,
    url: &str,
    token: Option<&str>,
    body: serde_json::Value,
) -> Result<serde_json::Value> {
    let mut req = agent.post(url).set("Content-Type", "application/json");
    if let Some(t) = token {
        req = req.set("Authorization", &format!("Bearer {t}"));
    }
    match req.send_json(body) {
        Ok(r) => r
            .into_json()
            .with_context(|| format!("{url} 回的不是 JSON")),
        // 正文里才是真原因（哪个字段不对、DAG 不存在、口令错），只报状态码没法排查。
        Err(ureq::Error::Status(code, r)) => {
            let text = r.into_string().unwrap_or_default();
            bail!("{url} 回 HTTP {code}: {}", text.trim())
        }
        Err(e) => bail!("请求 {url} 失败: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dag_id_里的特殊字符要编码() {
        assert_eq!(encode_segment("my_dag-1.0"), "my_dag-1.0");
        assert_eq!(encode_segment("a b"), "a%20b");
        // 不编码的话这里会多出一层路径，打到完全不同的接口上。
        assert_eq!(encode_segment("a/b"), "a%2Fb");
        assert_eq!(encode_segment("中"), "%E4%B8%AD");
    }

    #[test]
    fn 约束地址按_airflow_和_python_版本拼() {
        let f = Airflow::from_env();
        let url = (f.spec.constraints.as_ref().unwrap())("3.1.8", (3, 12));
        assert_eq!(
            url,
            "https://raw.githubusercontent.com/apache/airflow/constraints-3.1.8/constraints-3.12.txt"
        );
    }

    #[test]
    fn 声明里只有一个进程且它听着端口() {
        let f = Airflow::from_env();
        assert_eq!(f.spec.procs.len(), 1);
        assert!(f.spec.procs[0].port().is_some());
    }
}
