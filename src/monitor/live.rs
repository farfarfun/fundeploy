use std::io::{self, Write};
use std::time::{Duration, Instant};

use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute, queue,
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use sysinfo::{Disks, System};
use unicode_width::UnicodeWidthStr;

use super::{capacity_text, cpu_model};
use crate::theme::{self, AMBER, BLUE, BORDER, LAVENDER, MINT, MUTED, TEXT};

const TRACK: u8 = 243;

pub(super) struct Snapshot<'a> {
    pub host: &'a str,
    pub os: &'a str,
    pub uptime: &'a str,
    pub cores: &'a str,
    pub system: &'a System,
    pub disks: &'a Disks,
    pub gpus: &'a [(String, f32)],
}

pub(super) struct Screen {
    offset: usize,
    color: bool,
}

impl Screen {
    pub fn open() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        let screen = Self {
            offset: 0,
            color: std::env::var_os("NO_COLOR").is_none(),
        };
        execute!(io::stdout(), EnterAlternateScreen, Hide)?;
        Ok(screen)
    }

    pub fn show(&mut self, snapshot: &Snapshot<'_>) -> io::Result<bool> {
        let deadline = Instant::now() + Duration::from_secs(2);
        self.draw(snapshot)?;
        while Instant::now() < deadline
            && event::poll(deadline.saturating_duration_since(Instant::now()))?
        {
            match event::read()? {
                Event::Resize(..) => {}
                Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                    KeyCode::Char('q' | 'Q') | KeyCode::Esc => return Ok(true),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(true);
                    }
                    KeyCode::Up => self.offset = self.offset.saturating_sub(1),
                    KeyCode::Down => self.offset += 1,
                    _ => continue,
                },
                _ => continue,
            }
            self.draw(snapshot)?;
        }
        Ok(false)
    }

    fn draw(&mut self, snapshot: &Snapshot<'_>) -> io::Result<()> {
        let (cols, rows) = terminal::size().unwrap_or((80, 24));
        let margin = if cols >= 32 { "  " } else { "" };
        let width = usize::from(cols).saturating_sub(margin.len() + 1).min(112);
        let height = usize::from(rows);
        let color = self.color;
        #[cfg(windows)]
        let color = color && crossterm::ansi_support::supports_ansi();

        let mut header = Column::new(width, color.then_some(BLUE));
        if height >= 8 {
            header.heading(&format!("  本机状态  ·  {}", snapshot.host));
            let summary = format!("{}  ·  {}", snapshot.os, snapshot.uptime);
            if summary.width() + 2 > width {
                header.detail(&format!("  {}", snapshot.os));
                header.detail(&format!("  {}", snapshot.uptime));
            } else {
                header.detail(&format!("  {summary}"));
            }
            header.rows.push(String::new());
        }
        let body = snapshot.body(width, color);
        let footer_rows = if height >= 5 { 2 } else { 0 };
        let room = height.saturating_sub(header.rows.len() + footer_rows);
        self.offset = self.offset.min(body.len().saturating_sub(room));
        let mut lines = header.rows;
        lines.extend(body.iter().skip(self.offset).take(room).cloned());
        if footer_rows != 0 {
            let mut footer = Column::new(width, color.then_some(BLUE));
            if body.len() > room {
                footer.detail(&format!(
                    "  ↑↓ 滚动  {}/{}  ·  Q 退出",
                    self.offset + room,
                    body.len()
                ));
            } else {
                footer.detail("  Q / Esc / Ctrl+C 退出");
            }
            lines.push(String::new());
            lines.extend(footer.rows);
        }

        let mut out = io::BufWriter::new(io::stdout().lock());
        queue!(out, MoveTo(0, 0), Clear(ClearType::FromCursorDown))?;
        for (index, line) in lines.iter().enumerate() {
            if index != 0 {
                out.write_all(b"\r\n")?;
            }
            out.write_all(margin.as_bytes())?;
            out.write_all(line.as_bytes())?;
        }
        out.flush()
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), Show, LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

impl Snapshot<'_> {
    fn body(&self, width: usize, color: bool) -> Vec<String> {
        const GAP: &str = "   ";
        let wide = width >= 77;
        let column = if wide { (width - GAP.len()) / 2 } else { width };
        let framed = column >= 20;
        let content = column - if framed { 4 } else { 0 };
        let [mut cpu, mut memory, mut gpu, mut disk] = [BLUE, LAVENDER, MINT, AMBER]
            .map(|accent| Column::new(content, color.then_some(accent)));

        if !self.system.cpus().is_empty() {
            cpu.usage("当前利用率", self.system.global_cpu_usage());
        }
        cpu.detail(cpu_model(self.system));
        cpu.detail(&format!(
            "{} 物理核  ·  {} 逻辑核",
            self.cores,
            self.system.cpus().len()
        ));

        memory.capacity(
            "物理内存",
            self.system.used_memory(),
            self.system.total_memory(),
        );
        if self.system.total_swap() > 0 {
            memory.capacity(
                "交换空间",
                self.system.used_swap(),
                self.system.total_swap(),
            );
        }

        if self.gpus.is_empty() {
            gpu.detail("暂无读数");
        }
        for (name, percent) in self.gpus {
            gpu.usage(name, *percent);
        }

        if self.disks.list().is_empty() {
            disk.detail("未检测到可读取的卷");
        }
        let mut volumes: Vec<_> = self.disks.list().iter().collect();
        volumes.sort_by_key(|disk| disk.mount_point());
        for volume in volumes {
            let total = volume.total_space();
            disk.capacity(
                &volume.mount_point().to_string_lossy(),
                total.saturating_sub(volume.available_space()),
                total,
            );
        }

        if !wide {
            return [(cpu, "CPU"), (memory, "内存"), (gpu, "GPU"), (disk, "磁盘")]
                .map(|(card, title)| card.card(title, 0, framed))
                .join(&String::new());
        }
        let mut rows = Vec::new();
        for ((left, left_title), (right, right_title)) in [
            ((cpu, "CPU"), (memory, "内存")),
            ((gpu, "GPU"), (disk, "磁盘")),
        ] {
            let height = left.rows.len().max(right.rows.len());
            let left = left.card(left_title, height, framed);
            let right = right.card(right_title, height, framed);
            for (left, right) in left.into_iter().zip(right) {
                rows.push(format!("{left}{GAP}{right}"));
            }
            rows.push(String::new());
        }
        rows.pop();
        rows
    }
}

struct Column {
    width: usize,
    accent: Option<u8>,
    rows: Vec<String>,
}

impl Column {
    fn new(width: usize, accent: Option<u8>) -> Self {
        Self {
            width,
            accent,
            rows: Vec::new(),
        }
    }

    fn heading(&mut self, text: &str) {
        self.line(text, self.accent.unwrap_or(TEXT), true);
    }

    fn detail(&mut self, text: &str) {
        self.line(text, MUTED, false);
    }

    fn line(&mut self, text: &str, foreground: u8, bold: bool) {
        let text = theme::clip(text, self.width);
        let padding = " ".repeat(self.width - text.width());
        self.rows
            .push(self.paint(&(text + &padding), foreground, bold));
    }

    fn paint(&self, text: &str, foreground: u8, bold: bool) -> String {
        if self.accent.is_none() {
            return text.to_owned();
        }
        theme::paint(text, foreground, bold)
    }

    fn card(mut self, title: &str, height: usize, framed: bool) -> Vec<String> {
        if !framed {
            self.heading(title);
            self.rows.rotate_right(1);
            return self.rows;
        }
        let accent = self.accent.unwrap_or(TEXT);
        let top = format!(
            "{}{}{}",
            self.paint("╭─ ", BORDER, false),
            self.paint(title, accent, true),
            self.paint(
                &format!(" {}╮", "─".repeat(self.width - title.width() - 1)),
                BORDER,
                false
            )
        );
        let edge = self.paint("│ ", BORDER, false);
        let end = self.paint(" │", BORDER, false);
        let blank = self.paint(&" ".repeat(self.width), TEXT, false);
        self.rows.resize(height.max(self.rows.len()), blank);
        let bottom = self.paint(&format!("╰{}╯", "─".repeat(self.width + 2)), BORDER, false);
        std::iter::once(top)
            .chain(self.rows.into_iter().map(|row| format!("{edge}{row}{end}")))
            .chain(std::iter::once(bottom))
            .collect()
    }

    fn usage(&mut self, label: &str, percent: f32) {
        let percent = percent.clamp(0., 100.);
        let number = format!("{percent:5.1}%");
        let bars = (self.width / 3).clamp(8, 24);
        let label_width = if self.width >= 32 {
            self.width - bars - number.len() - 3
        } else {
            0
        };
        let label = if label.width() > label_width {
            self.line(label, TEXT, false);
            ""
        } else {
            label
        };
        if self.width < bars + number.len() + 1 {
            self.line(number.trim(), self.accent.unwrap_or(TEXT), true);
            return;
        }
        let fill = (percent * bars as f32 / 100.).round() as usize;
        let fill_bar = "━".repeat(fill);
        let track = "─".repeat(bars - fill);
        let label = theme::clip(label, label_width);
        let prefix = if label_width > 0 {
            format!("{label}{}", " ".repeat(label_width - label.width() + 2))
        } else {
            String::new()
        };
        let padding = " ".repeat(self.width - prefix.width() - bars - number.len() - 1);
        let accent = self.accent.unwrap_or(TEXT);
        self.rows.push(format!(
            "{}{}{}{}",
            self.paint(&prefix, TEXT, false),
            self.paint(&fill_bar, accent, true),
            self.paint(&track, TRACK, false),
            self.paint(&format!(" {number}{padding}"), accent, true),
        ));
    }

    fn capacity(&mut self, label: &str, used: u64, total: u64) {
        if total == 0 {
            self.line(label, TEXT, false);
            self.detail("容量未知");
            return;
        }
        let used = used.min(total);
        self.usage(label, 100. * used as f32 / total as f32);
        self.detail(&capacity_text(used, total));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_keeps_its_value_visible_with_long_labels() {
        let label = "/卷/非常长的挂载目录/备份和归档";
        for width in [12, 24, 37, 54] {
            let mut column = Column::new(width, None);
            column.usage(label, 97.5);
            assert!(column.rows.iter().all(|line| line.width() == width));
            assert!(column.rows.last().unwrap().contains("97.5%"));
            assert!(column.rows[0].starts_with("/卷/"));
        }
    }

    #[test]
    fn clipping_fits_wide_characters_and_removes_control_characters() {
        assert_eq!(theme::clip("甲乙丙", 4), "甲…");
        assert_eq!(theme::clip("甲\x1b\n乙", 6), "甲  乙");
        assert_eq!(theme::clip("甲", 1), "…");
        assert!(theme::clip("甲", 0).is_empty());
    }
}
