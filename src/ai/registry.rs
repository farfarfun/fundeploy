//! AI 命令行工具声明。

use super::{Ai, Leftover, Official};

pub fn all() -> Vec<Ai> {
    vec![claude(), codex(), cursor()]
}

pub fn names() -> Vec<String> {
    all().iter().map(|t| t.name.to_string()).collect()
}

pub fn find(name: &str) -> Option<Ai> {
    let want = match name {
        "claude-code" => "claude",
        "openai-codex" => "codex",
        "cursor-agent" | "agent" => "cursor",
        other => other,
    };
    all().into_iter().find(|t| t.name == want)
}

fn claude() -> Ai {
    Ai {
        name: "claude",
        summary: "Claude Code",
        bins: &["claude"],
        version_args: &["--version"],
        official: Some(Official {
            url_env: "FUNDEPLOY_CLAUDE_INSTALL_URL",
            url: "https://claude.ai/install.sh",
            self_update: &[],
            leftovers: &[
                Leftover {
                    base_env: "",
                    base: ".local/bin",
                    rel: "claude",
                },
                Leftover {
                    base_env: "",
                    base: ".local/share",
                    rel: "claude",
                },
            ],
        }),
        package: Some("@anthropic-ai/claude-code"),
        cask: Some("claude-code"),
    }
}

fn codex() -> Ai {
    Ai {
        name: "codex",
        summary: "OpenAI Codex",
        bins: &["codex"],
        version_args: &["--version"],
        official: Some(Official {
            url_env: "FUNDEPLOY_CODEX_INSTALL_URL",
            url: "https://chatgpt.com/codex/install.sh",
            self_update: &[],
            leftovers: &[
                Leftover {
                    base_env: "CODEX_INSTALL_DIR",
                    base: ".local/bin",
                    rel: "codex",
                },
                Leftover {
                    base_env: "CODEX_INSTALL_DIR",
                    base: ".local/bin",
                    rel: "codex-code-mode-host",
                },
                Leftover {
                    base_env: "CODEX_HOME",
                    base: ".codex",
                    rel: "packages/standalone",
                },
            ],
        }),
        package: Some("@openai/codex"),
        cask: Some("codex"),
    }
}

fn cursor() -> Ai {
    Ai {
        name: "cursor",
        summary: "Cursor CLI",
        // 上游改过名字：老版本装出来的是 cursor-agent，新版本是 agent。
        bins: &["agent", "cursor-agent"],
        version_args: &["--version"],
        official: Some(Official {
            url_env: "FUNDEPLOY_CURSOR_INSTALL_URL",
            url: "https://cursor.com/install",
            self_update: &["update"],
            leftovers: &[
                Leftover {
                    base_env: "",
                    base: ".local/bin",
                    rel: "agent",
                },
                Leftover {
                    base_env: "",
                    base: ".local/bin",
                    rel: "cursor-agent",
                },
                Leftover {
                    base_env: "",
                    base: ".local/share",
                    rel: "cursor-agent",
                },
            ],
        }),
        // Cursor 目前只公布官方安装器，npm 上和 brew 里都没有这个 CLI。
        package: None,
        cask: None,
    }
}
