//! 系统包管理器：这台机器上有哪个、拿它装、拿它卸。
//!
//! 各家的子命令、确认参数、要不要先刷新索引都不一样，差别全在下面几张小表里。

use anyhow::{bail, Result};

use super::cmd;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pm {
    Apt,
    Dnf,
    Yum,
    Pacman,
    Zypper,
    Apk,
    Brew,
}

impl Pm {
    /// 探测顺序。前面的优先——一台机器上同时有 dnf 和 yum 时，dnf 才是现役的那个。
    pub const ALL: [Pm; 7] = [
        Pm::Apt,
        Pm::Dnf,
        Pm::Yum,
        Pm::Pacman,
        Pm::Zypper,
        Pm::Apk,
        Pm::Brew,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Pm::Apt => "apt",
            Pm::Dnf => "dnf",
            Pm::Yum => "yum",
            Pm::Pacman => "pacman",
            Pm::Zypper => "zypper",
            Pm::Apk => "apk",
            Pm::Brew => "brew",
        }
    }

    /// 真正要调的可执行文件。
    ///
    /// apt 调的是 `apt-get`：`apt` 那个面向人的前端在非终端下会打一行「此接口
    /// 不稳定，不要在脚本里用」，而且它的输出格式本来就不保证。
    pub fn bin(self) -> &'static str {
        match self {
            Pm::Apt => "apt-get",
            other => other.name(),
        }
    }

    /// 这台机器上用哪个。
    ///
    /// macOS 上只认 brew。系统自带的 `/usr/bin` 不归任何包管理器管，而装了
    /// MacPorts 的机器上 port 和 brew 谁说了算，不该由我们替人决定。
    pub fn detect() -> Option<Pm> {
        if cfg!(target_os = "macos") {
            return cmd::has("brew").then_some(Pm::Brew);
        }
        Pm::ALL.into_iter().find(|p| cmd::has(p.bin()))
    }

    /// 装之前要不要先刷新索引。
    ///
    /// Debian 系必须刷：容器镜像出厂时 `/var/lib/apt/lists` 通常是空的，不刷
    /// 直接装就是 `Unable to locate package`。其余几家要么自带索引，要么装的
    /// 时候自己会刷。
    fn refresh_argv(self) -> Option<Vec<String>> {
        match self {
            Pm::Apt => Some(vec!["apt-get".into(), "update".into(), "-y".into()]),
            _ => None,
        }
    }

    fn install_argv(self, pkgs: &[&str]) -> Vec<String> {
        let head: &[&str] = match self {
            Pm::Apt => &["apt-get", "install", "-y"],
            Pm::Dnf => &["dnf", "install", "-y"],
            Pm::Yum => &["yum", "install", "-y"],
            Pm::Pacman => &["pacman", "-S", "--noconfirm", "--needed"],
            Pm::Zypper => &["zypper", "--non-interactive", "install"],
            Pm::Apk => &["apk", "add"],
            Pm::Brew => &["brew", "install"],
        };
        join(head, pkgs)
    }

    fn remove_argv(self, pkgs: &[&str]) -> Vec<String> {
        let head: &[&str] = match self {
            Pm::Apt => &["apt-get", "remove", "-y"],
            Pm::Dnf => &["dnf", "remove", "-y"],
            Pm::Yum => &["yum", "remove", "-y"],
            Pm::Pacman => &["pacman", "-R", "--noconfirm"],
            Pm::Zypper => &["zypper", "--non-interactive", "remove"],
            Pm::Apk => &["apk", "del"],
            Pm::Brew => &["brew", "uninstall"],
        };
        join(head, pkgs)
    }

    /// 要不要提权。
    ///
    /// brew 坚决不能用 sudo——它会把自己的目录改成 root 所有，之后每一条 brew
    /// 命令都得继续 sudo，而上游明确拒绝支持这种状态。
    fn needs_root(self) -> bool {
        self != Pm::Brew
    }

    pub fn install(self, pkgs: &[&str]) -> Result<()> {
        if pkgs.is_empty() {
            bail!("{} 下没有对应的包可装", self.name());
        }
        if let Some(refresh) = self.refresh_argv() {
            // 刷索引失败不致命：索引是旧的但仍然可用的情况很常见（源挂了一个、
            // 签名过期），而真正装不上的时候下一条命令自己会报。
            if cmd::run(self.privileged(refresh)).is_err() {
                crate::ui::warn(format!("{} 刷新索引失败，继续试着安装。", self.name()));
            }
        }
        cmd::run(self.privileged(self.install_argv(pkgs)))
    }

    pub fn uninstall(self, pkgs: &[&str]) -> Result<()> {
        if pkgs.is_empty() {
            bail!("{} 下没有对应的包可卸", self.name());
        }
        cmd::run(self.privileged(self.remove_argv(pkgs)))
    }

    /// 按需在命令前面插 sudo。
    fn privileged(self, argv: Vec<String>) -> Vec<String> {
        if !self.needs_root() || is_root() || !cmd::has("sudo") {
            // 不是 root 又没有 sudo 时原样执行：报出来的是包管理器自己那句
            // 「需要 root 权限」，比我们转述一遍准。
            return argv;
        }
        let mut out = vec!["sudo".to_string()];
        out.extend(argv);
        out
    }
}

fn is_root() -> bool {
    // Safe: getuid 不碰内存，也不会失败。
    unsafe { libc::getuid() == 0 }
}

fn join(head: &[&str], tail: &[&str]) -> Vec<String> {
    head.iter().chain(tail).map(|s| s.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apt_调的是_apt_get() {
        // apt 在非终端下会警告自己的接口不稳定。
        assert_eq!(Pm::Apt.bin(), "apt-get");
        assert_eq!(Pm::Brew.bin(), "brew");
    }

    #[test]
    fn 每一家的装卸命令都不需要人回车() {
        for pm in Pm::ALL {
            let i = pm.install_argv(&["x"]).join(" ");
            let r = pm.remove_argv(&["x"]).join(" ");
            for argv in [&i, &r] {
                let auto = argv.contains("-y")
                    || argv.contains("--noconfirm")
                    || argv.contains("--non-interactive")
                    || pm == Pm::Apk      // apk 不问
                    || pm == Pm::Brew; // brew 不问
                assert!(auto, "{} 会停在确认上: {argv}", pm.name());
            }
            assert!(i.ends_with(" x"), "包名要在最后: {i}");
            assert!(r.ends_with(" x"), "包名要在最后: {r}");
        }
    }

    #[test]
    fn brew_不加_sudo() {
        // 用 sudo 跑 brew 会把它自己的目录改成 root 所有，之后再也回不去。
        let argv = Pm::Brew.privileged(vec!["brew".into(), "install".into()]);
        assert_eq!(argv[0], "brew");
    }

    #[test]
    fn 只有_apt_需要先刷索引() {
        assert!(Pm::Apt.refresh_argv().is_some());
        for pm in Pm::ALL.into_iter().filter(|p| *p != Pm::Apt) {
            assert!(pm.refresh_argv().is_none(), "{} 不该刷", pm.name());
        }
    }

    #[test]
    fn 没有包名时不去跑命令() {
        // 空包名列表传给 apt-get install 等于「什么都不装」，静默成功；
        // 但调用方走到这儿多半是某个工具在这家包管理器下根本没有包。
        assert!(Pm::Apt.install(&[]).is_err());
        assert!(Pm::Apt.uninstall(&[]).is_err());
    }
}
