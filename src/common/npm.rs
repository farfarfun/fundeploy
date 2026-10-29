//! 全局 npm 包的安装与查询。
//!
//! pnpm 和 npm 各有各的全局根目录，互相看不见对方装的包。所以这一层的原则和
//! Python 侧是同一条：**先看现有的那份是谁装的，然后继续用同一个工具**。
//! 换一个工具装，结果是两个全局根下各有一份，哪份生效取决于 PATH 顺序——
//! 表现为「升级成功、行为没变」。

use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

use super::cmd;

/// 解析到的 npm 系工具。pnpm 与 npm 的全局安装子命令不同名，所以要记住是哪个。
#[derive(Debug, Clone)]
pub struct Npm {
    bin: PathBuf,
    is_pnpm: bool,
    registry_scope: Option<String>,
}

impl Npm {
    /// 机器上所有可用的 npm 系工具，按「装新东西时优先用谁」排序。
    ///
    /// **pnpm 在前**：前端包普遍用 `only-allow pnpm` 锁包管理器，裸
    /// `npm install -g` 会在 preinstall 阶段直接失败。
    fn candidates(override_bin: Option<&str>) -> Result<Vec<Self>> {
        if let Some(b) = override_bin.filter(|s| !s.is_empty()) {
            let p = PathBuf::from(b);
            if p.is_file() {
                return Ok(vec![Self::at(p)]);
            }
            // 也允许传命令名而非路径。
            return match cmd::find(b) {
                Some(found) => Ok(vec![Self::at(PathBuf::from(found))]),
                None => bail!("指定的 npm 可执行文件无效: {b}"),
            };
        }

        let found: Vec<Self> = ["pnpm", "npm"]
            .iter()
            .filter_map(|n| cmd::find(n))
            .map(|p| Self::at(PathBuf::from(p)))
            .collect();
        if found.is_empty() {
            bail!("未找到 npm/pnpm（可先运行 `fundeploy dev nodejs install`）");
        }
        Ok(found)
    }

    /// 装新东西时该用的那个。
    pub fn resolve(override_bin: Option<&str>) -> Result<Self> {
        Ok(Self::candidates(override_bin)?.remove(0))
    }

    /// 伺候某个具体的包该用哪个工具。
    ///
    /// 包已经装了，就用**装了它的那个**，哪怕按偏好本该用另一个。换工具装只会
    /// 在另一个全局根下再堆一份，而 PATH 上跑的还是老的。只有在哪儿都没装时，
    /// 才按偏好顺序挑。
    pub fn for_package(pkg: &str, override_bin: Option<&str>) -> Result<Self> {
        let mut all = Self::candidates(override_bin)?;
        let owned: Vec<_> = all.iter().map(|n| n.installed_version(pkg)).collect();
        Ok(all.remove(pick(&owned)))
    }

    /// 同一个包被两个工具各装了一份时的警告。
    ///
    /// 这种状态下 PATH 上跑哪一份是碰运气，而升级只会动其中一份。
    pub fn shadow_warning(&self, pkg: &str) -> Option<String> {
        let owners: Vec<(String, String)> = Self::candidates(None)
            .ok()?
            .iter()
            .filter_map(|n| n.installed_version(pkg).map(|v| (n.name(), v)))
            .collect();
        shadow_text(pkg, &owners)
    }

    /// 这个工具的名字（pnpm / npm）。
    pub fn name(&self) -> String {
        self.bin
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "npm".into())
    }

    fn at(bin: PathBuf) -> Self {
        let is_pnpm = bin
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|n| n.starts_with("pnpm"));
        Self {
            bin,
            is_pnpm,
            registry_scope: None,
        }
    }

    pub fn with_registry_scope(mut self, scope: Option<&str>) -> Self {
        self.registry_scope = scope.map(str::to_string);
        self
    }

    fn registry_args(&self) -> Result<Vec<String>> {
        let Some(scope) = &self.registry_scope else {
            return Ok(vec![]);
        };
        let key = format!("{scope}:registry");
        let registry = cmd::capture([self.bin_str().as_str(), "config", "get", &key])?;
        if matches!(registry.as_str(), "" | "undefined" | "null") {
            bail!("未配置 {key}，请先在 npm/pnpm 配置中设置该私有源。");
        }
        // pnpm 12 不读取 npm_config_registry，显式参数兼容 npm 和 pnpm。
        Ok(vec!["--registry".into(), registry])
    }

    pub fn bin(&self) -> &Path {
        &self.bin
    }

    pub fn is_pnpm(&self) -> bool {
        self.is_pnpm
    }

    fn bin_str(&self) -> String {
        self.bin.to_string_lossy().into_owned()
    }

    /// 装一个全局包。`spec` 形如 `pkg@1.2.3`。
    pub fn install_global(&self, spec: &str) -> Result<()> {
        let sub = if self.is_pnpm { "add" } else { "install" };
        let mut args = vec![self.bin_str(), sub.into(), "-g".into(), spec.into()];
        args.extend(self.registry_args()?);
        cmd::run(args)
    }

    /// 卸一个全局包。失败不报错，只返回 false——卸载时包本来就可能不在。
    pub fn uninstall_global(&self, pkg: &str) -> bool {
        let sub = if self.is_pnpm { "remove" } else { "uninstall" };
        cmd::try_run([self.bin_str().as_str(), sub, "-g", pkg])
            .map(|o| o.success())
            .unwrap_or(false)
    }

    /// registry 上的最新版本号。查不到返回 None。
    pub fn latest_version(&self, pkg: &str) -> Option<String> {
        let mut args = vec![self.bin_str(), "view".into(), pkg.into(), "version".into()];
        args.extend(self.registry_args().ok()?);
        let out = cmd::try_run(args).ok()?;
        if !out.success() {
            return None;
        }
        // `view` 在某些 registry 上会多打几行（比如 deprecation 警告走了 stdout），
        // 版本号是最后一行。
        out.stdout
            .lines()
            .rev()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .map(str::to_string)
    }

    /// 拼安装用的 spec。
    ///
    /// `version` 为 None（要最新版）时**先把具体版本号查出来再按精确版本装**，
    /// 不直接用 `@latest`：pnpm 的 dist-tag 解析缓存偶发会命中过期记录，装出比
    /// latest 旧的版本——即使此刻 `view` 已经能查到新版本号。查不到才退回 `@latest`。
    pub fn spec(&self, pkg: &str, version: Option<&str>) -> String {
        match version.filter(|v| !v.is_empty()) {
            Some(v) => format!("{pkg}@{}", v.trim_start_matches('v')),
            None => match self.latest_version(pkg) {
                Some(v) => format!("{pkg}@{v}"),
                None => format!("{pkg}@latest"),
            },
        }
    }

    /// 已装的全局包版本。未安装返回 None。
    pub fn installed_version(&self, pkg: &str) -> Option<String> {
        let mut args = vec![self.bin_str(), "ls".into(), "-g".into(), pkg.into()];
        if !self.is_pnpm {
            args.push("--depth=0".into());
        }
        args.push("--json".into());

        let out = cmd::try_run(&args).ok()?;
        // `ls` 在包没装时退出码非 0，但 JSON 仍然是有效的（dependencies 为空），
        // 所以这里不看退出码，直接解析。
        parse_ls_json(&out.stdout, pkg)
    }
}

/// 候选里该用第几个：已经装了这个包的那个，谁都没装就按偏好取第一个。
fn pick(owned: &[Option<String>]) -> usize {
    owned.iter().position(Option::is_some).unwrap_or(0)
}

/// 多份并存的警告文案。`owners` 是 (工具名, 版本)，少于两份时没什么好说的。
fn shadow_text(pkg: &str, owners: &[(String, String)]) -> Option<String> {
    if owners.len() < 2 {
        return None;
    }
    let list: Vec<String> = owners
        .iter()
        .map(|(tool, ver)| format!("{tool}（{ver}）"))
        .collect();
    Some(format!(
        "{pkg} 被装了不止一份: {}。\n\
         两个全局根互相看不见，PATH 上跑哪份不确定，升级也只会动其中一份。\n\
         先把多余的卸掉再继续。",
        list.join("、")
    ))
}

/// 从 `npm/pnpm ls -g --json` 的输出里取出某个包的版本。
///
/// 两种工具的输出形状不同：pnpm 返回数组（每个全局根一项），npm 返回对象。
/// 包名可能带 scope（`@scope/name`），所以必须按 JSON 解析而不是文本匹配。
fn parse_ls_json(stdout: &str, pkg: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).ok()?;

    let roots: Vec<&serde_json::Value> = match &v {
        serde_json::Value::Array(xs) => xs.iter().collect(),
        other => vec![other],
    };

    for root in roots {
        if let Some(ver) = root
            .get("dependencies")
            .and_then(|d| d.get(pkg))
            .and_then(|p| p.get("version"))
            .and_then(|s| s.as_str())
        {
            if !ver.is_empty() {
                return Some(ver.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 按文件名识别_pnpm() {
        assert!(Npm::at(PathBuf::from("/usr/bin/pnpm")).is_pnpm());
        assert!(Npm::at(PathBuf::from("/opt/node/bin/pnpm-9")).is_pnpm());
        assert!(!Npm::at(PathBuf::from("/usr/bin/npm")).is_pnpm());
    }

    #[test]
    fn 显式版本号的_v_前缀被剥掉() {
        let n = Npm::at(PathBuf::from("/usr/bin/npm"));
        assert_eq!(n.spec("foo", Some("v1.2.3")), "foo@1.2.3");
        assert_eq!(n.spec("foo", Some("1.2.3")), "foo@1.2.3");
    }

    #[test]
    fn 带_scope_的包名拼接正确() {
        let n = Npm::at(PathBuf::from("/usr/bin/npm"));
        assert_eq!(
            n.spec("@farfarfun/funread-web", Some("1.0.0")),
            "@farfarfun/funread-web@1.0.0"
        );
    }

    #[test]
    fn 解析_npm_形状的_ls_输出() {
        let json = r#"{
          "name": "globals",
          "dependencies": {
            "@farfarfun/funread-web": { "version": "1.2.3" }
          }
        }"#;
        assert_eq!(
            parse_ls_json(json, "@farfarfun/funread-web"),
            Some("1.2.3".into())
        );
    }

    #[test]
    fn 解析_pnpm_形状的_ls_输出() {
        // pnpm 把结果包在数组里，每个全局根一项。
        let json = r#"[{
          "path": "/home/u/.local/share/pnpm/global/5",
          "dependencies": {
            "@farfarfun/funread-web": { "version": "4.5.6", "from": "@farfarfun/funread-web" }
          }
        }]"#;
        assert_eq!(
            parse_ls_json(json, "@farfarfun/funread-web"),
            Some("4.5.6".into())
        );
    }

    #[test]
    fn 包没装时解析出_none() {
        assert_eq!(parse_ls_json(r#"{"dependencies":{}}"#, "foo"), None);
        assert_eq!(parse_ls_json("[]", "foo"), None);
        assert_eq!(parse_ls_json("{}", "foo"), None);
    }

    #[test]
    fn 非_json_输出不会_panic() {
        // registry 不通时 npm 会往 stdout 吐非 JSON 的报错。
        assert_eq!(parse_ls_json("", "foo"), None);
        assert_eq!(parse_ls_json("npm ERR! network timeout", "foo"), None);
    }

    #[test]
    fn 同名包在多个全局根里时取第一个找到的() {
        let json = r#"[
          {"path":"/a","dependencies":{}},
          {"path":"/b","dependencies":{"foo":{"version":"2.0.0"}}}
        ]"#;
        assert_eq!(parse_ls_json(json, "foo"), Some("2.0.0".into()));
    }

    #[test]
    fn 空版本号当作未安装() {
        let json = r#"{"dependencies":{"foo":{"version":""}}}"#;
        assert_eq!(parse_ls_json(json, "foo"), None);
    }

    #[test]
    fn 包已经装了就用装了它的那个工具() {
        // 候选顺序是 [pnpm, npm]，包在 npm 下。挑 pnpm 的话会在 pnpm 的全局根下
        // 再装一份，而 PATH 上跑的还是 npm 那份——「升级成功、行为没变」。
        assert_eq!(pick(&[None, Some("0.1.6".into())]), 1);
    }

    #[test]
    fn 哪儿都没装就按偏好取第一个() {
        assert_eq!(pick(&[None, None]), 0);
        assert_eq!(pick(&[]), 0);
    }

    #[test]
    fn 两边都装了时取偏好靠前的那个() {
        assert_eq!(pick(&[Some("1.0.0".into()), Some("2.0.0".into())]), 0);
    }

    #[test]
    fn 只装了一份不警告() {
        assert_eq!(shadow_text("foo", &[]), None);
        assert_eq!(shadow_text("foo", &[("npm".into(), "1.0.0".into())]), None);
    }

    #[test]
    fn 装了两份时警告里带上两个工具和各自的版本() {
        let w = shadow_text(
            "foo",
            &[
                ("pnpm".into(), "1.0.0".into()),
                ("npm".into(), "2.0.0".into()),
            ],
        )
        .expect("两份并存必须警告");
        assert!(w.contains("pnpm（1.0.0）"), "实际: {w}");
        assert!(w.contains("npm（2.0.0）"), "实际: {w}");
    }

    #[test]
    fn pnpm_与_npm_用不同的全局子命令() {
        // 这条区别装不上时的症状很隐蔽：pnpm 不认 `install -g <pkg>`，
        // 会把它当成「在当前目录装依赖」，于是命令成功但包没装到全局。
        assert!(Npm::at(PathBuf::from("/usr/bin/pnpm")).is_pnpm());
        assert!(!Npm::at(PathBuf::from("/usr/bin/npm")).is_pnpm());
    }
}
