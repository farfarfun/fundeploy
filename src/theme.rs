use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

// Xterm 256-color palette shared by the menus, status cards and monitor.
pub(crate) const SURFACE: u8 = 234;
pub(crate) const TEXT: u8 = 255;
pub(crate) const MUTED: u8 = 249;
pub(crate) const BORDER: u8 = 60;
pub(crate) const BLUE: u8 = 117;
pub(crate) const LAVENDER: u8 = 183;
pub(crate) const MINT: u8 = 115;
pub(crate) const AMBER: u8 = 222;

pub(crate) fn paint(text: &str, foreground: u8, bold: bool) -> String {
    use crossterm::style::{style, Color, Stylize};

    if text.is_empty() {
        return String::new();
    }
    let styled = style(text)
        .with(Color::AnsiValue(foreground))
        .on(Color::AnsiValue(SURFACE));
    if bold {
        styled.bold().to_string()
    } else {
        styled.to_string()
    }
}

pub(crate) fn clip(text: &str, width: usize) -> String {
    // 主机名、卷名可能带控制字符，不能直接写进终端。
    let mut text: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if text.width() <= width {
        return text;
    }
    if width == 0 {
        return String::new();
    }
    let mut remaining = width - 1;
    let end = text
        .char_indices()
        .find_map(
            |(index, ch)| match remaining.checked_sub(ch.width().unwrap_or(0)) {
                Some(next) => {
                    remaining = next;
                    None
                }
                None => Some(index),
            },
        )
        .unwrap_or(text.len());
    text.truncate(end);
    while text.width() >= width {
        text.pop();
    }
    text.push('…');
    text
}
