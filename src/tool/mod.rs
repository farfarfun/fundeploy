//! 工具层。
//!
//! 跟服务的分界线只有一条：**工具没有常驻进程**。装上就在那儿，没有起、停、
//! 重启、「端口起来了吗」可言，所以动作只有四个——装、升、卸、查——外加各自
//! 声明的额外动作（[`Extra`]）。
//!
//! 把没有进程的东西塞进 [`crate::service::Service`] 的代价是四个撒谎的方法：
//! `start` 报成功而什么都没起，`status` 答「已停止」而它压根不是跑的东西。
//! 少一个 trait 省不下多少代码，却要让每个使用者先学会哪些动作是假的。

pub mod brew;
pub mod cockpit_tools;
pub mod open_pencil;
pub mod registry;

use anyhow::Result;

pub use crate::Extra;

/// 一个工具能做的事。
pub trait Tool {
    fn name(&self) -> &str;
    fn summary(&self) -> &str;

    /// 安装。
    ///
    /// `version` 来自命令行的 `--version`。钉不了版本的工具（跟着系统包管理器
    /// 走的那些）应当在收到 `Some` 时明确报错，而不是装完最新版了事——「我指定
    /// 了版本，它装了别的」比「这个工具不支持指定版本」难查得多。
    fn install(&self, version: Option<&str>) -> Result<()>;
    fn upgrade(&self) -> Result<()>;
    fn uninstall(&self) -> Result<()>;
    fn status(&self) -> Result<()>;

    /// 这个工具额外支持的动作。
    fn extras(&self) -> &'static [Extra] {
        &[]
    }

    /// 执行一个额外动作。`args` 是动作名之后的全部参数。
    ///
    /// 名字由命令行对着 [`Tool::extras`] 核过才会传进来，走到默认实现只说明声明
    /// 和实现对不上，那是编程错误。
    fn extra(&self, name: &str, args: &[String]) -> Result<()> {
        let _ = args;
        anyhow::bail!("{} 声明了动作 {name} 却没有实现它", self.name())
    }
}

/// 卸载确认的跳过开关。
fn uninstall_env_var(name: &str) -> String {
    format!(
        "FUNDEPLOY_{}_UNINSTALL_YES",
        name.to_ascii_uppercase().replace(['-', '.'], "_")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 卸载确认的环境变量名由工具名推出() {
        assert_eq!(
            uninstall_env_var("open-pencil"),
            "FUNDEPLOY_OPEN_PENCIL_UNINSTALL_YES"
        );
    }

    #[test]
    fn 注册表里没有重名工具() {
        let names: Vec<String> = registry::all()
            .iter()
            .map(|t| t.name().to_string())
            .collect();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "有重名: {names:?}");
    }

    #[test]
    fn 按名字能找到工具() {
        for name in registry::names() {
            assert!(registry::find(&name).is_some(), "找不到 {name}");
        }
        assert!(registry::find("__不存在的工具__").is_none());
    }

    #[test]
    fn 工具名和服务名不重合() {
        // 两个域各有一张表，重名的话 `fundeploy tool x` 和 `fundeploy service x`
        // 指的是两样东西，而使用者多半以为是同一样。
        let services = crate::service::registry::names();
        for t in registry::names() {
            assert!(!services.contains(&t), "{t} 既是工具又是服务");
        }
    }

    #[test]
    fn 声明的额外动作名字不重样() {
        for t in registry::all() {
            let mut names: Vec<&str> = t.extras().iter().map(|e| e.name).collect();
            let total = names.len();
            names.sort_unstable();
            names.dedup();
            assert_eq!(names.len(), total, "{} 的额外动作有重名", t.name());
        }
    }
}
