# 发布 fundeploy

发布由 [release.yml](../.github/workflows/release.yml) 驱动：推送 `v*` tag 后，四个平台的
二进制、`.deb` 和 GitHub Release 始终生成；APT 仓库与 Homebrew Tap 在配置了对应 Secret
之后自动发布，没配就跳过。

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
3. `release` —— 打两个架构的 `.deb`、算 `SHA256SUMS`、渲染 Homebrew formula、发 Release
4. `verify-install` —— 在 ubuntu x86_64 / ubuntu arm64 / macOS 上照 `install.sh` 真装一遍，
   核对版本号，并跑 `list` 和 `upgrade --check`
5. `apt-repository` + `deploy-apt-pages` —— 签名的 APT 仓库发到 GitHub Pages
6. `homebrew-tap` —— 把 formula 推到 `farfarfun/homebrew-tap`

`verify-install` 只能放在发布之后：`install.sh` 要有一个真实存在的 release 才跑得通。
这一步红了说明刚发出去的那一版装不上，得立刻补一个。

## 一次性配置

1. 创建公开仓库 `farfarfun/homebrew-tap`，至少包含一个默认分支。
2. 在当前仓库 Settings → Pages 中选择 **GitHub Actions** 作为发布源。
3. 添加以下 Actions Secrets：

| Secret | 用途 |
| --- | --- |
| `APT_GPG_PRIVATE_KEY` | ASCII-armored APT 签名私钥 |
| `APT_GPG_PASSPHRASE` | 私钥密码；无密码密钥可不设置 |
| `HOMEBREW_TAP_DEPLOY_KEY` | `farfarfun/homebrew-tap` 专用的可写 SSH Deploy Key 私钥 |

可用现有 GPG 密钥配置 APT 签名：

```bash
gpg --list-secret-keys --keyid-format LONG
gpg --armor --export-secret-keys <KEY_ID> | gh secret set APT_GPG_PRIVATE_KEY
gh secret set APT_GPG_PASSPHRASE
```

Homebrew Tap 使用仅限该仓库的可写 Deploy Key：

```bash
ssh-keygen -t ed25519 -N '' -C 'fundeploy release workflow' -f homebrew_tap_deploy
gh api --method POST repos/farfarfun/homebrew-tap/keys \
  -f title='fundeploy release workflow' \
  -f key="$(cat homebrew_tap_deploy.pub)" \
  -F read_only=false
gh secret set HOMEBREW_TAP_DEPLOY_KEY < homebrew_tap_deploy
```

## 脚本

三个脚本都能单独跑，工作流只是按顺序调它们：

```bash
# 把一个构建好的静态二进制打成 .deb
packaging/build-deb.sh target/x86_64-unknown-linux-musl/release/fundeploy 0.3.0 amd64 dist

# 从 SHA256SUMS 渲染 Homebrew formula（装预编好的二进制，不从源码编）
packaging/render-homebrew-formula.sh 0.3.0 dist/SHA256SUMS dist/fundeploy.rb

# 把一批 .deb 组装成签名好的 APT 仓库
packaging/build-apt-repo.sh dist public <GPG密钥ID>
```

`.deb` 没有 `Depends`：二进制是静态链接的，装上就能跑。`git` / `curl` / `ca-certificates`
只有个别子命令要用，写成 `Recommends`。

APT 仓库每个架构扫出自己的 `Packages`——包是 per-arch 的，索引共用会让 arm64 机器在列表里
看到 amd64 的包，apt 直接判定「没有可安装的候选」。

本地跑这几个脚本需要 `dpkg-deb`、`apt-ftparchive`、`dpkg-scanpackages` 和 GnuPG。
`dist/` 已被 Git 忽略。
