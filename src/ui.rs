//! 终端输出与交互。
//!
//! 约定：进度与结果走 stdout，警告与错误走 stderr。这样 `fundeploy service x status`
//! 的输出可以直接重定向进文件而不混进警告。

use anyhow::{bail, Result};
use owo_colors::OwoColorize;
use std::io::IsTerminal;
use std::sync::OnceLock;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::theme::{clip, paint, AMBER, BLUE, BORDER, LAVENDER, MINT, MUTED, SURFACE, TEXT};

fn color_allowed() -> bool {
    // 按 no-color.org 约定，空字符串视同未设；非空才禁用颜色。
    if std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()) {
        return false;
    }
    if std::env::var("TERM").is_ok_and(|t| t == "dumb") {
        return false;
    }
    true
}

/// stdout 该不该上色。
fn color() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| color_allowed() && std::io::stdout().is_terminal())
}

/// stderr 该不该上色（避免仅重定向 stderr 时把 ANSI 码写进错误日志）。
fn color_stderr() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| color_allowed() && std::io::stderr().is_terminal())
}

/// 能不能问人。重定向、CI、显式 NONINTERACTIVE 都算不能。
pub fn interactive() -> bool {
    if truthy_env("NONINTERACTIVE") || truthy_env("CI") {
        return false;
    }
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

fn truthy_env(name: &str) -> bool {
    crate::common::env::flag(name, false)
}

/// 一个步骤的开始。
pub fn step(msg: impl AsRef<str>) {
    let msg = msg.as_ref();
    if color() {
        println!("{} {msg}", paint(" › ", BLUE, true));
    } else {
        println!("==> {msg}");
    }
}

/// 分节标题，重定向时使用纯文本。
pub fn heading(msg: impl AsRef<str>) {
    let msg = msg.as_ref();
    if color() {
        println!("{}", paint(&format!("  {msg}  "), BLUE, true));
    } else {
        println!("== {msg} ==");
    }
}

pub fn info(msg: impl AsRef<str>) {
    println!("{}", msg.as_ref());
}

/// 跟在上一行后面的补充说明，缩进两格。
///
/// 转暗色：这里放的是日志路径、安装提示这类「在那儿、但不用盯着看」的东西，
/// 和正文同色会把真正要看的状态淹掉。
pub fn hint(msg: impl AsRef<str>) {
    for line in msg.as_ref().lines() {
        if color() {
            println!("  {}", line.dimmed());
        } else {
            println!("  {line}");
        }
    }
}

/// 错误。走 stderr，和 `warn` 一样不混进 stdout。
pub fn error(msg: impl AsRef<str>) {
    let msg = msg.as_ref();
    if color_stderr() {
        eprintln!("{} {msg}", "错误:".red().bold());
    } else {
        eprintln!("错误: {msg}");
    }
}

/// 次要内容转暗：端口号、版本号、路径。
pub fn muted(s: impl AsRef<str>) -> String {
    let s = s.as_ref();
    if color() {
        s.dimmed().to_string()
    } else {
        s.to_string()
    }
}

/// 主体加粗：服务名、工具名——一列里最该先被看到的那个词。
pub fn strong(s: impl AsRef<str>) -> String {
    let s = s.as_ref();
    if color() {
        s.bold().to_string()
    } else {
        s.to_string()
    }
}

/// 状态的四种成色。
#[derive(Clone, Copy)]
pub enum State {
    /// 在跑、装好了。
    Up,
    /// 该在跑却没在跑——要让人一眼看见。
    Down,
    /// 半开半合：多进程服务只起来了一部分。
    Partial,
    /// 没装、没配，不算故障。
    Idle,
}

impl State {
    fn mark(self) -> &'static str {
        match self {
            Self::Idle => "○",
            _ => "●",
        }
    }
}

/// `● 在运行` 这样的状态标记。
///
/// 用圆点而不是纯文字：状态列里中英文混排，宽度对不齐，一个前置符号能让眼睛
/// 直接扫到这一列。空心点留给「本来就没有」，和「出问题了」区分开。
pub fn state(st: State, label: impl AsRef<str>) -> String {
    let label = label.as_ref();
    if color() {
        card_state(st, label)
    } else {
        format!("{} {label}", st.mark())
    }
}

fn card_state(state: State, label: &str) -> String {
    let foreground = match state {
        State::Up => MINT,
        State::Down => 210,
        State::Partial => AMBER,
        State::Idle => MUTED,
    };
    paint(&format!("{} {label}", state.mark()), foreground, false)
}

/// 一张盒绘卡片：标题 + 右上角状态 + 若干字段 + 框外脚注。
///
/// ```text
/// ╭─ paperclip ───────────────────── ○ 未运行 ─╮
/// │  版本       2026.1014.0-canary.11          │
/// │  端口 8804  ○ 没监听                       │
/// ╰────────────────────────────────────────────╯
///   ! 官方服务管理器不可用，容器 / WSL1 里常见
/// ```
///
/// # 框宽是跟着终端走的
///
/// 盒绘最容易坏在宽度上：框线比终端宽，每行都会被硬折，右边框散落满屏。所以
/// 框宽取「内容需要的」和「终端给得起的」里的小值，放不下的字段值中间省略
/// （见 [`ellipsize`]）。日志路径这种长字符串正是撑破框的典型。
///
/// 输出不是终端时（重定向、CI）**退回无框的对齐键值**：框线字符进了日志文件
/// 只是噪音，而那时也问不出终端宽度。
pub struct Card {
    title: String,
    badge: Option<(State, String)>,
    /// 字段值一律以**纯文本**存着，状态符号等到渲染时才加。
    ///
    /// 不能存上好色的字符串：框宽要按可见宽度算，而 ANSI 转义码在
    /// [`cols`] 眼里也是字符；[`ellipsize`] 更会把转义序列拦腰截断，屏幕上
    /// 就是一串乱码。
    rows: Vec<CardRow>,
    notes: Vec<String>,
}

/// 框的最大宽度。再宽就只是把字段拉得满屏乱飞，眼睛跟不住。
const CARD_MAX: usize = 88;

/// 卡片里的一行。
enum CardRow {
    /// 分节标题。同一张卡片里混着两类字段时（比如「包版本」和「运行状态」），
    /// 键名往往长得差不多，不分节看起来就是同一组东西重复了两遍。
    Section(String),
    /// 键、纯文本的值、可选的状态成色。
    Field(String, String, Option<State>),
}

/// 分节之后字段要缩进，让层级看得出来。
const SECTION_INDENT: usize = 2;

impl Card {
    pub fn new(title: impl AsRef<str>) -> Self {
        Self {
            title: title.as_ref().to_string(),
            badge: None,
            rows: Vec::new(),
            notes: Vec::new(),
        }
    }

    /// 右上角的整体结论，比如「○ 未运行」。
    pub fn badge(mut self, st: State, label: impl AsRef<str>) -> Self {
        self.badge = Some((st, label.as_ref().to_string()));
        self
    }

    pub fn row(mut self, key: impl AsRef<str>, value: impl AsRef<str>) -> Self {
        self.rows.push(CardRow::Field(
            key.as_ref().to_string(),
            value.as_ref().to_string(),
            None,
        ));
        self
    }

    /// 带状态符号的字段。
    pub fn row_state(mut self, key: impl AsRef<str>, st: State, label: impl AsRef<str>) -> Self {
        self.rows.push(CardRow::Field(
            key.as_ref().to_string(),
            label.as_ref().to_string(),
            Some(st),
        ));
        self
    }

    /// 起一个新分节。后面的字段会缩进到它下面。
    pub fn section(mut self, name: impl AsRef<str>) -> Self {
        self.rows.push(CardRow::Section(name.as_ref().to_string()));
        self
    }

    /// 这张卡片有没有分节——有的话所有字段都缩进，免得对不齐。
    fn has_sections(&self) -> bool {
        self.rows.iter().any(|r| matches!(r, CardRow::Section(_)))
    }

    /// 框外的补充说明。上游那种一长段英文报错放这儿，不参与框宽计算。
    pub fn note(mut self, msg: impl AsRef<str>) -> Self {
        self.notes.push(msg.as_ref().to_string());
        self
    }

    pub fn print(self) {
        if !color() {
            // 没有 tty 就别画框：问不出宽度，而且框线在日志里只是噪音。
            self.print_plain();
            return;
        }
        let indent = if self.has_sections() {
            SECTION_INDENT
        } else {
            0
        };
        let budget = term_cols().saturating_sub(3).min(CARD_MAX);
        if budget < 28 {
            self.print_plain();
            return;
        }
        let key_w = self
            .rows
            .iter()
            .filter_map(|r| match r {
                CardRow::Field(k, ..) => Some(cols(k)),
                CardRow::Section(_) => None,
            })
            .max()
            .unwrap_or(0)
            .min(budget / 3);
        // 带状态的值前面要留出「● 」两列。
        let val_w = self
            .rows
            .iter()
            .filter_map(|r| match r {
                CardRow::Field(_, v, st) => Some(cols(v) + if st.is_some() { 2 } else { 0 }),
                CardRow::Section(_) => None,
            })
            .max()
            .unwrap_or(0);
        // 分节标题自己也得装得下。
        let section_need = self
            .rows
            .iter()
            .filter_map(|r| match r {
                CardRow::Section(n) => Some(2 + cols(n) + 1),
                CardRow::Field(..) => None,
            })
            .max()
            .unwrap_or(0);

        // 框内布局: "│" + 2 空格 + 缩进 + 键 + 2 空格 + 值 + 1 空格 + "│"
        let inner_need = 2 + indent + key_w + 2 + val_w + 1;
        // 标题那行也要装得下: "╭─ " + 标题 + " ─ " + 徽章 + " ─╮"
        let badge_w = self.badge.as_ref().map(|(_, l)| cols(l) + 2).unwrap_or(0);
        let title_need = 3 + cols(&self.title) + 3 + badge_w + 3;

        let inner = inner_need
            .max(title_need)
            .max(section_need)
            .clamp(20, budget);

        self.print_top(inner);
        let mut first_section = true;
        for row in &self.rows {
            match row {
                CardRow::Section(name) => {
                    // 节与节之间空一行；第一节紧贴标题，不空。
                    if !first_section {
                        println!(
                            "{}{}{}",
                            paint("│", BORDER, false),
                            paint(&" ".repeat(inner), TEXT, false),
                            paint("│", BORDER, false)
                        );
                    }
                    first_section = false;
                    let name = ellipsize(name, inner - 3);
                    println!(
                        "{}{}{}",
                        paint("│", BORDER, false),
                        paint(&pad(&format!("  {name}"), inner), LAVENDER, true),
                        paint("│", BORDER, false)
                    );
                }
                CardRow::Field(k, v, st) => {
                    // 先按纯文本算好省略和补齐，再上色——顺序反了框就歪。
                    let mark_w = if st.is_some() { 2 } else { 0 };
                    let val_room = inner.saturating_sub(2 + indent + key_w + 2 + 1 + mark_w);
                    let k = ellipsize(k, key_w);
                    let v = ellipsize(v, val_room);
                    let shown = st.map_or_else(|| paint(&v, TEXT, false), |s| card_state(s, &v));
                    let visible = 2 + indent + key_w + 2 + mark_w + cols(&v);
                    println!(
                        "{}{}{}{}{}",
                        paint("│", BORDER, false),
                        paint(
                            &format!("{}{k}  ", " ".repeat(indent + 2), k = pad(&k, key_w)),
                            MUTED,
                            false
                        ),
                        shown,
                        paint(&" ".repeat(inner.saturating_sub(visible)), TEXT, false),
                        paint("│", BORDER, false)
                    );
                }
            }
        }
        println!(
            "{}",
            paint(&format!("╰{}╯", "─".repeat(inner)), BORDER, false)
        );
        for n in &self.notes {
            println!("  {} {}", "!".yellow(), n.dimmed());
        }
    }

    /// 标题行：`╭─ 标题 ───…─── 徽章 ─╮`，中间用框线填满。
    ///
    fn print_top(&self, inner: usize) {
        let badge = self
            .badge
            .as_ref()
            .map(|(st, label)| (*st, ellipsize(label, (inner / 2).saturating_sub(4))));
        let badge_width = badge.as_ref().map_or(0, |(_, label)| cols(label) + 5);
        let title = ellipsize(&self.title, inner.saturating_sub(badge_width + 3));
        let rule = "─".repeat(inner.saturating_sub(badge_width + cols(&title) + 3));
        let right = badge.map_or_else(String::new, |(st, label)| {
            format!(
                "{}{}{}",
                paint(" ", BORDER, false),
                card_state(st, &label),
                paint(" ─", BORDER, false)
            )
        });
        println!(
            "{}{}{}{}{}",
            paint("╭─ ", BORDER, false),
            paint(&title, BLUE, true),
            paint(&format!(" {rule}"), BORDER, false),
            right,
            paint("╮", BORDER, false)
        );
    }

    /// 无框退回版：同样对齐，只是没有框线。
    fn print_plain(self) {
        let key_w = self
            .rows
            .iter()
            .filter_map(|r| match r {
                CardRow::Field(k, ..) => Some(cols(k)),
                CardRow::Section(_) => None,
            })
            .max()
            .unwrap_or(0);
        let indent = if self.has_sections() {
            SECTION_INDENT
        } else {
            0
        };
        match &self.badge {
            Some((st, label)) => println!("{} {}", self.title, state(*st, label)),
            None => println!("{}", self.title),
        }
        for row in &self.rows {
            match row {
                CardRow::Section(name) => println!("  {name}"),
                CardRow::Field(k, v, st) => {
                    let shown = match st {
                        Some(s) => state(*s, v),
                        None => v.clone(),
                    };
                    println!("  {}{}  {shown}", " ".repeat(indent), pad(k, key_w));
                }
            }
        }
        for n in &self.notes {
            println!("  ! {n}");
        }
    }
}

/// 一个转着的小圈，给「不知道还要多久，但得让人知道没死」的等待用。
///
/// 典型场景是 start 之后等端口被监听：最长能等半分钟，期间屏幕上一个字都不动，
/// 跟卡死没有区别。
///
/// 非终端时内部是 [`indicatif::ProgressBar::hidden`]，`tick` 和收尾全都不画东西
/// ——CI 的日志里不该出现一串刷屏的回车。进度画到 **stderr**，这样
/// `fundeploy ... > out.txt` 里不会混进动画残渣。
pub struct Spinner(indicatif::ProgressBar);

impl Spinner {
    pub fn new(msg: impl Into<String>) -> Self {
        let bar = if color_stderr() {
            let b = indicatif::ProgressBar::new_spinner();
            b.set_style(
                indicatif::ProgressStyle::with_template("{spinner:.cyan} {msg} 已过 {elapsed}")
                    .expect("spinner 模板应始终合法"),
            );
            b
        } else {
            indicatif::ProgressBar::hidden()
        };
        bar.set_message(msg.into());
        Self(bar)
    }

    pub fn tick(&self) {
        self.0.tick();
    }

    /// 等到了。把那行动画换成一条静态的成功行。
    pub fn done(self, msg: impl AsRef<str>) {
        self.0.finish_and_clear();
        ok(msg);
    }

    /// 没等到。只清掉动画，失败的话由调用方自己说——它知道为什么。
    pub fn give_up(self) {
        self.0.finish_and_clear();
    }
}

/// farfarfun 的那句话。四段分色打，读起来有节奏。
///
/// 出处是组织主页 `.github/profile/README.md`：活到老，学到老，玩到老，老牛老牛了。
const SLOGAN: [&str; 4] = ["活到老", "学到老", "玩到老", "老牛老牛了"];

/// 顶部品牌区块：自己是谁、哪一版、谁做的。
///
/// 只在「人主动打开菜单」和 `about` 里打。每条命令都顶一段 banner 很快就会变成
/// 噪音——`fundeploy service x status` 的输出是会被重定向进文件、被 grep 的。
pub fn brand() {
    let version = env!("CARGO_PKG_VERSION");
    if !color() {
        println!("fundeploy {version}  开发环境、工具与服务管理");
        println!("farfarfun  {}", SLOGAN.join("，"));
        println!("https://github.com/farfarfun");
        return;
    }

    menu_banner();
    // 四段各上一色，从冷到暖：读的时候自然有停顿，也不至于花。
    let painted = format!(
        "{} {} {} {} {} {} {}",
        SLOGAN[0].cyan(),
        "·".dimmed(),
        SLOGAN[1].green(),
        "·".dimmed(),
        SLOGAN[2].yellow(),
        "·".dimmed(),
        SLOGAN[3].magenta().bold()
    );
    println!("  {}  {painted}", "farfarfun".magenta().bold());
    println!("  {}", "https://github.com/farfarfun".dimmed());
}

pub fn menu_banner() {
    let version = env!("CARGO_PKG_VERSION");
    if !color() {
        println!("fundeploy {version}  开发环境、工具与服务管理");
        return;
    }
    let width = term_cols().saturating_sub(1).min(58);
    let title = "  fundeploy";
    let version = format!("v{version}  ");
    let subtitle = ellipsize("  开发环境、工具与服务管理", width.max(1));
    if cols(title) + cols(&version) <= width {
        println!(
            "{}{}",
            paint(&pad(title, width - cols(&version)), BLUE, true),
            paint(&version, LAVENDER, true)
        );
    } else {
        println!(
            "{}",
            paint(&ellipsize(title.trim(), width.max(1)), BLUE, true)
        );
    }
    println!("{}", paint(&pad(&subtitle, width), MUTED, false));
}

/// 给 inquire 装上和自家输出同一套色板。
///
/// 必须在任何交互之前调一次。inquire 默认有自己的配色，不走 [`color`]，所以
/// `NO_COLOR` 和重定向时要显式给它一份空配置，否则 ANSI 码会漏进管道里。
pub fn init_theme() {
    use inquire::ui::{Attributes, Color, RenderConfig, StyleSheet, Styled};

    if !color() {
        inquire::set_global_render_config(RenderConfig::empty());
        return;
    }

    let accent = Color::AnsiValue(BLUE);
    let surface = Color::AnsiValue(SURFACE);
    let mut cfg = RenderConfig::default_colored();
    cfg.prompt_prefix = Styled::new("◆").with_fg(accent).with_bg(surface);
    cfg.prompt = StyleSheet::new().with_attr(Attributes::BOLD);
    // 答完把 `?` 换成对勾，和 ok() 的 `✓` 一致：回头看历史记录时，哪些问完了
    // 一眼就分得出。
    cfg.answered_prompt_prefix = Styled::new("✓")
        .with_fg(Color::AnsiValue(MINT))
        .with_bg(surface);
    cfg.highlighted_option_prefix = Styled::new("❯").with_fg(accent).with_bg(surface);
    cfg.scroll_up_prefix = Styled::new("↑")
        .with_fg(Color::AnsiValue(LAVENDER))
        .with_bg(surface);
    cfg.scroll_down_prefix = Styled::new("↓")
        .with_fg(Color::AnsiValue(LAVENDER))
        .with_bg(surface);
    cfg.selected_option = Some(
        StyleSheet::new()
            .with_fg(accent)
            .with_bg(Color::AnsiValue(24))
            .with_attr(Attributes::BOLD),
    );
    cfg.answer = StyleSheet::new()
        .with_fg(Color::AnsiValue(MINT))
        .with_bg(surface);
    cfg.help_message = StyleSheet::new()
        .with_fg(Color::AnsiValue(MUTED))
        .with_bg(surface);
    cfg.canceled_prompt_indicator = Styled::new("已取消").with_fg(Color::DarkGrey);
    inquire::set_global_render_config(cfg);
}

pub fn ok(msg: impl AsRef<str>) {
    let msg = msg.as_ref();
    if color() {
        println!("{} {msg}", "✓".green().bold());
    } else {
        println!("✓ {msg}");
    }
}

pub fn warn(msg: impl AsRef<str>) {
    let msg = msg.as_ref();
    if color_stderr() {
        eprintln!("{} {msg}", "警告:".yellow().bold());
    } else {
        eprintln!("警告: {msg}");
    }
}

pub fn blank() {
    println!();
}

/// 按终端显示宽度右补空格，用来排表格的列。
///
/// `format!("{:<12}")` 补的是**字符数**，而终端里一个汉字占两列。中英混排的表
/// 格用它排出来是歪的——「清华大学」和 `aliyun` 都算 4 和 6 个字符，实际却是
/// 8 列和 6 列。
pub fn pad(s: &str, width: usize) -> String {
    format!("{s}{}", " ".repeat(width.saturating_sub(cols(s))))
}

/// 一段文字在终端里占几列。
pub fn cols(s: &str) -> usize {
    s.width()
}

/// 终端有多少列。问不出来（重定向、没有 tty）就按 80 算。
///
/// 这个值决定盒绘的框宽。问不出来时**必须**给个保守的默认值而不是「不限宽」：
/// 框线一旦比终端宽，每一行都会被硬折到下一行，右边框散落得满屏都是。
pub fn term_cols() -> usize {
    if let Ok((cols, _)) = crossterm::terminal::size() {
        if cols > 2 {
            return usize::from(cols);
        }
    }
    80
}

/// 把过长的内容掐到 `max` 列，省略号放中间。
///
/// 中间省略而不是截尾：这里装的多半是路径，`~/.paperclip/run/default/app.log`
/// 掐掉尾巴就只剩一堆目录名，分不出是哪个文件；留头留尾才看得出「哪个服务的
/// 什么文件」。
pub fn ellipsize(s: &str, max: usize) -> String {
    if cols(s) <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let keep = max - 1;
    let tail_budget = keep / 2;
    let head_budget = keep - tail_budget;

    let mut head = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > head_budget {
            break;
        }
        head.push(c);
        used += w;
    }

    let mut tail: Vec<char> = Vec::new();
    let mut used = 0;
    for c in s.chars().rev() {
        let w = c.width().unwrap_or(0);
        if used + w > tail_budget {
            break;
        }
        tail.push(c);
        used += w;
    }
    tail.reverse();

    clip(
        &format!("{head}…{}", tail.into_iter().collect::<String>()),
        max,
    )
}

/// `键: 值` 一行，值为 None 时打「未安装」。
pub fn kv(key: impl AsRef<str>, value: Option<&str>) {
    match value {
        Some(v) => println!("{}: {v}", key.as_ref()),
        None => println!("{}: 未安装", key.as_ref()),
    }
}

/// 危险操作确认。
///
/// `env_yes` 是跳过询问的环境变量名——没有它，卸载这类操作在 CI 和脚本里就
/// 完全没法用；但默认不能靠「检测不到终端就当同意」，那会让一次误连的
/// 重定向把东西删掉。所以无人值守时必须显式给这个变量，否则直接报错退出。
pub fn confirm_destructive(prompt: &str, env_yes: &str) -> Result<()> {
    if truthy_env(env_yes) {
        return Ok(());
    }
    if !interactive() {
        bail!("{prompt}\n当前是无人值守环境，确认请设 {env_yes}=1 后重试。");
    }
    let yes = inquire::Confirm::new(prompt).with_default(false).prompt()?;
    if !yes {
        bail!("已取消。");
    }
    Ok(())
}

/// 问一句是/否，默认「是」，答「不」不算错。
///
/// 和 [`confirm_destructive`] 三处不同，都是为「本来就该发生」的那类动作准备的
/// ——典型是 `stop` 停不下来时按端口强停：
///
/// - **回答「不」返回 false 而不是报错**，调用方自己决定怎么往下走。
/// - **默认选项是「是」**，常规一次 restart 只需一个回车。
/// - **无人值守时按「是」办**，不报错退出。`stop` 跑在 cron 和 CI 里是常态，
///   在那里停下来问等于让 restart 断在半路——而这恰恰是加这道确认要避免的事。
///
/// `env_yes` 给「在终端里也别问我」用。
pub fn confirm_default_yes(prompt: &str, env_yes: &str) -> bool {
    if truthy_env(env_yes) || !interactive() {
        return true;
    }
    // 读不到回答（Ctrl+C、Esc）当成「不」：那是人主动中断，不是默认路径。
    inquire::Confirm::new(prompt)
        .with_default(true)
        .prompt()
        .unwrap_or(false)
}

/// 问一个值。`default` 非空时直接回车就取它。
pub fn input(prompt: &str, default: &str) -> Result<String> {
    if !interactive() {
        bail!("{prompt}\n当前是无人值守环境，请把这个值直接写在命令行上。");
    }
    let mut q = inquire::Text::new(prompt);
    if !default.is_empty() {
        q = q.with_default(default);
    }
    Ok(q.prompt()?.trim().to_string())
}

/// 问一个口令，输入不回显。
///
/// 比起让操作者写在命令行上，这里少了一条泄漏路径：shell 历史。
pub fn password(prompt: &str) -> Result<String> {
    if !interactive() {
        bail!("{prompt}\n当前是无人值守环境，请把这个值直接写在命令行上。");
    }
    Ok(inquire::Password::new(prompt)
        .with_display_mode(inquire::PasswordDisplayMode::Masked)
        .without_confirmation()
        .prompt()?)
}

/// 从一组候选里选一个。返回 None 表示用户取消。
pub fn select<'a>(prompt: &str, options: &'a [String]) -> Result<Option<&'a String>> {
    if options.is_empty() {
        bail!("没有可选项");
    }
    if !interactive() {
        bail!("{prompt}\n当前是无人值守环境，请把选项直接写在命令行上。");
    }
    struct OptionRow<'a> {
        full: &'a String,
        visible: String,
    }
    impl std::fmt::Display for OptionRow<'_> {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.visible)
        }
    }

    let width = term_cols().saturating_sub(3).clamp(1, 76);
    let height = crossterm::terminal::size().map_or(24, |(_, rows)| usize::from(rows));
    let row_width = options
        .iter()
        .map(|option| cols(option))
        .max()
        .unwrap_or(0)
        .min(width);
    let rows = options
        .iter()
        .map(|full| OptionRow {
            full,
            visible: pad(&clip(full, width), row_width),
        })
        .collect();
    let help = if width >= 47 {
        "↑↓ 选择 · 输入筛选 · Enter 确认 · Esc 返回"
    } else {
        "↑↓ · 输入筛选 · ↵ · Esc"
    };
    let help = ellipsize(help, width);
    match inquire::Select::new(prompt, rows)
        .with_page_size(height.saturating_sub(4).clamp(1, 15))
        .with_help_message(&help)
        .with_scorer(&|input, row, _, index| {
            inquire::Select::<OptionRow<'_>>::DEFAULT_SCORER(input, row, row.full, index)
        })
        .with_formatter(&|row| row.value.visible.trim_end().to_owned())
        .prompt()
    {
        Ok(item) => Ok(Some(item.full)),
        Err(inquire::InquireError::OperationCanceled) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 补齐按显示宽度算而不是字符数() {
        // 「清华大学」占 8 列，补到 12 列只该加 4 个空格；按字符数算会加 8 个，
        // 表格就歪了。
        assert_eq!(pad("清华大学", 12).len() - "清华大学".len(), 4);
        assert_eq!(pad("aliyun", 12), "aliyun      ");
        assert_eq!(pad("官方", 4), "官方");
        // 超出宽度时不截断也不 panic。
        assert_eq!(pad("非常非常长的名字", 4), "非常非常长的名字");
    }

    #[test]
    fn 环境变量的真值形式都认() {
        let _g = crate::common::env::test_env::EnvGuard::set(&[("__FD_TEST_TRUTHY", None)]);
        for v in [
            "1", "y", "yes", "true", "on", "YES", "True", " 1 ", " true ",
        ] {
            std::env::set_var("__FD_TEST_TRUTHY", v);
            assert!(truthy_env("__FD_TEST_TRUTHY"), "没认出 {v}");
        }
        for v in ["", "   ", "0", "no", "false", "off", "随便"] {
            std::env::set_var("__FD_TEST_TRUTHY", v);
            assert!(!truthy_env("__FD_TEST_TRUTHY"), "误认了 {v}");
        }
    }

    #[test]
    fn 未设置的环境变量是假值() {
        let _g = crate::common::env::test_env::EnvGuard::set(&[("__FD_TEST_UNSET", None)]);
        assert!(!truthy_env("__FD_TEST_UNSET"));
    }

    #[test]
    fn 无人值守下没给确认变量就报错而不是默认同意() {
        // 守的是「误把确认当同意」：测试环境里没有 tty，若实现把它当同意，
        // 这里就会返回 Ok，而真实场景下对应的是把东西直接卸掉。
        let _g = crate::common::env::test_env::EnvGuard::set(&[
            ("NONINTERACTIVE", Some("1")),
            ("__FD_TEST_YES", None),
        ]);
        let err = confirm_destructive("要删吗", "__FD_TEST_YES")
            .unwrap_err()
            .to_string();
        assert!(err.contains("__FD_TEST_YES=1"), "实际: {err}");
    }

    #[test]
    fn 给了确认变量就不再询问() {
        let _g = crate::common::env::test_env::EnvGuard::set(&[("__FD_TEST_YES2", Some("1"))]);
        confirm_destructive("要删吗", "__FD_TEST_YES2").unwrap();
    }

    #[test]
    fn 默认同意的那种确认在无人值守下就是同意() {
        // 跟上面那条正好相反，也必须相反：stop 收尾的强停走的是这条，在 cron 和 CI
        // 里停下来问（或者报错）等于让 restart 断在半路。
        let _g = crate::common::env::test_env::EnvGuard::set(&[
            ("NONINTERACTIVE", Some("1")),
            ("__FD_TEST_YES3", None),
        ]);
        assert!(confirm_default_yes("要停吗", "__FD_TEST_YES3"));
    }

    #[test]
    fn 默认同意的那种确认也认跳过变量() {
        let _g = crate::common::env::test_env::EnvGuard::set(&[("__FD_TEST_YES4", Some("1"))]);
        assert!(confirm_default_yes("要停吗", "__FD_TEST_YES4"));
    }
}
