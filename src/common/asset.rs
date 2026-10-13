//! 从 release 的附件清单里挑一个本机能用的。
//!
//! 按名字匹配，而不是照模板把文件名拼出来：上游改过附件命名（版本号的位置、
//! 架构写法、加不加 `-1` 这种 rpm release 号），拼出来的名字一旦对不上就是 404，
//! 而实际上 release 里躺着一个能用的包。顺带也就支持了「某个版本只发了部分
//! 平台」——挑不到就明说，并把有哪些列出来。

use super::github::{Asset, Release};
use super::platform::{Arch, Os, Platform};

/// 桌面安装包在本平台的后缀偏好，排在前面的优先。
///
/// Linux 上 AppImage 排第一：下下来 chmod +x 就能跑，不需要 root，也不碰系统包
/// 数据库。deb/rpm 要 sudo，装进去之后再卸是另一件事。
pub fn desktop_exts(os: Os) -> &'static [&'static str] {
    match os {
        Os::Linux => &[".appimage", ".deb", ".rpm"],
        Os::MacOs => &[".dmg"],
    }
}

/// 这个架构在附件名里可能的几种写法。
///
/// 同一个 Tauri 项目在不同版本、不同格式里换过写法（`amd64` / `x86_64` / `x64`
/// 都见过，同一个 release 里 deb 用 `amd64` 而 rpm 用 `x86_64`），所以全都认。
fn arch_tokens(arch: Arch) -> &'static [&'static str] {
    match arch {
        Arch::X86_64 => &["amd64", "x86_64", "x64"],
        Arch::Aarch64 => &["arm64", "aarch64"],
    }
}

/// 所有架构写法的并集，用来判断一个附件名里到底提没提架构。
const ALL_ARCH_TOKENS: &[&str] = &["amd64", "x86_64", "x64", "arm64", "aarch64"];

/// 按本平台的桌面包偏好挑一个。
pub fn pick(assets: &[Asset], plat: Platform) -> Option<&Asset> {
    pick_ext(assets, plat, desktop_exts(plat.os))
}

/// 同 [`pick`]，但只认给定的后缀。
///
/// 给只会处理一种格式的调用方用：挑不到就该说「这个版本没发 AppImage」，
/// 而不是退而求其次拿个 deb 回来当 AppImage 用。
pub fn pick_ext<'a>(assets: &'a [Asset], plat: Platform, exts: &[&str]) -> Option<&'a Asset> {
    for ext in exts {
        let candidates: Vec<&Asset> = assets
            .iter()
            .filter(|a| a.name.to_ascii_lowercase().ends_with(ext))
            .collect();

        if let Some(a) = candidates.iter().find(|a| {
            let n = a.name.to_ascii_lowercase();
            arch_tokens(plat.arch).iter().any(|t| n.contains(t))
        }) {
            return Some(a);
        }
        // 名字里压根没提架构的，当通用包收下（macOS 的 universal dmg 就是这样）。
        // 但提了**别的**架构的不行——那是给另一台机器的。
        if let Some(a) = candidates.iter().find(|a| {
            let n = a.name.to_ascii_lowercase();
            !ALL_ARCH_TOKENS.iter().any(|t| n.contains(t))
        }) {
            return Some(a);
        }
    }
    None
}

/// 这个 release 都有哪些附件，用于挑不到时把实情摆出来。
pub fn names(release: &Release) -> String {
    if release.assets.is_empty() {
        return "（一个附件都没有，多半是 CI 还没传完）".into();
    }
    release
        .assets
        .iter()
        .map(|a| a.name.as_str())
        .collect::<Vec<_>>()
        .join("、")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assets(names: &[&str]) -> Vec<Asset> {
        names
            .iter()
            .map(|n| Asset {
                name: (*n).into(),
                browser_download_url: format!("https://example.invalid/{n}"),
            })
            .collect()
    }

    fn plat(os: Os, arch: Arch) -> Platform {
        Platform { os, arch }
    }

    #[test]
    fn linux_优先挑_appimage() {
        let a = assets(&[
            "OpenPencil_0.13.2_amd64.deb",
            "OpenPencil_0.13.2_amd64.AppImage",
            "OpenPencil_0.13.2_x64.dmg",
        ]);
        let got = pick(&a, plat(Os::Linux, Arch::X86_64)).unwrap();
        assert_eq!(got.name, "OpenPencil_0.13.2_amd64.AppImage");
    }

    #[test]
    fn 没有_appimage_时退到_deb() {
        let a = assets(&["OpenPencil_0.13.2_amd64.deb"]);
        let got = pick(&a, plat(Os::Linux, Arch::X86_64)).unwrap();
        assert_eq!(got.name, "OpenPencil_0.13.2_amd64.deb");
    }

    #[test]
    fn 限定后缀时不退而求其次() {
        // 只会处理 AppImage 的调用方拿到一个 deb，接下来就是 chmod +x 一个
        // deb 然后执行它——报错会是「格式不对」，而真相是上游这版没发 AppImage。
        let a = assets(&["X_1.0.0_amd64.deb"]);
        let p = plat(Os::Linux, Arch::X86_64);
        assert!(pick(&a, p).is_some());
        assert!(pick_ext(&a, p, &[".appimage"]).is_none());
    }

    #[test]
    fn 架构不对的附件不会被当成本机的() {
        // 这条最要紧：拿一个 arm64 的包在 x86 机器上跑，报错是「格式不对」，
        // 跟「上游没发这个平台」看着完全不同。
        let a = assets(&["OpenPencil_0.13.2_arm64.AppImage"]);
        assert!(pick(&a, plat(Os::Linux, Arch::X86_64)).is_none());
    }

    #[test]
    fn 架构的几种写法都认() {
        for name in [
            "X_1.0.0_x86_64.AppImage",
            "X_1.0.0_amd64.AppImage",
            "X_1.0.0_x64.AppImage",
        ] {
            let a = assets(&[name]);
            assert!(
                pick(&a, plat(Os::Linux, Arch::X86_64)).is_some(),
                "没认出 {name}"
            );
        }
        for name in ["X_1.0.0_aarch64.AppImage", "X_1.0.0_arm64.AppImage"] {
            let a = assets(&[name]);
            assert!(
                pick(&a, plat(Os::Linux, Arch::Aarch64)).is_some(),
                "没认出 {name}"
            );
        }
    }

    #[test]
    fn 名字里不提架构的当通用包() {
        // macOS 的 universal dmg 不带架构后缀。
        let a = assets(&["X_1.0.0_universal.dmg"]);
        assert!(pick(&a, plat(Os::MacOs, Arch::Aarch64)).is_some());
    }

    #[test]
    fn macos_按架构区分两份_dmg() {
        let a = assets(&["X_1.0.0_x64.dmg", "X_1.0.0_aarch64.dmg"]);
        assert_eq!(
            pick(&a, plat(Os::MacOs, Arch::Aarch64)).unwrap().name,
            "X_1.0.0_aarch64.dmg"
        );
        assert_eq!(
            pick(&a, plat(Os::MacOs, Arch::X86_64)).unwrap().name,
            "X_1.0.0_x64.dmg"
        );
    }

    #[test]
    fn 大小写不影响匹配() {
        let a = assets(&["X_1.0.0_AMD64.appimage"]);
        assert!(pick(&a, plat(Os::Linux, Arch::X86_64)).is_some());
    }

    /// open-pencil v0.15.1 的真实附件清单。
    ///
    /// 挑附件这件事唯一会出错的地方就是上游改了命名，所以拿一份真的来钉住：
    /// 每个 `.sig`、`latest.json`、Windows 的 exe/msi 都在里面，一条规则漏了
    /// 就会挑中它们中的某一个。
    const OPEN_PENCIL_V0_15_1: &[&str] = &[
        "latest.json",
        "OpenPencil-0.15.1-1.x86_64.rpm",
        "OpenPencil-0.15.1-1.x86_64.rpm.sig",
        "OpenPencil_0.15.1_aarch64.dmg",
        "OpenPencil_0.15.1_amd64.AppImage",
        "OpenPencil_0.15.1_amd64.AppImage.sig",
        "OpenPencil_0.15.1_amd64.deb",
        "OpenPencil_0.15.1_amd64.deb.sig",
        "OpenPencil_0.15.1_arm64-setup.exe",
        "OpenPencil_0.15.1_arm64_en-US.msi",
        "OpenPencil_0.15.1_x64-setup.exe",
        "OpenPencil_0.15.1_x64.dmg",
        "OpenPencil_0.15.1_x64_en-US.msi",
        "OpenPencil_aarch64.app.tar.gz",
    ];

    /// cockpit-tools v1.3.68 的真实附件清单。
    ///
    /// 比上面那份更全：Linux 两个架构的 AppImage/deb/rpm 都有，还混着一堆
    /// `latest-*.json` 和 `SHA256SUMS.txt`。
    const COCKPIT_V1_3_68: &[&str] = &[
        "Cockpit.Tools-1.3.68-1.aarch64.rpm",
        "Cockpit.Tools-1.3.68-1.aarch64.rpm.sig",
        "Cockpit.Tools-1.3.68-1.x86_64.rpm",
        "Cockpit.Tools-1.3.68-1.x86_64.rpm.sig",
        "Cockpit.Tools_1.3.68_aarch64.AppImage",
        "Cockpit.Tools_1.3.68_aarch64.AppImage.sig",
        "Cockpit.Tools_1.3.68_aarch64.dmg",
        "Cockpit.Tools_1.3.68_amd64.AppImage",
        "Cockpit.Tools_1.3.68_amd64.AppImage.sig",
        "Cockpit.Tools_1.3.68_amd64.deb",
        "Cockpit.Tools_1.3.68_amd64.deb.sig",
        "Cockpit.Tools_1.3.68_arm64.deb",
        "Cockpit.Tools_1.3.68_arm64.deb.sig",
        "Cockpit.Tools_1.3.68_universal.dmg",
        "Cockpit.Tools_1.3.68_x64-portable.zip",
        "Cockpit.Tools_1.3.68_x64-setup.exe",
        "Cockpit.Tools_1.3.68_x64-setup.exe.sig",
        "Cockpit.Tools_1.3.68_x64.dmg",
        "Cockpit.Tools_1.3.68_x64_en-US.msi",
        "Cockpit.Tools_1.3.68_x64_en-US.msi.sig",
        "Cockpit.Tools_aarch64.app.tar.gz",
        "Cockpit.Tools_universal.app.tar.gz",
        "Cockpit.Tools_x64.app.tar.gz",
        "latest-darwin-aarch64-app.json",
        "latest-linux-aarch64-appimage.json",
        "latest-linux-x86_64-appimage.json",
        "latest.json",
        "SHA256SUMS.txt",
    ];

    #[test]
    fn 从真实的_release_里挑对了() {
        let a = assets(OPEN_PENCIL_V0_15_1);
        assert_eq!(
            pick(&a, plat(Os::Linux, Arch::X86_64)).unwrap().name,
            "OpenPencil_0.15.1_amd64.AppImage"
        );
        assert_eq!(
            pick(&a, plat(Os::MacOs, Arch::Aarch64)).unwrap().name,
            "OpenPencil_0.15.1_aarch64.dmg"
        );
        assert_eq!(
            pick(&a, plat(Os::MacOs, Arch::X86_64)).unwrap().name,
            "OpenPencil_0.15.1_x64.dmg"
        );
        // 上游这一版没给 linux/arm64 发任何包。挑不到要明说，不能拿 x86 的凑数。
        assert!(pick(&a, plat(Os::Linux, Arch::Aarch64)).is_none());
    }

    #[test]
    fn 两个架构都发了包时各挑各的() {
        let a = assets(COCKPIT_V1_3_68);
        let appimage = [".appimage"];
        assert_eq!(
            pick_ext(&a, plat(Os::Linux, Arch::X86_64), &appimage)
                .unwrap()
                .name,
            "Cockpit.Tools_1.3.68_amd64.AppImage"
        );
        assert_eq!(
            pick_ext(&a, plat(Os::Linux, Arch::Aarch64), &appimage)
                .unwrap()
                .name,
            "Cockpit.Tools_1.3.68_aarch64.AppImage"
        );
        // universal.dmg 和带架构的 dmg 同时存在时，带架构的那个更对。
        assert_eq!(
            pick(&a, plat(Os::MacOs, Arch::X86_64)).unwrap().name,
            "Cockpit.Tools_1.3.68_x64.dmg"
        );
    }

    #[test]
    fn 不相干的附件不会被挑中() {
        // release 里常常还挂着 latest.json、签名文件这些。
        let a = assets(&["latest.json", "X_1.0.0_amd64.AppImage.sig"]);
        assert!(pick(&a, plat(Os::Linux, Arch::X86_64)).is_none());
    }

    #[test]
    fn 没有附件时挑不出东西() {
        assert!(pick(&[], plat(Os::Linux, Arch::X86_64)).is_none());
    }
}
