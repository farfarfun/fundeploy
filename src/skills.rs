//! 同步 Gitee 上某个组织里的「技能」仓库。
//!
//! 组织里名字带 `--` 的仓库才算技能仓库（命名约定是 `<领域>--<名字>`），别的
//! 一概不碰。每个仓库在本地占一个同名子目录。
//!
//! 同步完会把仓库里除 `.git` 之外的点目录递归删掉。`.github`、`.cursor`、
//! `.claude` 这些装的是**那个仓库自己开发时**的配置，跟着同步到本地之后，读技能
//! 的工具会把它们当成当前环境的配置来用。

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::common::{cmd, env};
use crate::ui;

const DEFAULT_ORG: &str = "farfarfun-skills";
const PER_PAGE: usize = 100;

/// 翻页的上限。
///
/// Gitee 偶尔会对越界的页码返回最后一页而不是空数组，没有上限的话「翻到不足一页
/// 为止」就永远翻不完。
const MAX_PAGES: usize = 50;

/// 删本地仓库时跳过确认的环境变量。
const ENV_RM_YES: &str = "FUNDEPLOY_SKILLS_RM_YES";

// ---------------------------------------------------------------- 配置

struct Conf {
    org: String,
    dir: PathBuf,
    api: String,
}

impl Conf {
    fn from_env() -> Result<Self> {
        let org = env::str_or("FUNDEPLOY_SKILLS_ORG", DEFAULT_ORG);
        if org.is_empty() || !org.chars().all(ok_char) {
            bail!("组织名里有不该有的字符: {org}");
        }
        // 目录默认跟着组织名走：指到另一个组织时，两边的仓库不该堆在同一个
        // 目录里互相覆盖。
        let dir = match env::opt("FUNDEPLOY_SKILLS_DIR") {
            Some(d) => env::expand_tilde(&d),
            None => env::home().join("workspace").join("github").join(&org),
        };
        let api = env::str_or(
            "FUNDEPLOY_SKILLS_API",
            &format!("https://gitee.com/api/v5/orgs/{org}/repos"),
        );
        guard(&dir)?;
        Ok(Self { org, dir, api })
    }
}

/// 目标目录的安全闸。
///
/// 删除那条路会对这底下的目录做递归删除，而目录名来自环境变量。这道闸不是防
/// 攻击的——能设这个变量的人本来就能直接 `rm`——是防手滑：少打一级路径，或者
/// 变量展开成了空串，代价就是整台机器。
fn guard(dir: &Path) -> Result<()> {
    if !dir.is_absolute() {
        bail!("FUNDEPLOY_SKILLS_DIR 必须是绝对路径: {}", dir.display());
    }
    // 已存在就先解析符号链接：/a/b/c 完全可能是一条指向 / 的链接。
    let real = unfirmlink(dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf()));
    if real == env::home() {
        bail!("不能直接拿 $HOME 当目标目录: {}", real.display());
    }
    const SYSTEM: &[&str] = &[
        "/", "/bin", "/boot", "/dev", "/etc", "/home", "/lib", "/lib64", "/opt", "/proc", "/root",
        "/run", "/sbin", "/srv", "/sys", "/tmp", "/usr", "/var", "/Users",
    ];
    if SYSTEM.contains(&real.to_string_lossy().as_ref()) {
        bail!("不能拿系统目录当目标目录: {}", real.display());
    }
    // 组件数含根，所以 /a/b/c 是 4。层级太浅的多半是路径打漏了一段。
    if real.components().count() < 4 {
        bail!("目标目录至少要三层: {}", real.display());
    }
    Ok(())
}

/// 还原 macOS 的 firmlink。
///
/// Catalina 起系统卷只读，`/Users`、`/opt`、`/home` 这些实际在数据卷上，
/// canonicalize 出来是 `/System/Volumes/Data/<原路径>`。不还原的话上面那张系统
/// 目录名单在 Mac 上形同虚设——`/home` 解析完变成一个五层深、名字也对不上的
/// 路径，一路放行。
fn unfirmlink(p: PathBuf) -> PathBuf {
    match p.strip_prefix("/System/Volumes/Data") {
        Ok(rest) => Path::new("/").join(rest),
        Err(_) => p,
    }
}

fn ok_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')
}

/// 技能仓库的命名约定：含 `--`，只用安全字符，不以点开头。
///
/// 字符集必须收紧，因为这个名字会直接拼进本地路径——组织里真要是有人建了个名字
/// 带 `/` 的仓库，同步就会写到目标目录外面去。
fn valid_repo(name: &str) -> bool {
    name.contains("--") && !name.starts_with('.') && name.chars().all(ok_char)
}

// ---------------------------------------------------------------- 远端

#[derive(Deserialize)]
struct GiteeRepo {
    name: String,
}

/// 一个远端技能仓库。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Remote {
    name: String,
    url: String,
}

fn fetch_remote(c: &Conf) -> Result<Vec<Remote>> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build();

    let mut names: Vec<String> = Vec::new();
    for page in 1..=MAX_PAGES {
        let url = format!("{}?type=all&page={page}&per_page={PER_PAGE}", c.api);
        let body = agent
            .get(&url)
            .set("User-Agent", "fundeploy")
            .call()
            .with_context(|| format!("查 Gitee 仓库列表失败: {url}"))?
            .into_string()
            .context("读 Gitee 的响应失败")?;
        let page_repos: Vec<GiteeRepo> = serde_json::from_str(&body)
            .with_context(|| format!("Gitee 第 {page} 页返回的不是仓库列表"))?;

        let n = page_repos.len();
        names.extend(
            page_repos
                .into_iter()
                .map(|r| r.name)
                .filter(|n| valid_repo(n)),
        );
        // 不足一页就是到头了，比等一个空页少一次请求。
        if n < PER_PAGE {
            break;
        }
    }
    names.sort();
    names.dedup();
    Ok(names
        .into_iter()
        .map(|name| {
            let url = format!("https://gitee.com/{}/{name}.git", c.org);
            Remote { name, url }
        })
        .collect())
}

// ---------------------------------------------------------------- 状态

/// 一个技能仓库在本机的状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// 本地没有。
    Missing,
    /// 装着，没有本地改动。
    Clean,
    /// 装着，有本地改动。
    Dirty,
    /// 同名的东西在，但不是我们装的：不是 git 仓库，或者 origin 指向别处，
    /// 又或者压根不是目录。
    Foreign,
}

impl State {
    fn label(self) -> &'static str {
        match self {
            State::Missing => "未安装",
            State::Clean => "已安装",
            State::Dirty => "已安装，有改动",
            State::Foreign => "目录冲突",
        }
    }
}

struct Sync {
    conf: Conf,
    remote: Vec<Remote>,
}

impl Sync {
    fn open() -> Result<Self> {
        cmd::require("git", "先装 git。")?;
        let conf = Conf::from_env()?;
        let remote = fetch_remote(&conf)?;
        Ok(Self { conf, remote })
    }

    fn dir_of(&self, name: &str) -> PathBuf {
        self.conf.dir.join(name)
    }

    fn url_of(&self, name: &str) -> Option<&str> {
        self.remote
            .iter()
            .find(|r| r.name == name)
            .map(|r| r.url.as_str())
    }

    /// 这个目录是不是我们装的那个仓库。
    ///
    /// 判据是 origin 指向组织里的同名仓库，而不是「是不是个 git 仓库」。目标
    /// 目录底下完全可能躺着一个人自己 clone 的同名仓库，只认名字就 `reset
    /// --hard` 下去，洗掉的是别人的东西。
    fn managed(&self, name: &str) -> bool {
        let dir = self.dir_of(name);
        let Ok(o) = cmd::try_run([
            "git",
            "-C",
            &dir.to_string_lossy(),
            "remote",
            "get-url",
            "origin",
        ]) else {
            return false;
        };
        if !o.success() {
            return false;
        }
        let actual = o.stdout.trim().trim_end_matches(".git");
        if self
            .url_of(name)
            .is_some_and(|u| u.trim_end_matches(".git") == actual)
        {
            return true;
        }
        let org = &self.conf.org;
        [
            format!("https://gitee.com/{org}/{name}"),
            format!("git@gitee.com:{org}/{name}"),
            format!("ssh://git@gitee.com/{org}/{name}"),
        ]
        .iter()
        .any(|e| e == actual)
    }

    fn state(&self, name: &str) -> State {
        let dir = self.dir_of(name);
        // symlink_metadata 不跟随符号链接：目录项要是一条指向别处的链接，
        // 后面那些 reset --hard / 递归删除动的就不是我们以为的地方。
        let Ok(md) = std::fs::symlink_metadata(&dir) else {
            return State::Missing;
        };
        if !md.is_dir() || !self.managed(name) {
            return State::Foreign;
        }
        if is_dirty(&dir) {
            State::Dirty
        } else {
            State::Clean
        }
    }

    /// 目标目录里已经装着的技能仓库。
    fn local(&self) -> Vec<String> {
        let Ok(rd) = std::fs::read_dir(&self.conf.dir) else {
            return Vec::new();
        };
        let mut out: Vec<String> = rd
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| valid_repo(n))
            .filter(|n| self.managed(n))
            .collect();
        out.sort();
        out
    }

    fn list(&self) -> Result<()> {
        println!(
            "远端 {} 个技能仓库，目标目录 {}",
            self.remote.len(),
            self.conf.dir.display()
        );
        for r in &self.remote {
            println!("{}{}", ui::pad(self.state(&r.name).label(), 18), r.name);
        }
        // 远端没有、本地还留着的也要报出来：那多半是上游改名或者删库了，不报的话
        // 这个目录会一直攒着没人认领的东西。
        for name in self.local() {
            if self.url_of(&name).is_none() {
                let tail = if is_dirty(&self.dir_of(&name)) {
                    "仅本地，有改动"
                } else {
                    "仅本地"
                };
                println!("{}{name}", ui::pad(tail, 18));
            }
        }
        Ok(())
    }

    fn install_one(&self, name: &str) -> Result<()> {
        let url = self
            .url_of(name)
            .ok_or_else(|| anyhow::anyhow!("远端没有 {name}"))?
            .to_string();
        match self.state(name) {
            State::Clean | State::Dirty => {
                ui::info(format!("{name} 已经装着了，跳过。"));
                return Ok(());
            }
            State::Foreign => bail!(
                "{} 已经被别的东西占着（不是这个组织的仓库），没动它。",
                self.dir_of(name).display()
            ),
            State::Missing => {}
        }
        let dir = self.dir_of(name);
        std::fs::create_dir_all(&self.conf.dir)
            .with_context(|| format!("建目录失败: {}", self.conf.dir.display()))?;
        ui::step(format!("装 {name}"));
        // --depth 1：技能仓库要的是内容，历史是纯粹的下载量。
        cmd::run(["git", "clone", "--depth", "1", &url, &dir.to_string_lossy()])?;
        strip_dot_dirs(&dir)
    }

    fn update_one(&self, name: &str) -> Result<()> {
        if self.url_of(name).is_none() {
            bail!("远端没有 {name}");
        }
        match self.state(name) {
            State::Missing => bail!("{name} 还没装，先 install。"),
            State::Foreign => bail!(
                "{} 不是这个组织的仓库，没动它。",
                self.dir_of(name).display()
            ),
            _ => {}
        }
        let dir = self.dir_of(name).to_string_lossy().into_owned();
        ui::step(format!("更新 {name}"));
        // 本地改动会被覆盖，这是这条命令说好的语义：技能仓库是只读的消费端，
        // 真要改就去上游改。
        //
        // fetch 带 --depth 1：clone 时是浅的，fetch 不说深度的话会把整部历史拉
        // 回来，仓库越同步越大。
        cmd::run([
            "git", "-C", &dir, "fetch", "--force", "--prune", "--depth", "1", "origin", "HEAD",
        ])?;
        cmd::run(["git", "-C", &dir, "reset", "--hard", "FETCH_HEAD"])?;
        cmd::run(["git", "-C", &dir, "clean", "-ffdx"])?;
        strip_dot_dirs(Path::new(&dir))
    }

    fn remove_one(&self, name: &str) -> Result<()> {
        let dir = self.dir_of(name);
        match self.state(name) {
            State::Missing => {
                ui::info(format!("{name} 本来就没装。"));
                return Ok(());
            }
            State::Foreign => bail!("{} 不是这个组织的仓库，没动它。", dir.display()),
            State::Dirty => bail!(
                "{name} 有本地改动，没删。先 `git -C {} status` 看一眼。",
                dir.display()
            ),
            State::Clean => {}
        }
        ui::step(format!("删 {name}"));
        std::fs::remove_dir_all(&dir).with_context(|| format!("删不掉 {}", dir.display()))
    }

    /// 装上缺的、更新已有的。
    fn sync(&self) -> Result<()> {
        let (mut added, mut updated, mut skipped) = (0, 0, 0);
        let mut failed: Vec<String> = Vec::new();
        for r in &self.remote {
            let (what, res) = match self.state(&r.name) {
                State::Missing => ("装", self.install_one(&r.name)),
                State::Clean | State::Dirty => ("更新", self.update_one(&r.name)),
                State::Foreign => {
                    ui::warn(format!(
                        "跳过 {}：{} 被别的东西占着。",
                        r.name,
                        self.dir_of(&r.name).display()
                    ));
                    skipped += 1;
                    continue;
                }
            };
            match res {
                Ok(()) if what == "装" => added += 1,
                Ok(()) => updated += 1,
                Err(e) => {
                    // 一个仓库失败不该把剩下几十个也带停：这条命令的价值就在于
                    // 一次把全部拉齐，中途退出等于每次都得从头再来一遍。
                    ui::warn(format!("{}{} 失败：{e}", what, r.name));
                    failed.push(r.name.clone());
                }
            }
        }
        ui::blank();
        ui::ok(format!(
            "装了 {added} 个，更新 {updated} 个，跳过 {skipped} 个。"
        ));
        if !failed.is_empty() {
            bail!("有 {} 个没弄成：{}", failed.len(), failed.join("、"));
        }
        Ok(())
    }

    fn remove_all(&self, yes: bool) -> Result<()> {
        let names = self.local();
        if names.is_empty() {
            ui::info("本地一个技能仓库都没有。");
            return Ok(());
        }
        for n in &names {
            println!("  {n}");
        }
        if !yes {
            ui::confirm_destructive(
                &format!("要删掉上面这 {} 个仓库吗", names.len()),
                ENV_RM_YES,
            )?;
        }
        let mut failed = Vec::new();
        for n in &names {
            if let Err(e) = self.remove_one(n) {
                ui::warn(e.to_string());
                failed.push(n.clone());
            }
        }
        if !failed.is_empty() {
            bail!("有 {} 个没删掉：{}", failed.len(), failed.join("、"));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- 工作区

/// 这个仓库有没有本地改动。
///
/// 我们自己删掉的点目录不算。同步的最后一步就是把除 `.git` 之外的点目录删干净，
/// 而它们多半是被 git 跟踪着的（`.github/workflows` 几乎人手一份），于是
/// `git status` 从同步完成的那一刻起就一直挂着一片 deleted。照它判，仓库永远是
/// 「有改动」，删除永远被拒——装上去就再也删不掉了。
fn is_dirty(dir: &Path) -> bool {
    let Ok(o) = cmd::try_run(["git", "-C", &dir.to_string_lossy(), "status", "--porcelain"]) else {
        return false;
    };
    o.stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .any(|l| !is_our_removal(l))
}

/// `status --porcelain` 的这一行是不是我们删点目录留下的。
fn is_our_removal(line: &str) -> bool {
    // 格式是「XY 路径」：X 是暂存区状态，Y 是工作区状态。我们只动工作区，所以
    // 必须恰好是 ` D`；已经暂存的删除（`D `）是人自己下的手，得算改动。
    let Some(path) = line.strip_prefix(" D ") else {
        return false;
    };
    // 路径里有特殊字符时 git 会加引号转义。认不出来就当是人改的——判错的方向是
    // 「拒绝删除」，比误删安全。
    let path = path.trim();
    if path.starts_with('"') {
        return false;
    }
    let mut parts: Vec<&str> = path.split('/').collect();
    parts.pop(); // 最后一段是文件名，点文件不在我们的清理范围里
    parts.iter().any(|p| p.starts_with('.'))
}

/// 递归删掉仓库里除 `.git` 之外的点目录。
///
/// 全程不跟随符号链接：跟进去就可能走出这个仓库，在别人的目录里删东西。
fn strip_dot_dirs(root: &Path) -> Result<()> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let rd = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e).with_context(|| format!("读不了 {}", dir.display())),
        };
        for entry in rd.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(ft) = entry.file_type() else { continue };

            if name.starts_with('.') {
                if name == ".git" {
                    continue;
                }
                if ft.is_symlink() {
                    // 删链接本身，不碰它指向的东西。
                    std::fs::remove_file(&path)
                        .with_context(|| format!("删不掉 {}", path.display()))?;
                } else if ft.is_dir() {
                    std::fs::remove_dir_all(&path)
                        .with_context(|| format!("删不掉 {}", path.display()))?;
                }
                // 点文件（.gitignore 之类）留着，它们不会被当成环境配置读。
                continue;
            }
            if ft.is_dir() {
                stack.push(path);
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- 入口

pub fn list() -> Result<()> {
    Sync::open()?.list()
}

pub fn sync() -> Result<()> {
    let s = Sync::open()?;
    s.list()?;
    ui::blank();
    s.sync()
}

pub fn install(name: &str) -> Result<()> {
    Sync::open()?.install_one(name)
}

pub fn update(name: &str) -> Result<()> {
    Sync::open()?.update_one(name)
}

pub fn remove(name: Option<&str>, all: bool, yes: bool) -> Result<()> {
    let s = Sync::open()?;
    match (name, all) {
        (Some(n), _) => s.remove_one(n),
        (None, true) => s.remove_all(yes),
        (None, false) => bail!("给个仓库名，或者用 --all 删全部。"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 只认带两道杠的仓库名() {
        assert!(valid_repo("web--react"));
        assert!(valid_repo("a--b.c_d-e"));
        assert!(!valid_repo("plain"));
        assert!(!valid_repo("one-dash"));
    }

    #[test]
    fn 仓库名里的怪字符一律不认() {
        // 这个名字会被拼进本地路径，放行 / 等于让远端决定往哪儿写。
        assert!(!valid_repo("a--b/../../etc"));
        assert!(!valid_repo("../--.."));
        assert!(!valid_repo("a--b c"));
        assert!(!valid_repo("a--b;rm"));
        assert!(!valid_repo(".hidden--x"));
        assert!(!valid_repo(""));
    }

    #[test]
    fn 目标目录的闸拦得住手滑() {
        assert!(guard(Path::new("relative/path")).is_err());
        assert!(guard(Path::new("/")).is_err());
        assert!(guard(Path::new("/usr")).is_err());
        assert!(guard(Path::new("/home")).is_err());
        assert!(guard(Path::new("/a/b")).is_err(), "两层就该拦下");
        assert!(guard(&env::home()).is_err(), "$HOME 本身不能当目标");
        assert!(guard(Path::new("/a/b/c")).is_ok());
    }

    #[test]
    fn 系统目录名单不被_macos_的_firmlink_绕过() {
        // Mac 上 canonicalize("/home") 给的是 /System/Volumes/Data/home，
        // 照原样比名单就全放行了。这里直接验还原那一步，Linux 上也跑得到。
        assert_eq!(
            unfirmlink("/System/Volumes/Data/home".into()),
            Path::new("/home")
        );
        assert_eq!(
            unfirmlink("/System/Volumes/Data/Users/x".into()),
            Path::new("/Users/x")
        );
        assert_eq!(unfirmlink("/home/x".into()), Path::new("/home/x"));
        // 前缀只在整段匹配时才算，不能被名字开头像它的目录骗过去。
        assert_eq!(
            unfirmlink("/System/Volumes/Datum/x".into()),
            Path::new("/System/Volumes/Datum/x")
        );
    }

    #[test]
    fn 我们自己删的点目录不算本地改动() {
        // 同步的最后一步会删掉这些被跟踪的点目录，照搬 git status 的话仓库
        // 从此永远是「有改动」，删除永远被拒。
        assert!(is_our_removal(" D .github/workflows/ci.yml"));
        assert!(is_our_removal(" D nested/.cursor/rules"));
        assert!(!is_our_removal(" D README.md"), "普通文件被删是人干的");
        assert!(!is_our_removal(" D .gitignore"), "点文件我们不删");
        assert!(!is_our_removal("D  .github/x"), "暂存过的删除是人干的");
        assert!(!is_our_removal(" M .github/x"));
        assert!(!is_our_removal("?? .github/x"));
        assert!(!is_our_removal(r#" D ".github/带空格 的"#));
    }

    #[test]
    fn 点目录删光_点文件和_git_留着() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        for sub in [
            ".github/workflows",
            ".git/info",
            "nested/.cursor",
            "nested/keep",
        ] {
            std::fs::create_dir_all(root.join(sub)).unwrap();
            std::fs::write(root.join(sub).join("f"), "x").unwrap();
        }
        std::fs::write(root.join(".gitignore"), "x").unwrap();

        strip_dot_dirs(root).unwrap();

        assert!(!root.join(".github").exists());
        assert!(!root.join("nested/.cursor").exists());
        assert!(root.join(".git/info/f").exists(), ".git 不能动");
        assert!(root.join(".gitignore").exists(), "点文件不在清理范围");
        assert!(root.join("nested/keep/f").exists());
    }

    #[test]
    fn 点链接只删链接本身_不碰目标() {
        let d = tempfile::tempdir().unwrap();
        let outside = d.path().join("outside");
        std::fs::create_dir_all(outside.join("precious")).unwrap();
        std::fs::write(outside.join("precious/f"), "x").unwrap();

        let repo = d.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        std::os::unix::fs::symlink(&outside, repo.join(".link")).unwrap();

        strip_dot_dirs(&repo).unwrap();

        assert!(!repo.join(".link").exists(), "链接本身该删掉");
        assert!(outside.join("precious/f").exists(), "链接指向的东西不能动");
    }

    #[test]
    fn 不顺着普通目录的符号链接走出仓库() {
        // 跟进去就会在别人的目录里删点目录。
        let d = tempfile::tempdir().unwrap();
        let outside = d.path().join("outside");
        std::fs::create_dir_all(outside.join(".secret")).unwrap();

        let repo = d.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        std::os::unix::fs::symlink(&outside, repo.join("plain")).unwrap();

        strip_dot_dirs(&repo).unwrap();

        assert!(outside.join(".secret").exists(), "不该走到仓库外面去");
    }

    /// 造一个「远端」裸仓库和一份 Sync，整套流程不碰网络。
    fn 搭台() -> (tempfile::TempDir, Sync, String) {
        let d = tempfile::tempdir().unwrap();
        let name = "demo--skills".to_string();
        let upstream = d.path().join("upstream.git");
        let seed = d.path().join("seed");

        let up = upstream.to_string_lossy().into_owned();
        let sd = seed.to_string_lossy().into_owned();
        cmd::run(["git", "init", "-q", "--bare", "-b", "main", &up]).unwrap();
        cmd::run(["git", "init", "-q", "-b", "main", &sd]).unwrap();
        for (k, v) in [("user.name", "t"), ("user.email", "t@e.com")] {
            cmd::run(["git", "-C", &sd, "config", k, v]).unwrap();
        }
        std::fs::create_dir_all(seed.join(".github/workflows")).unwrap();
        std::fs::write(seed.join(".github/workflows/ci.yml"), "x").unwrap();
        std::fs::write(seed.join("version"), "one\n").unwrap();
        cmd::run(["git", "-C", &sd, "add", "."]).unwrap();
        cmd::run(["git", "-C", &sd, "commit", "-qm", "one"]).unwrap();
        cmd::run(["git", "-C", &sd, "remote", "add", "origin", &up]).unwrap();
        cmd::run(["git", "-C", &sd, "push", "-q", "-u", "origin", "main"]).unwrap();

        // 目标目录要够深，否则被 guard 拦下。
        let target = d.path().join("a").join("b").join("repos");
        let s = Sync {
            conf: Conf {
                org: "testorg".into(),
                dir: target,
                api: String::new(),
            },
            remote: vec![Remote {
                name: name.clone(),
                url: up,
            }],
        };
        (d, s, name)
    }

    fn 改上游(d: &Path, text: &str) {
        let sd = d.join("seed").to_string_lossy().into_owned();
        std::fs::write(d.join("seed/version"), text).unwrap();
        cmd::run(["git", "-C", &sd, "commit", "-qam", "next"]).unwrap();
        cmd::run(["git", "-C", &sd, "push", "-q", "-u", "origin", "main"]).unwrap();
    }

    #[test]
    fn 装完就该是干净的() {
        let (_d, s, name) = 搭台();
        assert_eq!(s.state(&name), State::Missing);
        s.install_one(&name).unwrap();

        assert!(!s.dir_of(&name).join(".github").exists(), "点目录该被清掉");
        assert!(s.dir_of(&name).join("version").exists());
        // 清点目录留下一片 deleted，照搬 git status 这里就会是 Dirty，
        // 然后这个仓库再也删不掉。
        assert_eq!(s.state(&name), State::Clean);
    }

    #[test]
    fn 更新会把本地改动覆盖掉() {
        let (d, s, name) = 搭台();
        s.install_one(&name).unwrap();
        let repo = s.dir_of(&name);
        std::fs::write(repo.join("version"), "local\n").unwrap();
        std::fs::write(repo.join("untracked"), "x").unwrap();
        assert_eq!(s.state(&name), State::Dirty);

        改上游(d.path(), "two\n");
        s.update_one(&name).unwrap();

        assert_eq!(
            std::fs::read_to_string(repo.join("version")).unwrap(),
            "two\n"
        );
        assert!(!repo.join("untracked").exists(), "clean -ffdx 该清掉它");
        assert!(!repo.join(".github").exists(), "更新完也要清点目录");
        assert_eq!(s.state(&name), State::Clean);
    }

    #[test]
    fn 本地另有提交也会被拉回远端的版本() {
        let (d, s, name) = 搭台();
        s.install_one(&name).unwrap();
        let repo = s.dir_of(&name).to_string_lossy().into_owned();
        std::fs::write(s.dir_of(&name).join("version"), "mine\n").unwrap();
        cmd::run([
            "git",
            "-C",
            &repo,
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@e.com",
            "commit",
            "-qam",
            "mine",
        ])
        .unwrap();

        改上游(d.path(), "three\n");
        s.update_one(&name).unwrap();

        assert_eq!(
            std::fs::read_to_string(s.dir_of(&name).join("version")).unwrap(),
            "three\n"
        );
    }

    #[test]
    fn 有改动的仓库拒绝删除_干净的能删() {
        let (_d, s, name) = 搭台();
        s.install_one(&name).unwrap();
        std::fs::write(s.dir_of(&name).join("version"), "dirty\n").unwrap();
        assert!(s.remove_one(&name).is_err(), "有改动不能删");
        assert!(s.dir_of(&name).exists());

        std::fs::write(s.dir_of(&name).join("version"), "one\n").unwrap();
        s.remove_one(&name).unwrap();
        assert!(!s.dir_of(&name).exists());
    }

    #[test]
    fn 别人的同名仓库不会被当成自己的() {
        let (_d, s, name) = 搭台();
        // 目标目录里先放一个 origin 指向别处的同名仓库。
        let dir = s.dir_of(&name);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.to_string_lossy().into_owned();
        cmd::run(["git", "init", "-q", "-b", "main", &p]).unwrap();
        cmd::run([
            "git",
            "-C",
            &p,
            "remote",
            "add",
            "origin",
            "https://example.com/someone/demo--skills.git",
        ])
        .unwrap();

        assert_eq!(s.state(&name), State::Foreign);
        // 三条写操作都必须停手，否则洗掉的是别人的东西。
        assert!(s.install_one(&name).is_err());
        assert!(s.update_one(&name).is_err());
        assert!(s.remove_one(&name).is_err());
        assert!(dir.join(".git").exists());
    }

    #[test]
    fn 指向别处的符号链接不算已安装() {
        let (_d, s, name) = 搭台();
        std::fs::create_dir_all(&s.conf.dir).unwrap();
        std::os::unix::fs::symlink("/etc", s.dir_of(&name)).unwrap();
        assert_eq!(s.state(&name), State::Foreign);
        assert!(s.remove_one(&name).is_err(), "绝不能对着链接做递归删除");
    }

    #[test]
    fn sync_把缺的装上_已有的更新() {
        let (d, s, name) = 搭台();
        s.sync().unwrap();
        assert_eq!(s.state(&name), State::Clean);

        改上游(d.path(), "four\n");
        s.sync().unwrap();
        assert_eq!(
            std::fs::read_to_string(s.dir_of(&name).join("version")).unwrap(),
            "four\n"
        );
    }

    #[test]
    fn 只扫得到受管的本地仓库() {
        let (_d, s, name) = 搭台();
        s.install_one(&name).unwrap();
        std::fs::create_dir_all(s.conf.dir.join("not--managed")).unwrap();
        std::fs::create_dir_all(s.conf.dir.join("nodashes")).unwrap();
        assert_eq!(s.local(), vec![name]);
    }
}
