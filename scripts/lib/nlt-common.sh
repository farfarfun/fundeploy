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

# 解析 nlt-tools 可执行路径（bin 包装 → 仓库源两种布局）；找不到返回非 0。
_nlt_resolve_nlt_tools() {
  local NLTDEPLOY_ROOT="${NLTDEPLOY_ROOT:-${HOME}/.local/nltdeploy}"
  local _cand
  for _cand in \
    "${NLTDEPLOY_ROOT}/bin/nlt-tools" \
    "${_NLT_COMMON_LIB_DIR}/../../tools/nlt-tools.sh" \
    "${_NLT_COMMON_LIB_DIR}/../tools/nlt-tools.sh"; do
    if [[ -f "${_cand}" || -x "${_cand}" ]]; then
      printf '%s\n' "${_cand}"
      return 0
    fi
  done
  # 也尝试 PATH 上已有的 nlt-tools（适用于仓库内 shebang 直接执行的场景）
  command -v nlt-tools >/dev/null 2>&1 && { command -v nlt-tools; return 0; }
  return 1
}

# 确保 gum 可用（WAR-402）：
#   1. 已可用 → 立即返回 0（O(1) 幂等路径）。
#   2. 找到 nlt-tools → 委派 `nlt-tools gum install`（规范路径，服务内部依赖走此处）。
#   3. 兜底 → 原始 curl 直接拉取 utils/setup.sh 安装（独立脚本 / 首次 bootstrap 场景）。
_nlt_ensure_gum() {
  export PATH="${HOME}/opt/gum/bin:${PATH}"
  command -v gum >/dev/null 2>&1 && return 0

  if [[ -x "${HOME}/opt/gum/bin/gum" ]]; then
    export PATH="${HOME}/opt/gum/bin:${PATH}"
    command -v gum >/dev/null 2>&1 && return 0
  fi

  # 规范路径：委派给 nlt-tools gum install（幂等）。
  local _nlt_tools_bin
  if _nlt_tools_bin="$(_nlt_resolve_nlt_tools 2>/dev/null)"; then
    echo "未检测到 gum，执行: nlt-tools gum install" >&2
    if [[ -x "${_nlt_tools_bin}" ]]; then
      "${_nlt_tools_bin}" gum install || {
        echo "错误: nlt-tools gum install 失败。" >&2
        return 1
      }
    else
      bash "${_nlt_tools_bin}" gum install || {
        echo "错误: nlt-tools gum install 失败。" >&2
        return 1
      }
    fi
    export PATH="${HOME}/opt/gum/bin:${PATH}"
    command -v gum >/dev/null 2>&1 && return 0
    echo "错误: gum 仍未可用（nlt-tools gum install 后）。" >&2
    return 1
  fi

  # 兜底：curl 直接拉取（独立脚本 / 首次 bootstrap 场景，nlt-tools 尚未安装时）。
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
