//! 本机资源面板。CPU 利用率由两次采样的差值计算。

mod gpu;

use std::fmt::Write as _;
#[cfg(windows)]
use std::io::IsTerminal;
use std::io::{self, Write};
use std::{
    thread,
    time::{Duration, Instant},
};

use anyhow::Result;
use crossterm::{
    cursor::MoveTo,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute, queue,
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use sysinfo::{DiskRefreshKind, Disks, System};

#[derive(clap::Args, Clone, Default)]
pub struct Options {
    /// 输出一次快照后退出（重定向输出时自动启用）
    #[arg(long)]
    once: bool,
}

impl Options {
    pub fn run(&self) -> Result<()> {
        #[cfg(unix)]
        let interactive = crate::ui::interactive();
        #[cfg(windows)]
        let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();
        let live = interactive && !self.once && std::env::var("TERM").as_deref() != Ok("dumb");
        let _screen = if live { Some(Screen::open()?) } else { None };
        let mut sys = System::new();
        sys.refresh_cpu_usage();
        let host = System::host_name().unwrap_or_else(|| "本机".into());
        let os = System::long_os_version().unwrap_or_else(|| std::env::consts::OS.into());
        let cores = System::physical_core_count()
            .map(|n| n.to_string())
            .unwrap_or_else(|| "未知".into());
        let mut disks = Disks::new();
        thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);

        loop {
            sys.refresh_cpu_usage();
            sys.refresh_memory();
            disks.refresh_specifics(true, DiskRefreshKind::nothing().with_storage());
            let uptime = System::uptime() / 60;
            let mut panel = format!(
                "本机状态 · {host}\n{os} · 已运行 {} 天 {} 小时 {} 分\n",
                uptime / 1440,
                uptime / 60 % 24,
                uptime % 60
            );
            let cpu = sys
                .cpus()
                .first()
                .map(|c| c.brand().trim())
                .filter(|s| !s.is_empty())
                .unwrap_or("未知型号");
            writeln!(
                panel,
                "\nCPU   {cpu}\n      {cores} 个物理核 · {} 个逻辑核",
                sys.cpus().len()
            )?;
            if !sys.cpus().is_empty() {
                writeln!(panel, "      {}  当前利用率", meter(sys.global_cpu_usage()))?;
            }
            writeln!(
                panel,
                "\n内存  {}",
                capacity(sys.used_memory(), sys.total_memory())
            )?;
            if sys.total_swap() > 0 {
                writeln!(
                    panel,
                    "交换  {}",
                    capacity(sys.used_swap(), sys.total_swap())
                )?;
            }
            writeln!(panel, "\n磁盘  已用 / 总量")?;
            if disks.list().is_empty() {
                writeln!(panel, "      未检测到可读取的卷")?;
            }
            for disk in &disks {
                let total = disk.total_space();
                writeln!(
                    panel,
                    "      {}  {}",
                    disk.mount_point().display(),
                    capacity(total.saturating_sub(disk.available_space()), total)
                )?;
            }
            writeln!(panel, "\nGPU   利用率")?;
            let gpus = gpu::sample();
            if gpus.is_empty() {
                writeln!(panel, "      暂无读数（设备或驱动未提供利用率）")?;
            }
            for (name, percent) in gpus {
                writeln!(panel, "      {}  {name}", meter(percent))?;
            }
            if live {
                writeln!(panel, "\n自动刷新 · Q / Ctrl+C 退出")?;
            }

            let mut out = io::stdout().lock();
            if live {
                queue!(out, MoveTo(0, 0), Clear(ClearType::FromCursorDown))?;
                // 原始模式下换行不会自动回到行首。
                out.write_all(panel.trim_end().replace('\n', "\r\n").as_bytes())?;
            } else {
                out.write_all(panel.as_bytes())?;
            }
            out.flush()?;
            drop(out);
            if !live || exit_requested()? {
                return Ok(());
            }
        }
    }
}

struct Screen;

impl Screen {
    fn open() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        let screen = Self;
        execute!(io::stdout(), EnterAlternateScreen)?;
        Ok(screen)
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

fn exit_requested() -> io::Result<bool> {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline
        && event::poll(deadline.saturating_duration_since(Instant::now()))?
    {
        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Release
                && (matches!(key.code, KeyCode::Char('q' | 'Q') | KeyCode::Esc)
                    || key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL))
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn meter(percent: f32) -> String {
    let percent = percent.clamp(0.0, 100.0);
    let fill = (percent / 10.0).round() as usize;
    format!(
        "[{}{}] {percent:5.1}%",
        "━".repeat(fill),
        "─".repeat(10 - fill)
    )
}

fn capacity(used: u64, total: u64) -> String {
    if total == 0 {
        return "容量未知".into();
    }
    let used = used.min(total);
    let unit = 1024_f64.powi(3);
    format!(
        "{}  {:.2} / {:.2} GiB",
        meter(100.0 * used as f32 / total as f32),
        used as f64 / unit,
        total as f64 / unit
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capacity_shows_usage_and_handles_missing_totals() {
        assert_eq!(capacity(0, 0), "容量未知");
        assert!(capacity(1 << 30, 2 << 30).contains("50.0%  1.00 / 2.00 GiB"));
    }
}
