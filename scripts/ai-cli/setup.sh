#!/usr/bin/env bash
# AI CLI 统一入口：各工具的安装方式由独立子目录负责。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=common.sh
source "${SCRIPT_DIR}/common.sh"

TOOLS=(claude codex cursor)

usage() {
  cat <<'EOF'
用法: nltdeploy ai <工具|all> [安装方式] <动作> [args...]
      nltdeploy ai                    # 交互选择工具、安装方式和动作
      nltdeploy ai list

工具与安装方式（官方方式优先且为默认）:
  claude   official | brew | npm | pnpm
  codex    official | brew | npm | pnpm
  cursor   official
  all      依次使用各工具的 official 方式

动作:
  install | update | uninstall | status

环境变量:
  CLAUDE_INSTALL_URL | CODEX_INSTALL_URL | CURSOR_INSTALL_URL  覆盖官方安装器地址
  NLT_ASSUME_YES=1   允许非交互删除官方安装产物

示例:
  nltdeploy ai claude install
  nltdeploy ai claude pnpm install
  nltdeploy ai codex brew update
  nltdeploy ai cursor official install
EOF
}

tool_script() {
  local tool="$1"
  printf '%s/%s/setup.sh\n' "${SCRIPT_DIR}" "${tool}"
}

run_tool() {
  local tool="$1" script
  shift
  script="$(tool_script "${tool}")"
  [[ -f "${script}" ]] || ai_die "找不到 ${tool} 安装脚本: ${script}"
  bash "${script}" "$@"
}

interactive_main() {
  ai_interactive || { usage >&2; ai_die "无参数模式需要交互式 TTY"; }
  local tool
  tool="$(ai_pick "选择 AI CLI 工具" "${TOOLS[@]}" all)" || ai_die "未选择工具"
  if [[ "${tool}" == "all" ]]; then
    local item
    for item in "${TOOLS[@]}"; do run_tool "${item}"; done
  else
    run_tool "${tool}"
  fi
}

main() {
  if [[ $# -eq 0 ]]; then
    # 与其余入口一致：非交互时打印帮助而非进入 gum 菜单。
    if [[ "${NONINTERACTIVE:-0}" == "1" || ! -t 0 ]]; then
      usage
      return 0
    fi
    interactive_main
    return
  fi

  local tool="$1"
  shift
  case "${tool}" in
    help|-h|--help) usage ;;
    list|--list) printf '%s\n' "${TOOLS[@]}" ;;
    all)
      local item
      for item in "${TOOLS[@]}"; do run_tool "${item}" "$@"; done
      ;;
    claude|codex|cursor) run_tool "${tool}" "$@" ;;
    # 退出码 2 = 用法错误，与其余入口对齐。
    *) echo "错误: 未知工具: ${tool}" >&2; usage >&2; exit 2 ;;
  esac
}

main "$@"
