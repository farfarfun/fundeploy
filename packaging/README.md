# 发布 nltdeploy

发布流程由 [release.yml](../.github/workflows/release.yml) 驱动。推送 `v*` tag 后，GitHub Release 和 `.deb` 始终生成；APT 仓库与 Homebrew Tap 在配置对应 Secret 后自动发布。

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
ssh-keygen -t ed25519 -N '' -C 'nltdeploy release workflow' -f homebrew_tap_deploy
gh api --method POST repos/farfarfun/homebrew-tap/keys \
  -f title='nltdeploy release workflow' \
  -f key="$(cat homebrew_tap_deploy.pub)" \
  -F read_only=false
gh secret set HOMEBREW_TAP_DEPLOY_KEY < homebrew_tap_deploy
```

## 发布

先修改仓库根目录的 `VERSION`，确保该版本尚未使用且工作区改动已经提交，然后推送同版本 tag：

```bash
version="$(tr -d '[:space:]' < VERSION)"
git tag -a "v${version}" -m "v${version}"
git push origin "v${version}"
```

工作流会校验 tag 与 `VERSION` 一致，然后执行全部冒烟测试、创建 `nltdeploy_<version>_all.deb`、生成 Formula、发布 GitHub Release、部署签名 APT Pages，并更新 `farfarfun/homebrew-tap`。

## 本地验证

```bash
bash packaging/build-deb.sh
bash tests/package_smoke.sh
```

`dist/` 已被 Git 忽略。APT 仓库构建需要 `apt-ftparchive`、`dpkg-scanpackages` 和 GnuPG。
