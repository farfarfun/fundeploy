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
assert_contains     "连接 URL 输出经过脱敏" "${_celery}" 'redact_connection_url'
assert_not_contains "不得直接输出 broker URL" "${_celery}" 'echo "CELERY_BROKER_URL=${CELERY_BROKER_URL}"'

_funfluid="$(cat "${_REPO_ROOT}/scripts/services/funfluid-web/setup.sh")"
assert_contains     "funfluid 前端默认监听回环" "${_funfluid}" 'FUNFLUID_WEB_FRONTEND_HOST:-127.0.0.1'
assert_contains     "外部监听给出安全提示"     "${_funfluid}" '请确认防火墙和访问控制配置'

# sub2api 是订阅 API 网关（data/ 下有上游渠道 key），默认绑 0.0.0.0 等于把网关
# 交给整个网段。用真实的 status 输出断言，而不是 grep 源码里的字面量。
_out="$(NONINTERACTIVE=1 SUB2API_SERVICE_HOME="$(mktemp -d)" \
  bash "${_REPO_ROOT}/scripts/services/sub2api/setup-manual.sh" status 2>&1)"
assert_contains     "sub2api 默认监听回环"   "${_out}" "监听: http://127.0.0.1:"
assert_not_contains "sub2api 默认不绑全网卡" "${_out}" "0.0.0.0"

# 聚合 status 的默认值必须与服务脚本一致，否则会展示出一个服务不会绑定的地址。
_svcs="$(cat "${_REPO_ROOT}/scripts/services/fundeploy-services.sh")"
assert_contains "status 汇总的 Flower 默认地址对齐回环" "${_svcs}" 'FLOWER_ADDRESS:-127.0.0.1'

echo "== 凭据不得出现在命令行/进程列表里 =="
# 只看代码行：注释里提到这些写法（用于说明为何不再这么写）是允许的。
_celery_code="$(sed 's/[[:space:]]*#.*$//' "${_REPO_ROOT}/scripts/services/celery/setup.sh")"
# /proc/<pid>/cmdline 对本机所有用户可读，守护进程的命令行等于长期公开的明文。
assert_not_contains "Flower 认证不得走 --basic-auth" "${_celery_code}" '--basic-auth'
assert_contains     "Flower 认证改走环境变量"        "${_celery}" 'export FLOWER_BASIC_AUTH'
# start-flower 与 run-flower 必须走同一条认证/告警路径，否则前台会跑出无认证实例。
assert_eq "prepare_flower_env 被 start/run 两条路径调用" "2" \
  "$(grep -c '^  prepare_flower_env$' "${_REPO_ROOT}/scripts/services/celery/setup.sh")"

echo "== 连接 URL 脱敏必须真的脱敏（行为断言，不只看字面量）=="
# 只抽取纯函数，避免执行整个服务脚本。
eval "$(sed -n '/^redact_connection_url() {/,/^}/p' "${_REPO_ROOT}/scripts/services/celery/setup.sh")"
assert_eq "剥掉 userinfo 只留 host:port" "redis://db.internal:6379" \
  "$(redact_connection_url 'redis://admin:s3cr3t@db.internal:6379/0')"
assert_not_contains "脱敏结果不含密码" \
  "$(redact_connection_url 'redis://admin:s3cr3t@db.internal:6379/0')" 's3cr3t'
assert_not_contains "脱敏结果不含查询串里的 token" \
  "$(redact_connection_url 'amqp://u:p@mq:5672/vhost?token=abc123')" 'abc123'
assert_eq "无 userinfo 时保持可读" "redis://localhost:6379" \
  "$(redact_connection_url 'redis://localhost:6379/0')"
assert_eq "无法解析时不泄漏原值" "<configured>" \
  "$(redact_connection_url 'admin:s3cr3t@db.internal:6379')"

echo "== 第三方依赖必须带版本下限（SPEC §5）=="
assert_not_contains "不得安装裸包"       "${_celery_code}" 'pip install celery redis flower'
assert_not_contains "升级也不得用裸包"   "${_celery_code}" 'pip install -U celery redis flower'
assert_contains     "celery 带版本下限"  "${_celery}" 'CELERY_PKG_SPEC:-celery>='
assert_contains     "redis 带版本下限"   "${_celery}" 'CELERY_REDIS_PKG_SPEC:-redis>='
assert_contains     "flower 带版本下限"  "${_celery}" 'CELERY_FLOWER_PKG_SPEC:-flower>='
assert_contains     "优先用 uv 建环境"   "${_celery}" 'uv venv "$CELERY_VENV"'

echo "== 敏感文件权限 =="
_s2m="$(cat "${_REPO_ROOT}/scripts/services/sub2api/setup-manual.sh")"
assert_contains "sub2api.env 以 umask 077 创建" "${_s2m}" 'umask 077'
_csm="$(cat "${_REPO_ROOT}/scripts/services/code-server/setup-manual.sh")"
assert_contains "code-server 日志收紧权限"      "${_csm}" 'umask 077'

echo "== ssh-keyscan 必须核对指纹 =="
_ghn="$(cat "${_REPO_ROOT}/scripts/tools/github-net/setup.sh")"
assert_contains "含 GitHub 官方 ed25519 指纹" "${_ghn}" "SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU"
assert_not_contains "不得再无条件追加 known_hosts" "${_ghn}" 'ssh-keyscan -p 443 ssh.github.com >> '

assert_summary
