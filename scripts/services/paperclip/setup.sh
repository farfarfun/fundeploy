#!/usr/bin/env bash
# Paperclip (https://github.com/paperclipai/paperclip) 本机服务。
# CLI 安装、升级和后台服务均交给 Paperclip 官方命令管理。

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ -f "${SCRIPT_DIR}/../lib/fundeploy-common.sh" ]]; then
  # shellcheck source=../lib/fundeploy-common.sh
  source "${SCRIPT_DIR}/../lib/fundeploy-common.sh"
elif [[ -f "${SCRIPT_DIR}/../../lib/fundeploy-common.sh" ]]; then
  # shellcheck source=../../lib/fundeploy-common.sh
  source "${SCRIPT_DIR}/../../lib/fundeploy-common.sh"
else
  echo "错误: 找不到 lib/fundeploy-common.sh（已检查 ${SCRIPT_DIR}/../lib 与 ${SCRIPT_DIR}/../../lib）" >&2
  exit 1
fi

PAPERCLIP_HOME="${PAPERCLIP_HOME:-${HOME}/.paperclip}"
PAPERCLIP_INSTANCE_ID="${PAPERCLIP_INSTANCE_ID:-default}"
PAPERCLIP_NPM_REGISTRY="${PAPERCLIP_NPM_REGISTRY:-https://registry.npmjs.org}"
PAPERCLIP_NODE_MIN_VERSION="${PAPERCLIP_NODE_MIN_VERSION:-24.11.0}"
PAPERCLIP_NODE_MIN_MAJOR="${PAPERCLIP_NODE_MIN_MAJOR:-24}"

usage() {
  cat <<USAGE
用法: ./setup.sh [command [args...]]

命令:
  install          官方 managed install（正式版）
  install-canary   官方 managed install（开发版）
  install-prod     install 的别名
  update [canary|prod]  官方升级、备份、迁移及服务重启（默认 canary）
  onboard          首次配置；NONINTERACTIVE=1 时默认追加 --yes
  plugin list                  列出 awesome-paperclip 收录的插件
  plugin install [npm-package] 安装插件；不指定包名时从清单选择
  plugin installed             列出当前已安装的插件
  start             启动官方后台服务
  run               使用 paperclipai run 前台启动
  stop              停止官方后台服务
  restart           热重启官方后台服务
  status            查看官方 supervisor 与健康状态
  logs [options]    查看官方服务日志，例如 logs -f
  uninstall         卸载官方服务和 CLI，保留 ${PAPERCLIP_HOME} 数据

说明:
  - Linux 使用 systemd --user，macOS 使用 LaunchAgent。
  - 配置由 paperclipai onboard / configure 管理。
  - PAPERCLIP_INSTANCE_ID 默认为 ${PAPERCLIP_INSTANCE_ID}。
USAGE
}

die() { echo "错误: $*" >&2; exit 1; }

node_version_meets() {
  local min="$1"
  command -v node >/dev/null 2>&1 || return 1
  node -e '
    const min = process.argv[1].split(".").map(Number);
    const cur = process.versions.node.split(".").map(Number);
    for (let i = 0; i < 3; i++) {
      if ((cur[i] || 0) > (min[i] || 0)) process.exit(0);
      if ((cur[i] || 0) < (min[i] || 0)) process.exit(1);
    }
  ' "$min" 2>/dev/null
}

paperclip_ensure_node_version() {
  node_version_meets "${PAPERCLIP_NODE_MIN_VERSION}" && return 0

  local nvm_dir="${NVM_DIR:-${HOME}/.nvm}"
  [[ -s "${nvm_dir}/nvm.sh" ]] || return 0
  # shellcheck source=/dev/null
  source "${nvm_dir}/nvm.sh" >/dev/null 2>&1 || return 0
  command -v nvm >/dev/null 2>&1 || return 0
  nvm use "${PAPERCLIP_NODE_MIN_MAJOR}" >/dev/null 2>&1 || true
}

require_node() {
  paperclip_ensure_node_version
  command -v node >/dev/null 2>&1 || die "需要 Node.js ${PAPERCLIP_NODE_MIN_VERSION}+（https://nodejs.org/）"
  node_version_meets "${PAPERCLIP_NODE_MIN_VERSION}" || die "需要 Node.js ${PAPERCLIP_NODE_MIN_VERSION}+，当前: $(node --version)。可执行: nvm install ${PAPERCLIP_NODE_MIN_MAJOR}"
}

paperclip_executable() {
  if [[ -x "${HOME}/.local/bin/paperclipai" ]]; then
    printf '%s\n' "${HOME}/.local/bin/paperclipai"
  else
    command -v paperclipai 2>/dev/null || return 1
  fi
}

paperclip_cli() {
  require_node
  local executable
  executable="$(paperclip_executable)" || die "未找到 paperclipai，请先执行: $0 install"
  PAPERCLIP_HOME="${PAPERCLIP_HOME}" PAPERCLIP_INSTANCE_ID="${PAPERCLIP_INSTANCE_ID}" "$executable" "$@"
}

cmd_install() {
  require_node
  command -v npx >/dev/null 2>&1 || die "未找到 npx（Node.js 自带）"
  PAPERCLIP_HOME="${PAPERCLIP_HOME}" npx --yes --registry "${PAPERCLIP_NPM_REGISTRY}" paperclipai@latest install --yes "$@"
  paperclip_cli --version
}

cmd_update() {
  local channel="${1:-canary}" option
  case "$channel" in
    canary) option=--canary ;;
    prod) option=--latest ;;
    *) die "未知更新渠道: ${channel}（支持 canary / prod）" ;;
  esac

  paperclip_cli update "$option"
}

cmd_onboard() {
  if [[ "${NONINTERACTIVE:-}" == "1" ]]; then
    paperclip_cli onboard --yes "$@"
  else
    paperclip_cli onboard "$@"
  fi
}

paperclip_plugin_catalog_records() {
  cat <<'CATALOG'
Agent Pixels|@agent-pixels/paperclip-plugin|https://github.com/gcampton/Agent-Pixels|Pixel Agents for Paperclip with custom behaviors, models, rooms, and security cam access.
obsidian-paperclip||https://github.com/istib/obsidian-paperclip|Obsidian integration for browsing, commenting on, and assigning Paperclip issues.
paperclip-aperture|@tomismeta/paperclip-aperture|https://github.com/tomismeta/paperclip-aperture|Alternative Focus view that ranks approvals, issue activity, and human-facing events.
paperclip-live-analytics-plugin|@agent-analytics/paperclip-live-analytics-plugin|https://github.com/Agent-Analytics/paperclip-live-analytics-plugin|Live visitor map, dashboard widget, and Agent Analytics settings page.
paperclip-plugin-acp|paperclip-plugin-acp|https://github.com/mvanhorn/paperclip-plugin-acp|ACP runtime for Claude Code, Codex, and Gemini CLI from chat platforms.
paperclip-plugin-avp|paperclip-plugin-avp|https://github.com/creatorrmode-lead/paperclip-plugin-avp|Trust and reputation layer using Agent Veil Protocol.
paperclip-plugin-chat|@paperclipai/plugin-chat|https://github.com/webprismdevin/paperclip-plugin-chat|Interactive AI chat copilot for tasks, agents, and workspaces.
paperclip-plugin-company-wizard|@yesterday-ai/paperclip-plugin-company-wizard|https://github.com/yesterday-ai/paperclip-plugin-company-wizard|AI-powered company setup assistant with presets.
paperclip-plugin-discord|paperclip-plugin-discord|https://github.com/mvanhorn/paperclip-plugin-discord|Bidirectional Discord integration.
paperclip-plugin-github-issues|paperclip-plugin-github-issues|https://github.com/mvanhorn/paperclip-plugin-github-issues|Bidirectional GitHub Issues sync.
paperclip-plugin-linear|@oldharlem/paperclip-plugin-linear|https://github.com/Oldharlem/paperclip-linear-plugin|Bidirectional Linear sync with webhooks and an agent tool.
paperclip-plugin-slack|paperclip-plugin-slack|https://github.com/mvanhorn/paperclip-plugin-slack|Slack notifications for issue lifecycle events.
paperclip-plugin-telegram|paperclip-plugin-telegram|https://github.com/mvanhorn/paperclip-plugin-telegram|Telegram notifications for issue lifecycle events.
paperclip-plugin-writbase|paperclip-plugin-writbase|https://github.com/Writbase/paperclip-plugin-writbase|Bidirectional sync between Paperclip issues and WritBase tasks.
paperclip-plugin-hindsight|paperclip-plugin-hindsight|https://github.com/vectorize-io/hindsight/tree/main/hindsight-integrations/paperclip-plugin|Persistent long-term memory for Paperclip agents.
CATALOG
}

cmd_plugin_list() {
  local name package url description
  trap '' PIPE
  while IFS='|' read -r name package url description; do
    printf '%s%s\n  %s\n  %s\n' "$name" "${package:+ (${package})}" "$description" "$url" 2>/dev/null || return 0
  done < <(paperclip_plugin_catalog_records)
}

cmd_plugin_install() {
  local package="${1:-}" pick name url description i
  if [[ -n "$package" ]]; then
    shift
    paperclip_cli plugin install "$package" "$@"
    return
  fi

  local -a labels=() packages=() urls=()
  while IFS='|' read -r name package url description; do
    [[ -n "$package" ]] || continue
    labels+=("${name} - ${description}")
    packages+=("${package}")
    urls+=("${url}")
  done < <(paperclip_plugin_catalog_records)

  pick="$(fundeploy_ui_choose "Paperclip / 从 awesome-paperclip 选择插件" "${labels[@]}")" || return 0
  for i in "${!labels[@]}"; do
    [[ "${labels[$i]}" == "$pick" ]] || continue
    echo "==> 安装 Paperclip 插件 ${packages[$i]}（来源: ${urls[$i]}）" >&2
    paperclip_cli plugin install "${packages[$i]}"
    return
  done
  die "无法识别所选插件"
}

cmd_plugin() {
  local action="${1:-install}"
  [[ $# -eq 0 ]] || shift
  case "$action" in
    list) cmd_plugin_list ;;
    install) cmd_plugin_install "$@" ;;
    installed) paperclip_cli plugin list "$@" ;;
    *) die "未知插件命令: ${action}（支持 list / install / installed）" ;;
  esac
}

cmd_run() {
  require_node
  local executable
  executable="$(paperclip_executable)" || die "未找到 paperclipai，请先执行: $0 install"
  exec env PAPERCLIP_HOME="${PAPERCLIP_HOME}" PAPERCLIP_INSTANCE_ID="${PAPERCLIP_INSTANCE_ID}" "$executable" run "$@"
}

cmd_uninstall() {
  echo "Paperclip 数据目录 ${PAPERCLIP_HOME} 不会删除。" >&2
  fundeploy_confirm_destructive "确认卸载 Paperclip 官方服务和 CLI？" PAPERCLIP_UNINSTALL_YES || return 1

  paperclip_cli service uninstall
  paperclip_cli uninstall
  echo "已卸载 Paperclip 服务和 CLI；数据保留在 ${PAPERCLIP_HOME}。"
}

dispatch() {
  local cmd="${1:-}"
  shift || true
  case "$cmd" in
    install | install-prod) cmd_install ;;
    install-canary) cmd_install --canary ;;
    update) cmd_update "$@" ;;
    onboard) cmd_onboard "$@" ;;
    plugin) cmd_plugin "$@" ;;
    run) cmd_run "$@" ;;
    start | stop | restart | status | logs) paperclip_cli service "$cmd" "$@" ;;
    uninstall) cmd_uninstall ;;
    help | -h | --help) usage ;;
    *) echo "未知命令: ${cmd}" >&2; usage >&2; exit 2 ;;
  esac
}

interactive_main() {
  declare -F fundeploy_ui_apply_theme >/dev/null 2>&1 && fundeploy_ui_apply_theme
  fundeploy_ui_banner "Paperclip 官方服务" "PAPERCLIP_HOME=${PAPERCLIP_HOME}" "instance=${PAPERCLIP_INSTANCE_ID}"
  echo ""
  set +e
  while true; do
    local pick
    pick="$(fundeploy_ui_choose "fundeploy / service / paperclip / 选择动作" \
      "install-canary" "install-prod" "update" "onboard" "plugin install" "start" "run" "stop" "restart" "status" "logs" "uninstall" "help" "quit")" || break
    [[ -n "$pick" ]] || break
    case "$pick" in
      quit) break ;;
      help) usage; continue ;;
      "plugin install") ( dispatch plugin install ) ;;
      *) ( dispatch "$pick" ) ;;
    esac
    echo ""
  done
  set -e
}

main() {
  if [[ $# -eq 0 ]]; then
    _fundeploy_ensure_gum || exit 1
    interactive_main
  else
    dispatch "$@"
  fi
}

main "$@"
