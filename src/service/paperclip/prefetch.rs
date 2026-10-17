//! npm 预取、安装期进度测量，以及「这台机器真会下哪些包」的筛选。
//!
//! 这一摊和 Paperclip 的生命周期（装/起/停/查）是两件事，只是都落在同一个上游 CLI
//! 上：上游 managed install 把 npm 的输出整个捕获掉了，于是「装到哪一步了」只能从
//! 外面观察——盯暂存目录、量 npm 缓存、照着 lockfile 自己算该下多少。
//!
//! 父模块只用到少数几个入口（[`InstallWatch`]、[`Npmrc`]，以及挂在
//! [`Paperclip`] 上的 `prefetch` / `npm_env` / `print_network_hint`），
//! 其余都是它们的零件。

use super::*;
// 对同级模块的依赖写明，不靠父模块转一手：进度条样式那几样是公共件，
// 这边只是用户之一。
use super::progress::{downloading_style, unpacking_style};

/// 装包期间顶掉 `~/.npmrc` 的一份最小 npmrc，临时文件随值一起消失。
///
/// 最外层那次 `npx paperclipai@latest` 会读 `~/.npmrc`，并把读到的配置以
/// `npm_config_*` 注入子进程。已知 npm bug（npm/cli#9783、#9912、#9968）会把这样继承
/// 来的 allow-scripts 当成命令行显式传入的 `--allow-scripts`，直接报 EALLOWSCRIPTS；
/// 只清环境变量不够——配置本来就来自文件。
pub(super) struct Npmrc {
    pub(super) path: PathBuf,
}

impl Npmrc {
    pub(super) fn write(registry: &str) -> Option<Self> {
        let path =
            std::env::temp_dir().join(format!("fundeploy-paperclip-npmrc.{}", std::process::id()));
        let registry = registry.trim_end_matches('/');
        std::fs::write(
            &path,
            format!("registry={registry}/\n@paperclipai:registry={registry}/\n"),
        )
        .ok()?;
        Some(Self { path })
    }
}

impl Drop for Npmrc {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// 两次量大小之间至少隔这么久。
pub(super) const MEASURE_EVERY: Duration = Duration::from_secs(2);

/// 轮询子进程是否退出、以及暂存目录在不在的间隔。
///
/// 暂存目录 rename 成正式 payload 之后，上游紧接着就打那行结果，所以这一问要跟得紧
/// ——慢了就会让进度条盖掉它。一次 `read_dir`（store 里只有两三个条目）的代价可以忽略。
pub(super) const WATCH_TICK: Duration = Duration::from_millis(500);

/// managed install / update 期间，盯着上游正在写的暂存目录报进度。
///
/// 上游把 npm 的输出整个捕获掉了（`installNpmPayload` 用 `execFile` 系的 runCommand
/// 跑 `npm install --prefix <暂存目录>`，不是继承 stdio），所以从「Installing
/// paperclipai@x」到装完之间，这条命令一个字都不打。那一段实测要写 2.5 GiB、7 万多个
/// 文件，几分钟到几十分钟，看起来跟卡死一模一样。
///
/// 但它正在写的地方是公开的：`<home>/cli/installs/npm/.<版本>.tmp-<pid>-<毫秒>`，装完
/// rename 成正式 payload。盯着那个目录长大就能把那段沉默翻译成进度；分母用 `current`
/// 指向的当前 payload——上游每次都是从零拼一棵新树，新旧两版体积相差一成上下，够估，
/// 估小了就把分母顶上去（见 [`InstallWatch::tick`]）。
///
/// 上游在这一段不打输出，也正好意味着进度条不会和它抢同一片终端：暂存目录在的那段，
/// 就是它闷着的那段。
///
/// 这段沉默里有两件事先后发生，实测 100 秒 + 70 秒，所以分两段报：
///
/// 1. **下载**：npm 先把 tarball 收进 `~/.npm/_cacache`，这期间暂存目录建好了但是空的。
///    只盯暂存目录的话，最难熬的前一半会停在 `0 B` 上——那正是这次要解决的观感。这一段
///    改看缓存的增量，没有分母（不知道总共要下多少），所以不报 ETA。
/// 2. **解包**：往暂存目录里铺 7 万多个文件，有分母可报，ETA 这时才有意义。
pub(super) struct InstallWatch {
    /// `<home>/cli/installs/npm`，暂存目录和正式 payload 都在这儿。
    store: PathBuf,
    /// `<home>/cli/current`，指向当前 payload，用来估分母。
    current: PathBuf,
    /// npm 的内容寻址缓存。不存在（第一次用 npm）就是 None，那时下载段只报时间。
    cache: Option<PathBuf>,
    bar: Option<indicatif::ProgressBar>,
    /// 下次真正去量大小的时刻。
    next: Instant,
    /// 暂存目录最后一次量到的字节数。只增不减，见 [`InstallWatch::tick`]。
    last: u64,
    /// 开工时缓存的体积，下载段的进度是相对它的增量。
    cache_base: u64,
    started: Option<Instant>,
}

impl InstallWatch {
    pub(super) fn new(home: &Path) -> Self {
        let cli = home.join("cli");
        let cache = npm_cacache();
        Self {
            store: cli.join("installs").join("npm"),
            current: cli.join("current"),
            cache: cache.is_dir().then_some(cache),
            bar: None,
            next: Instant::now(),
            last: 0,
            cache_base: 0,
            started: None,
        }
    }

    /// 轮询回调，每 [`WATCH_TICK`] 一次。
    pub(super) fn tick(&mut self) {
        let Some(staging) = staging_dir(&self.store) else {
            // 还没出现（上游在解析版本号），或者已经 rename 走了 / 失败被删了。
            self.finish();
            return;
        };
        if self.bar.is_none() {
            self.open(&staging);
        }
        if Instant::now() < self.next {
            return;
        }
        let t0 = Instant::now();
        let now = dir_size(&staging);
        // 只认涨的那一侧：量到一半目录被 rename 走时这一趟会半途读空，照原样写进去
        // 就是进度条往回跳，收尾那行也会报一个莫名其妙的小数字。
        if now > self.last {
            if self.last == 0 {
                self.unpacking();
            }
            self.last = now;
            if let Some(bar) = &self.bar {
                // 分母是按旧版估的，新版更大就把它顶上去——不然最后那几个百分点会
                // 停在 100% 上，还报着「还要 0s」。
                if bar.length().is_some_and(|len| now > len) {
                    bar.set_length(now);
                }
                bar.set_position(now);
            }
        } else if self.last == 0 {
            // 还在下载段：暂存目录是空的，看缓存涨了多少。
            if let (Some(cache), Some(bar)) = (&self.cache, &self.bar) {
                bar.set_position(dir_size(cache).saturating_sub(self.cache_base));
            }
        }
        // 下一次至少等这一趟实际耗时的 10 倍：依赖树越大走一趟越贵（见 dir_size），
        // 这条让「看进度」这件事本身始终只占个位数的 I/O，不跟安装抢。
        self.next = Instant::now() + MEASURE_EVERY.max(t0.elapsed() * 10);
    }

    fn open(&mut self, staging: &Path) {
        let name = staging.file_name().unwrap_or_default().to_string_lossy();
        let label = staging_version(&name).unwrap_or("依赖树").to_string();
        self.cache_base = self.cache.as_deref().map(dir_size).unwrap_or(0);
        self.started = Some(Instant::now());
        ui::hint("上游装 payload 这一段不打任何输出，下面的进度是 fundeploy 在旁边量的。");
        let bar = progress_bar();
        bar.set_style(downloading_style(self.cache.is_some()));
        bar.set_message(format!("{label} 下载依赖"));
        bar.enable_steady_tick(STEADY_TICK);
        self.bar = Some(bar);
    }

    /// 下载段转解包段：换模板、换分母，并把速率和 ETA 的估算从头算。
    ///
    /// 不 `reset()` 不行：位置要从「缓存增量」跳到「暂存目录大小」，两个量毫无关系，
    /// 留着上一段的历史只会算出一个荒唐的 ETA。
    fn unpacking(&mut self) {
        let Some(bar) = &self.bar else { return };
        // 分母只是个估计值（上游每次从零拼一棵新树，新旧两版体积差几个百分点），
        // 所以说出来它是哪儿来的；没有旧 payload（全新安装）就只报已写入量。
        let total = dir_size(&self.current);
        bar.reset();
        if total > 0 {
            bar.set_length(total);
            bar.set_style(unpacking_style());
            bar.set_message(format!("解包（分母 {} 按当前版本估）", human_size(total)));
        } else {
            bar.set_style(downloading_style(true));
            bar.set_message("解包".to_string());
        }
    }

    /// 收尾。进度条还在就清掉，并留一行说明这一步花了多少。
    pub(super) fn finish(&mut self) {
        let Some(bar) = self.bar.take() else { return };
        bar.finish_and_clear();
        let used = self.started.take().map(|t| t.elapsed()).unwrap_or_default();
        if self.last == 0 {
            // 一个字节都没量到：暂存目录刚建好就没了（上游在下载段就失败了）。
            return;
        }
        // 措辞刻意中性：失败时暂存目录也会被上游的 finally 删掉，这行不该宣布成功。
        ui::hint(format!(
            "这一步写了约 {}，用了 {}。",
            human_size(self.last),
            human_secs(used)
        ));
    }
}

/// npm 的内容寻址缓存目录。
///
/// npm 默认 `~/.npm/_cacache`，`npm_config_cache` 能改——上游不在命令行上传
/// `--cache`，所以这个环境变量对它那条 `npm install` 同样有效（见
/// [`Paperclip::npm_env`] 里对哪些键透得进去的说明）。
///
/// 刻意不去跑 `npm config get cache`：那是一次几百毫秒的 node 启动，而这里只是为了
/// 量个大小、报个数字，猜错的代价仅仅是少报一行。
pub(super) fn npm_cacache() -> PathBuf {
    env::opt("npm_config_cache")
        .map(|v| env::expand_tilde(&v).join("_cacache"))
        .unwrap_or_else(|| env::home().join(".npm").join("_cacache"))
}

/// npm 缓存的冷热，以及它对这次下载意味着什么。
///
/// 判据只能是体积：缓存是内容寻址的，里面有多少属于 paperclip 这棵树，不把几万个
/// 索引条目逐个解开是问不出来的。但「比整棵树还小」这一条已经够下结论——装不满。
///
/// `download` 是这次全量要下的估计值（见 [`Paperclip::print_network_hint`]）。
pub(super) fn cache_state(download: u64) -> String {
    let cache = npm_cacache();
    cache_state_of(dir_size(&cache), &cache, download)
}

pub(super) fn cache_state_of(size: u64, cache: &Path, download: u64) -> String {
    if size == 0 {
        return format!("npm 缓存 {} 还是空的，这次是全量下载。", cache.display());
    }
    if size < download {
        return format!(
            "npm 缓存现有 {}，装不满这棵树，有一部分还得新下。",
            human_size(size)
        );
    }
    format!(
        "npm 缓存现有 {}，命中的 tarball 不再下载；PAPERCLIP_NPM_PREFER_OFFLINE=1 \
         连「问一遍有没有更新」的那些请求也省掉。",
        human_size(size)
    )
}

/// prefetch 的参数：通道和要不要丢后台。顺序不限，都可以省。
pub(super) fn prefetch_args(args: &[&str], default: Channel) -> Result<(Channel, bool)> {
    let mut ch = None;
    let mut background = false;
    for a in args {
        match *a {
            "--background" | "-b" => background = true,
            other => ch = Some(Channel::parse(other)?),
        }
    }
    Ok((ch.unwrap_or(default), background))
}

/// 从 lockfile 里挑出来的预热清单。
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Picked {
    /// 要灌的 `名字@版本`，已去重排序。
    pub(super) specs: Vec<String>,
    /// 装在父包 tarball 里、不单独下的（`inBundle`）。
    pub(super) bundled: usize,
    /// 平台不符、npm 装的时候也会跳过的。
    pub(super) other_platform: usize,
    /// `resolved` 不在钉死的 registry 上的。正常情况下一个都没有。
    pub(super) foreign: Vec<String>,
}

/// lockfile 里「这台机器真会去下」的那些包。
///
/// 三道筛子，少一道都会多下或者下错：
///
/// 1. **没有 `resolved` 的跳过**。它们是 `inBundle`——装在父包的 tarball 里，没有自己
///    的下载地址。实测 888 个条目里有 348 个是这种，全在 `@paperclipai/server` 肚子里。
/// 2. **`os` / `cpu` / `libc` 不匹配的跳过**。npm 装的时候自己会按这三个字段过滤，
///    预热不跟着过滤就会把另外一百个平台的二进制也灌进缓存（esbuild 一家 25 个平台）。
/// 3. **`resolved` 不在钉死的 registry 上的跳过**。npm 的缓存键里带着完整 URL，所以
///    预热必须跟 install 走同一个源才算命中；顺带也是道闸门，这棵树里本该一个外部
///    URL 都没有。
pub(super) fn prefetch_specs(lock: &serde_json::Value, plat: &NpmPlatform) -> Picked {
    let mut out = Picked::default();
    let Some(pkgs) = lock["packages"].as_object() else {
        return out;
    };
    let mut specs = std::collections::BTreeSet::new();
    for (path, m) in pkgs {
        // 空路径是根条目，也就是 npm 为这次解析现编的那份 package.json。
        if path.is_empty() {
            continue;
        }
        let Some(resolved) = m["resolved"].as_str() else {
            if m["inBundle"].as_bool() == Some(true) {
                out.bundled += 1;
            }
            continue;
        };
        if !plat.allows(m) {
            out.other_platform += 1;
            continue;
        }
        if !on_pinned_registry(resolved) {
            out.foreign.push(resolved.to_string());
            continue;
        }
        let (Some(name), Some(version)) = (entry_name(path, m), m["version"].as_str()) else {
            continue;
        };
        specs.insert(format!("{name}@{version}"));
    }
    out.specs = specs.into_iter().collect();
    out
}

/// 一个 lockfile 条目的真名。
///
/// 路径上的目录名不一定是包名：别名依赖（`"@openai/codex-linux-x64":
/// "npm:@openai/codex@0.160.0-linux-x64"`）装在以别名命名的目录里，真名在 `name`
/// 字段上。实测这棵树里恰好 12 个条目带 `name`，全是别名——而且正是最大的那几个
/// （codex 一家 141 + 155 MiB），按目录名去灌只会 404。
pub(super) fn entry_name<'a>(path: &'a str, m: &'a serde_json::Value) -> Option<&'a str> {
    if let Some(n) = m["name"].as_str() {
        return Some(n);
    }
    path.rsplit("node_modules/")
        .next()
        .filter(|s| !s.is_empty())
}

/// 这个下载地址是不是落在钉死的那个 registry 上。
pub(super) fn on_pinned_registry(url: &str) -> bool {
    url.strip_prefix(NPM_REGISTRY)
        .is_some_and(|rest| rest.starts_with('/'))
}

/// npm 眼里的本机平台：`os` / `cpu` / `libc` 三个字段该拿什么值去比。
#[derive(Debug, Clone, Copy)]
pub(super) struct NpmPlatform {
    os: &'static str,
    cpu: &'static str,
    /// 只有 linux 的包会声明 libc，别的平台上是 `None`（当没约束处理）。
    libc: Option<&'static str>,
}

impl NpmPlatform {
    pub(super) fn current() -> Self {
        let os = match std::env::consts::OS {
            "macos" => "darwin",
            other => other,
        };
        let cpu = match std::env::consts::ARCH {
            "x86_64" => "x64",
            "aarch64" => "arm64",
            other => other,
        };
        let libc = (os == "linux").then(|| if musl_system() { "musl" } else { "glibc" });
        Self { os, cpu, libc }
    }

    /// 这个条目适用于本机吗。
    fn allows(&self, m: &serde_json::Value) -> bool {
        constraint_ok(&m["os"], Some(self.os))
            && constraint_ok(&m["cpu"], Some(self.cpu))
            && constraint_ok(&m["libc"], self.libc)
    }
}

/// 一个 `os` / `cpu` / `libc` 约束跟本机的取值合不合。
///
/// 字段缺失就是没约束。写法有两种而且会混用：允许列表（`["linux"]`）和排除
/// （`["!win32"]`）。判定跟 npm 自己一致——命中任一排除项就否，有允许列表而没命中
/// 也否。本机取值未知时（非 linux 机器上的 `libc`）当没约束处理。
pub(super) fn constraint_ok(v: &serde_json::Value, value: Option<&str>) -> bool {
    let Some(list) = v.as_array() else {
        return true;
    };
    let Some(value) = value else {
        return true;
    };
    let mut has_allow = false;
    let mut matched = false;
    for item in list.iter().filter_map(|i| i.as_str()) {
        match item.strip_prefix('!') {
            Some(neg) => {
                if neg == value {
                    return false;
                }
            }
            None => {
                has_allow = true;
                matched |= item == value;
            }
        }
    }
    !has_allow || matched
}

/// 这台机器的 C 库是不是 musl。
///
/// 判据跟 detect-libc 一个意思：musl 的那个动态链接器在不在（Alpine 上是
/// `/lib/ld-musl-x86_64.so.1`）。刻意不看 fundeploy 自己是怎么编出来的——CI 出的是
/// musl 静态包，而它跑在 glibc 机器上照样是 glibc 的机器，`cfg!(target_env = "musl")`
/// 问的是编译机不是这台机器。
pub(super) fn musl_system() -> bool {
    std::fs::read_dir("/lib").is_ok_and(|d| {
        d.flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with("ld-musl-"))
    })
}

/// `npm view … version --json` 的输出里那个版本号。
///
/// 通常是一个 JSON 字符串；spec 命中多个版本时 npm 给的是数组，那种情况取最后一个
/// （npm 按版本升序排），跟它自己 `npm view` 的习惯一致。
pub(super) fn npm_view_version(stdout: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).ok()?;
    let s = match &v {
        serde_json::Value::Array(a) => a.last()?.as_str()?,
        other => other.as_str()?,
    };
    (!s.is_empty()).then(|| s.to_string())
}

/// 预热那一段的进度条：分母是包数，顺带报已经灌进去多少字节。
///
/// 刻意不摆 `{eta}`：包的大小差三个数量级（几 KiB 的 shim 和 155 MiB 的 codex 都算
/// 一个），按个数外推出来的「还要多久」没有意义。
pub(super) fn prefetch_style() -> indicatif::ProgressStyle {
    indicatif::ProgressStyle::with_template(
        "{msg} [{bar:24.cyan/blue}] {pos}/{len} 个包 已过 {elapsed}",
    )
    .unwrap()
    .progress_chars("=>-")
}

/// install store 里正在写的那个暂存目录。
///
/// 上游的命名是 `.<版本>.tmp-<pid>-<毫秒>`，认 `.` 开头加 `.tmp-` 两个特征——正式
/// payload（`<版本>`）和那份临时 npmrc（`.npmrc-<pid>-<毫秒>`，是文件不是目录）都
/// 不会被误认。
///
/// 可能有不止一个：上一次被 kill -9 打断时，上游 `finally` 里的清理没来得及跑。按
/// （mtime, 名字）取最大的那个——名字里带着创建时的毫秒数，两条判据同向，不必依赖
/// 文件系统的 mtime 精度。
pub(super) fn staging_dir(store: &Path) -> Option<PathBuf> {
    std::fs::read_dir(store)
        .ok()?
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.starts_with('.') && n.contains(".tmp-")
        })
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .max_by_key(|e| {
            (
                e.metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::UNIX_EPOCH),
                e.file_name(),
            )
        })
        .map(|e| e.path())
}

/// 从暂存目录名里取出正在装的版本号。
pub(super) fn staging_version(name: &str) -> Option<&str> {
    let v = name.strip_prefix('.')?.rsplit_once(".tmp-")?.0;
    (!v.is_empty()).then_some(v)
}

// Paperclip 身上跟预取、npm 环境有关的那几个动作，理由同 log.rs 顶上那段。
// 这几个方法和本文件里的 InstallWatch / Picked / NpmPlatform 是一摊事，放在
// 一起读才连得上。
impl Paperclip {
    /// 装包期间要透给 npm 的环境改动。
    ///
    /// 官方 managed install 把 registry 钉死在 registry.npmjs.org：它另写一份只含
    /// registry 的临时 npmrc、用 `npm_config_userconfig` 强制 npm 子进程改用它，命令行
    /// 上再传一次 `--registry`。这是刻意的防投毒设计，有上游测试守着，所以 `~/.npmrc`
    /// 里的 registry / proxy / fetch-* 对它一概无效（文件被整体替换），
    /// `npm_config_registry` 也压不过命令行。唯一透得进去的是环境变量——它以
    /// `{...process.env, npm_config_userconfig}` 起 npm。因此这里只设它没有在命令行上
    /// 显式传过的那些键：取回重试与超时，以及代理。
    pub(super) fn npm_env(&self, npmrc: Option<&Npmrc>) -> Vec<EnvEdit> {
        let mut e: Vec<EnvEdit> = vec![
            ("npm_config_allow_scripts".to_string(), None),
            (
                "npm_config_fetch_retries".to_string(),
                Some(self.fetch.retries.to_string()),
            ),
            (
                "npm_config_fetch_retry_mintimeout".to_string(),
                Some(self.fetch.min_timeout.to_string()),
            ),
            (
                "npm_config_fetch_retry_maxtimeout".to_string(),
                Some(self.fetch.max_timeout.to_string()),
            ),
            (
                "npm_config_fetch_timeout".to_string(),
                Some(self.fetch.timeout.to_string()),
            ),
        ];
        if let Some(rc) = npmrc {
            e.push((
                "npm_config_userconfig".to_string(),
                Some(rc.path.display().to_string()),
            ));
        }
        if self.prefer_offline {
            e.push((
                "npm_config_prefer_offline".to_string(),
                Some("true".to_string()),
            ));
        }
        let Some(proxy) = &self.proxy else {
            return e;
        };
        ui::info(format!("npm 下载走代理 {proxy}"));
        for key in [
            "npm_config_proxy",
            "npm_config_https_proxy",
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "http_proxy",
            "https_proxy",
        ] {
            e.push((key.to_string(), Some(proxy.clone())));
        }
        if let Some(no) = &self.no_proxy {
            for key in ["NO_PROXY", "no_proxy"] {
                e.push((key.to_string(), Some(no.clone())));
            }
        }
        e
    }

    /// 装之前把这次要下多少、能省掉多少说清楚。
    ///
    /// 下载量和落盘量必须分开报：实测同一棵树是 **1.0 GiB 下载 / 2.7 GiB 落盘**，
    /// 差了近三倍。这行原来只报落盘量，读起来像是每次升级都得下 2.7 GiB。
    ///
    /// 绝对值仍然不写死（依赖树一直在长）：落盘量按本机当前 payload 现量，下载量按
    /// [`TARBALL_RATIO`] 折算。而真正决定这次要等多久的不是总量，是缓存热不热，所以
    /// 还要去量一眼 `_cacache`——那是这条提示里唯一能让人当场做点什么的数字。
    pub(super) fn print_network_hint(&self, ch: Channel) {
        if !self.network_hint {
            return;
        }
        let measured = dir_size(&self.home.join("cli").join("current"));
        let disk = if measured > 0 {
            measured
        } else {
            MEASURED_DISK
        };
        let download = (disk as f64 * TARBALL_RATIO) as u64;
        ui::warn(format!(
            "Paperclip 的依赖树很大：全量约 {} 下载 / {} 落盘{}。而且每次升级都是从零拼一棵\
             新的——上游装到一个临时目录再原子换过去，换来的是失败不留残骸、回滚只动一个\
             符号链接。",
            human_size(download),
            human_size(disk),
            if measured > 0 {
                ""
            } else {
                "（本机还没装过，按上次实测估）"
            },
        ));
        ui::hint(cache_state(download));
        ui::hint(
            "下载量的七成是各家 agent CLI 的二进制（codex / claude-agent-sdk / copilot / \
             opencode，而且每个都有两份：上游的版本范围对不上，同一个工具的两个版本并存），\
             升级时它们大多不变——缓存热了之后 canary 日更通常只下一两百 MiB。所以别清 \
             ~/.npm/_cacache。",
        );
        ui::hint(format!(
            "想先把下载跑完再装: fundeploy service paperclip prefetch {} --background",
            ch.name()
        ));
        if self.proxy.is_none() {
            ui::hint(
                "官方 managed install 把 registry 钉死在 npmjs.org，换镜像对它无效（刻意的\
                 防投毒设计，有上游测试守着），受限网络请走代理：\
                 PAPERCLIP_HTTPS_PROXY=http://127.0.0.1:7890。",
            );
        }
        ui::hint("设 PAPERCLIP_NETWORK_HINT=0 隐藏以上几行。");
    }

    /// 把目标版本要下的 tarball 预先灌进 npm 缓存。
    ///
    /// # 为什么这件事做得到
    ///
    /// npm 的缓存是内容寻址的，键就是 tarball 的完整 URL（实测
    /// `make-fetch-happen:request-cache:https://registry.npmjs.org/…tgz`），谁灌进去
    /// 的都算：`npm cache add` 之后再去装同一个包，npm 在同一个键上命中，一个字节都
    /// 不再下（实测日志里是 `npm http cache … 0ms (cache hit)`）。
    ///
    /// 所以必须跟 install 走同一个 registry——键里带着主机名，换个源灌进去的那份对
    /// 上游那条命令就是不存在（实测用 npmmirror 灌出来的键是 `cdn.npmmirror.com/…`）。
    ///
    /// # 为什么值得做
    ///
    /// managed install 是一条前台命令、全程不打输出，而那条命令里最久的一段就是下载
    /// （冷缓存实测约 1 GiB）。下载这件事可以提前、可以挪后台、断了重来也只是重下没
    /// 命中的那几个；install 那条命令却必须一口气跑完，中间断了就是整棵树重拼。
    pub(super) fn prefetch(&self, args: &[&str]) -> Result<()> {
        let (ch, background) = prefetch_args(args, self.channel.unwrap_or(Channel::Canary))?;
        if background {
            return self.prefetch_background(ch);
        }
        let r = self.prefetch_now(ch);
        // 收掉自己那份 PID 文件，成败都收：PID 会被系统回收，留着它会让下一次
        // `--background` 把一个毫不相干的新进程当成「还有预热在跑」而拒绝开工。
        let pid_file = self.run_dir.join("prefetch.pid");
        if proc::read_pid_file(&pid_file).is_some_and(|p| p == std::process::id() as i32) {
            let _ = std::fs::remove_file(&pid_file);
        }
        r
    }

    fn prefetch_now(&self, ch: Channel) -> Result<()> {
        cmd::require("npm", "它随 Node.js 一起装：https://nodejs.org/")?;
        let npmrc = Npmrc::write(NPM_REGISTRY);
        let env = self.npm_env(npmrc.as_ref());

        let version = self.prefetch_target(ch, &env)?;
        let lock = self.resolve_tree(&version, &env)?;
        let picked = prefetch_specs(&lock, &NpmPlatform::current());
        if !picked.foreign.is_empty() {
            // 一个都不该有：这棵树里 539 个 resolved 全在钉死的那个 registry 上。真冒
            // 出来一个，说明上游改了依赖的写法，那时候该来看一眼而不是默默去下。
            ui::warn(format!(
                "有 {} 个包的下载地址不在 {} 上，跳过不灌: {}",
                picked.foreign.len(),
                NPM_REGISTRY,
                picked.foreign.join(" ")
            ));
        }
        if picked.specs.is_empty() {
            bail!("从 lockfile 里没挑出一个要下的包，解析结果不对（期待几百个）");
        }
        ui::step(format!(
            "{version}: 本机要下 {} 个包（另外 {} 个是别的平台、{} 个装在父包的 tarball 里）",
            picked.specs.len(),
            picked.other_platform,
            picked.bundled
        ));
        self.fill_cache(&picked.specs, &env)
    }

    /// 一批批灌缓存，并把进度报出来。
    fn fill_cache(&self, specs: &[String], env: &[EnvEdit]) -> Result<()> {
        let total = specs.len();
        let cache = npm_cacache();
        let before = dir_size(&cache);

        let bar = progress_bar();
        bar.set_style(prefetch_style());
        bar.set_length(total as u64);
        // 第一批跑完才有字节数可报，而第一批里只要有个 155 MiB 的 codex 就是好几分钟，
        // 那几分钟里这行得先说清楚它在干什么。
        bar.set_message("灌缓存".to_string());
        bar.enable_steady_tick(STEADY_TICK);
        let mut next = Instant::now();
        let mut failed = 0usize;
        let mut first_err: Option<String> = None;

        for chunk in specs.chunks(PREFETCH_BATCH) {
            let mut argv = vec![
                "npm".to_string(),
                "cache".to_string(),
                "add".to_string(),
                "--registry".to_string(),
                NPM_REGISTRY.to_string(),
            ];
            argv.extend(chunk.iter().cloned());
            // 输出收起来：成功时 `npm cache add` 一个字都不打（实测），失败时要的是
            // 末尾那几行汇总，而不是 12 个包的 npm 日志盖掉进度条。
            match cmd::try_run_with(&argv, env) {
                Ok(o) if o.success() => {}
                Ok(o) => {
                    failed += chunk.len();
                    first_err.get_or_insert_with(|| last_lines(&o.stderr, 3));
                }
                Err(e) => {
                    failed += chunk.len();
                    first_err.get_or_insert_with(|| e.to_string());
                }
            }
            bar.inc(chunk.len() as u64);
            // 量缓存跟 InstallWatch 用同一套限频：一趟 dir_size 在几 GiB 的缓存上要
            // 几百毫秒，而缓存全命中的时候每批几乎是瞬间完事，不限频就全花在量上了。
            if Instant::now() >= next {
                let t0 = Instant::now();
                let added = dir_size(&cache).saturating_sub(before);
                bar.set_message(format!("已灌 {}", human_size(added)));
                next = Instant::now() + MEASURE_EVERY.max(t0.elapsed() * 10);
            }
            if bar.is_hidden() {
                // 非终端（后台跑、重定向到日志）那条路上进度条是隐藏的，日志里得留下
                // 点东西，否则后台那几十分钟在日志里是一片空白。
                ui::info(format!("已灌 {}/{total} 个包", bar.position()));
            }
        }
        bar.finish_and_clear();

        let added = dir_size(&cache).saturating_sub(before);
        if let Some(e) = first_err {
            ui::warn(format!("有没灌上的，第一条失败是:\n{e}"));
        }
        if failed == total {
            bail!("{total} 个包一个都没灌进缓存，预热没起作用");
        }
        if added == 0 {
            ui::ok(format!("{total} 个包缓存里本来就有，这次一个字节都没下。"));
        } else {
            ui::ok(format!(
                "预热完成: {} 个包就绪，缓存涨了 {}{}。",
                total - failed,
                human_size(added),
                if failed > 0 {
                    format!("，{failed} 个没灌上（装的时候会自己再下）")
                } else {
                    String::new()
                }
            ));
        }
        ui::hint(format!(
            "接下来 `fundeploy service paperclip {}` 的下载段就只剩没命中的那几个了。",
            if self.cli_path().is_some() {
                "upgrade"
            } else {
                "install"
            }
        ));
        Ok(())
    }

    /// 通道当前指向的版本。
    fn prefetch_target(&self, ch: Channel, env: &[EnvEdit]) -> Result<String> {
        // 先问官方：`update --check` 给的那个 target，就是 install / upgrade 自己要去
        // 装的那一个。
        if let Some(c) = self.update_check(ch) {
            let v = if c.target.is_empty() {
                c.current
            } else {
                c.target
            };
            if !v.is_empty() {
                ui::step(format!("{} 通道指向 {v}（官方 update --check）", ch.name()));
                return Ok(v);
            }
        }
        // 问不出来（还没装 CLI，或者离线）就退回 dist-tag。猜错的代价有限：差的那几个
        // 是 @paperclipai/* 自己那十来个包，大头（各家 agent CLI 的二进制）相邻两版用
        // 的是同一份，照样命中。
        let spec = format!("paperclipai@{}", ch.dist_tag());
        let out = cmd::try_run_with(
            [
                "npm",
                "view",
                &spec,
                "version",
                "--registry",
                NPM_REGISTRY,
                "--json",
            ],
            env,
        )?;
        if !out.success() {
            bail!(
                "问不出 {} 通道的版本（paperclipai 没装，npm view 也失败了）:\n{}",
                ch.name(),
                last_lines(&out.stderr, 3)
            );
        }
        let v = npm_view_version(&out.stdout).ok_or_else(|| {
            anyhow!(
                "认不出 npm view {spec} version 的输出: {}",
                out.stdout.trim()
            )
        })?;
        ui::step(format!(
            "{} 通道指向 {v}（npm 的 {} dist-tag；paperclipai 还没装，问不到官方那边）",
            ch.name(),
            ch.dist_tag()
        ));
        Ok(v)
    }

    /// 解析出完整依赖图，拿到每个包的下载地址。
    ///
    /// `--package-lock-only` 只写一份 lockfile、不装任何东西，但它要把每个包的
    /// manifest 问一遍（实测 888 个条目约 50 秒）。
    ///
    /// # 那个 allow-remote
    ///
    /// npm 12 新加了 `--allow-remote`，默认 `none`；而 `@paperclipai/server` 带
    /// `bundleDependencies`，arborist 为了读出里面捆了哪些包，必须先把它的 tarball
    /// 取下来解开（`pacote.extract`），于是整条解析被 `EALLOWREMOTE` 顶回来。npm 11
    /// 没有这条策略，所以这件事只在 npm 12 上发作。
    ///
    /// 这里放开它的前提是取回的范围已经被另一道闸门管住：[`prefetch_specs`] 逐条核对
    /// `resolved` 的主机名，不在钉死的 registry 上的一个都不灌。
    fn resolve_tree(&self, version: &str, env: &[EnvEdit]) -> Result<serde_json::Value> {
        let scratch = Scratch::new("paperclip-prefetch")?;
        let mut env = env.to_vec();
        env.push((
            "npm_config_allow_remote".to_string(),
            Some("all".to_string()),
        ));

        let bar = progress_bar();
        bar.set_style(downloading_style(false));
        bar.set_message("解析依赖图（只问元数据，不装东西）".to_string());
        bar.enable_steady_tick(STEADY_TICK);
        let code = cmd::spawn_status_in(
            [
                "npm",
                "install",
                "--prefix",
                &scratch.path().display().to_string(),
                "--package-lock-only",
                &format!("paperclipai@{version}"),
                "--registry",
                NPM_REGISTRY,
                "--no-audit",
                "--no-fund",
                // 自己画进度条，不跟 npm 的抢同一行。
                "--no-progress",
                // npm ≥11.19 会在末尾列出被跳过的 install 脚本。那是装的时候才相关的
                // 事，这一步什么都没装，留着只会让人以为出了问题。
                "--loglevel",
                "error",
            ],
            &env,
            None,
        );
        bar.finish_and_clear();
        match code? {
            Some(0) => {}
            other => bail!(
                "解析 paperclipai@{version} 的依赖图失败（退出码 {}）",
                other.map_or_else(|| "信号终止".to_string(), |c| c.to_string())
            ),
        }

        let path = scratch.path().join("package-lock.json");
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("读不到解析出来的 lockfile: {}", path.display()))?;
        serde_json::from_str(&raw).with_context(|| format!("解析不了 {}", path.display()))
    }

    /// 把预热丢到后台去跑。
    ///
    /// 预热天然适合后台：它只往 npm 的缓存里写，不碰 payload、不碰服务、不改任何
    /// 配置，断了重来也只是重下没命中的那几个。
    fn prefetch_background(&self, ch: Channel) -> Result<()> {
        let exe = std::env::current_exe().context("取不到 fundeploy 自己的路径")?;
        let log = self.run_dir.join("prefetch.log");
        let pid_file = self.run_dir.join("prefetch.pid");
        if let Some(pid) = proc::read_pid_file(&pid_file).filter(|p| proc::is_alive(*p)) {
            bail!(
                "已经有一个预热在后台跑着（PID {pid}）。看进度: tail -f {}",
                log.display()
            );
        }
        let argv = vec![
            exe.display().to_string(),
            "service".to_string(),
            "paperclip".to_string(),
            "prefetch".to_string(),
            ch.name().to_string(),
        ];
        let pid = proc::spawn_detached(&argv, &env::home(), &self.cli_env(&self.config()), &log)?;
        proc::write_pid_file(&pid_file, pid)?;
        ui::ok(format!(
            "{} 通道的预热已在后台开始（PID {pid}）。",
            ch.name()
        ));
        ui::hint(format!("看进度: tail -f {}", log.display()));
        ui::hint(format!("要停掉: kill {pid}"));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 暂存目录不会和正式_payload_认错() {
        let store = tempfile::tempdir().unwrap();
        // 正式 payload（装完 rename 出来的）和那份临时 npmrc 都不该被当成暂存目录。
        std::fs::create_dir(store.path().join("2026.1011.0-canary.7")).unwrap();
        std::fs::write(store.path().join(".npmrc-123-456"), "x").unwrap();
        assert_eq!(staging_dir(store.path()), None);

        let s = store.path().join(".2026.1012.0-canary.1.tmp-123-456");
        std::fs::create_dir(&s).unwrap();
        assert_eq!(staging_dir(store.path()), Some(s));
    }

    #[test]
    fn 有多个暂存目录时挑新的那个() {
        // 上一次被 kill -9 打断的话，上游 finally 里的清理没跑完，残骸会留在 store 里。
        let store = tempfile::tempdir().unwrap();
        let old = store.path().join(".1.0.0.tmp-1-100");
        let new = store.path().join(".2.0.0.tmp-1-200");
        std::fs::create_dir(&old).unwrap();
        std::fs::create_dir(&new).unwrap();
        // mtime 和名字两条判据在这儿同向，不依赖文件系统的 mtime 精度。
        assert_eq!(staging_dir(store.path()), Some(new));
    }

    #[test]
    fn 暂存目录名里认得出版本号() {
        assert_eq!(
            staging_version(".2026.1011.0-canary.7.tmp-98765-1760000000000"),
            Some("2026.1011.0-canary.7")
        );
        // 不是暂存目录的名字一个都不认，宁可让进度条显示「依赖树」。
        assert_eq!(staging_version("2026.1011.0"), None);
        assert_eq!(staging_version(".npmrc-123-456"), None);
        assert_eq!(staging_version("..tmp-1-2"), None);
    }

    /// 造一个只有路径、不碰环境的 watch：cache 固定为 None，免得测试去量本机那个
    /// 几 GB 的 npm 缓存。
    fn watch(home: &Path) -> InstallWatch {
        let cli = home.join("cli");
        InstallWatch {
            store: cli.join("installs").join("npm"),
            current: cli.join("current"),
            cache: None,
            bar: None,
            next: Instant::now(),
            last: 0,
            cache_base: 0,
            started: None,
        }
    }

    #[test]
    fn 暂存目录还空着的时候不摆分母() {
        let home = tempfile::tempdir().unwrap();
        let store = home.path().join("cli/installs/npm");
        std::fs::create_dir_all(&store).unwrap();
        std::fs::create_dir_all(home.path().join("cli/current")).unwrap();
        std::fs::write(home.path().join("cli/current/payload"), vec![0u8; 1000]).unwrap();

        let mut w = watch(home.path());
        // 还没建暂存目录：什么都不该发生。
        w.tick();
        assert!(w.bar.is_none(), "上游还在解析版本号，不该先画进度条");

        // 暂存目录建好但是空的——npm 正在往自己的缓存里下东西。这一段没有分母，
        // 摆上去就会算出「还要 0s」。
        std::fs::create_dir(store.join(".1.2.3.tmp-1-1")).unwrap();
        w.tick();
        let bar = w.bar.as_ref().expect("该画进度条了");
        assert_eq!(bar.length(), None, "下载段不该有分母");
        assert_eq!(w.last, 0);
    }

    #[test]
    fn 开始解包之后换成有分母的那一段() {
        let home = tempfile::tempdir().unwrap();
        let store = home.path().join("cli/installs/npm");
        let staging = store.join(".1.2.3.tmp-1-1");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::create_dir_all(home.path().join("cli/current")).unwrap();
        std::fs::write(home.path().join("cli/current/payload"), vec![0u8; 1000]).unwrap();

        let mut w = watch(home.path());
        w.tick();
        std::fs::write(staging.join("a"), vec![0u8; 400]).unwrap();
        w.next = Instant::now(); // 跳过限频，不然这一跳要等 MEASURE_EVERY
        w.tick();
        let bar = w.bar.as_ref().unwrap();
        assert_eq!(bar.length(), Some(1000), "分母该是当前 payload 的体积");
        assert_eq!(bar.position(), 400);

        // 新版比旧版大的话分母要跟着顶上去，否则最后几个百分点会停在 100% 还报
        // 「还要 0s」——实测 2.3 GiB 的估计值对上了 2.5 GiB 的实际值。
        std::fs::write(staging.join("b"), vec![0u8; 900]).unwrap();
        w.next = Instant::now();
        w.tick();
        let bar = w.bar.as_ref().unwrap();
        assert_eq!(bar.length(), Some(1300));
        assert_eq!(bar.position(), 1300);

        // rename 走了（或者失败被删了）：进度条收掉，量到的总量留着报。
        std::fs::remove_dir_all(&staging).unwrap();
        w.tick();
        assert!(
            w.bar.is_none(),
            "暂存目录没了，进度条必须立刻让位给上游的输出"
        );
        assert_eq!(w.last, 1300);
    }

    #[test]
    fn 量到一半被_rename_走不会让进度往回跳() {
        let home = tempfile::tempdir().unwrap();
        let staging = home.path().join("cli/installs/npm/.1.2.3.tmp-1-1");
        std::fs::create_dir_all(&staging).unwrap();
        let mut w = watch(home.path());
        w.tick();
        std::fs::write(staging.join("a"), vec![0u8; 500]).unwrap();
        w.next = Instant::now();
        w.tick();
        assert_eq!(w.last, 500);

        // 模拟「这一趟读到的比上一趟少」：目录被清空了但还在。
        std::fs::remove_file(staging.join("a")).unwrap();
        w.next = Instant::now();
        w.tick();
        assert_eq!(w.last, 500, "只认涨的那一侧");
        assert_eq!(w.bar.as_ref().unwrap().position(), 500);
    }

    #[test]
    fn 缓存冷热三种说法各自说准() {
        let cache = Path::new("/tmp/不存在的缓存/_cacache");
        let full = 1_000_000_000;
        // 空缓存：必须明说这次是全量，并且把路径摆出来（接着那句「别清它」才有着落）。
        let s = cache_state_of(0, cache, full);
        assert!(
            s.contains("空的") && s.contains("全量") && s.contains("_cacache"),
            "实际: {s}"
        );
        // 比整棵树小：只能说「装不满」。说不出更细的——缓存是内容寻址的，里面有多少
        // 属于这棵树，不把几万个索引条目逐个解开是问不出来的。
        let s = cache_state_of(full / 2, cache, full);
        assert!(s.contains("装不满"), "实际: {s}");
        assert!(!s.contains("全量"), "半热的缓存不该说成全量: {s}");
        // 够大：这时候才值得提 prefer-offline，冷缓存下它一点用都没有。
        let s = cache_state_of(full * 2, cache, full);
        assert!(s.contains("PREFER_OFFLINE"), "实际: {s}");
    }

    #[test]
    fn 预热的参数顺序不限也都能省() {
        // 通道省掉时跟 check 一样默认 canary（装了这个项目的机器基本都在跟开发版）。
        assert_eq!(
            prefetch_args(&[], Channel::Canary).unwrap(),
            (Channel::Canary, false)
        );
        assert_eq!(
            prefetch_args(&["prod"], Channel::Canary).unwrap(),
            (Channel::Prod, false)
        );
        // 两个参数谁先谁后都认：命令行上人怎么顺手怎么打。
        for args in [
            vec!["canary", "--background"],
            vec!["--background", "canary"],
            vec!["-b", "canary"],
        ] {
            assert_eq!(
                prefetch_args(&args, Channel::Prod).unwrap(),
                (Channel::Canary, true),
                "args={args:?}"
            );
        }
        // 拼错的通道必须当场报错，不能默默按默认值去灌另一个通道的包。
        let err = prefetch_args(&["canry"], Channel::Canary)
            .unwrap_err()
            .to_string();
        assert!(err.contains("canry"), "实际: {err}");
    }

    /// 造一份跟实测结构一致的 lockfile 片段。字段取名和取值都照 npm 写的来。
    fn lock_fixture() -> serde_json::Value {
        serde_json::json!({
            "packages": {
                // 根条目：npm 为这次解析现编的 package.json，没有包可下。
                "": { "dependencies": { "paperclipai": "^2026.1015.0" } },
                "node_modules/paperclipai": {
                    "version": "2026.1015.0-canary.9",
                    "resolved": "https://registry.npmjs.org/paperclipai/-/paperclipai-2026.1015.0-canary.9.tgz"
                },
                // 别名依赖：目录名是别名，真名在 name 字段上。
                "node_modules/@openai/codex-linux-x64": {
                    "name": "@openai/codex",
                    "version": "0.156.1-linux-x64",
                    "resolved": "https://registry.npmjs.org/@openai/codex/-/codex-0.156.1-linux-x64.tgz",
                    "optional": true, "os": ["linux"], "cpu": ["x64"]
                },
                // 同一个工具的另一个版本，上游版本范围对不上导致两份并存——两份都要下。
                "node_modules/@paperclipai/server/node_modules/@openai/codex-linux-x64": {
                    "name": "@openai/codex",
                    "version": "0.160.0-linux-x64",
                    "resolved": "https://registry.npmjs.org/@openai/codex/-/codex-0.160.0-linux-x64.tgz",
                    "optional": true, "os": ["linux"], "cpu": ["x64"]
                },
                // 别的平台：npm 装的时候也会跳过。
                "node_modules/@openai/codex-win32-x64": {
                    "name": "@openai/codex",
                    "version": "0.156.1-win32-x64",
                    "resolved": "https://registry.npmjs.org/@openai/codex/-/codex-0.156.1-win32-x64.tgz",
                    "optional": true, "os": ["win32"], "cpu": ["x64"]
                },
                // 装在父包 tarball 里的，没有自己的下载地址。
                "node_modules/@paperclipai/server/node_modules/acpx": {
                    "version": "0.8.1", "inBundle": true
                },
                // 外部 URL：这棵树里本该没有，出现就只报不灌。
                "node_modules/somewhere-else": {
                    "version": "1.0.0",
                    "resolved": "https://example.com/somewhere-else-1.0.0.tgz"
                }
            }
        })
    }

    #[test]
    fn 预热清单只留本机真会下的那些() {
        let linux = NpmPlatform {
            os: "linux",
            cpu: "x64",
            libc: Some("glibc"),
        };
        let picked = prefetch_specs(&lock_fixture(), &linux);
        assert_eq!(
            picked.specs,
            vec![
                // 别名条目按真名灌：按目录名（@openai/codex-linux-x64）去灌是 404。
                "@openai/codex@0.156.1-linux-x64".to_string(),
                "@openai/codex@0.160.0-linux-x64".to_string(),
                "paperclipai@2026.1015.0-canary.9".to_string(),
            ]
        );
        assert_eq!(picked.bundled, 1, "inBundle 的不单独下");
        assert_eq!(picked.other_platform, 1, "win32 那份该跳过");
        assert_eq!(
            picked.foreign,
            vec!["https://example.com/somewhere-else-1.0.0.tgz"]
        );

        // 换台 mac 就该挑出另一批；这里的 fixture 里 linux 专属的那两个都得让开。
        let mac = NpmPlatform {
            os: "darwin",
            cpu: "arm64",
            libc: None,
        };
        let picked = prefetch_specs(&lock_fixture(), &mac);
        assert_eq!(
            picked.specs,
            vec!["paperclipai@2026.1015.0-canary.9".to_string()]
        );
        assert_eq!(picked.other_platform, 3);
    }

    #[test]
    fn 平台约束的两种写法都认() {
        let v = |s: &str| serde_json::from_str::<serde_json::Value>(s).unwrap();
        // 字段缺失就是没约束。
        assert!(constraint_ok(&serde_json::Value::Null, Some("linux")));
        // 允许列表。
        assert!(constraint_ok(&v(r#"["linux","darwin"]"#), Some("linux")));
        assert!(!constraint_ok(&v(r#"["darwin"]"#), Some("linux")));
        // 排除写法：npm 里确实有 ["!win32"] 这种。
        assert!(constraint_ok(&v(r#"["!win32"]"#), Some("linux")));
        assert!(!constraint_ok(&v(r#"["!linux"]"#), Some("linux")));
        // 混用时排除优先。
        assert!(!constraint_ok(&v(r#"["linux","!linux"]"#), Some("linux")));
        // 本机取值未知（非 linux 机器上的 libc）当没约束，不能把包全筛掉。
        assert!(constraint_ok(&v(r#"["glibc"]"#), None));
    }

    #[test]
    fn 认得出_npm_view_给的版本号() {
        assert_eq!(
            npm_view_version("\"2026.1015.0-canary.9\"\n").as_deref(),
            Some("2026.1015.0-canary.9")
        );
        // spec 命中多个版本时 npm 给数组，按升序，要的是最后那个。
        assert_eq!(
            npm_view_version(r#"["2026.1014.0","2026.1015.0"]"#).as_deref(),
            Some("2026.1015.0")
        );
        // 认不出来必须是 None，好让调用方报错而不是拿空字符串去拼 spec。
        assert_eq!(npm_view_version(""), None);
        assert_eq!(npm_view_version("npm error code E404"), None);
        assert_eq!(npm_view_version("\"\""), None);
    }
}
