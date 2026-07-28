#!/usr/bin/env bash
# Sub2API 官方模式：调用上游安装脚本，并通过 systemd 管理服务。

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

OFFICIAL_INSTALLER_URL="https://raw.githubusercontent.com/Wei-Shaw/sub2api/main/deploy/install.sh"
SERVICE_NAME="sub2api"

die() { echo "错误: $*" >&2; exit 1; }

usage() {
  cat <<'EOF'
用法: setup-offical.sh [command] [选项]

命令:
  install [上游选项]      使用官方脚本安装（/opt/sub2api + systemd，端口默认 8802）
  update [上游选项]       使用官方脚本升级
  start / stop / restart  通过 systemd 管理服务
  status                  查看 systemd 服务状态
  logs [journalctl 选项]  查看服务日志（默认持续跟踪）
  uninstall [上游选项]    使用官方脚本卸载；支持 -y、--purge

示例:
  nltdeploy service sub2api official install
  nltdeploy service sub2api official restart
  nltdeploy service sub2api official logs -n 100
  nltdeploy service sub2api official uninstall -y
EOF
}

as_root() {
  if [[ "$(id -u)" -eq 0 ]]; then
    "$@"
  else
    command -v sudo >/dev/null 2>&1 || die "需要 root 权限，但未找到 sudo"
    sudo "$@"
  fi
}

run_official() {
  command -v curl >/dev/null 2>&1 || die "需要 curl"
  local command="$1"
  shift
  echo "==> 调用 Sub2API 官方脚本: ${command}（默认端口 8802）" >&2
  if [[ "$(id -u)" -eq 0 ]]; then
    curl -fsSL "${OFFICIAL_INSTALLER_URL}" \
      | sed 's/^SERVER_PORT="8080"$/SERVER_PORT="8802"/' \
      | bash -s -- "${command}" "$@"
  else
    command -v sudo >/dev/null 2>&1 || die "官方脚本需要 root 权限，但未找到 sudo"
    curl -fsSL "${OFFICIAL_INSTALLER_URL}" \
      | sed 's/^SERVER_PORT="8080"$/SERVER_PORT="8802"/' \
      | sudo bash -s -- "${command}" "$@"
  fi
}

cmd_uninstall() {
  local arg confirmed=0
  for arg in "$@"; do
    [[ "$arg" == "-y" || "$arg" == "--yes" ]] && confirmed=1
  done
  if [[ "$confirmed" == "0" && -t 0 ]]; then
    nlt_ui_confirm "确认使用官方脚本卸载 Sub2API？" || return 0
    set -- "$@" -y
  fi
  run_official uninstall "$@"
}

interactive_main() {
  _nlt_ensure_gum || exit 1
  nlt_ui_banner "nltdeploy / service / sub2api / official" "官方脚本 · /opt/sub2api · systemd · 端口 8802" >&2
  set +e
  while true; do
    local pick
    pick="$(nlt_ui_choose "nltdeploy / service / sub2api / official / 选择动作" \
      "install           安装" \
      "update            更新" \
      "start             启动" \
      "stop              停止" \
      "restart           重启" \
      "status            状态" \
      "logs              日志" \
      "uninstall         卸载" \
      "help              命令帮助" \
      "quit              返回")" || break
    pick="${pick%% *}"
    case "$pick" in
      install) run_official install ;;
      update) run_official upgrade ;;
      start|stop|restart) as_root systemctl "$pick" "${SERVICE_NAME}" ;;
      status) as_root systemctl status "${SERVICE_NAME}" --no-pager ;;
      logs) as_root journalctl -u "${SERVICE_NAME}" -f ;;
      uninstall) cmd_uninstall ;;
      help) usage ;;
      *) break ;;
    esac
    echo ""
  done
  set -e
}

main() {
  local cmd="${1:-}"
  [[ $# -eq 0 ]] || shift
  case "$cmd" in
    "")
      [[ "${NONINTERACTIVE:-}" != "1" ]] || { usage >&2; exit 1; }
      interactive_main
      ;;
    install) run_official install "$@" ;;
    update|upgrade) run_official upgrade "$@" ;;
    start|stop|restart) as_root systemctl "$cmd" "${SERVICE_NAME}" ;;
    status) as_root systemctl status "${SERVICE_NAME}" --no-pager ;;
    logs|log)
      if [[ $# -eq 0 ]]; then
        as_root journalctl -u "${SERVICE_NAME}" -f
      else
        as_root journalctl -u "${SERVICE_NAME}" "$@"
      fi
      ;;
    uninstall|remove) cmd_uninstall "$@" ;;
    help|-h|--help) usage ;;
    *) die "未知命令: ${cmd}（使用 --help 查看）" ;;
  esac
}

main "$@"
