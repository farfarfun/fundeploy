//! 薄入口：只负责把错误打成人话并给出退出码，逻辑全在 lib 里。

fn main() {
    if let Err(err) = fundeploy::cli_main() {
        // anyhow 的 chain 从具体到抽象，倒过来打更像人话：
        //   错误: uv tool install 失败
        //     └ 起不了进程: uv ...
        eprintln!("错误: {err}");
        for cause in err.chain().skip(1) {
            eprintln!("  └ {cause}");
        }
        std::process::exit(1);
    }
}
