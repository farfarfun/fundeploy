#!/usr/bin/env bash
# 供应链加固的回归测试（全部离线，不触网、不以 root 执行任何东西）。
#
# 覆盖的历史问题：
#   1. FUNDEPLOY_GITHUB_HUB_PROXY_PREFIX / RAW_MIRROR_BASE 不校验 scheme，
#      可把所有 GitHub 族下载重定向到明文 http:// 主机。
#   2. pip-sources 会把 http:// 镜像写成 index-url，并为所有源（含 HTTPS）
#      生成 trusted-host —— 对 HTTPS 主机而言这等于关掉证书校验。
#   3. sub2api 官方脚本从可变的 main 分支 `curl | sudo bash`，无任何校验。
set -uo pipefail

_TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
_REPO_ROOT="$(cd "${_TEST_DIR}/.." && pwd)"
# shellcheck source=lib/assert.sh
source "${_TEST_DIR}/lib/assert.sh"
_ASSERT_NAME="supplychain_smoke"

echo "== GitHub 下载改写：必须拒绝非 https 前缀 =="
# shellcheck source=../scripts/lib/fundeploy-github-download.sh
source "${_REPO_ROOT}/scripts/lib/fundeploy-github-download.sh"
_U="https://raw.githubusercontent.com/o/r/v/f.txt"

_out="$(FUNDEPLOY_GITHUB_HUB_PROXY_PREFIX='https://ok.example/' _fundeploy_github_download_resolve_url "$_U" 2>/dev/null)"
assert_eq "https 前缀应生效" "https://ok.example/${_U}" "${_out}"

_out="$(FUNDEPLOY_GITHUB_HUB_PROXY_PREFIX='http://evil.example/' _fundeploy_github_download_resolve_url "$_U" 2>/dev/null)"
assert_eq "http 前缀应被拒绝并原样返回" "${_U}" "${_out}"

_out="$(FUNDEPLOY_GITHUB_DOWNLOAD_MODE=mirror_raw FUNDEPLOY_GITHUB_RAW_MIRROR_BASE='http://evil.example/raw' \
  _fundeploy_github_download_resolve_url "$_U" 2>/dev/null)"
assert_eq "http 镜像基址应被拒绝" "${_U}" "${_out}"

_out="$(FUNDEPLOY_GITHUB_DOWNLOAD_MODE=mirror_raw FUNDEPLOY_GITHUB_RAW_MIRROR_BASE='https://ok.example/raw' \
  _fundeploy_github_download_resolve_url "$_U" 2>/dev/null)"
assert_eq "https 镜像基址应生效" "https://ok.example/raw/o/r/v/f.txt" "${_out}"

echo "== 共享 curl 封装必须设置 TLS 下限 =="
_src="$(cat "${_REPO_ROOT}/scripts/lib/fundeploy-github-download.sh")"
assert_contains "含 --proto '=https'"       "${_src}" "--proto '=https'"
assert_contains "含 --proto-redir '=https'" "${_src}" "--proto-redir '=https'"

echo "== pip-sources：明文源分类与默认排除 =="
_pipsrc="${_REPO_ROOT}/scripts/tools/pip-sources/setup.sh"
# 只抽取纯函数，避免执行整个脚本。
eval "$(sed -n '/^get_pip_source_info() {/,/^}/p' "${_pipsrc}")"
eval "$(sed -n '/^get_pip_source_url() {/,/^}/p' "${_pipsrc}")"
eval "$(sed -n '/^_pip_source_is_insecure() {/,/^}/p' "${_pipsrc}")"

for s in hust tbsite tbsite_aliyun; do
  assert_true "识别 ${s} 为明文 HTTP" _pip_source_is_insecure "${s}"
done
for s in tsinghua aliyun ustc official antfin; do
  assert_false "识别 ${s} 为 HTTPS" _pip_source_is_insecure "${s}"
done

_pipsrc_text="$(cat "${_pipsrc}")"
assert_contains "默认排除明文源需要显式开关" "${_pipsrc_text}" "PIP_SOURCES_ALLOW_INSECURE"
assert_contains "trusted-host 仅对明文源生成" "${_pipsrc_text}" '_pip_source_is_insecure "$source_name" || continue'

echo "== sub2api 官方安装器必须锁定且可校验 =="
_sub="$(cat "${_REPO_ROOT}/scripts/services/sub2api/setup-offical.sh")"
assert_not_contains "不得再引用可变的 main 分支" "${_sub}" "sub2api/main/deploy/install.sh"
assert_contains     "URL 需锁定 commit SHA"      "${_sub}" 'SUB2API_INSTALLER_REF'
assert_contains     "需带默认 sha256"            "${_sub}" 'SUB2API_INSTALLER_SHA256'
# 只看代码行，注释里提到该模式（用于说明为何不再这么写）是允许的。
_sub_code="$(sed 's/[[:space:]]*#.*$//' "${_REPO_ROOT}/scripts/services/sub2api/setup-offical.sh")"
assert_not_contains "不得再 curl 管道进 sudo bash" "${_sub_code}" '| sudo bash'
assert_not_contains "不得再 curl 管道进 bash -s"   "${_sub_code}" '| bash -s'
assert_contains     "端口改写需校验是否生效"      "${_sub}" '端口改写失败'
# REF 必须是 40 位十六进制的 commit SHA，而非分支名。
_ref="$(printf '%s\n' "${_sub}" | sed -n 's/^SUB2API_INSTALLER_REF="\${SUB2API_INSTALLER_REF:-\([0-9a-f]*\)}"/\1/p' | head -1)"
assert_eq "REF 为 40 位 commit SHA" "40" "${#_ref}"

echo "== 服务不得默认把无认证界面暴露到全网 =="
_celery="$(cat "${_REPO_ROOT}/scripts/services/celery/setup.sh")"
assert_contains     "Flower 默认监听回环"   "${_celery}" 'FLOWER_ADDRESS:-127.0.0.1'
assert_contains     "支持 basic auth"       "${_celery}" 'FLOWER_BASIC_AUTH'

echo "== 敏感文件权限 =="
_s2m="$(cat "${_REPO_ROOT}/scripts/services/sub2api/setup-manual.sh")"
assert_contains "sub2api.env 以 umask 077 创建" "${_s2m}" 'umask 077'
_csm="$(cat "${_REPO_ROOT}/scripts/services/code-server/setup-manual.sh")"
assert_contains "code-server 日志收紧权限"      "${_csm}" 'umask 077'

echo "== ssh-keyscan 必须核对指纹 =="
_ghn="$(cat "${_REPO_ROOT}/scripts/tools/github-net/setup.sh")"
assert_contains "含 GitHub 官方 ed25519 指纹" "${_ghn}" "SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU"
assert_not_contains "不得再无条件追加 known_hosts" "${_ghn}" 'ssh-keyscan -p 443 ssh.github.com >> '

echo "== GitHub Actions 第三方 action 必须锁 SHA =="
_sync="$(cat "${_REPO_ROOT}/.github/workflows/sync.yml")"
assert_not_contains "不得使用 @master" "${_sync}" "hub-mirror-action@master"
assert_contains     "需锁定到 commit SHA" "${_sync}" "hub-mirror-action@ec47170fa9d126a4bee5a1ab9359845bb1395248"
assert_contains     "需声明最小权限" "${_sync}" "contents: read"

assert_summary
