//! fundeploy —— 开发环境、工具与服务管理。
//!
//! # 分层
//!
//! ```text
//! main.rs / cli.rs            参数解析与分发
//!   ↓
//! service / tool / dev / ai   四个域，各自一张注册表
//!   ↓
//! common                      进程、端口、包管理器、下载、版本比较
//!   ↓
//! ui                          菜单与进度
//! ```
//!
//! # 服务层的设计
//!
//! 服务之间的差异绝大部分是**数据**而非流程：装什么包、监听哪个端口、几个组件、
//! 组件之间的启停顺序。所以 [`service::Service`] 是一个带默认实现的 trait，
//! 装/起/停/查的流程只写一次，每个具体服务退化成一份 [`service::spec::ServiceSpec`]
//! 声明。上游特有的坑（某个 CLI 的 stop 必须带参数、某份配置文件上游不会自己生成）
//! 通过覆盖 trait 的钩子方法表达。

pub mod cli;
pub mod common;
pub mod service;
pub mod ui;

pub use cli::cli_main;

/// 程序自身版本。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
