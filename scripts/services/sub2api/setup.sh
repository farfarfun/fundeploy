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

OFFICIAL_SCRIPT="${SCRIPT_DIR}/setup-offical.sh"
MANUAL_SCRIPT="${SCRIPT_DIR}/setup-manual.sh"

usage() {
  cat <<'EOF'
用法: fundeploy service sub2api <模式> [动作]

模式:
  official  上游官方脚本 + systemd，默认安装到 /opt/sub2api
  manual    本地二进制 + PID 管理，默认安装到 ~/opt/sub2api

两种模式均支持 install、update、start、stop、restart、status、uninstall。
官方模式额外支持 logs，默认端口均为 8802。

示例:
  fundeploy service sub2api official install
  fundeploy service sub2api official restart
  fundeploy service sub2api manual install
  fundeploy service sub2api manual start

默认: 不写模式时使用 official；run 和版本查询仅由 manual 支持。
兼容: install-official 等价于 official install。
EOF
}

interactive_main() {
  _nlt_ensure_gum || exit 1
  nlt_ui_banner "fundeploy / service / sub2api" "选择互相独立的安装与服务管理模式" >&2
  set +e
  while true; do
    local pick
    pick="$(nlt_ui_choose "fundeploy / service / sub2api / 选择模式" \
      "official   官方模式 · 上游脚本 + systemd" \
      "manual     手动模式 · 本地二进制 + PID" \
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
    run|list-versions|versions)
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
