use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::monitor::Options;

#[derive(Parser)]
#[command(name = "fundeploy", version, about = "本机资源面板")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// 本机资源面板（CPU、内存、磁盘、GPU）
    Monitor(Options),
}

pub fn cli_main() -> Result<()> {
    match Cli::parse().command {
        Some(Command::Monitor(options)) => options.run(),
        None => Options::default().run(),
    }
}
