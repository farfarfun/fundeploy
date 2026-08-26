#!/usr/bin/env bash
# code-server 总入口：在手动安装与官方安装之间显式路由。

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ -f "${SCRIPT_DIR}/../lib/nlt-common.sh" ]]; then
  # shellcheck source=../lib/nlt-common.sh
  source "${SCRIPT_DIR}/../lib/nlt-common.sh"
elif [[ -f "${SCRIPT_DIR}/../../lib/nlt-common.sh" ]]; then
  # shellcheck source=../../lib/nlt-common.sh
  source "${SCRIPT_DIR}/../../lib/nlt-common.sh"
else
  echo "错误: 找不到 lib/nlt-common.sh" >&2
  exit 1
fi

OFFICIAL_SCRIPT="${SCRIPT_DIR}/setup-offical.sh"
MANUAL_SCRIPT="${SCRIPT_DIR}/setup-manual.sh"

usage() {
  cat <<'EOF'
用法: fundeploy service code-server <模式> [动作]

模式:
  official  官方 install.sh + 系统包管理器和系统服务
  manual    官方 Release 包 + 本地 PID，默认安装到 ~/opt/code-server

两种模式均支持 install、update、start、stop、restart、status、uninstall。
官方模式额外支持 logs。

示例:
  fundeploy service code-server official install
  fundeploy service code-server official start
  fundeploy service code-server manual install
  fundeploy service code-server manual start

默认: 不写模式时使用 official；run 仅由 manual 支持。
兼容: install-official 等价于 official install。
EOF
}

interactive_main() {
  _nlt_ensure_gum || exit 1
  nlt_ui_banner "fundeploy / service / code-server" "选择互相独立的安装与服务管理模式" >&2
  set +e
  while true; do
    local pick
    pick="$(nlt_ui_choose "fundeploy / service / code-server / 选择模式" \
      "official   官方模式 · install.sh + 系统服务" \
      "manual     手动模式 · Release 包 + 本地 PID" \
      "help       命令帮助" \
      "back       返回")" || break
    pick="${pick%% *}"
    case "$pick" in
      official) bash "${OFFICIAL_SCRIPT}" ;;
      manual) bash "${MANUAL_SCRIPT}" ;;
      help) usage ;;
      *) break ;;
    esac
    echo ""
  done
  set -e
}

main() {
  local cmd="${1:-}"
  case "$cmd" in
    "")
      [[ "${NONINTERACTIVE:-}" != "1" ]] || { usage >&2; exit 1; }
      interactive_main
      ;;
    official|offical)
      shift
      exec bash "${OFFICIAL_SCRIPT}" "$@"
      ;;
    manual|local)
      shift
      exec bash "${MANUAL_SCRIPT}" "$@"
      ;;
    install-official)
      shift
      exec bash "${OFFICIAL_SCRIPT}" install "$@"
      ;;
    install|update|start|stop|restart|status|uninstall)
      exec bash "${OFFICIAL_SCRIPT}" "$@"
      ;;
    run)
      exec bash "${MANUAL_SCRIPT}" "$@"
      ;;
    help|-h|--help) usage ;;
    *)
      echo "错误: 未知模式或命令: ${cmd}" >&2
      usage >&2
      exit 2
      ;;
  esac
}

main "$@"
