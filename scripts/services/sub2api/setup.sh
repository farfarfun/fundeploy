#!/usr/bin/env bash
# Sub2API 总入口：在手动安装与官方安装之间显式路由。

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

MANUAL_SCRIPT="${SCRIPT_DIR}/setup-manual.sh"
OFFICIAL_SCRIPT="${SCRIPT_DIR}/setup-offical.sh"

usage() {
  cat <<'EOF'
用法: nltdeploy service sub2api <模式> [动作]

模式:
  manual    本地二进制 + PID 管理，默认安装到 ~/opt/sub2api
  official  上游官方脚本 + systemd，默认安装到 /opt/sub2api

两种模式均支持 install、update、start、stop、restart、status、uninstall。
官方模式额外支持 logs，默认端口均为 8802。

示例:
  nltdeploy service sub2api manual install
  nltdeploy service sub2api manual start
  nltdeploy service sub2api official install
  nltdeploy service sub2api official restart

兼容: 不写模式时沿用 manual；install-official 等价于 official install。
EOF
}

interactive_main() {
  _nlt_ensure_gum || exit 1
  nlt_ui_banner "nltdeploy / service / sub2api" "选择互相独立的安装与服务管理模式" >&2
  set +e
  while true; do
    local pick
    pick="$(nlt_ui_choose "nltdeploy / service / sub2api / 选择模式" \
      "manual     手动模式 · 本地二进制 + PID" \
      "official   官方模式 · 上游脚本 + systemd" \
      "help       命令帮助" \
      "back       返回")" || break
    pick="${pick%% *}"
    case "$pick" in
      manual) bash "${MANUAL_SCRIPT}" ;;
      official) bash "${OFFICIAL_SCRIPT}" ;;
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
    manual|local)
      shift
      exec bash "${MANUAL_SCRIPT}" "$@"
      ;;
    official|offical)
      shift
      exec bash "${OFFICIAL_SCRIPT}" "$@"
      ;;
    install-official)
      shift
      exec bash "${OFFICIAL_SCRIPT}" install "$@"
      ;;
    install|update|start|run|stop|restart|status|list-versions|versions|uninstall)
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
