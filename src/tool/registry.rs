//! 工具声明。

use super::open_pencil::OpenPencil;
use super::Tool;

pub fn all() -> Vec<Box<dyn Tool>> {
    vec![Box::new(OpenPencil::from_env())]
}

pub fn names() -> Vec<String> {
    all().iter().map(|t| t.name().to_string()).collect()
}

pub fn find(name: &str) -> Option<Box<dyn Tool>> {
    all().into_iter().find(|t| t.name() == name)
}
