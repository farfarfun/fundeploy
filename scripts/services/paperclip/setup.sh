#!/usr/bin/env bash
# Paperclip（https://github.com/paperclipai/paperclip）本机服务：
# 通过 pnpm 全局安装 paperclipai CLI，无源码克隆。
#
# 用法：
#   ./setup.sh              # gum 菜单
#   ./setup.sh install      # pnpm add -g paperclipai@latest
#   ./setup.sh update       # 升级后自动迁移数据库，并保持原运行状态
#   ./setup.sh onboard      # 首次配置（默认 NONINTERACTIVE=1 时自动加 --yes）
#   ./setup.sh plugin install # 从 awesome-paperclip 选择并安装插件
#   ./setup.sh start        # 后台启动（paperclipai run）
#   ./setup.sh run          # 前台启动（终端附着；不写 PID）
#   ./setup.sh stop / restart / status / uninstall
#
# 环境变量：
#   PAPERCLIP_SERVICE_HOME   本脚本管理根目录（默认 ~/opt/paperclip）
#   PAPERCLIP_HOME           上游数据目录（默认 ~/.paperclip，遵循官方默认）
#   PAPERCLIP_PORT           Paperclip 监听端口（默认 8804；启动时 export PORT 同值）
#   PAPERCLIP_HOST           Paperclip 监听地址（默认 0.0.0.0）
#   PAPERCLIP_DATABASE_URL   数据库连接串（默认 postgresql://paperclip:paperclip@localhost:5432/paperclip）
#   PAPERCLIP_AUTH_DISABLE_SIGN_UP  是否禁止注册（默认 true）
#   PAPERCLIP_NPM_REGISTRY   用于版本对照的公共 npm registry（默认 https://registry.npmjs.org）
#   PAPERCLIP_NPM_PACKAGE    npm 包规格（默认 paperclipai@latest）
#   PAPERCLIP_START_HEALTH_TIMEOUT_SEC  start 时等待 /api/health 健康检查最长秒数（默认 60）
#   PAPERCLIP_SKIP_START_HEALTH_CHECK=1 跳过 HTTP 健康检查
#   NONINTERACTIVE=1         跳过 gum 确认；onboard 默认加 --yes
#   PAPERCLIP_UNINSTALL_YES=1 非 TTY 卸载确认

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

PAPERCLIP_SERVICE_HOME="${PAPERCLIP_SERVICE_HOME:-${HOME}/opt/paperclip}"
PAPERCLIP_HOME="${PAPERCLIP_HOME:-${HOME}/.paperclip}"
PAPERCLIP_PORT="${PAPERCLIP_PORT:-8804}"
PAPERCLIP_HOST="${PAPERCLIP_HOST:-0.0.0.0}"
PAPERCLIP_DATABASE_URL="${PAPERCLIP_DATABASE_URL:-postgresql://paperclip:paperclip@localhost:5432/paperclip}"
PAPERCLIP_AUTH_DISABLE_SIGN_UP="${PAPERCLIP_AUTH_DISABLE_SIGN_UP:-true}"
PAPERCLIP_NPM_REGISTRY="${PAPERCLIP_NPM_REGISTRY:-https://registry.npmjs.org}"
# npm 的 paperclip 是另一个 UI 工具；Paperclip AI 官方包名是 paperclipai。
PAPERCLIP_NPM_PACKAGE="${PAPERCLIP_NPM_PACKAGE:-paperclipai@latest}"

PAPERCLIP_RUN_DIR="${PAPERCLIP_SERVICE_HOME}/run"
PAPERCLIP_LOG_DIR="${PAPERCLIP_SERVICE_HOME}/log"
PID_FILE="${PAPERCLIP_RUN_DIR}/paperclip.pid"
LOG_FILE="${PAPERCLIP_LOG_DIR}/paperclip.run.log"
HOST_VERSION_FIX_PID_FILE="${PAPERCLIP_RUN_DIR}/paperclip-host-version-fix.pid"
HOST_VERSION_LOADER_FILE="${PAPERCLIP_SERVICE_HOME}/paperclip-host-version-loader.mjs"
HOST_VERSION_REGISTER_FILE="${PAPERCLIP_SERVICE_HOME}/paperclip-host-version-register.mjs"

usage() {
  cat <<USAGE
用法: ./setup.sh [command [args...]]

  无参数：gum 菜单。

命令:
  install     pnpm 全局安装 ${PAPERCLIP_NPM_PACKAGE}
  update      全局升级后自动迁移数据库，并保持原运行状态
  onboard     首次配置；NONINTERACTIVE=1 时默认追加 --yes
  plugin list                  列出 awesome-paperclip 收录的插件
  plugin install [npm-package] 安装插件；不指定包名时从清单选择
  plugin installed             列出当前已安装的插件
  start       后台启动: http://${PAPERCLIP_HOST}:${PAPERCLIP_PORT}
  run         前台启动: 同 start 的运行环境，但终端附着、不写 PID
  stop        停止后台进程
  restart     stop 后 start
  status      查看 PID、日志位置与 HTTP 健康检查（/api/health）
  uninstall   卸载全局 CLI 并删除 ${PAPERCLIP_SERVICE_HOME}（不删除 PAPERCLIP_HOME 数据目录）

说明:
  - 安装后直接使用全局命令：paperclipai onboard --yes / run
  - 默认数据目录为 ${PAPERCLIP_HOME}（官方默认 ~/.paperclip）
  - 默认监听地址: http://${PAPERCLIP_HOST}:${PAPERCLIP_PORT}
  - 默认导出 DATABASE_URL=${PAPERCLIP_DATABASE_URL}
  - 默认导出 PAPERCLIP_AUTH_DISABLE_SIGN_UP=${PAPERCLIP_AUTH_DISABLE_SIGN_UP}
  - 健康检查: /api/health
  - 若 ~/.npmrc 的 registry 指向私有源，会同时查询它和公共源并安装较新版本。
USAGE
}

die() { echo "错误: $*" >&2; exit 1; }

ensure_dirs() {
  mkdir -p "${PAPERCLIP_RUN_DIR}" "${PAPERCLIP_LOG_DIR}"
}

paperclip_prepare_host_version_fix() {
  # ponytail: remove this loader after Paperclip passes serverVersion to createApp upstream.
  cat >"${HOST_VERSION_LOADER_FILE}" <<'EOF'
const needle = 'hostVersion: opts.hostVersion ?? "0.0.0"';
const replacement = 'hostVersion: opts.hostVersion ?? process.env.PAPERCLIP_BUILD_VERSION ?? "0.0.0"';

export async function load(url, context, nextLoad) {
  const loaded = await nextLoad(url, context);
  if (!url.endsWith('/@paperclipai/server/dist/app.js') || loaded.source == null) return loaded;
  const source = typeof loaded.source === 'string'
    ? loaded.source
    : Buffer.from(loaded.source).toString('utf8');
  return source.includes(needle) ? { ...loaded, source: source.replace(needle, replacement) } : loaded;
}
EOF
  cat >"${HOST_VERSION_REGISTER_FILE}" <<'EOF'
import { register } from 'node:module';
register('./paperclip-host-version-loader.mjs', import.meta.url);
EOF
  PAPERCLIP_NODE_REGISTER_URL="$(node -e 'process.stdout.write(require("node:url").pathToFileURL(process.argv[1]).href)' "${HOST_VERSION_REGISTER_FILE}")"
}

process_alive() {
  local pid="$1"
  kill -0 "$pid" 2>/dev/null
}

port_listener_pid() {
  _fundeploy_listener_pid_for_port "$1"
}

read_pid() {
  if [[ ! -f "$PID_FILE" ]]; then
    echo ""
    return
  fi
  tr -d '[:space:]' <"$PID_FILE" || true
}

require_node() {
  command -v node >/dev/null 2>&1 || die "需要 Node.js 20+（https://nodejs.org/）"
  local major
  major="$(node -p 'parseInt(process.versions.node.split(".")[0], 10)')"
  if (( major < 20 )); then
    die "需要 Node.js 20+，当前: $(node --version)"
  fi
}

paperclip_export_runtime_env() {
  local version import_option
  export HOST="${PAPERCLIP_HOST}"
  export PORT="${PAPERCLIP_PORT}"
  export DATABASE_URL="${PAPERCLIP_DATABASE_URL}"
  export PAPERCLIP_AUTH_DISABLE_SIGN_UP="${PAPERCLIP_AUTH_DISABLE_SIGN_UP}"
  export PAPERCLIP_HOME
  if [[ -n "${PAPERCLIP_NODE_REGISTER_URL:-}" ]]; then
    if [[ -z "${PAPERCLIP_BUILD_VERSION:-}" ]]; then
      version="$(paperclipai --version 2>/dev/null | head -1 || true)"
      version="${version%% *}"
      [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([+-][0-9A-Za-z.-]+)?$ ]] && export PAPERCLIP_BUILD_VERSION="$version"
    fi
    import_option="--import=${PAPERCLIP_NODE_REGISTER_URL}"
    case " ${NODE_OPTIONS:-} " in
      *" ${import_option} "*) ;;
      *) export NODE_OPTIONS="${NODE_OPTIONS:+${NODE_OPTIONS} }${import_option}" ;;
    esac
  fi
}

paperclip_prompt_sign_up_policy() {
  if command -v gum >/dev/null 2>&1; then
    local pick
    pick="$(gum choose --header "选择注册策略" "不允许注册（默认）" "允许注册")" || return 1
    case "$pick" in
      "允许注册") PAPERCLIP_AUTH_DISABLE_SIGN_UP="false" ;;
      *) PAPERCLIP_AUTH_DISABLE_SIGN_UP="true" ;;
    esac
    return 0
  fi

  echo "请选择注册策略：" >&2
  echo "  1) 不允许注册（默认）" >&2
  echo "  2) 允许注册" >&2
  local sel
  read -r -p "输入 1-2 [1]: " sel
  case "${sel:-1}" in
    2) PAPERCLIP_AUTH_DISABLE_SIGN_UP="false" ;;
    *) PAPERCLIP_AUTH_DISABLE_SIGN_UP="true" ;;
  esac
}

require_paperclip_cli() {
  command -v paperclipai >/dev/null 2>&1 || die "未找到全局 paperclipai，请先执行: $0 install"
}

paperclip_cli() {
  require_paperclip_cli
  paperclip_export_runtime_env
  paperclipai "$@"
}

paperclip_plugin_catalog_records() {
  # awesome-paperclip Plugins 清单快照；随 fundeploy 版本更新，不在用户运行时联网解析。
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
  while IFS='|' read -r name package url description; do
    printf '%s%s\n  %s\n  %s\n' "$name" "${package:+ (${package})}" "$description" "$url"
  done < <(paperclip_plugin_catalog_records)
}

cmd_plugin_install() {
  local package="${1:-}" pick name url description i
  require_node
  if [[ -n "$package" ]]; then
    shift
    paperclip_ensure_running_host_version_fix
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
    package="${packages[$i]}"
    echo "==> 安装 Paperclip 插件 ${package}（来源: ${urls[$i]}）" >&2
    paperclip_ensure_running_host_version_fix
    paperclip_cli plugin install "$package"
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

paperclip_server_pid() {
  port_listener_pid "${PAPERCLIP_PORT}"
}

paperclip_ensure_running_host_version_fix() {
  local pid server_pid fix_pid=""
  server_pid="$(paperclip_server_pid)"
  [[ -n "$server_pid" ]] || return 0
  [[ -f "$HOST_VERSION_FIX_PID_FILE" ]] && fix_pid="$(tr -d '[:space:]' <"$HOST_VERSION_FIX_PID_FILE" || true)"
  [[ -n "$fix_pid" ]] && process_alive "$fix_pid" && return 0

  pid="$(read_pid)"
  if [[ -n "$pid" ]] && process_alive "$pid"; then
    echo "==> 重启 Paperclip，修复插件主机版本识别…" >&2
    cmd_restart
    return
  fi
  die "当前 Paperclip 不是由 fundeploy 后台管理，请先用 fundeploy service paperclip restart 重启后再安装插件"
}

paperclip_curl_health_http_code() {
  curl -sS -o /dev/null -w '%{http_code}' -m 3 "http://127.0.0.1:${PAPERCLIP_PORT}/api/health" 2>/dev/null || echo "000"
}

paperclip_wait_ready() {
  local pid="$1"
  local max="${PAPERCLIP_START_HEALTH_TIMEOUT_SEC:-60}"
  local i=0
  while (( i < max )); do
    if ! process_alive "$pid"; then
      return 2
    fi
    if command -v curl >/dev/null 2>&1; then
      local code
      code="$(paperclip_curl_health_http_code)"
      if [[ "$code" == "200" ]]; then
        return 0
      fi
    elif (( i >= 8 )); then
      echo "[WARN] 未安装 curl，无法校验健康检查；进程仍存活则视为启动成功。" >&2
      return 0
    fi
    sleep 1
    i=$((i + 1))
  done
  return 1
}

paperclip_start_failure_hints() {
  echo "---- 最近日志（${LOG_FILE}）----" >&2
  tail -n 80 "${LOG_FILE}" 2>/dev/null >&2 || true
}

paperclip_select_npm_package() {
  PAPERCLIP_INSTALL_REGISTRY="${PAPERCLIP_NPM_REGISTRY}"
  PAPERCLIP_INSTALL_PACKAGE="${PAPERCLIP_NPM_PACKAGE}"

  local private_registry
  private_registry="$(pnpm config get registry 2>/dev/null || true)"
  [[ -n "${private_registry}" && "${private_registry}" != "null" ]] || return 0
  [[ "${private_registry%/}" != "${PAPERCLIP_NPM_REGISTRY%/}" ]] || return 0

  local package_name="${PAPERCLIP_NPM_PACKAGE}"
  if [[ "${package_name}" == @*/*@* || "${package_name}" != @* && "${package_name}" == *@* ]]; then
    package_name="${package_name%@*}"
  fi

  local public_version private_version
  public_version="$(pnpm view "${PAPERCLIP_NPM_PACKAGE}" version --registry "${PAPERCLIP_NPM_REGISTRY}" 2>/dev/null || true)"
  private_version="$(pnpm view "${PAPERCLIP_NPM_PACKAGE}" version --registry "${private_registry}" 2>/dev/null || true)"
  [[ -n "${public_version}" ]] || echo "[WARN] 公共 npm 源查询失败: ${PAPERCLIP_NPM_REGISTRY}" >&2
  [[ -n "${private_version}" ]] || echo "[WARN] .npmrc 私有源查询失败: ${private_registry}" >&2
  [[ -n "${public_version}${private_version}" ]] || die "公共源和私有源均无法查询 ${PAPERCLIP_NPM_PACKAGE}"

  if [[ -n "${private_version}" ]] && { [[ -z "${public_version}" ]] || pnpm view "${package_name}@>${public_version} <=${private_version}" version --registry "${private_registry}" >/dev/null 2>&1; }; then
    PAPERCLIP_INSTALL_REGISTRY="${private_registry}"
    PAPERCLIP_INSTALL_PACKAGE="${package_name}@${private_version}"
  else
    PAPERCLIP_INSTALL_PACKAGE="${package_name}@${public_version}"
  fi

  echo "==> Paperclip 版本：公共源 ${public_version:--}，私有源 ${private_version:--}；选择 ${PAPERCLIP_INSTALL_PACKAGE}" >&2
}

cmd_install() {
  require_node
  ensure_dirs
  command -v pnpm >/dev/null 2>&1 || die "未找到 pnpm"

  local managed_entrypoint="${PAPERCLIP_HOME}/cli/current/node_modules/paperclipai/dist/index.js"
  if [[ -f "${PAPERCLIP_HOME}/cli/.managed-install" ]]; then
    [[ -f "${managed_entrypoint}" ]] || die "Paperclip managed install 已损坏，请重新执行官方 paperclipai install"
    echo "==> 检测到 Paperclip managed install，迁移到 pnpm 全局安装…" >&2
    paperclip_export_runtime_env
    node "${managed_entrypoint}" uninstall
    command -v npm >/dev/null 2>&1 && npm uninstall -g paperclipai
  fi

  paperclip_select_npm_package
  echo "==> pnpm 全局安装 Paperclip CLI（${PAPERCLIP_INSTALL_PACKAGE}）…" >&2
  pnpm add -g --registry "${PAPERCLIP_INSTALL_REGISTRY}" "${PAPERCLIP_INSTALL_PACKAGE}"
  paperclip_cli --version
  echo "安装完成。首次使用可执行: $0 onboard"
}

cmd_update() {
  local pid was_running=0
  pid="$(read_pid)"
  [[ -n "${pid}" ]] && process_alive "${pid}" && was_running=1
  cmd_install

  if (( was_running == 0 )) && [[ ! -f "${PAPERCLIP_HOME}/instances/default/config.json" && ! -f "${PAPERCLIP_HOME}/config.json" ]]; then
    echo "尚未完成 onboard，跳过数据库迁移。"
    return 0
  fi

  (( was_running == 0 )) || cmd_stop --yes || die "停止旧版 Paperclip 失败，已中止数据库迁移"
  echo "==> 启动新版 Paperclip 并应用数据库迁移…" >&2
  PAPERCLIP_MIGRATION_AUTO_APPLY=true PAPERCLIP_SKIP_START_HEALTH_CHECK=0 cmd_start
  if (( was_running == 0 )); then
    cmd_stop --yes || die "数据库迁移完成，但临时 Paperclip 进程停止失败"
    echo "更新及数据库迁移完成，服务保持停止。"
  else
    echo "更新及数据库迁移完成，服务已恢复运行。"
  fi
}

cmd_onboard() {
  require_node
  ensure_dirs
  if [[ "${NONINTERACTIVE:-}" == "1" ]]; then
    paperclip_cli onboard --yes "$@"
  else
    paperclip_cli onboard "$@"
  fi
}

cmd_start() {
  require_node
  require_paperclip_cli
  ensure_dirs
  local existing
  existing="$(read_pid)"
  if [[ -n "$existing" ]] && process_alive "$existing"; then
    echo "Paperclip 已在运行（PID ${existing}）。如需重启: $0 restart" >&2
    exit 1
  fi
  local server_pid
  server_pid="$(paperclip_server_pid)"
  if [[ -n "$server_pid" ]]; then
    die "端口 ${PAPERCLIP_PORT} 已被 PID ${server_pid} 占用，请先执行 $0 stop 或手动清理。"
  fi
  rm -f "$PID_FILE"
  echo "==> 启动 Paperclip（paperclipai run），日志: ${LOG_FILE}" >&2
  echo "    监听: http://${PAPERCLIP_HOST}:${PAPERCLIP_PORT}" >&2
  echo "    允许注册: $( [[ "${PAPERCLIP_AUTH_DISABLE_SIGN_UP}" == "true" ]] && printf '%s' '否' || printf '%s' '是' )" >&2
  paperclip_prepare_host_version_fix
  paperclip_export_runtime_env
  nohup paperclipai run >>"${LOG_FILE}" 2>&1 &
  local cpid=$!
  echo "$cpid" >"$PID_FILE"
  echo "$cpid" >"$HOST_VERSION_FIX_PID_FILE"
  sleep 1
  if ! process_alive "$cpid"; then
    rm -f "$PID_FILE" "$HOST_VERSION_FIX_PID_FILE"
    paperclip_start_failure_hints
    die "进程已退出，启动失败。若是首次使用，请先执行: $0 onboard"
  fi
  if [[ "${PAPERCLIP_SKIP_START_HEALTH_CHECK:-}" == "1" ]]; then
    echo "已启动 PID ${cpid}（已跳过 HTTP 健康检查）"
    return 0
  fi
  echo "==> 等待 http://127.0.0.1:${PAPERCLIP_PORT}/api/health 就绪（最长 ${PAPERCLIP_START_HEALTH_TIMEOUT_SEC:-60}s）…" >&2
  local wr=0
  paperclip_wait_ready "$cpid" || wr=$?
  if [[ "$wr" -eq 0 ]]; then
    echo "已启动 PID ${cpid}，健康检查通过。"
    return 0
  fi
  if [[ "$wr" -eq 2 ]]; then
    rm -f "$PID_FILE" "$HOST_VERSION_FIX_PID_FILE"
    paperclip_start_failure_hints
    die "进程已退出，启动失败。若是首次使用，请先执行: $0 onboard"
  fi
  echo "错误: 在 ${PAPERCLIP_START_HEALTH_TIMEOUT_SEC:-60}s 内未通过健康检查；进程可能仍在运行（PID ${cpid}）。" >&2
  paperclip_start_failure_hints
  die "启动校验失败。"
}

cmd_run() {
  require_node
  require_paperclip_cli
  ensure_dirs
  local existing
  existing="$(read_pid)"
  if [[ -n "$existing" ]] && process_alive "$existing"; then
    die "Paperclip 已在后台运行（PID ${existing}）。请先执行 $0 stop，再使用 run 前台调试。"
  fi
  echo "==> 前台启动 Paperclip（paperclipai run），按 Ctrl+C 结束；不写 PID。" >&2
  echo "    监听: http://${PAPERCLIP_HOST}:${PAPERCLIP_PORT}" >&2
  echo "    允许注册: $( [[ "${PAPERCLIP_AUTH_DISABLE_SIGN_UP}" == "true" ]] && printf '%s' '否' || printf '%s' '是' )" >&2
  paperclip_prepare_host_version_fix
  paperclip_export_runtime_env
  echo "$$" >"$HOST_VERSION_FIX_PID_FILE"
  exec paperclipai run
}

# cmd_stop [--yes]
#   --yes 跳过确认（供 restart / uninstall 调用，避免二次追问）。
#   用户拒绝时返回 1（而非 exit 0），否则 restart/uninstall 会被静默截断。
cmd_stop() {
  local assume_yes=""
  [[ "${1:-}" == "--yes" ]] && assume_yes=1
  local pid
  pid="$(read_pid)"
  if [[ -z "$assume_yes" ]] && [[ -n "$pid" ]] && fundeploy_interactive; then
    # fundeploy_ui_confirm 在缺 gum 时降级为 read y/N，无需 _fundeploy_ensure_gum 硬拉依赖。
    fundeploy_ui_confirm "停止 Paperclip（PID ${pid:--}）？" || return 1
  fi

  local server_pid
  server_pid="$(paperclip_server_pid)"

  if [[ -z "$pid" ]]; then
    echo "未找到 Paperclip PID 文件，视为主进程未启动。" >&2
    rm -f "$PID_FILE"
  elif ! process_alive "$pid"; then
    echo "Paperclip PID ${pid} 不存在，清理 PID 文件。"
    rm -f "$PID_FILE"
  else
    kill "$pid" 2>/dev/null || true
    local w=0
    while process_alive "$pid" && (( w < 30 )); do
      sleep 1
      w=$((w + 1))
    done
    if process_alive "$pid"; then
      kill -KILL "$pid" 2>/dev/null || true
    fi
    rm -f "$PID_FILE"
  fi

  if [[ -n "$server_pid" ]] && { [[ -z "$pid" ]] || [[ "$server_pid" != "$pid" ]]; }; then
    kill "$server_pid" 2>/dev/null || true
  fi
  rm -f "$HOST_VERSION_FIX_PID_FILE"
  echo "已停止。"
}

cmd_restart() {
  cmd_stop --yes || die "停止失败，已中止 restart（服务可能仍在运行）"
  cmd_start
}

cmd_status() {
  local pid
  pid="$(read_pid)"
  echo "PAPERCLIP_SERVICE_HOME=${PAPERCLIP_SERVICE_HOME}"
  echo "PAPERCLIP_HOME=${PAPERCLIP_HOME}"
  echo "PAPERCLIP_PORT=${PAPERCLIP_PORT}"
  echo "PAPERCLIP_HOST=${PAPERCLIP_HOST}"
  echo "PAPERCLIP_DATABASE_URL=${PAPERCLIP_DATABASE_URL}"
  echo "PAPERCLIP_AUTH_DISABLE_SIGN_UP=${PAPERCLIP_AUTH_DISABLE_SIGN_UP}"
  echo "PAPERCLIP_NPM_PACKAGE=${PAPERCLIP_NPM_PACKAGE}"
  echo "PAPERCLIP_NPM_REGISTRY=${PAPERCLIP_NPM_REGISTRY}"
  echo "LOG_FILE=${LOG_FILE}"
  local server_pid
  server_pid="$(paperclip_server_pid)"
  if [[ -n "$pid" ]] && process_alive "$pid"; then
    if [[ -n "$server_pid" && "$server_pid" != "$pid" ]]; then
      echo "状态: Paperclip 运行中 PID ${pid}（监听进程 PID ${server_pid}）"
    else
      echo "状态: Paperclip 运行中 PID ${pid}"
    fi
  elif [[ -n "$server_pid" ]]; then
    echo "状态: Paperclip 监听中（PID 文件缺失/失效，监听进程 PID ${server_pid}）"
  else
    echo "状态: Paperclip 未运行"
    rm -f "$PID_FILE"
  fi
  if command -v curl >/dev/null 2>&1; then
    echo ""
    echo "==> GET http://127.0.0.1:${PAPERCLIP_PORT}/api/health"
    curl -sS -m 3 "http://127.0.0.1:${PAPERCLIP_PORT}/api/health" || echo "（无法连接，可能未启动或端口不同）"
    echo ""
  fi
}

cmd_uninstall() {
  echo "将删除目录: ${PAPERCLIP_SERVICE_HOME}" >&2
  echo "Paperclip 数据目录默认位于: ${PAPERCLIP_HOME}（不会自动删除）" >&2
  # 先确认再动手：确认通过后 cmd_stop 用 --yes，避免二次追问把卸载拦腰截断。
  fundeploy_confirm_destructive "确认永久删除 ${PAPERCLIP_SERVICE_HOME}？" PAPERCLIP_UNINSTALL_YES || return 1
  cmd_stop --yes || fundeploy_ui_warn "停止失败，仍继续卸载。"
  if [[ -f "${PAPERCLIP_HOME}/cli/.managed-install" ]]; then
    paperclip_cli uninstall
    command -v npm >/dev/null 2>&1 && npm uninstall -g paperclipai
  else
    command -v pnpm >/dev/null 2>&1 || die "未找到 pnpm，无法卸载全局 paperclipai"
    pnpm remove -g paperclipai
  fi
  fundeploy_safe_rm "${PAPERCLIP_SERVICE_HOME}" || return 1
  echo "已卸载 paperclipai CLI 并删除 ${PAPERCLIP_SERVICE_HOME}。若需彻底清理数据，请自行删除 ${PAPERCLIP_HOME}"
}

dispatch() {
  local cmd="${1:-}"
  shift || true
  case "$cmd" in
    install) cmd_install ;;
    update) cmd_update ;;
    onboard) cmd_onboard "$@" ;;
    plugin) cmd_plugin "$@" ;;
    start) cmd_start ;;
    run) cmd_run ;;
    stop) cmd_stop ;;
    restart) cmd_restart ;;
    status) cmd_status ;;
    uninstall) cmd_uninstall ;;
    help | -h | --help) usage ;;
    *)
      echo "未知命令: ${cmd}" >&2
      usage >&2
      exit 2
      ;;
  esac
}

interactive_main() {
  declare -F fundeploy_ui_apply_theme >/dev/null 2>&1 && fundeploy_ui_apply_theme
  if declare -F fundeploy_ui_banner >/dev/null 2>&1; then
    fundeploy_ui_banner "Paperclip 本地服务（pnpm 全局安装）" "PAPERCLIP_HOME=${PAPERCLIP_HOME}" "PAPERCLIP_NPM_PACKAGE=${PAPERCLIP_NPM_PACKAGE}"
  else
    gum style --bold --foreground 212 "Paperclip 本地服务（pnpm 全局安装）"
    gum style "PAPERCLIP_HOME=${PAPERCLIP_HOME}"
    gum style "PAPERCLIP_NPM_PACKAGE=${PAPERCLIP_NPM_PACKAGE}"
  fi
  echo ""
  set +e
  while true; do
    local pick
    pick="$(fundeploy_ui_choose "fundeploy / service / paperclip / 选择动作" \
      "install" "update" "onboard" "plugin install" "start" "run" "stop" "restart" "status" "uninstall" "help" "quit")" || break
    [[ -z "$pick" ]] && break
    case "$pick" in
      quit) break ;;
      help) usage; continue ;;
      start | run | restart)
        paperclip_prompt_sign_up_policy || continue
        ;;
    esac
    if [[ "$pick" == "plugin install" ]]; then
      ( dispatch plugin install )
    else
      ( dispatch "$pick" )
    fi
    echo ""
  done
  set -e
}

main() {
  if [[ $# -gt 0 ]]; then
    case "$1" in
      help | -h | --help)
        dispatch "$@"
        return 0
        ;;
    esac
  fi
  if [[ $# -eq 0 ]]; then
    _fundeploy_ensure_gum || exit 1
    interactive_main
    return 0
  fi
  dispatch "$@"
}

main "$@"
