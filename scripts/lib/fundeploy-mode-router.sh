#!/usr/bin/env bash
# official/manual 双模式服务的公共路由。

_FUNDEPLOY_MODE_LIB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=fundeploy-common.sh
source "${_FUNDEPLOY_MODE_LIB}/fundeploy-common.sh"

fundeploy_mode_usage() {
  cat <<EOF
用法: fundeploy service ${FUNDEPLOY_MODE_SERVICE} <模式> [动作]

模式:
  official  ${FUNDEPLOY_MODE_OFFICIAL_DESC}
  manual    ${FUNDEPLOY_MODE_MANUAL_DESC}

两种模式均支持 install、update、start、stop、restart、status、uninstall。
${FUNDEPLOY_MODE_EXTRA_NOTE}

示例:
  fundeploy service ${FUNDEPLOY_MODE_SERVICE} official install
  fundeploy service ${FUNDEPLOY_MODE_SERVICE} official start
  fundeploy service ${FUNDEPLOY_MODE_SERVICE} manual install
  fundeploy service ${FUNDEPLOY_MODE_SERVICE} manual start

默认: 不写模式时使用 official；${FUNDEPLOY_MODE_MANUAL_NOTE}
兼容: install-official 等价于 official install。
EOF
}

_fundeploy_mode_is_manual_action() {
  case " run ${FUNDEPLOY_MODE_MANUAL_ACTIONS:-} " in
    *" $1 "*) return 0 ;;
    *) return 1 ;;
  esac
}

_fundeploy_mode_interactive() {
  _fundeploy_ensure_gum || exit 1
  fundeploy_ui_banner "fundeploy / service / ${FUNDEPLOY_MODE_SERVICE}" "选择互相独立的安装与服务管理模式" >&2
  set +e
  while true; do
    local pick
    pick="$(fundeploy_ui_choose "fundeploy / service / ${FUNDEPLOY_MODE_SERVICE} / 选择模式" \
      "official   官方模式 · ${FUNDEPLOY_MODE_OFFICIAL_MENU}" \
      "manual     手动模式 · ${FUNDEPLOY_MODE_MANUAL_MENU}" \
      "help       命令帮助" \
      "back       返回")" || break
    case "${pick%% *}" in
      official) bash "${FUNDEPLOY_MODE_OFFICIAL_SCRIPT}" ;;
      manual) bash "${FUNDEPLOY_MODE_MANUAL_SCRIPT}" ;;
      help) fundeploy_mode_usage ;;
      *) break ;;
    esac
    echo ""
  done
  set -e
}

fundeploy_mode_router_main() {
  local cmd="${1:-}"
  case "$cmd" in
    "")
      [[ "${NONINTERACTIVE:-}" != "1" ]] || { fundeploy_mode_usage >&2; return 1; }
      _fundeploy_mode_interactive
      ;;
    official|offical)
      shift
      exec bash "${FUNDEPLOY_MODE_OFFICIAL_SCRIPT}" "$@"
      ;;
    manual|local)
      shift
      exec bash "${FUNDEPLOY_MODE_MANUAL_SCRIPT}" "$@"
      ;;
    install-official)
      shift
      exec bash "${FUNDEPLOY_MODE_OFFICIAL_SCRIPT}" install "$@"
      ;;
    install|update|start|stop|restart|status|uninstall)
      exec bash "${FUNDEPLOY_MODE_OFFICIAL_SCRIPT}" "$@"
      ;;
    help|-h|--help)
      fundeploy_mode_usage
      ;;
    *)
      if _fundeploy_mode_is_manual_action "$cmd"; then
        exec bash "${FUNDEPLOY_MODE_MANUAL_SCRIPT}" "$@"
      fi
      echo "错误: 未知模式或命令: ${cmd}" >&2
      fundeploy_mode_usage >&2
      return 2
      ;;
  esac
}
