//! 开发工具声明。

use super::go::Go;
use super::nodejs::NodeJs;
use super::pnpm::Pnpm;
use super::rust::Rust;
use super::uv::Uv;
use super::Dev;

pub fn all() -> Vec<Box<dyn Dev>> {
    vec![
        Box::new(Go),
        Box::new(NodeJs),
        Box::new(Pnpm),
        Box::new(Rust),
        Box::new(Uv),
    ]
}

pub fn names() -> Vec<String> {
    all().iter().map(|d| d.name().to_string()).collect()
}

pub fn find(name: &str) -> Option<Box<dyn Dev>> {
    // node / nodejs 都认：两个名字都在用，而「名字打错了」和「不支持这个东西」
    // 报出来是同一句话。
    let want = match name {
        "node" => "nodejs",
        "golang" => "go",
        "cargo" | "rustc" | "rustup" => "rust",
        other => other,
    };
    all().into_iter().find(|d| d.name() == want)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 常见的别名都认() {
        for (alias, real) in [
            ("node", "nodejs"),
            ("golang", "go"),
            ("cargo", "rust"),
            ("rustup", "rust"),
        ] {
            assert_eq!(find(alias).map(|d| d.name().to_string()), Some(real.into()));
        }
    }
}
