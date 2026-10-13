# 发布 fundeploy

发布由 [release.yml](../.github/workflows/release.yml) 驱动：推送 `v*` tag 后，四个平台的
二进制、`.deb` 和 GitHub Release 始终生成。

**apt 源和 Homebrew formula 不在这个仓库里生成。** 它们由
[farfarfun/packages](https://github.com/farfarfun/packages) 按清单重建——那边一个源管全组织
的包，用户加一次就够，以后 `apt install funget` 直接有。这儿只负责把附件挂上 Release，
发完吱一声。

## 发布一个版本

改 `Cargo.toml` 里的 `version`，提交，推同版本的 tag：

```bash
version="$(grep -m1 '^version = ' Cargo.toml | cut -d'"' -f2)"
git tag -a "v${version}" -m "v${version}"
git push origin "v${version}"
```

版本号只有 `Cargo.toml` 这一个来源——二进制自报的版本来自 `CARGO_PKG_VERSION`，
`check` job 会核对 tag 与它一致，对不上直接失败。

工作流顺序：

1. `check` —— 核对 tag 与 `Cargo.toml`
2. `build` —— 四个 target 各出一个 `fundeploy-<target>.tar.gz`。Linux 走 musl 全静态，
   arm64 用原生 arm runner 构建而不是交叉编译
3. `release` —— 打两个架构的 `.deb`、算 `SHA256SUMS`、发 Release
4. `verify-install` —— 在 ubuntu x86_64 / ubuntu arm64 / macOS 上照 `install.sh` 真装一遍，
   核对版本号，并跑 `list` 和 `upgrade --check`
5. `notify-packages` —— 通知 `farfarfun/packages` 重建 apt 索引和 formula
6. `publish-crate` —— `cargo publish`

`verify-install` 只能放在发布之后：`install.sh` 要有一个真实存在的 release 才跑得通。
这一步红了说明刚发出去的那一版装不上，得立刻补一个。后两步都排在它后面——crates.io
上的版本撤不回来，apt 源也不该收一个装不上的包。

附件名是 `farfarfun/packages` 认包的依据，别改：`fundeploy-<target>.tar.gz`、
`fundeploy_<版本>_<架构>.deb`、`SHA256SUMS`。

## 一次性配置

| Secret | 用途 | 没配会怎样 |
| --- | --- | --- |
| `PACKAGES_DISPATCH_TOKEN` | 对 `farfarfun/packages` 有写权限的 PAT | 跳过通知，那边每天 04:17 兜底扫一遍 |
| `CARGO_REGISTRY_TOKEN` | crates.io 的 API token | 跳过 crates.io 发布 |

跨仓库 `repository_dispatch` 用不了默认的 `GITHUB_TOKEN`，只能是 PAT：

```bash
gh secret set PACKAGES_DISPATCH_TOKEN   # 粘一个有 packages 写权限的 token
cargo login                             # 然后把 ~/.cargo/credentials.toml 里的 token
gh secret set CARGO_REGISTRY_TOKEN      # 设进来
```

APT 签名密钥和 Homebrew 的部署密钥都搬到 `farfarfun/packages` 了，见那边的 README。

## 脚本

```bash
# 把一个构建好的静态二进制打成 .deb
packaging/build-deb.sh target/x86_64-unknown-linux-musl/release/fundeploy 0.3.1 amd64 dist
```

`.deb` 没有 `Depends`：二进制是静态链接的，装上就能跑。`git` / `curl` / `ca-certificates`
只有个别子命令要用，写成 `Recommends`。

本地跑需要 `dpkg-deb`。`dist/` 已被 Git 忽略。
