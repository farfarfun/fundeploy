//! awesome-paperclip 插件目录。
//!
//! 一张静态表加一个打印函数。它跟进程管理、安装流程都不相干，而且会随上游生态
//! 增补——放在主文件里，每加一个插件都要在一个两千行的文件中间改一笔。

use super::*;

/// awesome-paperclip 收录的插件。`pkg` 为空表示它不是 npm 包，装法见各自仓库。
pub(super) struct Plugin {
    name: &'static str,
    /// 装的时候交给 `paperclipai plugin install` 的包名。
    pub(super) pkg: &'static str,
    /// 来源仓库，装之前报出来——这是在往自己机器上装别人的代码。
    pub(super) url: &'static str,
    about: &'static str,
}

const CATALOG: &[Plugin] = &[
    Plugin {
        name: "Agent Pixels",
        pkg: "@agent-pixels/paperclip-plugin",
        url: "https://github.com/gcampton/Agent-Pixels",
        about: "像素风 agent，可自定义行为、模型、房间，还能看监控摄像头",
    },
    Plugin {
        name: "obsidian-paperclip",
        pkg: "",
        url: "https://github.com/istib/obsidian-paperclip",
        about: "Obsidian 集成：在笔记里浏览、评论、指派 Paperclip issue",
    },
    Plugin {
        name: "paperclip-aperture",
        pkg: "@tomismeta/paperclip-aperture",
        url: "https://github.com/tomismeta/paperclip-aperture",
        about: "另一种 Focus 视图，按审批、issue 活动和待人处理的事件排序",
    },
    Plugin {
        name: "paperclip-live-analytics-plugin",
        pkg: "@agent-analytics/paperclip-live-analytics-plugin",
        url: "https://github.com/Agent-Analytics/paperclip-live-analytics-plugin",
        about: "实时访客地图、仪表盘挂件和 Agent Analytics 设置页",
    },
    Plugin {
        name: "paperclip-plugin-acp",
        pkg: "paperclip-plugin-acp",
        url: "https://github.com/mvanhorn/paperclip-plugin-acp",
        about: "在聊天平台里跑 Claude Code、Codex、Gemini CLI 的 ACP 运行时",
    },
    Plugin {
        name: "paperclip-plugin-avp",
        pkg: "paperclip-plugin-avp",
        url: "https://github.com/creatorrmode-lead/paperclip-plugin-avp",
        about: "基于 Agent Veil Protocol 的信任与声誉层",
    },
    Plugin {
        name: "paperclip-plugin-chat",
        pkg: "@paperclipai/plugin-chat",
        url: "https://github.com/webprismdevin/paperclip-plugin-chat",
        about: "面向任务、agent 和工作区的交互式 AI 聊天助手",
    },
    Plugin {
        name: "paperclip-plugin-company-wizard",
        pkg: "@yesterday-ai/paperclip-plugin-company-wizard",
        url: "https://github.com/yesterday-ai/paperclip-plugin-company-wizard",
        about: "带预设的 AI 公司初始化向导",
    },
    Plugin {
        name: "paperclip-plugin-discord",
        pkg: "paperclip-plugin-discord",
        url: "https://github.com/mvanhorn/paperclip-plugin-discord",
        about: "Discord 双向集成",
    },
    Plugin {
        name: "paperclip-plugin-github-issues",
        pkg: "paperclip-plugin-github-issues",
        url: "https://github.com/mvanhorn/paperclip-plugin-github-issues",
        about: "GitHub Issues 双向同步",
    },
    Plugin {
        name: "paperclip-plugin-linear",
        pkg: "@oldharlem/paperclip-plugin-linear",
        url: "https://github.com/Oldharlem/paperclip-linear-plugin",
        about: "Linear 双向同步，带 webhook 和一个 agent 工具",
    },
    Plugin {
        name: "paperclip-plugin-slack",
        pkg: "paperclip-plugin-slack",
        url: "https://github.com/mvanhorn/paperclip-plugin-slack",
        about: "issue 生命周期事件推到 Slack",
    },
    Plugin {
        name: "paperclip-plugin-telegram",
        pkg: "paperclip-plugin-telegram",
        url: "https://github.com/mvanhorn/paperclip-plugin-telegram",
        about: "issue 生命周期事件推到 Telegram",
    },
    Plugin {
        name: "paperclip-plugin-writbase",
        pkg: "paperclip-plugin-writbase",
        url: "https://github.com/Writbase/paperclip-plugin-writbase",
        about: "Paperclip issue 与 WritBase 任务双向同步",
    },
    Plugin {
        name: "paperclip-plugin-hindsight",
        pkg: "paperclip-plugin-hindsight",
        url: "https://github.com/vectorize-io/hindsight/tree/main/hindsight-integrations/paperclip-plugin",
        about: "给 Paperclip agent 的长期记忆",
    },
];

pub(super) fn print_catalog() {
    for p in CATALOG {
        if p.pkg.is_empty() {
            ui::info(format!("{}（不是 npm 包，装法见仓库）", p.name));
        } else {
            ui::info(format!("{}（{}）", p.name, p.pkg));
        }
        ui::hint(p.about);
        ui::hint(p.url);
    }
}

/// 从清单里挑一个来装。返回 `None` 表示人取消了。
///
/// 挑选留在这个模块里，而不是把 [`CATALOG`] 导出去让调用方自己遍历：清单的形状
/// （哪些条目不是 npm 包、因此装不了）是这张表自己的事，漏掉那道过滤的后果是
/// 选单里混进一个选了就报错的条目。
pub(super) fn pick() -> Result<Option<&'static Plugin>> {
    // 不是 npm 包的条目挑不了，只在 list 里露面。
    let pickable: Vec<&Plugin> = CATALOG.iter().filter(|p| !p.pkg.is_empty()).collect();
    let labels: Vec<String> = pickable
        .iter()
        .map(|p| format!("{} - {}", p.name, p.about))
        .collect();
    let Some(picked) = ui::select("从 awesome-paperclip 挑一个插件", &labels)? else {
        return Ok(None);
    };
    let i = labels
        .iter()
        .position(|l| l == picked)
        .ok_or_else(|| anyhow!("认不出选中的插件"))?;
    Ok(Some(pickable[i]))
}

// Paperclip 身上跟插件有关的那两个动作，理由同 log.rs 顶上那段。
impl Paperclip {
    pub(super) fn plugin(&self, args: &[&str]) -> Result<()> {
        let (action, rest) = match args.split_first() {
            Some((a, rest)) => (*a, rest),
            None => ("install", &[][..]),
        };
        match action {
            "list" => {
                print_catalog();
                Ok(())
            }
            "installed" => self.must(&prepend(&["plugin", "list"], rest)),
            "install" => self.plugin_install(rest),
            other => bail!("未知的插件命令 {other}（支持 list / install / installed）"),
        }
    }

    fn plugin_install(&self, rest: &[&str]) -> Result<()> {
        let npmrc = Npmrc::write(NPM_REGISTRY);
        let npm = self.npm_env(npmrc.as_ref());
        if let Some((pkg, more)) = rest.split_first() {
            return self.must_with(&prepend(&["plugin", "install", pkg], more), &npm);
        }

        // 没给包名就从清单里挑。
        let Some(p) = plugin::pick()? else {
            return Ok(());
        };
        ui::step(format!("安装 Paperclip 插件 {}（来源: {}）", p.pkg, p.url));
        self.must_with(&["plugin", "install", p.pkg], &npm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 插件清单里的包名不重复() {
        let mut pkgs: Vec<&str> = CATALOG
            .iter()
            .map(|p| p.pkg)
            .filter(|p| !p.is_empty())
            .collect();
        let n = pkgs.len();
        pkgs.sort_unstable();
        pkgs.dedup();
        assert_eq!(pkgs.len(), n, "插件清单里有重复的包名");
        assert!(CATALOG.iter().all(|p| p.url.starts_with("https://")));
    }
}
