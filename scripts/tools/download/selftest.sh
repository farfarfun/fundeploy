#!/usr/bin/env bash
# fundeploy tool download 轻量自测（resolve-url 固定用例）
set -euo pipefail

_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
_NLT_LIB=""
if [[ -f "${_SCRIPT_DIR}/../lib/nlt-common.sh" ]]; then
  _NLT_LIB="$(cd "${_SCRIPT_DIR}/../lib" && pwd)"
elif [[ -f "${_SCRIPT_DIR}/../../lib/nlt-common.sh" ]]; then
  _NLT_LIB="$(cd "${_SCRIPT_DIR}/../../lib" && pwd)"
else
  echo "selftest: 找不到 lib" >&2
  exit 1
fi
# shellcheck source=../../lib/nlt-github-download.sh
source "${_NLT_LIB}/nlt-github-download.sh"

_die() {
  echo "selftest FAIL: $*" >&2
  exit 1
}

_expect() {
  local name="$1" want="$2" got="$3"
  if [[ "$got" != "$want" ]]; then
    _die "${name}: mismatch want=${want} got=${got}"
  fi
}

# 在干净环境下 source 库并解析（子 shell）
_resolve_under_env() {
  local url="$1"
  (
    unset FUNDEPLOY_GITHUB_HUB_PROXY_PREFIX FUNDEPLOY_GITHUB_DOWNLOAD_MODE FUNDEPLOY_GITHUB_RAW_MIRROR_BASE 2>/dev/null || true
    export PATH="${PATH}"
    export HOME="${HOME:-/tmp}"
    [[ -n "${2+x}" ]] && export FUNDEPLOY_GITHUB_HUB_PROXY_PREFIX="$2"
    [[ -n "${3+x}" ]] && export FUNDEPLOY_GITHUB_DOWNLOAD_MODE="$3"
    [[ -n "${4+x}" ]] && export FUNDEPLOY_GITHUB_RAW_MIRROR_BASE="$4"
    # shellcheck source=../../lib/nlt-github-download.sh
    source "${_NLT_LIB}/nlt-github-download.sh"
    _nlt_github_download_resolve_url "$url"
  )
}

_run() {
  local name="$1" url="$2" want="$3"
  shift 3
  local got
  got="$(_resolve_under_env "$url" "$@")"
  _expect "$name" "$want" "$got"
}

# 1) 默认 off：GitHub URL 不变
_run "off_github" "https://github.com/octocat/Hello-World/releases/download/v1.0/a.tgz" \
  "https://github.com/octocat/Hello-World/releases/download/v1.0/a.tgz" "" "" ""

# 2) hub 前缀：整 URL 拼接
_run "hub_proxy" "https://github.com/octocat/Hello-World/releases/download/v1.0/a.tgz" \
  "https://proxy.example/https://https://github.com/octocat/Hello-World/releases/download/v1.0/a.tgz" \
  "https://proxy.example/https://" "" ""

# 3) 非 GitHub：直通
_run "not_github" "https://example.com/path" "https://example.com/path" \
  "https://proxy.example/https://" "" ""

# 4) mirror_raw：仅 raw 主机
_run "mirror_raw" "https://raw.githubusercontent.com/o/r/v/f.txt" \
  "https://mirror.example/ghraw/o/r/v/f.txt" "" "mirror_raw" "https://mirror.example/ghraw"

# 5) hub：已是前缀开头则不再双写
_run "hub_no_double" \
  "https://proxy.example/https://https://github.com/x/y" \
  "https://proxy.example/https://https://github.com/x/y" \
  "https://proxy.example/https://" "" ""

echo "selftest OK"
