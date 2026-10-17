//! 进度条样式与时长格式化。
//!
//! 单独成文件是因为这些东西没有任何 Paperclip 语义——换个服务照抄就能用。放在
//! 主文件里只会让人在找「装到哪一步了」的逻辑时反复划过它们。

use super::*;

/// 量一次大小要几百毫秒而且限了频，中间这几秒屏幕上得有东西在动，否则看着又像卡住了。
pub(super) const STEADY_TICK: Duration = Duration::from_millis(200);

/// 一条待设样式的进度条。非终端环境返回隐藏的那种，省掉所有绘制。
pub(super) fn progress_bar() -> indicatif::ProgressBar {
    if std::io::IsTerminal::is_terminal(&std::io::stderr()) {
        indicatif::ProgressBar::new_spinner()
    } else {
        indicatif::ProgressBar::hidden()
    }
}

/// 没有分母的那一段：报已经过去多久、攒了多少。
///
/// 刻意不摆 `{eta}`：这一段不知道总共要下多少，indicatif 会拿 0 顶上，屏幕上就是
/// 「还要 0s」——比没有数字更糟。`bytes` 为假时连字节数都不报（猜不到 npm 的缓存在哪）。
pub(super) fn downloading_style(bytes: bool) -> indicatif::ProgressStyle {
    let t = if bytes {
        "{msg} {spinner} {bytes} {bytes_per_sec} 已过 {elapsed}"
    } else {
        "{msg} {spinner} 已过 {elapsed}"
    };
    indicatif::ProgressStyle::with_template(t).unwrap()
}

/// 有分母的那一段。
///
/// 模板比 [`crate::common::download`] 那条多一项 ETA：那边是几秒钟的文件下载，这边
/// 是几分钟到几十分钟，「还要多久」正是这里要回答的问题。
pub(super) fn unpacking_style() -> indicatif::ProgressStyle {
    indicatif::ProgressStyle::with_template(
        "{msg} [{bar:24.cyan/blue}] {bytes}/{total_bytes} {bytes_per_sec} 还要 {eta}",
    )
    .unwrap()
    .progress_chars("=>-")
}

/// 时长转成人看的。
pub(super) fn human_secs(d: Duration) -> String {
    let s = d.as_secs();
    if s < 60 {
        format!("{s} 秒")
    } else {
        format!("{} 分 {} 秒", s / 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 时长说成人话() {
        assert_eq!(human_secs(Duration::from_secs(0)), "0 秒");
        assert_eq!(human_secs(Duration::from_secs(48)), "48 秒");
        assert_eq!(human_secs(Duration::from_secs(60)), "1 分 0 秒");
        assert_eq!(human_secs(Duration::from_secs(372)), "6 分 12 秒");
    }
}
