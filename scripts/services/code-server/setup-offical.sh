#!/usr/bin/env bash
# code-server 官方模式：调用官方安装脚本，并使用官方建议的系统服务。

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ -f "${SCRIPT_DIR}/../lib/fundeploy-common.sh" ]]; then
  # shellcheck source=../lib/fundeploy-common.sh
  source "${SCRIPT_DIR}/../lib/fundeploy-common.sh"
elif [[ -f "${SCRIPT_DIR}/../../lib/fundeploy-common.sh" ]]; then
  # shellcheck source=../../lib/fundeploy-common.sh
  source "${SCRIPT_DIR}/../../lib/fundeploy-common.sh"
else
  echo "错误: 找不到 lib/fundeploy-common.sh" >&2
  exit 1
fi

OFFICIAL_INSTALLER_URL="https://code-server.dev/install.sh"
CODE_SERVER_OFFICIAL_USER="${CODE_SERVER_OFFICIAL_USER:-${SUDO_USER:-$(id -un)}}"
CODE_SERVER_OFFICIAL_PREFIX="${CODE_SERVER_OFFICIAL_PREFIX:-${HOME}/.local}"
SERVICE_UNIT="code-server@${CODE_SERVER_OFFICIAL_USER}"

die() { echo "错误: $*" >&2; exit 1; }

usage() {
  cat <<EOF
用法: setup-offical.sh [command] [选项]

命令:
  install [官方选项]      执行 https://code-server.dev/install.sh
  update [官方选项]       再次执行官方脚本以升级
  start / stop / restart  Linux 使用 ${SERVICE_UNIT}；macOS 使用 brew services
  status                  查看官方服务状态
  logs [journalctl 选项]  查看 Linux systemd 日志（默认持续跟踪）
  uninstall [-y] [--purge] 按官方安装结果卸载；--purge 同时删除用户配置和数据

官方脚本选项可直接透传，例如 --version 4.112.0、--method standalone、--dry-run。
默认用户: ${CODE_SERVER_OFFICIAL_USER}

示例:
  fundeploy service code-server official install
  fundeploy service code-server official start
  fundeploy service code-server official update
  fundeploy service code-server official uninstall -y
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
  echo "==> 执行 code-server 官方安装脚本" >&2
  curl -fsSL "${OFFICIAL_INSTALLER_URL}" | sh -s -- "$@"
}

service_action() {
  local action="$1"
  shift
  case "$(uname -s)" in
    Darwin)
      command -v brew >/dev/null 2>&1 || die "官方服务管理需要 Homebrew；当前安装可能是 standalone"
      case "$action" in
        start) brew services start code-server ;;
        stop) brew services stop code-server ;;
        restart) brew services restart code-server ;;
        status) brew services info code-server ;;
        logs) die "macOS Homebrew 服务没有统一 journal；请使用 brew services info code-server" ;;
      esac
      ;;
    Linux)
      command -v systemctl >/dev/null 2>&1 || die "当前系统没有 systemd；请使用 manual 模式"
      as_root systemctl cat "${SERVICE_UNIT}" >/dev/null 2>&1 \
        || die "未找到 ${SERVICE_UNIT}；官方脚本可能使用了 standalone/npm，请改用 manual 模式管理服务"
      case "$action" in
        start) as_root systemctl enable --now "${SERVICE_UNIT}" ;;
        stop) as_root systemctl stop "${SERVICE_UNIT}" ;;
        restart) as_root systemctl restart "${SERVICE_UNIT}" ;;
        status) as_root systemctl status "${SERVICE_UNIT}" --no-pager ;;
        logs)
          if [[ $# -eq 0 ]]; then
            as_root journalctl -u "${SERVICE_UNIT}" -f
          else
            as_root journalctl -u "${SERVICE_UNIT}" "$@"
          fi
          ;;
      esac
      ;;
    *) die "官方服务管理暂不支持: $(uname -s)" ;;
  esac
}

disable_service() {
  case "$(uname -s)" in
    Darwin) command -v brew >/dev/null 2>&1 && brew services stop code-server || true ;;
    Linux) command -v systemctl >/dev/null 2>&1 && as_root systemctl disable --now "${SERVICE_UNIT}" || true ;;
  esac
}

cmd_uninstall() {
  local arg yes=0 purge=0 removed=0 npm_root=""
  local -a standalone_dirs=()
  for arg in "$@"; do
    case "$arg" in
      -y|--yes) yes=1 ;;
      --purge) purge=1 ;;
      *) die "未知卸载选项: ${arg}" ;;
    esac
  done
  [[ "${CODE_SERVER_OFFICIAL_UNINSTALL_YES:-0}" != "1" ]] || yes=1
  if [[ "$yes" == "0" ]]; then
    [[ -t 0 ]] || die "非交互卸载请加 -y"
    fundeploy_ui_confirm "确认卸载官方模式的 code-server？" || return 0
  fi

  disable_service
  if command -v dpkg-query >/dev/null 2>&1 && dpkg-query -W -f='${Status}' code-server 2>/dev/null | grep -q 'ok installed'; then
    as_root apt-get remove -y code-server
    removed=1
  elif command -v rpm >/dev/null 2>&1 && rpm -q code-server >/dev/null 2>&1; then
    as_root rpm -e code-server
    removed=1
  elif command -v pacman >/dev/null 2>&1 && pacman -Q code-server >/dev/null 2>&1; then
    as_root pacman -R --noconfirm code-server
    removed=1
  elif command -v brew >/dev/null 2>&1 && brew list --formula code-server >/dev/null 2>&1; then
    brew uninstall code-server
    removed=1
  elif [[ -d "${CODE_SERVER_OFFICIAL_PREFIX}/lib" ]] && find "${CODE_SERVER_OFFICIAL_PREFIX}/lib" -mindepth 1 -maxdepth 1 -type d -name 'code-server-*' -print -quit | grep -q .; then
    while IFS= read -r arg; do
      standalone_dirs+=("$arg")
    done < <(find "${CODE_SERVER_OFFICIAL_PREFIX}/lib" -mindepth 1 -maxdepth 1 -type d -name 'code-server-*' -print)
    rm -rf -- "${standalone_dirs[@]}"
    rm -f -- "${CODE_SERVER_OFFICIAL_PREFIX}/bin/code-server"
    removed=1
  elif command -v npm >/dev/null 2>&1; then
    npm_root="$(npm root -g 2>/dev/null || true)"
    if [[ -n "$npm_root" && -d "${npm_root}/code-server" ]]; then
      npm uninstall --global code-server
      removed=1
    fi
  fi
  [[ "$removed" == "1" ]] || die "未识别到官方安装；请按原安装方式卸载"

  if [[ "$purge" == "1" ]]; then
    rm -rf -- "${HOME}/.local/share/code-server" "${HOME}/.config/code-server"
  fi
  echo "已卸载官方模式的 code-server。"
}

interactive_main() {
  _fundeploy_ensure_gum || exit 1
  fundeploy_ui_banner "fundeploy / service / code-server / official" "官方脚本 · 系统包管理器 · ${SERVICE_UNIT}" >&2
  set +e
  while true; do
    local pick
    pick="$(fundeploy_ui_choose "fundeploy / service / code-server / official / 选择动作" \
      "install           安装" \
      "update            更新" \
      "start             启动并设为开机启动" \
      "stop              停止" \
      "restart           重启" \
      "status            状态" \
      "logs              日志" \
      "uninstall         卸载" \
      "help              命令帮助" \
      "quit              返回")" || break
    pick="${pick%% *}"
    case "$pick" in
      install|update) run_official ;;
      start|stop|restart|status|logs) service_action "$pick" ;;
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
    install|update|upgrade) run_official "$@" ;;
    start|stop|restart|status) service_action "$cmd" ;;
    logs|log) service_action logs "$@" ;;
    uninstall|remove) cmd_uninstall "$@" ;;
    help|-h|--help) usage ;;
    *) die "未知命令: ${cmd}（使用 --help 查看）" ;;
  esac
}

main "$@"
