//! 当前机器的操作系统与架构。
//!
//! 上游各家给附件起名的习惯不一样（`linux-amd64` / `linux_x86_64` / `macos-arm64`
//! …），所以这里只给出规范化的枚举，拼名字交给各服务自己。

use anyhow::{bail, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Linux,
    MacOs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    X86_64,
    Aarch64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Platform {
    pub os: Os,
    pub arch: Arch,
}

impl Platform {
    pub fn current() -> Result<Self> {
        let os = match std::env::consts::OS {
            "linux" => Os::Linux,
            "macos" => Os::MacOs,
            other => bail!("不支持的操作系统: {other}"),
        };
        let arch = match std::env::consts::ARCH {
            "x86_64" => Arch::X86_64,
            "aarch64" => Arch::Aarch64,
            other => bail!("不支持的架构: {other}"),
        };
        Ok(Self { os, arch })
    }
}

impl Os {
    /// Go 生态的写法：linux / darwin。
    pub fn go(&self) -> &'static str {
        match self {
            Os::Linux => "linux",
            Os::MacOs => "darwin",
        }
    }

    /// code-server 等项目用的写法：linux / macos。
    pub fn macos_style(&self) -> &'static str {
        match self {
            Os::Linux => "linux",
            Os::MacOs => "macos",
        }
    }
}

impl Arch {
    /// Go 生态的写法：amd64 / arm64。
    pub fn go(&self) -> &'static str {
        match self {
            Arch::X86_64 => "amd64",
            Arch::Aarch64 => "arm64",
        }
    }

    /// uname -m 的写法：x86_64 / aarch64。
    pub fn uname(&self) -> &'static str {
        match self {
            Arch::X86_64 => "x86_64",
            Arch::Aarch64 => "aarch64",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 本机平台能识别出来() {
        // CI 跑在 linux/macos 的 x86_64 或 arm64 上，识别不出来说明表要补。
        Platform::current().unwrap();
    }

    #[test]
    fn 两套命名都对() {
        assert_eq!(Os::MacOs.go(), "darwin");
        assert_eq!(Os::MacOs.macos_style(), "macos");
        assert_eq!(Arch::Aarch64.go(), "arm64");
        assert_eq!(Arch::Aarch64.uname(), "aarch64");
        assert_eq!(Arch::X86_64.go(), "amd64");
        assert_eq!(Arch::X86_64.uname(), "x86_64");
    }
}
