//! 本机资源面板。CPU 利用率由两次采样的差值计算。

mod gpu;
mod live;

use std::fmt::Write as _;
#[cfg(windows)]
use std::io::IsTerminal;
use std::io::{self, Write};
use std::thread;

use anyhow::Result;
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
        let mut screen = if live {
            Some(live::Screen::open()?)
        } else {
            None
        };
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
            let uptime = format!(
                "已运行 {} 天 {} 小时 {} 分",
                uptime / 1440,
                uptime / 60 % 24,
                uptime % 60
            );
            let gpus = gpu::sample();
            if let Some(screen) = &mut screen {
                let snapshot = live::Snapshot {
                    host: &host,
                    os: &os,
                    uptime: &uptime,
                    cores: &cores,
                    system: &sys,
                    disks: &disks,
                    gpus: &gpus,
                };
                if screen.show(&snapshot)? {
                    return Ok(());
                }
                continue;
            }
            let mut panel = format!("本机状态 · {host}\n{os} · {uptime}\n");
            let cpu = cpu_model(&sys);
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
            if gpus.is_empty() {
                writeln!(panel, "      暂无读数（设备或驱动未提供利用率）")?;
            }
            for (name, percent) in gpus {
                writeln!(panel, "      {}  {name}", meter(percent))?;
            }
            let mut out = io::stdout().lock();
            out.write_all(panel.as_bytes())?;
            out.flush()?;
            return Ok(());
        }
    }
}

fn cpu_model(sys: &System) -> &str {
    sys.cpus()
        .first()
        .map(|c| c.brand().trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("未知型号")
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
    format!(
        "{}  {}",
        meter(100.0 * used as f32 / total as f32),
        capacity_text(used, total)
    )
}

fn capacity_text(used: u64, total: u64) -> String {
    let unit = 1024_f64.powi(3);
    format!("{:.2} / {:.2} GiB", used as f64 / unit, total as f64 / unit)
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
