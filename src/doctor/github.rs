//! GitHub 连不上的诊断与修复。
//!
//! 到 GitHub 有三条路，各有各的死法：22 端口常被运营商或公司防火墙整段丢弃，
//! HTTPS 常被 TLS 中间盒掐断，而 `ssh.github.com:443` 上那份 sshd 因为走的是
//! 443，大多数时候还活着。所以诊断就是把三条都探一遍，再把 git 的地址改写规则
//! 指到活着的那条上。
//!
//! 「地址改写规则」指 `url.<base>.insteadOf`：配好之后 `git clone
//! https://github.com/a/b` 会被 git 自己换成 `git@github.com:a/b`，已有仓库的
//! remote、脚本里写死的地址、README 里抄来的命令都不用动。

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use clap::ValueEnum;

use crate::common::block::Marker;
use crate::common::{cmd, env};
use crate::ui;

/// 探一条通道最多等多久。
const PROBE_TIMEOUT: Duration = Duration::from_secs(12);

/// 交给 ssh 自己的连接超时，单位秒。
///
/// 比 [`PROBE_TIMEOUT`] 小一截，是为了让 ssh 有机会自己说出「connect to host …
/// Connection timed out」再退出——那句话本身就是诊断结论，被我们从外面一刀砍掉
/// 就只剩「超时了」三个字。
const SSH_CONNECT_TIMEOUT: u32 = 8;

/// 拿来探路的公开仓库。用公开仓库，不碰使用者自己的业务仓库。
const PROBE_REPO: &str = "github/gitignore.git";

const HTTPS_BASE: &str = "https://github.com/";
const SSH_BASE: &str = "git@github.com:";
/// `git clone ssh://git@github.com/a/b` 这种写法，两条通道都要把它收编。
const SSH_URL_BASE: &str = "ssh://git@github.com/";

/// fundeploy 会往这两个 base 底下写 insteadOf，换通道时也从这两个底下清。
const MANAGED_BASES: [&str; 2] = [HTTPS_BASE, SSH_BASE];

/// GitHub 公示的 ed25519 主机公钥。
///
/// 写死而不是现场 `ssh-keyscan` 抓一份：这条命令只在网络有问题的时候才会被跑，
/// 正是中间人最可能在场的时刻，而「把网络当场返回的密钥直接记成可信」等于让
/// 被怀疑的那条链路自己给自己作保。出处是 <https://api.github.com/meta> 的
/// `ssh_keys`；`github.com:22` 和 `ssh.github.com:443` 用的是同一把。
const GITHUB_ED25519: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl";

/// 上面那把密钥的 SHA256 指纹，GitHub 文档里公示的就是这个串。
///
/// 程序本身不靠它做判断（直接比公钥更直接），留着是为了写进输出，让人能拿它跟
/// 官方文档对一眼。
const GITHUB_ED25519_FP: &str = "SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU";

// ---------------------------------------------------------------- 通道

/// 到 GitHub 的一条路。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Channel {
    Ssh22,
    Ssh443,
    Https,
}

impl Channel {
    /// 推荐顺序，从好到差。
    ///
    /// SSH 排在 HTTPS 前面：改成 SSH 之后 push 跟着能用，HTTPS 还得再配一次
    /// 凭据。22 排在 443 前面：它不用往 `~/.ssh/config` 里写东西。
    const ALL: [Channel; 3] = [Channel::Ssh22, Channel::Ssh443, Channel::Https];

    fn label(self) -> &'static str {
        match self {
            Channel::Ssh22 => "SSH(22)",
            Channel::Ssh443 => "SSH(443)",
            Channel::Https => "HTTPS",
        }
    }

    fn about(self) -> &'static str {
        match self {
            Channel::Ssh22 => "直连 22 端口",
            Channel::Ssh443 => "SSH 走 443，绕开屏蔽 22 端口的网络",
            Channel::Https => "走 443 的 HTTPS，不需要 SSH key",
        }
    }

    /// `--fix` 时写在命令行上的名字。
    fn key(self) -> &'static str {
        match self {
            Channel::Ssh22 => "ssh22",
            Channel::Ssh443 => "ssh443",
            Channel::Https => "https",
        }
    }

    /// 这条通道最终连到哪台主机的哪个端口。HTTPS 不走 ssh，没有。
    fn ssh_host(self) -> Option<(&'static str, u16)> {
        match self {
            Channel::Ssh22 => Some(("github.com", 22)),
            Channel::Ssh443 => Some(("ssh.github.com", 443)),
            Channel::Https => None,
        }
    }

    /// 要写进 git 配置的改写规则：把右边这些写法都改写成 `base` 开头。
    ///
    /// ssh22 和 ssh443 写的规则一模一样——443 的差异全在 `~/.ssh/config` 里，
    /// 地址形态还是 `git@github.com:`，git 这一层看不出区别。
    fn rewrites(self) -> (&'static str, [&'static str; 2]) {
        match self {
            Channel::Https => (HTTPS_BASE, [SSH_BASE, SSH_URL_BASE]),
            _ => (SSH_BASE, [HTTPS_BASE, SSH_URL_BASE]),
        }
    }

    /// 改完之后拿这个地址验一遍。
    fn probe_url(self) -> String {
        match self {
            Channel::Https => format!("{HTTPS_BASE}{PROBE_REPO}"),
            _ => format!("{SSH_BASE}{PROBE_REPO}"),
        }
    }
}

/// `--fix` 的取值。
#[derive(ValueEnum, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fix {
    /// 按诊断结果自己挑一条
    Auto,
    /// SSH 直连 22 端口
    #[value(name = "ssh22")]
    Ssh22,
    /// SSH 走 443 端口
    #[value(name = "ssh443")]
    Ssh443,
    /// HTTPS
    Https,
}

// ---------------------------------------------------------------- 探测

/// 一条通道的结论。
///
/// 不用「通 / 不通」两档：「22 端口被墙了」和「SSH key 还没配」在 ssh 眼里都是
/// 连不上，但该做的事完全相反——前者要换通道，后者换到哪条都一样不行，得去传
/// 公钥。混成一档，等于把人往错误的方向指。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Verdict {
    /// 通，而且 GitHub 认了这台机器。
    Ok,
    /// 端口通了，但 GitHub 不认这台机器：公钥没生成，或者没加到账号里。
    NoKey,
    /// 端口通了，但本机 known_hosts 里没有这台主机的密钥，ssh 自己先拒了。
    UnknownHost,
    /// 连不上。附一句原因。
    Blocked(String),
}

impl Verdict {
    fn usable(&self) -> bool {
        matches!(self, Verdict::Ok)
    }

    fn describe(&self) -> String {
        match self {
            Verdict::Ok => "通，认证过了".into(),
            Verdict::NoKey => "端口通，但 GitHub 不认这台机器的公钥".into(),
            Verdict::UnknownHost => "端口通，但本机不认识它的主机密钥".into(),
            Verdict::Blocked(why) => format!("不通（{why}）"),
        }
    }
}

/// 三条一起探。串行的话最坏要等满三次超时。
fn probe_all() -> Vec<(Channel, Verdict)> {
    std::thread::scope(|s| {
        let running: Vec<_> = Channel::ALL
            .iter()
            .map(|&ch| (ch, s.spawn(move || probe(ch))))
            .collect();
        running
            .into_iter()
            .map(|(ch, h)| {
                let v = h
                    .join()
                    .unwrap_or_else(|_| Verdict::Blocked("探测线程挂了".into()));
                (ch, v)
            })
            .collect()
    })
}

fn probe(ch: Channel) -> Verdict {
    match ch.ssh_host() {
        Some((host, port)) => probe_ssh(host, port),
        None => probe_https(),
    }
}

fn probe_ssh(host: &str, port: u16) -> Verdict {
    if !cmd::has("ssh") {
        return Verdict::Blocked("PATH 上没有 ssh".into());
    }
    let connect = format!("ConnectTimeout={SSH_CONNECT_TIMEOUT}");
    let port = port.to_string();
    let dest = format!("git@{host}");
    // BatchMode=yes：不许它停下来问密码或者「要信任这台主机吗」。诊断命令卡在
    // 一个没人看的提示符上，表现跟网络不通一样。
    let argv = [
        "ssh",
        "-o",
        "BatchMode=yes",
        "-o",
        &connect,
        "-T",
        "-p",
        &port,
        &dest,
    ];
    match cmd::try_run_timeout(argv, PROBE_TIMEOUT) {
        Ok(Some(o)) => classify_ssh(&format!("{}{}", o.stdout, o.stderr)),
        Ok(None) => Verdict::Blocked(format!("{} 秒没响应", PROBE_TIMEOUT.as_secs())),
        Err(e) => Verdict::Blocked(e.to_string()),
    }
}

/// 从 ssh 的输出判断结论。
///
/// 不看退出码：GitHub 的 sshd 不给 shell，认证成功之后也会立刻断开并返回 1，
/// 拿退出码判会把「完全正常」判成失败。
fn classify_ssh(out: &str) -> Verdict {
    if out.contains("successfully authenticated") {
        return Verdict::Ok;
    }
    if out.contains("Permission denied (publickey)") {
        return Verdict::NoKey;
    }
    if out.contains("Host key verification failed") || out.contains("authenticity of host") {
        return Verdict::UnknownHost;
    }
    Verdict::Blocked(last_line(out))
}

fn probe_https() -> Verdict {
    if !cmd::has("git") {
        return Verdict::Blocked("PATH 上没有 git".into());
    }
    let url = format!("{HTTPS_BASE}{PROBE_REPO}");
    // GIT_TERMINAL_PROMPT=0 顺便封掉 askpass：桌面环境下 git 会弹一个图形窗口
    // 问账号密码，而这条命令可能跑在 ssh 会话或者脚本里，谁也看不见那个窗口。
    let no_prompt = [("GIT_TERMINAL_PROMPT".to_string(), Some("0".to_string()))];
    match cmd::try_run_timeout_with(
        ["git", "ls-remote", "--exit-code", &url, "HEAD"],
        &no_prompt,
        PROBE_TIMEOUT,
    ) {
        Ok(Some(o)) if o.success() => Verdict::Ok,
        Ok(Some(o)) => Verdict::Blocked(first_line(&o.stderr)),
        Ok(None) => Verdict::Blocked(format!("{} 秒没响应", PROBE_TIMEOUT.as_secs())),
        Err(e) => Verdict::Blocked(e.to_string()),
    }
}

/// 取输出里最后一行有内容的。ssh 把警告摆在前面，结论留在最后。
fn last_line(s: &str) -> String {
    pick_line(s.lines().map(str::trim).rfind(|l| !l.is_empty()))
}

/// 取输出里第一行有内容的。
///
/// git 的顺序跟 ssh 反着来：真正的原因在第一行，后面跟着三行「请确认你有访问
/// 权限且仓库存在」之类放之四海而皆准的套话，取最后一行只会把套话端出来。
fn first_line(s: &str) -> String {
    pick_line(s.lines().map(str::trim).find(|l| !l.is_empty()))
}

fn pick_line(found: Option<&str>) -> String {
    found.unwrap_or("没有输出").to_string()
}

// ---------------------------------------------------------------- 诊断

/// 只看不改。
pub fn diagnose() -> Result<()> {
    let results = probe_all();
    report(&results);
    Ok(())
}

fn report(results: &[(Channel, Verdict)]) {
    ui::heading("GitHub 连通性");
    for (ch, v) in results {
        println!("{}{}", ui::pad(ch.label(), 12), v.describe());
    }

    ui::blank();
    ui::heading("当前配置");
    show_config();

    ui::blank();
    match pick(results) {
        Some(ch) => {
            ui::info(format!("建议走 {}：{}。", ch.label(), ch.about()));
            ui::hint(format!(
                "改过去: fundeploy doctor github --fix {}",
                ch.key()
            ));
            ui::hint("或者 fundeploy doctor github --fix 让它自己挑。");
        }
        // 三条都不通时，「网络不通」「缺 key」「缺主机密钥」要分开说。
        // 合成一句「请检查网络」等于什么也没说，而后两种其实网络好得很。
        None if results.iter().any(|(_, v)| *v == Verdict::NoKey) => {
            ui::warn("端口是通的，缺的是 SSH key。");
            ui::hint("生成: ssh-keygen -t ed25519 -C 你的邮箱");
            ui::hint("然后把 ~/.ssh/id_ed25519.pub 的内容贴到 https://github.com/settings/keys");
        }
        None if results.iter().any(|(_, v)| *v == Verdict::UnknownHost) => {
            ui::warn("端口是通的，缺的是 GitHub 的主机密钥。");
            ui::hint("写进 known_hosts: fundeploy doctor github --fix");
        }
        None => {
            ui::warn("三条通道都不通，多半是本机代理或者防火墙策略拦着。");
            ui::hint("看一眼代理: env | grep -i proxy; git config --global --get http.proxy");
        }
    }
}

fn show_config() {
    let mut any = false;
    for base in MANAGED_BASES {
        for from in rewrites_of(base) {
            println!("  git 改写  {from} → {base}");
            any = true;
        }
    }
    if !any {
        println!("  git 改写  没配");
    }

    let path = ssh_config_path();
    let has_block = marker().read(&path).is_some();
    println!(
        "  ssh 配置  {} {}",
        path.display(),
        if has_block {
            "里有 fundeploy 的 443 区块"
        } else {
            "里没有 fundeploy 的区块"
        }
    );
}

/// 建议走哪条。只在确实通了的里面挑。
///
/// [`Channel::ALL`] 本身就是优先级顺序，而 `probe_all` 保持这个顺序，所以取
/// 第一个命中的就是最优的那条。
fn pick(results: &[(Channel, Verdict)]) -> Option<Channel> {
    results.iter().find(|(_, v)| v.usable()).map(|(c, _)| *c)
}

/// `--fix` 自动挑哪条。
///
/// 先挑通的；都不通时挑一条「只差主机密钥」的——那个我们能当场补上，补完就通。
fn pick_fixable(results: &[(Channel, Verdict)]) -> Option<Channel> {
    pick(results).or_else(|| {
        results
            .iter()
            .find(|(_, v)| *v == Verdict::UnknownHost)
            .map(|(c, _)| *c)
    })
}

// ---------------------------------------------------------------- 修

/// 把 git 和 ssh 配到指定通道上。
pub fn fix(which: Fix) -> Result<()> {
    cmd::require("git", "先装 git。")?;

    let ch = match which {
        Fix::Auto => {
            let results = probe_all();
            report(&results);
            ui::blank();
            pick_fixable(&results).ok_or_else(|| {
                anyhow!(
                    "三条通道都不通，没有可以改过去的目标，配置一个字没动。\n\
                     先排查本机的代理和防火墙策略，或者指定一条: \
                     fundeploy doctor github --fix ssh443"
                )
            })?
        }
        Fix::Ssh22 => Channel::Ssh22,
        Fix::Ssh443 => Channel::Ssh443,
        Fix::Https => Channel::Https,
    };

    ui::step(format!("改成走 {}（{}）", ch.label(), ch.about()));
    apply(ch)?;
    ui::blank();
    verify(ch);
    Ok(())
}

fn apply(ch: Channel) -> Result<()> {
    // 先把 fundeploy 管的规则全清掉再写。只加不清的话，从 SSH 换回 HTTPS 之后
    // 两套规则同时在，谁生效取决于 git 匹配到哪条前缀更长——换完通道还是老样子。
    clear_rewrites();

    let (base, from) = ch.rewrites();
    for f in from {
        cmd::run(add_argv(base, f))?;
    }
    ui::ok(format!("git 改写规则：{} → {base}", from.join("、")));

    // ~/.ssh/config 里那个 443 区块只在选 443 时留着。换回 22 不把它删掉的话，
    // 所有发往 github.com 的 ssh 还是会被导到 443 去，改了等于没改。
    let path = ssh_config_path();
    if ch == Channel::Ssh443 {
        marker().write(&path, &ssh443_block())?;
        harden(&path, 0o600)?;
        ui::ok(format!("已写入 {}", path.display()));
    } else if marker().remove(&path)? {
        ui::ok(format!("已移除 {} 里的 443 区块", path.display()));
    }

    if let Some((host, port)) = ch.ssh_host() {
        trust(host, port)?;
    }
    Ok(())
}

/// 加一条改写规则的 argv。
fn add_argv(base: &str, from: &str) -> Vec<String> {
    // `--add` 不能省：insteadOf 是多值键，不带 `--add` 的 `git config` 是**替换**
    // 而不是追加，两条规则写下去只会剩最后一条，而被顶掉的那条恰恰是最常用的
    // 地址写法。
    vec![
        "git".into(),
        "config".into(),
        "--global".into(),
        "--add".into(),
        format!("url.{base}.insteadOf"),
        from.to_string(),
    ]
}

/// 清掉 fundeploy 写的全部改写规则。
fn clear_rewrites() {
    for base in MANAGED_BASES {
        // 键本来就不存在时 git 退出 5，那不是错误，是「没什么可清的」。
        let _ = cmd::try_run([
            "git",
            "config",
            "--global",
            "--unset-all",
            &format!("url.{base}.insteadOf"),
        ]);
    }
}

/// 读出某个 base 下现有的改写规则。
fn rewrites_of(base: &str) -> Vec<String> {
    cmd::try_run([
        "git",
        "config",
        "--global",
        "--get-all",
        &format!("url.{base}.insteadOf"),
    ])
    .map(|o| {
        o.stdout
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect()
    })
    .unwrap_or_default()
}

fn marker() -> Marker {
    Marker::new("github ssh443")
}

/// 写进 `~/.ssh/config` 的内容。
///
/// 没有 `IdentitiesOnly yes`：那一行在没有 `IdentityFile` 配套时会让 ssh 只认
/// 配置里点名的密钥，把 ssh-agent 里的全部忽略掉——拿 1Password / YubiKey 托管
/// 密钥的人一改完就变成 Permission denied，而且看不出跟这条命令有关系。
fn ssh443_block() -> Vec<String> {
    vec![
        "Host github.com".into(),
        "  HostName ssh.github.com".into(),
        "  Port 443".into(),
        "  User git".into(),
    ]
}

/// 把 GitHub 的主机公钥记进 known_hosts。
fn trust(host: &str, port: u16) -> Result<()> {
    // known_hosts 里非 22 端口的主机写成 [host]:port，ssh 按这个形式查。
    let who = if port == 22 {
        host.to_string()
    } else {
        format!("[{host}]:{port}")
    };
    let line = format!("{who} {GITHUB_ED25519}");
    let path = known_hosts_path();
    let existing = std::fs::read_to_string(&path).unwrap_or_default();

    if existing.lines().any(|l| l.trim() == line) {
        return Ok(());
    }
    if existing.lines().any(|l| host_matches(l, &who)) {
        // 这个主机名下已经记着另一把密钥。不覆盖：那等于替人做了「刚才认过的
        // 那把不算数」的决定，而这正是主机密钥校验存在的意义。
        ui::warn(format!(
            "{} 里已经有 {who} 的主机密钥，没有动它。",
            path.display()
        ));
        ui::hint(format!(
            "确认要换成 GitHub 公示的那把: ssh-keygen -R '{who}' 之后重跑。"
        ));
        return Ok(());
    }

    append_line(&path, &line)?;
    harden(&path, 0o600)?;
    ui::ok(format!("已记下 {who} 的主机公钥，指纹 {GITHUB_ED25519_FP}"));
    Ok(())
}

/// known_hosts 的这一行记的是不是这个主机。
fn host_matches(line: &str, who: &str) -> bool {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return false;
    }
    // 第一列是逗号分隔的主机名列表。散列过的（`|1|…`）这里认不出来，那种情况下
    // 顶多多写一条，ssh 两条都会接受。
    line.split_whitespace()
        .next()
        .is_some_and(|hosts| hosts.split(',').any(|h| h == who))
}

/// 改完之后真跑一次，确认这条路现在确实通。
fn verify(ch: Channel) {
    let url = ch.probe_url();
    ui::step("验证");
    let no_prompt = [("GIT_TERMINAL_PROMPT".to_string(), Some("0".to_string()))];
    match cmd::try_run_timeout_with(
        ["git", "ls-remote", "--exit-code", &url, "HEAD"],
        &no_prompt,
        PROBE_TIMEOUT,
    ) {
        Ok(Some(o)) if o.success() => ui::ok(format!("通了，{url} 拉得动。")),
        // 这里一律只警告不报错：配置确实已经写进去了，报 Err 退出非 0 会让人
        // 以为什么都没改，然后重复去跑同一条命令。
        Ok(Some(o)) => {
            ui::warn("配置写好了，但验证没过。git 说：");
            // 原样摊开而不是摘一行：验证失败是人要接着排查的地方，这时候任何
            // 摘要都可能恰好把那句有用的掐掉。
            for line in o.stderr.lines().filter(|l| !l.trim().is_empty()) {
                println!("    {line}");
            }
        }
        Ok(None) => ui::warn("配置写好了，但验证超时了。"),
        Err(e) => ui::warn(format!("配置写好了，但验证跑不起来：{e}")),
    }
}

// ---------------------------------------------------------------- 撤销

/// 撤掉 fundeploy 写进 git / ssh 配置里的东西。
pub fn undo() -> Result<()> {
    cmd::require("git", "先装 git。")?;

    clear_rewrites();
    ui::ok("已清掉 fundeploy 写的 git 地址改写规则。");

    let path = ssh_config_path();
    if marker().remove(&path)? {
        ui::ok(format!("已移除 {} 里的 443 区块", path.display()));
    } else {
        ui::info(format!("{} 里没有 fundeploy 的区块。", path.display()));
    }

    // known_hosts 不动。那条记录说的是「我认得这台主机」，跟走哪条通道无关，
    // 删掉只会让下次连接重新弹一遍指纹确认，而人多半会直接敲 yes——反倒比留着弱。
    ui::hint("known_hosts 里的 GitHub 主机密钥保留着，它跟走哪条通道无关。");
    Ok(())
}

// ---------------------------------------------------------------- 路径与落盘

/// `~/.ssh/config`。`FUNDEPLOY_SSH_CONFIG` 可以指到别处，测试用。
fn ssh_config_path() -> PathBuf {
    match env::opt("FUNDEPLOY_SSH_CONFIG") {
        Some(p) => env::expand_tilde(&p),
        None => env::home().join(".ssh").join("config"),
    }
}

/// `~/.ssh/known_hosts`。`FUNDEPLOY_KNOWN_HOSTS` 可以指到别处，测试用。
fn known_hosts_path() -> PathBuf {
    match env::opt("FUNDEPLOY_KNOWN_HOSTS") {
        Some(p) => env::expand_tilde(&p),
        None => env::home().join(".ssh").join("known_hosts"),
    }
}

/// 追加一行。
///
/// 整个读出来再写回去，而不是用 append 模式：原文件末尾缺换行时直接追加会把新
/// 内容粘在最后一行屁股上，两行一起失效，而 known_hosts 坏掉的表现是下次连接
/// 突然报主机密钥不对——没人会联想到是这里。
fn append_line(path: &Path, line: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("建目录失败: {}", dir.display()))?;
        harden(dir, 0o700)?;
    }
    let mut text = std::fs::read_to_string(path).unwrap_or_default();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(line);
    text.push('\n');
    std::fs::write(path, text).with_context(|| format!("写不了 {}", path.display()))
}

/// 收紧权限。ssh 会直接拒绝读组内或其他人可写的配置，宁可报错也不连。
fn harden(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .with_context(|| format!("改不了 {} 的权限", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 这些用例要改 `FUNDEPLOY_SSH_CONFIG` / `FUNDEPLOY_KNOWN_HOSTS`，而进程
    /// 环境是全局的，cargo test 默认并发跑。
    fn 排队() -> std::sync::MutexGuard<'static, ()> {
        static M: std::sync::Mutex<()> = std::sync::Mutex::new(());
        M.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn 认证成功要从输出里认出来_不能看退出码() {
        // GitHub 的 sshd 不给 shell，这条成功的连接退出码是 1。
        let out = "Hi octocat! You've successfully authenticated, \
                   but GitHub does not provide shell access.";
        assert_eq!(classify_ssh(out), Verdict::Ok);
    }

    #[test]
    fn 缺_key_和连不上要分开判() {
        assert_eq!(
            classify_ssh("git@github.com: Permission denied (publickey)."),
            Verdict::NoKey
        );
        let blocked = classify_ssh("ssh: connect to host github.com port 22: Connection timed out");
        assert!(matches!(blocked, Verdict::Blocked(_)));
        // 两者都判成「不通」的话，缺 key 的人会被指去换通道，换到哪条都一样不行。
        assert_ne!(classify_ssh("Permission denied (publickey)."), blocked);
    }

    #[test]
    fn 主机密钥没见过算单独一档() {
        assert_eq!(
            classify_ssh(
                "No ED25519 host key is known for github.com.\nHost key verification failed."
            ),
            Verdict::UnknownHost
        );
        assert_eq!(
            classify_ssh("The authenticity of host 'github.com (1.2.3.4)' can't be established."),
            Verdict::UnknownHost
        );
    }

    #[test]
    fn 认不出来的输出把最后一行原样带出来() {
        let v =
            classify_ssh("blah\nkex_exchange_identification: Connection closed by remote host\n");
        assert_eq!(
            v,
            Verdict::Blocked(
                "kex_exchange_identification: Connection closed by remote host".into()
            )
        );
    }

    #[test]
    fn 没有输出也不至于空着() {
        assert_eq!(last_line("   \n\n "), "没有输出");
        assert_eq!(first_line("   \n\n "), "没有输出");
        assert_eq!(last_line("a\nb\n"), "b");
        assert_eq!(first_line("a\nb\n"), "a");
    }

    #[test]
    fn git_的原因取第一行而不是末尾的套话() {
        let err = "fatal: unable to access 'https://github.com/a/b.git/': \
                   Failed to connect to github.com port 443: Connection refused\n\
                   Please make sure you have the correct access rights\n\
                   and the repository exists.\n";
        // 取最后一行会把「and the repository exists.」当成诊断结论报出来。
        assert!(
            first_line(err).contains("Connection refused"),
            "{}",
            first_line(err)
        );
    }

    #[test]
    fn 建议按_ssh22_ssh443_https_的顺序挑() {
        let all_ok: Vec<_> = Channel::ALL.iter().map(|&c| (c, Verdict::Ok)).collect();
        assert_eq!(pick(&all_ok), Some(Channel::Ssh22));

        let no_22 = vec![
            (Channel::Ssh22, Verdict::Blocked("x".into())),
            (Channel::Ssh443, Verdict::Ok),
            (Channel::Https, Verdict::Ok),
        ];
        assert_eq!(pick(&no_22), Some(Channel::Ssh443));
    }

    #[test]
    fn 一条都不通时不给建议() {
        let dead: Vec<_> = Channel::ALL
            .iter()
            .map(|&c| (c, Verdict::Blocked("x".into())))
            .collect();
        assert_eq!(pick(&dead), None);
        assert_eq!(pick_fixable(&dead), None);
    }

    #[test]
    fn 只差主机密钥的通道可以修() {
        // 这条是 pick 和 pick_fixable 的分界：建议里不该出现还没通的通道，
        // 但 --fix 能把主机密钥补上，补完它就通了。
        let results = vec![
            (Channel::Ssh22, Verdict::Blocked("x".into())),
            (Channel::Ssh443, Verdict::UnknownHost),
            (Channel::Https, Verdict::Blocked("x".into())),
        ];
        assert_eq!(pick(&results), None);
        assert_eq!(pick_fixable(&results), Some(Channel::Ssh443));
    }

    #[test]
    fn 缺_key_不算能修() {
        // 公钥得人自己传到 GitHub 去，这里改什么配置都没用。
        let results = vec![
            (Channel::Ssh22, Verdict::NoKey),
            (Channel::Ssh443, Verdict::NoKey),
            (Channel::Https, Verdict::Blocked("x".into())),
        ];
        assert_eq!(pick_fixable(&results), None);
    }

    #[test]
    fn 改写规则不会指向自己() {
        for ch in Channel::ALL {
            let (base, from) = ch.rewrites();
            // base 出现在 from 里就是自己改写自己，git 会拒绝或者绕成死循环。
            assert!(!from.contains(&base), "{} 的规则指向了自己", ch.label());
            assert_eq!(from.len(), 2, "{} 漏了一种地址写法", ch.label());
        }
    }

    #[test]
    fn 三种地址写法都被收编() {
        // 少收编一种，那种写法的仓库就会悄悄继续走旧通道，而人以为已经改好了。
        for ch in Channel::ALL {
            let (base, from) = ch.rewrites();
            let mut all: Vec<&str> = from.to_vec();
            all.push(base);
            for form in [HTTPS_BASE, SSH_BASE, SSH_URL_BASE] {
                assert!(all.contains(&form), "{} 没管 {form}", ch.label());
            }
        }
    }

    #[test]
    fn 写改写规则必须用_add() {
        // 不带 --add 的 git config 是替换：两条规则写下去只剩最后一条。
        let argv = add_argv(SSH_BASE, HTTPS_BASE);
        assert!(argv.contains(&"--add".to_string()), "实际: {argv:?}");
        assert!(argv.contains(&format!("url.{SSH_BASE}.insteadOf")));
    }

    #[test]
    fn ssh443_区块把_github_导到_443() {
        let body = ssh443_block().join("\n");
        assert!(body.contains("Host github.com"));
        assert!(body.contains("HostName ssh.github.com"));
        assert!(body.contains("Port 443"));
        // 见 ssh443_block 的注释：这一行会废掉 ssh-agent 里的密钥。
        assert!(!body.contains("IdentitiesOnly"), "实际:\n{body}");
    }

    #[test]
    fn known_hosts_的主机名按整条列匹配() {
        assert!(host_matches("github.com ssh-ed25519 AAAA", "github.com"));
        assert!(host_matches(
            "gitlab.com,github.com ssh-ed25519 AAAA",
            "github.com"
        ));
        assert!(host_matches(
            "[ssh.github.com]:443 ssh-rsa AAAA",
            "[ssh.github.com]:443"
        ));
        // 子串匹配会把这两条误判成已存在，然后跳过写入，443 那条永远配不上。
        assert!(!host_matches(
            "ssh.github.com ssh-ed25519 AAAA",
            "github.com"
        ));
        assert!(!host_matches(
            "[ssh.github.com]:443 ssh-rsa AAAA",
            "github.com"
        ));
        assert!(!host_matches("# github.com 的注释", "github.com"));
        assert!(!host_matches("", "github.com"));
    }

    #[test]
    fn 写进去的主机公钥跟官方公示的指纹对得上() {
        // 这把密钥是写死的，写错一个字符的后果是所有人的 known_hosts 里多一条
        // 永远对不上的记录。有 ssh-keygen 就当场算一遍核对。
        if !cmd::has("ssh-keygen") {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("k");
        std::fs::write(&f, format!("github.com {GITHUB_ED25519}\n")).unwrap();
        let out = cmd::capture(["ssh-keygen", "-lf", &f.to_string_lossy()]).unwrap();
        let fp = out.split_whitespace().nth(1).unwrap();
        assert_eq!(fp, GITHUB_ED25519_FP, "ssh-keygen 的输出: {out}");
    }

    #[test]
    fn 记主机公钥_重复跑不会写两遍() {
        let _g = 排队();
        let d = tempfile::tempdir().unwrap();
        let kh = d.path().join("known_hosts");
        std::env::set_var("FUNDEPLOY_KNOWN_HOSTS", &kh);

        trust("github.com", 22).unwrap();
        trust("github.com", 22).unwrap();
        trust("ssh.github.com", 443).unwrap();

        let text = std::fs::read_to_string(&kh).unwrap();
        assert_eq!(text.matches(GITHUB_ED25519).count(), 2, "实际:\n{text}");
        assert!(text.contains(&format!("github.com {GITHUB_ED25519}")));
        assert!(text.contains(&format!("[ssh.github.com]:443 {GITHUB_ED25519}")));
        std::env::remove_var("FUNDEPLOY_KNOWN_HOSTS");
    }

    #[test]
    fn 已经记着别的密钥时不覆盖() {
        let _g = 排队();
        let d = tempfile::tempdir().unwrap();
        let kh = d.path().join("known_hosts");
        std::fs::write(&kh, "github.com ssh-ed25519 AAAAsomethingelse\n").unwrap();
        std::env::set_var("FUNDEPLOY_KNOWN_HOSTS", &kh);

        trust("github.com", 22).unwrap();

        let text = std::fs::read_to_string(&kh).unwrap();
        assert!(text.contains("AAAAsomethingelse"), "原来那条被动了: {text}");
        assert!(!text.contains(GITHUB_ED25519), "不该擅自加上: {text}");
        std::env::remove_var("FUNDEPLOY_KNOWN_HOSTS");
    }

    #[test]
    fn 原文件末尾没换行也不会把两行粘一起() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("known_hosts");
        std::fs::write(&p, "old.example.com ssh-rsa AAAA").unwrap();
        append_line(&p, "new.example.com ssh-rsa BBBB").unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert_eq!(
            text,
            "old.example.com ssh-rsa AAAA\nnew.example.com ssh-rsa BBBB\n"
        );
    }

    #[test]
    fn ssh_配置区块写了能删干净() {
        let _g = 排队();
        let d = tempfile::tempdir().unwrap();
        let cfg = d.path().join("config");
        std::fs::write(&cfg, "Host myserver\n  User me\n").unwrap();
        std::env::set_var("FUNDEPLOY_SSH_CONFIG", &cfg);

        marker().write(&ssh_config_path(), &ssh443_block()).unwrap();
        let text = std::fs::read_to_string(&cfg).unwrap();
        assert!(text.contains("HostName ssh.github.com"));
        assert!(text.contains("Host myserver"));

        assert!(marker().remove(&ssh_config_path()).unwrap());
        // 删完必须跟没来过一样，别人的配置一个字不少、也不多留空行。
        assert_eq!(
            std::fs::read_to_string(&cfg).unwrap(),
            "Host myserver\n  User me\n"
        );
        std::env::remove_var("FUNDEPLOY_SSH_CONFIG");
    }

    #[test]
    fn 通道的名字能原样喂回_fix() {
        for ch in Channel::ALL {
            // 建议里打印的 `--fix <key>` 必须真能被解析，否则照着抄就报错。
            assert!(Fix::from_str(ch.key(), true).is_ok(), "{} 对不上", ch.key());
        }
    }
}
