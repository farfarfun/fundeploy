//! Python 包的安装与查询。
//!
//! 这里要处理的核心难点是**同一个包可能被装成两份**：一份由 `uv tool`
//! 装在工具 venv 里，一份由 `pip` 装在某个解释器的 site-packages 里。两份并存时
//! 实际跑哪个完全由 PATH 顺序决定，于是会出现「升级成功、行为没变」——升的是
//! 没人跑的那份。所以这一层的所有操作都围绕一个原则：
//!
//! **安装统一用 `uv pip install`，已有 CLI 就继续装进它所在的解释器。**
//! 旧的 `uv tool` 环境保留，只在查询来源和卸载时识别它。

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

use super::cmd;

/// 已有安装的来源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// 旧的 `uv tool` 独立工具 venv。
    UvTool,
    /// 常规 pip 路线（可能经由 uv pip，也可能是解释器自带的 pip）。
    Pip,
}

/// 一个用 Python 分发的包，带它提供的 CLI 名。
#[derive(Debug, Clone)]
pub struct PipPkg {
    pkg: String,
    cli: String,
    /// 显式指定的 pip 可执行文件。给了就只走它，不再探测。
    explicit_pip: Option<String>,
}

impl PipPkg {
    pub fn new(pkg: impl Into<String>, cli: impl Into<String>) -> Self {
        Self {
            pkg: pkg.into(),
            cli: cli.into(),
            explicit_pip: None,
        }
    }

    pub fn with_explicit_pip(mut self, bin: Option<&str>) -> Self {
        self.explicit_pip = bin.filter(|s| !s.is_empty()).map(str::to_string);
        self
    }

    pub fn pkg(&self) -> &str {
        &self.pkg
    }

    pub fn cli(&self) -> &str {
        &self.cli
    }

    /// 识别已有安装，供状态查询和卸载使用。
    pub fn mode(&self) -> Mode {
        if self.explicit_pip.is_some() {
            return Mode::Pip;
        }
        if !cmd::has("uv") {
            return Mode::Pip;
        }
        if self.uv_tool_version().is_some() {
            return Mode::UvTool;
        }
        Mode::Pip
    }

    /// 拼安装 spec。`version` 为 None 表示最新版。
    pub fn spec(&self, version: Option<&str>) -> String {
        match version.filter(|v| !v.is_empty()) {
            Some(v) => format!("{}=={}", self.pkg, v.trim_start_matches('v')),
            None => self.pkg.clone(),
        }
    }

    /// 用 uv pip 安装；不钉版本时加 -U，指定版本也支持回退。
    pub fn install(&self, version: Option<&str>) -> Result<()> {
        let spec = self.spec(version);
        let pinned = version.is_some_and(|v| !v.is_empty());

        // 已有安装时钉住它所在的解释器：`uv pip` 自己探测出来的环境未必是同一个，
        // 装进别的环境等于升级了一份没人跑的副本。
        let uv = cmd::require("uv", "可先运行 `fundeploy dev uv install`")?;
        let py = if let Some(pip) = &self.explicit_pip {
            let head = read_first_line(Path::new(pip))
                .with_context(|| format!("读不到 pip 入口的解释器: {pip}"))?;
            Some(parse_shebang_python(&head).context("pip 入口没有绝对路径解释器")?)
        } else {
            self.cli_python()
        };
        let mut args = vec![uv, "pip".into(), "install".into(), spec];
        if let Some(p) = py {
            args.push("--python".into());
            args.push(p.to_string_lossy().into_owned());
        }
        if !pinned {
            args.push("-U".into());
        }
        cmd::run(args)
    }

    /// 卸。返回是否确实执行了一次成功的卸载。
    pub fn uninstall(&self) -> bool {
        let ok = |r: Result<super::cmd::Output>| r.map(|o| o.success()).unwrap_or(false);

        if self.mode() == Mode::UvTool {
            return ok(cmd::try_run(["uv", "tool", "uninstall", &self.pkg]));
        }
        if let Some(pip) = &self.explicit_pip {
            return ok(cmd::try_run([pip, "uninstall", "-y", &self.pkg]));
        }
        if let Some(py) = self.cli_python() {
            let py = py.to_string_lossy().into_owned();
            if cmd::has("uv") {
                return ok(cmd::try_run([
                    "uv",
                    "pip",
                    "uninstall",
                    "--python",
                    &py,
                    &self.pkg,
                ]));
            }
            return ok(cmd::try_run([
                &py,
                "-m",
                "pip",
                "uninstall",
                "-y",
                &self.pkg,
            ]));
        }
        if let Some(py3) = cmd::find("python3") {
            return ok(cmd::try_run([
                &py3,
                "-m",
                "pip",
                "uninstall",
                "-y",
                &self.pkg,
            ]));
        }
        false
    }

    /// `uv tool` 管理的那份的版本。没有则 None。
    pub fn uv_tool_version(&self) -> Option<String> {
        if !cmd::has("uv") {
            return None;
        }
        let out = cmd::try_run(["uv", "tool", "list"]).ok()?;
        if !out.success() {
            return None;
        }
        parse_uv_tool_list(&out.stdout, &self.pkg)
    }

    /// PATH 上那个 CLI 实际由哪个解释器跑。
    ///
    /// console script 的 shebang 就是答案——`uv tool` 装的指向工具 venv，
    /// pip 装的指向对应 venv。拿不到解释器路径时返回 None。
    pub fn cli_python(&self) -> Option<PathBuf> {
        let cli = cmd::find(&self.cli)?;
        let head = read_first_line(Path::new(&cli))?;
        parse_shebang_python(&head)
    }

    /// PATH 上实际会跑到的那份的版本。
    ///
    /// 问解释器要包元数据，而不是跑 `--version`：后者由上游自己实现，
    /// 格式不统一，而且有些包根本没有 `--version`。
    pub fn version_on_path(&self) -> Option<String> {
        let py = self.cli_python()?;
        let script = format!(
            "import importlib.metadata as m\n\
             try: print(m.version({pkg:?}))\n\
             except Exception: pass\n",
            pkg = self.pkg
        );
        let out = cmd::try_run([&*py.to_string_lossy(), "-c", &script]).ok()?;
        let v = out.stdout.trim();
        (!v.is_empty()).then(|| v.to_string())
    }

    /// 优先报告实际运行和升级的那份；CLI 不在 PATH 时兼容旧工具环境。
    pub fn managed_version(&self) -> Option<String> {
        self.version_on_path().or_else(|| self.uv_tool_version())
    }

    /// 装完核对 CLI 是否可用，以及是否存在不同版本的旧工具环境。
    ///
    /// 返回 Some(警告文案) 表示检出了问题。两种情况：
    /// CLI 根本不在 PATH 上，或者 PATH 上那份和旧工具环境的版本不一致。
    pub fn shadow_warning(&self) -> Option<String> {
        let Some(cli_path) = cmd::find(&self.cli) else {
            return Some(format!(
                "{pkg} 已安装，但 {cli} 不在 PATH 上。\n\
                 请激活安装时使用的虚拟环境，或把它的 bin 目录加进 PATH。",
                pkg = self.pkg,
                cli = self.cli,
            ));
        };

        let managed = self.uv_tool_version()?;
        let on_path = self.version_on_path()?;
        if managed == on_path {
            return None;
        }
        Some(format!(
            "uv tool 环境里的 {pkg} 是 {managed}，但 PATH 上的 {cli_path} 是 {on_path}。\n\
             机器上有第二份安装把它遮住了，先卸掉多余的那份（或调整 PATH）再重试。",
            pkg = self.pkg,
        ))
    }
}

/// 读文件第一行。读不到（文件不存在、是二进制、无权限）返回 None。
fn read_first_line(path: &Path) -> Option<String> {
    use std::io::{BufRead, BufReader};
    let f = std::fs::File::open(path).ok()?;
    let mut line = String::new();
    BufReader::new(f).read_line(&mut line).ok()?;
    Some(line)
}

/// 从 shebang 里取解释器的绝对路径。
///
/// `#!/usr/bin/env python3` 这种壳形式定位不到具体环境，当探测失败处理——
/// 它只告诉你「去 PATH 里找 python3」，而那个 python3 未必是装了这个包的那个。
fn parse_shebang_python(line: &str) -> Option<PathBuf> {
    let rest = line.trim_end().strip_prefix("#!")?;
    let rest = rest.trim();
    if !rest.starts_with('/') || rest.contains(char::is_whitespace) {
        return None;
    }
    Some(PathBuf::from(rest))
}

/// 从 `uv tool list` 的输出里取某个包的版本。
///
/// 输出形如：
/// ```text
/// funread-api v1.1.116
/// - funread-api
/// ruff v0.6.1
/// - ruff
/// ```
/// 包名行之后缩进的 `- xxx` 是该工具提供的可执行文件，不是版本行。
fn parse_uv_tool_list(stdout: &str, pkg: &str) -> Option<String> {
    for line in stdout.lines() {
        let mut it = line.split_whitespace();
        let (Some(name), Some(ver)) = (it.next(), it.next()) else {
            continue;
        };
        if name != pkg {
            continue;
        }
        if let Some(v) = ver.strip_prefix('v') {
            return Some(v.to_string());
        }
    }
    None
}

/// 要求 Python 侧的安装手段至少有一个可用。
pub fn require_any_installer() -> Result<()> {
    if cmd::has("uv") {
        return Ok(());
    }
    bail!("未找到 uv（可先运行 `fundeploy dev uv install`）")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(name: &str) -> PipPkg {
        PipPkg::new(name, name)
    }

    #[test]
    fn spec_在不钉版本时就是包名() {
        assert_eq!(p("funread-api").spec(None), "funread-api");
        assert_eq!(p("funread-api").spec(Some("")), "funread-api");
    }

    #[test]
    fn spec_钉版本时用双等号且剥掉_v_前缀() {
        assert_eq!(
            p("funread-api").spec(Some("1.1.116")),
            "funread-api==1.1.116"
        );
        assert_eq!(
            p("funread-api").spec(Some("v1.1.116")),
            "funread-api==1.1.116"
        );
    }

    #[test]
    fn 显式_pip_强制走_pip_路线() {
        // 即使机器上有 uv，显式指定 pip 也不能改道——否则会在用户点名的
        // 那个环境之外另装一份。
        let pkg = p("foo").with_explicit_pip(Some("/opt/venv/bin/pip"));
        assert_eq!(pkg.mode(), Mode::Pip);
    }

    #[test]
    fn 空字符串的显式_pip_当作没给() {
        let pkg = p("foo").with_explicit_pip(Some(""));
        assert!(pkg.explicit_pip.is_none());
    }

    #[test]
    fn 解析绝对路径的_shebang() {
        assert_eq!(
            parse_shebang_python("#!/home/u/.local/share/uv/tools/funread-api/bin/python\n"),
            Some(PathBuf::from(
                "/home/u/.local/share/uv/tools/funread-api/bin/python"
            ))
        );
    }

    #[test]
    fn env_形式的_shebang_当探测失败() {
        // 它只说「去 PATH 找 python3」，而那个 python3 未必装了这个包。
        assert_eq!(parse_shebang_python("#!/usr/bin/env python3\n"), None);
    }

    #[test]
    fn 非_shebang_的首行返回_none() {
        assert_eq!(parse_shebang_python("import sys\n"), None);
        assert_eq!(parse_shebang_python(""), None);
        assert_eq!(parse_shebang_python("#!python\n"), None); // 不是绝对路径
    }

    #[test]
    fn shebang_末尾的回车被处理() {
        // Windows 换行写进来的脚本。
        assert_eq!(
            parse_shebang_python("#!/usr/bin/python3\r\n"),
            Some(PathBuf::from("/usr/bin/python3"))
        );
    }

    #[test]
    fn 解析_uv_tool_list() {
        let out = "funread-api v1.1.116\n- funread-api\nruff v0.6.1\n- ruff\n";
        assert_eq!(
            parse_uv_tool_list(out, "funread-api"),
            Some("1.1.116".into())
        );
        assert_eq!(parse_uv_tool_list(out, "ruff"), Some("0.6.1".into()));
    }

    #[test]
    fn uv_tool_list_里没有的包返回_none() {
        let out = "ruff v0.6.1\n- ruff\n";
        assert_eq!(parse_uv_tool_list(out, "funread-api"), None);
    }

    #[test]
    fn uv_tool_list_的可执行文件行不会被当成版本() {
        // `- funread-api` 这行第一列是 `-`，不该匹配到任何包名；
        // 若实现把它当版本行，会把 `funread-api` 的版本读成空。
        let out = "- funread-api\nfunread-api v1.1.116\n";
        assert_eq!(
            parse_uv_tool_list(out, "funread-api"),
            Some("1.1.116".into())
        );
        assert_eq!(parse_uv_tool_list(out, "-"), None);
    }

    #[test]
    fn uv_tool_list_空输出不会_panic() {
        assert_eq!(parse_uv_tool_list("", "foo"), None);
        assert_eq!(parse_uv_tool_list("\n\n", "foo"), None);
    }
}
