#!/usr/bin/env bash
# Paperclip（https://github.com/paperclipai/paperclip）本机服务：
# 通过 npm 全局安装 paperclipai CLI，无源码克隆、无 pnpm。
#
# 用法：
#   ./setup.sh              # gum 菜单
#   ./setup.sh install      # npm install -g paperclipai@latest
#   ./setup.sh update       # 等价于 install
#   ./setup.sh onboard      # 首次配置（默认 NONINTERACTIVE=1 时自动加 --yes）
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
#   PAPERCLIP_NPM_REGISTRY   npm registry（默认 https://registry.npmjs.org）
#   PAPERCLIP_NPM_PACKAGE    npm 包规格（默认 paperclipai@latest）
#   PAPERCLIP_START_HEALTH_TIMEOUT_SEC  start 时等待 /api/health 健康检查最长秒数（默认 60）
#   PAPERCLIP_SKIP_START_HEALTH_CHECK=1 跳过 HTTP 健康检查
#   NONINTERACTIVE=1         跳过 gum 确认；onboard 默认加 --yes
#   PAPERCLIP_UNINSTALL_YES=1 非 TTY 卸载确认

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ -f "${SCRIPT_DIR}/../lib/nlt-common.sh" ]]; then
  # shellcheck source=../lib/nlt-common.sh
  source "${SCRIPT_DIR}/../lib/nlt-common.sh"
elif [[ -f "${SCRIPT_DIR}/../../lib/nlt-common.sh" ]]; then
  # shellcheck source=../../lib/nlt-common.sh
  source "${SCRIPT_DIR}/../../lib/nlt-common.sh"
else
  echo "错误: 找不到 lib/nlt-common.sh（已检查 ${SCRIPT_DIR}/../lib 与 ${SCRIPT_DIR}/../../lib）" >&2
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

usage() {
  cat <<USAGE
用法: ./setup.sh [command [args...]]

  无参数：gum 菜单。

命令:
  install     npm 全局安装 ${PAPERCLIP_NPM_PACKAGE}
  update      全局升级到 ${PAPERCLIP_NPM_PACKAGE}
  onboard     首次配置；NONINTERACTIVE=1 时默认追加 --yes
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
  - 若你的全局 ~/.npmrc 指向私有 registry，可设置:
      PAPERCLIP_NPM_REGISTRY=https://registry.npmjs.org
USAGE
}

die() { echo "错误: $*" >&2; exit 1; }

ensure_dirs() {
  mkdir -p "${PAPERCLIP_RUN_DIR}" "${PAPERCLIP_LOG_DIR}"
}

process_alive() {
  local pid="$1"
  kill -0 "$pid" 2>/dev/null
}

port_listener_pid() {
  _nlt_listener_pid_for_port "$1"
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
  export HOST="${PAPERCLIP_HOST}"
  export PORT="${PAPERCLIP_PORT}"
  export DATABASE_URL="${PAPERCLIP_DATABASE_URL}"
  export PAPERCLIP_AUTH_DISABLE_SIGN_UP="${PAPERCLIP_AUTH_DISABLE_SIGN_UP}"
  export PAPERCLIP_HOME
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

paperclip_server_pid() {
  port_listener_pid "${PAPERCLIP_PORT}"
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

cmd_install() {
  require_node
  command -v npm >/dev/null 2>&1 || die "未找到 npm"
  ensure_dirs
  echo "==> npm 全局安装 Paperclip CLI（${PAPERCLIP_NPM_PACKAGE}）…" >&2
  npm install -g --registry "${PAPERCLIP_NPM_REGISTRY}" "${PAPERCLIP_NPM_PACKAGE}"
  paperclip_cli --version
  echo "安装完成。首次使用可执行: $0 onboard"
}

cmd_update() {
  cmd_install
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
  paperclip_export_runtime_env
  nohup paperclipai run >>"${LOG_FILE}" 2>&1 &
  local cpid=$!
  echo "$cpid" >"$PID_FILE"
  sleep 1
  if ! process_alive "$cpid"; then
    rm -f "$PID_FILE"
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
    rm -f "$PID_FILE"
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
  paperclip_export_runtime_env
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
  if [[ -z "$assume_yes" ]] && [[ -n "$pid" ]] && nlt_interactive; then
    # nlt_ui_confirm 在缺 gum 时降级为 read y/N，无需 _nlt_ensure_gum 硬拉依赖。
    nlt_ui_confirm "停止 Paperclip（PID ${pid:--}）？" || return 1
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
  nlt_confirm_destructive "确认永久删除 ${PAPERCLIP_SERVICE_HOME}？" PAPERCLIP_UNINSTALL_YES || return 1
  cmd_stop --yes || nlt_ui_warn "停止失败，仍继续卸载。"
  command -v npm >/dev/null 2>&1 || die "未找到 npm，无法卸载全局 paperclipai"
  npm uninstall -g paperclipai
  nlt_safe_rm "${PAPERCLIP_SERVICE_HOME}" || return 1
  echo "已卸载全局 paperclipai 并删除 ${PAPERCLIP_SERVICE_HOME}。若需彻底清理数据，请自行删除 ${PAPERCLIP_HOME}"
}

dispatch() {
  local cmd="${1:-}"
  shift || true
  case "$cmd" in
    install) cmd_install ;;
    update) cmd_update ;;
    onboard) cmd_onboard "$@" ;;
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
  declare -F nlt_ui_apply_theme >/dev/null 2>&1 && nlt_ui_apply_theme
  if declare -F nlt_ui_banner >/dev/null 2>&1; then
    nlt_ui_banner "Paperclip 本地服务（npm 全局安装）" "PAPERCLIP_HOME=${PAPERCLIP_HOME}" "PAPERCLIP_NPM_PACKAGE=${PAPERCLIP_NPM_PACKAGE}"
  else
    gum style --bold --foreground 212 "Paperclip 本地服务（npm 全局安装）"
    gum style "PAPERCLIP_HOME=${PAPERCLIP_HOME}"
    gum style "PAPERCLIP_NPM_PACKAGE=${PAPERCLIP_NPM_PACKAGE}"
  fi
  echo ""
  set +e
  while true; do
    local pick
    pick="$(nlt_ui_choose "nltdeploy / service / paperclip / 选择动作" \
      "install" "update" "onboard" "start" "run" "stop" "restart" "status" "uninstall" "help" "quit")" || break
    [[ -z "$pick" ]] && break
    case "$pick" in
      quit) break ;;
      help) usage; continue ;;
      start | run | restart)
        paperclip_prompt_sign_up_policy || continue
        ;;
    esac
    ( dispatch "$pick" )
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
    _nlt_ensure_gum || exit 1
    interactive_main
    return 0
  fi
  dispatch "$@"
}

main "$@"
