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
用法: fundeploy ai cursor [official] <install|update|uninstall|status>

Cursor 当前只公布官方安装器，省略安装方式时默认 official。
EOF
}

official_install() {
  ai_require curl "请先安装 curl"
  local url="${CURSOR_INSTALL_URL:-https://cursor.com/install}"
  _nlt_say_title "使用 Cursor CLI 官方安装器"
  # -L 不可省：cursor.com/install 会重定向，缺 -L 时 curl 只输出那段重定向
  # 响应体并以 0 退出，bash 执行空内容 → 静默「安装成功」但什么都没装。
  # 同级的 claude/codex 安装器本来就带 -fsSL。
  curl -fsSL --proto '=https' --proto-redir '=https' --tlsv1.2 "${url}" | bash -s -- "$@"
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
