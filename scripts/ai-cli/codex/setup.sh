#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../common.sh
source "${SCRIPT_DIR}/../common.sh"

AI_TOOL="OpenAI Codex"
AI_DEFAULT_METHOD=official
AI_METHODS=(official brew npm pnpm)
AI_METHOD_OPTIONS=(
  "official   OpenAI 独立安装器（推荐）"
  "brew       Homebrew cask"
  "npm        OpenAI npm 包"
  "pnpm       pnpm 安装 OpenAI npm 包"
)
PACKAGE="@openai/codex"

usage() {
  cat <<'EOF'
用法: nltdeploy ai codex [official|brew|npm|pnpm] <install|update|uninstall|status>

省略安装方式时默认 official。官方安装器支持 CODEX_INSTALL_DIR、CODEX_HOME 等变量。
EOF
}

official_install() {
  ai_require curl "请先安装 curl"
  local url="${CODEX_INSTALL_URL:-https://chatgpt.com/codex/install.sh}"
  _nlt_say_title "使用 OpenAI Codex 官方安装器"
  curl -fsSL "${url}" | sh -s -- "$@"
}

official_uninstall() {
  local install_dir="${CODEX_INSTALL_DIR:-${HOME}/.local/bin}"
  local codex_home="${CODEX_HOME:-${HOME}/.codex}"
  ai_safe_rm_home_path "${install_dir}/codex"
  ai_safe_rm_home_path "${install_dir}/codex-code-mode-host"
  ai_safe_rm_home_path "${codex_home}/packages/standalone"
}

brew_action() {
  ai_require brew "请先运行 nltdeploy tool brew install"
  case "$1" in
    install) brew install --cask codex ;;
    update) brew upgrade --cask codex ;;
    uninstall) brew uninstall --cask codex ;;
  esac
}

dispatch() {
  local method="$1" action="$2"
  shift 2
  if [[ "${action}" == "status" ]]; then
    if ai_has codex; then codex --version; else echo "codex: 未安装"; fi
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
