#!/usr/bin/env bash
# nltdeploy 公共片段：由各域 setup 脚本 source（路径：与脚本同树上一级 lib/）。
# 规范见 docs/superpowers/specs/2026-04-11-nltdeploy-tool-service-conventions.md
[[ -n "${_NLT_COMMON_LOADED:-}" ]] && return 0
_NLT_COMMON_LOADED=1

_NLT_COMMON_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=nlt-github-download.sh
source "${_NLT_COMMON_LIB_DIR}/nlt-github-download.sh"

_nltdeploy_raw_base() {
  printf '%s\n' "${NLTDEPLOY_RAW_BASE:-${nltdeploy_RAW_BASE:-https://raw.githubusercontent.com/farfarfun/nltdeploy/HEAD}}"
}

_nlt_gum_utils_setup_url() {
  printf '%s\n' "$(_nltdeploy_raw_base)/scripts/tools/utils/setup.sh"
}

# 返回监听指定 TCP 端口的首个 PID；未找到时输出空串。
_nlt_listener_pid_for_port() {
  local port="$1"
  if command -v lsof >/dev/null 2>&1; then
    lsof -tiTCP:"${port}" -sTCP:LISTEN -n -P 2>/dev/null | head -1
    return 0
  fi
  if command -v ss >/dev/null 2>&1; then
    ss -ltnp "( sport = :${port} )" 2>/dev/null | awk -F 'pid=' 'NR>1 && NF>1 {split($2,a,","); print a[1]; exit}'
    return 0
  fi
  echo ""
}

# 已安装 gum 则立即返回；否则拉取 scripts/tools/utils/setup.sh 安装（不单独做「仅检测并报错」）。
_nlt_ensure_gum() {
  export PATH="${HOME}/opt/gum/bin:${PATH}"
  command -v gum >/dev/null 2>&1 && return 0

  if [[ -x "${HOME}/opt/gum/bin/gum" ]]; then
    export PATH="${HOME}/opt/gum/bin:${PATH}"
    command -v gum >/dev/null 2>&1 && return 0
  fi

  command -v curl >/dev/null 2>&1 || {
    echo "错误: 需要 curl 以安装 gum。" >&2
    return 1
  }

  local _url
  _url="$(_nlt_gum_utils_setup_url)"
  echo "未检测到 gum，执行: curl -LsSf ${_url} | bash -s -- gum" >&2
  _nlt_github_download_curl -LsSf "${_url}" | bash -s -- gum || {
    echo "错误: gum 安装失败（网络或 NLTDEPLOY_RAW_BASE / nltdeploy_RAW_BASE）。" >&2
    return 1
  }

  export PATH="${HOME}/opt/gum/bin:${PATH}"
  command -v gum >/dev/null 2>&1 || {
    echo "错误: gum 仍未可用（预期 ~/opt/gum/bin）。" >&2
    return 1
  }
  return 0
}
