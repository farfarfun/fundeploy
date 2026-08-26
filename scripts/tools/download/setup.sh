#!/usr/bin/env bash
# fundeploy tool download — 对 GitHub 族 HTTPS URL 做可选改写后调用 curl，或单独解析 URL。
#
# 用法:
#   ./setup.sh curl [curl 参数…]     # 扫描参数中的 http(s) URL 并改写后 exec curl
#   ./setup.sh resolve-url <url>    # 打印改写后的一行 URL
#   ./setup.sh install|update|reinstall|uninstall   # 随 fundeploy 分发，无额外持久化
#   NONINTERACTIVE=1 ./setup.sh install  # 安装提示 + 运行自测
#   ./setup.sh                      # gum 菜单（需 gum）
#
set -euo pipefail

_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
_NLT_LIB=""
if [[ -f "${_SCRIPT_DIR}/../lib/nlt-common.sh" ]]; then
  _NLT_LIB="$(cd "${_SCRIPT_DIR}/../lib" && pwd)"
elif [[ -f "${_SCRIPT_DIR}/../../lib/nlt-common.sh" ]]; then
  _NLT_LIB="$(cd "${_SCRIPT_DIR}/../../lib" && pwd)"
else
  echo "错误: 找不到 lib/nlt-common.sh（已检查 ${_SCRIPT_DIR}/../lib 与 ${_SCRIPT_DIR}/../../lib）" >&2
  exit 1
fi

# shellcheck source=../../lib/nlt-common.sh
source "${_NLT_LIB}/nlt-common.sh"
# shellcheck source=../../lib/nlt-github-download.sh
source "${_NLT_LIB}/nlt-github-download.sh"

_dl_say() { printf '%s\n' "$*"; }
_dl_err() { printf '错误: %s\n' "$*" >&2; }

_usage() {
  cat <<EOF
用法: $(basename "${BASH_SOURCE[0]}") <子命令> [参数…]

  curl [curl 选项与 URL…]   对参数中的 https:// GitHub 族 URL 按需改写后执行 curl（透传退出码）
  resolve-url <url>        打印一行改写结果（调试用）

  install / update / reinstall / uninstall
                          本工具随 fundeploy 安装到 libexec；此处仅提示说明。
                          设置 NONINTERACTIVE=1 且 install 时会运行内置自测。

环境变量（默认均关闭，行为与直连 curl 一致）:
  FUNDEPLOY_GITHUB_HUB_PROXY_PREFIX   非空时优先：将完整原始 URL 拼在该前缀之后
  FUNDEPLOY_GITHUB_DOWNLOAD_MODE      off（默认）| mirror_raw | hub_proxy
  FUNDEPLOY_GITHUB_RAW_MIRROR_BASE    mirror_raw 模式下用于 raw.githubusercontent.com 的前缀

详见: scripts/tools/download/README.md
EOF
}

cmd_curl() {
  command -v curl >/dev/null 2>&1 || {
    _dl_err "需要 curl。"
    exit 1
  }
  _nlt_github_download_curl "$@"
  exit $?
}

cmd_resolve_url() {
  if [[ $# -lt 1 ]]; then
    _dl_err "缺少 URL。示例: resolve-url https://raw.githubusercontent.com/foo/bar"
    exit 2
  fi
  _nlt_github_download_resolve_url "$1"
}

_cmd_tool_meta() {
  local verb="$1"
  _dl_say "[fundeploy download] ${verb}：本工具随 fundeploy 提供，无需单独安装。"
  if [[ "${NONINTERACTIVE:-0}" == "1" ]] && [[ "$verb" == "install" ]]; then
    if [[ -x "${_SCRIPT_DIR}/selftest.sh" ]]; then
      bash "${_SCRIPT_DIR}/selftest.sh"
    fi
  fi
}

cmd_install() { _cmd_tool_meta "install"; }
cmd_update() { _cmd_tool_meta "update"; }
cmd_reinstall() { _cmd_tool_meta "reinstall"; }

cmd_uninstall() {
  _dl_say "[fundeploy download] uninstall：请使用 fundeploy uninstall。"
}

_interactive_main() {
  while true; do
    local pick
    pick="$(gum choose --header "fundeploy / download — GitHub 下载加速" \
      "查看用法 (help)" \
      "运行内置自测" \
      "退出")" || {
      _dl_say "已取消。"
      return 0
    }
    case "$pick" in
      *help*)
        _usage
        ;;
      *自测*)
        bash "${_SCRIPT_DIR}/selftest.sh" || true
        ;;
      *退出*)
        return 0
        ;;
      *)
        _usage
        ;;
    esac
    _dl_say ""
  done
}

main() {
  if [[ "${1:-}" == "help" || "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
    _usage
    exit 0
  fi

  if [[ $# -eq 0 ]]; then
    if [[ "${NONINTERACTIVE:-0}" == "1" ]]; then
      _usage
      exit 0
    fi
    _nlt_ensure_gum || exit 1
    _interactive_main
    exit 0
  fi

  local cmd="$1"
  shift
  case "$cmd" in
    curl) cmd_curl "$@" ;;
    resolve-url) cmd_resolve_url "$@" ;;
    install) cmd_install "$@" ;;
    update|upgrade) cmd_update "$@" ;;
    reinstall) cmd_reinstall "$@" ;;
    uninstall) cmd_uninstall "$@" ;;
    help) _usage ;;
    *)
      _dl_err "未知子命令: ${cmd}"
      _usage >&2
      exit 2
      ;;
  esac
}

main "$@"
