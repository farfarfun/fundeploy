#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../common.sh
source "${SCRIPT_DIR}/../common.sh"

AI_TOOL="Cursor CLI"
AI_DEFAULT_METHOD=official
AI_METHODS=(official)
AI_METHOD_OPTIONS=("official   Cursor 官方安装器")

usage() {
  cat <<'EOF'
用法: nltdeploy ai cursor [official] <install|update|uninstall|status>

Cursor 当前只公布官方安装器，省略安装方式时默认 official。
EOF
}

official_install() {
  ai_require curl "请先安装 curl"
  local url="${CURSOR_INSTALL_URL:-https://cursor.com/install}"
  _nlt_say_title "使用 Cursor CLI 官方安装器"
  curl "${url}" -fsS | bash -s -- "$@"
}

official_update() {
  if ai_has agent; then
    agent update
  elif ai_has cursor-agent; then
    cursor-agent update
  else
    official_install "$@"
  fi
}

official_uninstall() {
  ai_safe_rm_home_path "${HOME}/.local/bin/agent"
  ai_safe_rm_home_path "${HOME}/.local/bin/cursor-agent"
  ai_safe_rm_home_path "${HOME}/.local/share/cursor-agent"
}

dispatch() {
  local method="$1" action="$2"
  shift 2
  case "${action}" in
    install) official_install "$@" ;;
    update) official_update "$@" ;;
    uninstall) official_uninstall ;;
    status)
      if ai_has agent; then agent --version
      elif ai_has cursor-agent; then cursor-agent --version
      else echo "cursor: 未安装"
      fi
      ;;
  esac
}

ai_main "$@"
