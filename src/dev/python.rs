//! Python 虚拟环境（用 uv 建）。
//!
//! 这一个不走 [`super::Dev`] 那套「pkg 还是 source」的模子：它装的不是一样工具，
//! 而是一批按 Python 版本号分开的虚拟环境——`~/opt/py312`、`~/opt/py313`……每个
//! 里面预装同一组基础包。装法只有一种，动作也不一样（要按版本号来），硬塞进那个
//! trait 只会让两边都变形。

use std::path::PathBuf;

use anyhow::{bail, Context, Result};

use super::Dev;
use crate::common::{cmd, env, fs};
use crate::ui;

/// 建环境时可选的 Python 版本，新的在前。
const VERSIONS: [&str; 6] = ["3.14", "3.13", "3.12", "3.11", "3.10", "3.9"];
const DEFAULT: &str = "3.12";
const DEFAULT_PACKAGES: &str = "funbuild funinstall funsecret";

/// 环境根目录，默认 `~/opt`。
fn root() -> PathBuf {
    match env::opt("FUNDEPLOY_PYTHON_ROOT") {
        Some(d) => env::expand_tilde(&d),
        None => env::home().join("opt"),
    }
}

/// `3.12` → `~/opt/py312`。
fn env_path(version: &str) -> PathBuf {
    root().join(format!("py{}", version.replace('.', "")))
}

/// 每个环境里要预装的基础包。
fn packages() -> Vec<String> {
    env::str_or("FUNDEPLOY_PYTHON_PACKAGES", DEFAULT_PACKAGES)
        .split([' ', ',', '\t', '\n'])
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// 定下要操作哪个版本。
///
/// 环境变量也认，但命令行优先——Bash 版在脚本开头无条件写了一句
/// `PYTHON_VERSION=""`，把调用者传进来的值直接抹掉了，于是文档里写着的那个
/// 覆盖办法从来没生效过。
fn pick(version: Option<&str>) -> Result<String> {
    let v = version
        .map(str::to_string)
        .or_else(|| env::opt("FUNDEPLOY_PYTHON_VERSION"))
        .unwrap_or_else(|| DEFAULT.to_string());
    let v = v.trim().to_string();
    if v.is_empty() || !v.chars().all(|c| c.is_ascii_digit() || c == '.') {
        bail!("Python 版本号写法不对: {v}（形如 3.12）");
    }
    Ok(v)
}

/// uv 在哪儿。没有就去装一个。
fn uv() -> Result<String> {
    if let Some(p) = super::uv::Uv.bin_dir().map(|d| d.join("uv")) {
        if p.is_file() {
            return Ok(p.to_string_lossy().into_owned());
        }
    }
    if let Some(p) = cmd::find("uv") {
        return Ok(p);
    }
    ui::info("没找到 uv，先把它装上。");
    super::install(&super::uv::Uv, Some(super::Method::Source), None)?;
    super::uv::Uv
        .bin_dir()
        .map(|d| d.join("uv").to_string_lossy().into_owned())
        .filter(|p| std::path::Path::new(p).is_file())
        .or_else(|| cmd::find("uv"))
        .ok_or_else(|| anyhow::anyhow!("装完还是找不到 uv"))
}

/// 列出各版本环境的状态。
pub fn list() -> Result<()> {
    ui::heading(format!("Python 环境（{}）", root().display()));
    for v in VERSIONS {
        let p = env_path(v);
        let mark = if v == DEFAULT { "  (默认)" } else { "" };
        let state = if p.join("bin").join("python").is_file() {
            format!("已创建  {}{mark}", p.display())
        } else {
            format!("未创建{mark}")
        };
        ui::kv(format!("Python {v}"), Some(&state));
    }
    // 不在上面那张表里、但确实躺在根目录下的也要报：版本列表是会改的，改完
    // 之前建的环境不该从视野里消失，而目录还占着地方。
    let known: Vec<PathBuf> = VERSIONS.iter().map(|v| env_path(v)).collect();
    if let Ok(rd) = std::fs::read_dir(root()) {
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with("py") && p.join("bin/python").is_file() && !known.contains(&p) {
                ui::kv(
                    format!("{name}（不在版本表里）"),
                    Some(&p.display().to_string()),
                );
            }
        }
    }
    Ok(())
}

/// 建环境并装基础包。`force` 时先删掉重建。
pub fn install(version: Option<&str>, force: bool) -> Result<()> {
    let v = pick(version)?;
    let path = env_path(&v);
    let uv = uv()?;

    if path.exists() {
        if force {
            fs::guard_removable(&path)?;
            ui::step(format!("删掉重建 {}", path.display()));
            std::fs::remove_dir_all(&path).with_context(|| format!("删不掉 {}", path.display()))?;
        } else {
            // 默认保留：重建会把人往里装过的别的包一起扔掉，而这条命令的名字
            // 看不出有这个后果。要重建就显式 --force。
            ui::info(format!(
                "{} 已存在，保留它，只更新基础包。要推倒重来加 --force。",
                path.display()
            ));
        }
    }

    if !path.exists() {
        std::fs::create_dir_all(root())?;
        ui::step(format!("建 Python {v} 环境: {}", path.display()));
        cmd::run([&uv, "venv", &path.to_string_lossy(), "--python", &v])?;
    }

    let py = path.join("bin").join("python");
    let pkgs = packages();
    if pkgs.is_empty() {
        ui::info("没有要装的基础包。");
    } else {
        ui::step(format!("装基础包: {}", pkgs.join(" ")));
        let mut argv = vec![uv.clone(), "pip".into(), "install".into()];
        argv.extend(pkgs);
        argv.extend([
            "--python".into(),
            py.to_string_lossy().into_owned(),
            "-U".into(),
        ]);
        cmd::run(&argv)?;
    }

    ui::ok(format!("Python {v} 环境就绪: {}", path.display()));
    // 子进程改不了父 shell 的环境变量，所以这里只能给出命令让人自己执行。
    // Bash 版在自己的进程里 source 了一遍 activate 然后报「环境已激活！」——
    // 那个判断在脚本自己的进程里永远成立，而调用它的终端什么也没变。
    ui::hint(format!("用它:\n  source {}/bin/activate", path.display()));
    Ok(())
}

/// 把已有环境里的基础包更到最新。
pub fn update(version: Option<&str>) -> Result<()> {
    let v = pick(version)?;
    let path = env_path(&v);
    let py = path.join("bin").join("python");
    if !py.is_file() {
        bail!(
            "{} 还不是一个环境。先 `fundeploy dev python install --version {v}`。",
            path.display()
        );
    }
    let uv = uv()?;
    let pkgs = packages();
    if pkgs.is_empty() {
        ui::info("没有要更新的基础包。");
        return Ok(());
    }
    ui::step(format!("更新 {}: {}", path.display(), pkgs.join(" ")));
    let mut argv = vec![uv, "pip".into(), "install".into()];
    argv.extend(pkgs);
    argv.extend([
        "--python".into(),
        py.to_string_lossy().into_owned(),
        "-U".into(),
    ]);
    cmd::run(&argv)?;
    ui::ok(format!("Python {v} 的基础包已是最新。"));
    Ok(())
}

pub fn uninstall(version: Option<&str>, yes: bool) -> Result<()> {
    let v = pick(version)?;
    let path = env_path(&v);
    if !path.exists() {
        ui::info(format!("{} 本来就不存在。", path.display()));
        return Ok(());
    }
    if !yes {
        ui::confirm_destructive(
            &format!("要删掉 {} 吗", path.display()),
            "FUNDEPLOY_PYTHON_UNINSTALL_YES",
        )?;
    }
    fs::guard_removable(&path)?;
    std::fs::remove_dir_all(&path).with_context(|| format!("删不掉 {}", path.display()))?;
    ui::ok(format!("已删除 {}", path.display()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn 排队() -> std::sync::MutexGuard<'static, ()> {
        static M: std::sync::Mutex<()> = std::sync::Mutex::new(());
        M.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn 版本号变成目录名() {
        let _g = 排队();
        std::env::set_var("FUNDEPLOY_PYTHON_ROOT", "/tmp/pyroot");
        assert_eq!(env_path("3.12"), PathBuf::from("/tmp/pyroot/py312"));
        assert_eq!(env_path("3.9"), PathBuf::from("/tmp/pyroot/py39"));
        std::env::remove_var("FUNDEPLOY_PYTHON_ROOT");
    }

    #[test]
    fn 命令行给的版本压过环境变量() {
        // Bash 版在脚本开头无条件 PYTHON_VERSION=""，把传进来的值抹掉了。
        let _g = 排队();
        std::env::set_var("FUNDEPLOY_PYTHON_VERSION", "3.11");
        assert_eq!(pick(Some("3.13")).unwrap(), "3.13");
        assert_eq!(pick(None).unwrap(), "3.11");
        std::env::remove_var("FUNDEPLOY_PYTHON_VERSION");
        assert_eq!(pick(None).unwrap(), DEFAULT);
    }

    #[test]
    fn 怪字符的版本号不会被拼进路径() {
        for bad in ["../../etc", "3.12; rm -rf /", "", "latest"] {
            assert!(pick(Some(bad)).is_err(), "{bad} 该被拦下");
        }
    }

    #[test]
    fn 基础包列表可以用空格或逗号分隔() {
        let _g = 排队();
        std::env::remove_var("FUNDEPLOY_PYTHON_PACKAGES");
        assert_eq!(packages(), ["funbuild", "funinstall", "funsecret"]);
        std::env::set_var("FUNDEPLOY_PYTHON_PACKAGES", "a, b  c");
        assert_eq!(packages(), ["a", "b", "c"]);
        // 全项目一致的约定：空白等于没设。`FOO= fundeploy …` 在 shell 里是顺手
        // 就会写出来的东西，当成「设成了空」会让一堆地方静默变成空串。
        std::env::set_var("FUNDEPLOY_PYTHON_PACKAGES", "  ");
        assert_eq!(packages(), ["funbuild", "funinstall", "funsecret"]);
        std::env::remove_var("FUNDEPLOY_PYTHON_PACKAGES");
    }

    #[test]
    fn 默认版本在候选列表里() {
        assert!(VERSIONS.contains(&DEFAULT));
    }
}
