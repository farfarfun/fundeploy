#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../common.sh
source "${SCRIPT_DIR}/../common.sh"

AI_TOOL="Claude Code"
AI_DEFAULT_METHOD=official
AI_METHODS=(official brew npm pnpm)
AI_METHOD_OPTIONS=(
  "official   官方原生安装器（推荐）"
  "brew       Homebrew cask"
  "npm        官方 npm 包"
  "pnpm       pnpm 安装官方 npm 包"
)
PACKAGE="@anthropic-ai/claude-code"

usage() {
  cat <<'EOF'
用法: fundeploy ai claude [official|brew|npm|pnpm] <install|update|uninstall|status>

省略安装方式时默认 official。官方安装器额外参数会原样传递，例如:
  fundeploy ai claude official install stable
EOF
}

official_install() {
  ai_require curl "请先安装 curl"
  local url="${CLAUDE_INSTALL_URL:-https://claude.ai/install.sh}"
  _nlt_say_title "使用 Claude Code 官方安装器"
  curl -fsSL "${url}" | bash -s -- "$@"
}

official_uninstall() {
  ai_safe_rm_home_path "${HOME}/.local/bin/claude"
  ai_safe_rm_home_path "${HOME}/.local/share/claude"
}

brew_action() {
  ai_require brew "请先运行 fundeploy tool brew install"
  local action="$1" cask="${CLAUDE_BREW_CASK:-claude-code}"
  case "${action}" in
    install) brew install --cask "${cask}" ;;
    update) brew upgrade --cask "${cask}" ;;
    uninstall) brew uninstall --cask "${cask}" ;;
  esac
}

dispatch() {
  local method="$1" action="$2"
  shift 2
  if [[ "${action}" == "status" ]]; then
    if ai_has claude; then claude --version; else echo "claude: 未安装"; fi
    return
  fi
  case "${method}:${action}" in
    official:install|official:update) official_install "$@" ;;
    official:uninstall) official_uninstall ;;
    brew:*) brew_action "${action}" ;;
    npm:install|npm:update) ai_package_install npm "${PACKAGE}" ;;
    npm:uninstall) ai_package_uninstall npm "${PACKAGE}" ;;
    pnpm:install|pnpm:update) ai_package_install pnpm "${PACKAGE}" ;;
    pnpm:uninstall) ai_package_uninstall pnpm "${PACKAGE}" ;;
  esac
}

ai_main "$@"
