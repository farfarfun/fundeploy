#!/usr/bin/env bash
# Paperclip（https://github.com/paperclipai/paperclip）本机服务：
# 直接按上游官方 Quickstart 使用 npx paperclipai@latest，无源码克隆、无 pnpm。
#
# 用法：
#   ./setup.sh              # gum 菜单
#   ./setup.sh install      # 预检查 Node.js 并预热 npx 缓存
#   ./setup.sh update       # 等价于 install（实际运行始终使用 paperclipai@latest）
#   ./setup.sh onboard      # 首次配置（默认 NONINTERACTIVE=1 时自动加 --yes）
#   ./setup.sh start        # 后台启动（npx paperclipai run）
#   ./setup.sh run          # 前台启动（终端附着；不写 PID）
#   ./setup.sh stop / restart / status / uninstall
#
# 环境变量：
#   PAPERCLIP_SERVICE_HOME   本脚本管理根目录（默认 ~/opt/paperclip）
#   PAPERCLIP_HOME           上游数据目录（默认 ~/.paperclip，遵循官方默认）
#   PAPERCLIP_PORT           Paperclip 内部监听端口（默认 18804；启动时 export PORT 同值）
#   PAPERCLIP_HOST           Paperclip 内部监听地址（默认 127.0.0.1）
#   PAPERCLIP_PUBLIC_PORT    socat 对外映射端口（默认 8804）
#   PAPERCLIP_PUBLIC_BIND    socat 对外监听地址（默认 0.0.0.0）
#   PAPERCLIP_NPM_REGISTRY   npm registry（默认 https://registry.npmjs.org）
#   PAPERCLIP_NPM_PACKAGE    npm 包规格（默认 paperclipai@latest）
#   PAPERCLIP_NPM_CACHE      npm 缓存目录（默认 ${PAPERCLIP_SERVICE_HOME}/npm-cache）
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
PAPERCLIP_PORT="${PAPERCLIP_PORT:-18804}"
PAPERCLIP_HOST="${PAPERCLIP_HOST:-127.0.0.1}"
PAPERCLIP_PUBLIC_PORT="${PAPERCLIP_PUBLIC_PORT:-8804}"
PAPERCLIP_PUBLIC_BIND="${PAPERCLIP_PUBLIC_BIND:-0.0.0.0}"
PAPERCLIP_NPM_REGISTRY="${PAPERCLIP_NPM_REGISTRY:-https://registry.npmjs.org}"
PAPERCLIP_NPM_PACKAGE="${PAPERCLIP_NPM_PACKAGE:-paperclipai@latest}"
PAPERCLIP_NPM_CACHE="${PAPERCLIP_NPM_CACHE:-${PAPERCLIP_SERVICE_HOME}/npm-cache}"

PAPERCLIP_RUN_DIR="${PAPERCLIP_SERVICE_HOME}/run"
PAPERCLIP_LOG_DIR="${PAPERCLIP_SERVICE_HOME}/log"
PID_FILE="${PAPERCLIP_RUN_DIR}/paperclip.pid"
LOG_FILE="${PAPERCLIP_LOG_DIR}/paperclip.run.log"
SOCAT_PID_FILE="${PAPERCLIP_RUN_DIR}/paperclip-socat.pid"
SOCAT_LOG_FILE="${PAPERCLIP_LOG_DIR}/paperclip.socat.log"

usage() {
  cat <<USAGE
用法: ./setup.sh [command [args...]]

  无参数：gum 菜单。

命令:
  install     预检查 Node.js 20+ / socat，并通过 npx 预热 ${PAPERCLIP_NPM_PACKAGE} 缓存
  update      等价于 install；实际运行始终使用 ${PAPERCLIP_NPM_PACKAGE}
  onboard     首次配置；NONINTERACTIVE=1 时默认追加 --yes
  start       后台启动: 127.0.0.1:${PAPERCLIP_PORT} 运行 Paperclip，并用 socat 映射到 ${PAPERCLIP_PUBLIC_BIND}:${PAPERCLIP_PUBLIC_PORT}
  run         前台启动: 同 start 的运行环境，但终端附着、不写 PID
  stop        停止后台进程
  restart     stop 后 start
  status      查看 PID、日志位置与内外网 HTTP 健康检查（/api/health）
  uninstall   停止进程并删除 ${PAPERCLIP_SERVICE_HOME}（不删除 PAPERCLIP_HOME 数据目录）

说明:
  - 脚本遵循官方 Quickstart：npx paperclipai onboard --yes / run
  - 默认数据目录为 ${PAPERCLIP_HOME}（官方默认 ~/.paperclip）
  - 默认内部地址: http://${PAPERCLIP_HOST}:${PAPERCLIP_PORT}
  - 默认公网映射: http://${PAPERCLIP_PUBLIC_BIND}:${PAPERCLIP_PUBLIC_PORT} -> http://${PAPERCLIP_HOST}:${PAPERCLIP_PORT}
  - 健康检查: /api/health
  - 若你的全局 ~/.npmrc 指向私有 registry，可设置:
      PAPERCLIP_NPM_REGISTRY=https://registry.npmjs.org
USAGE
}

die() { echo "错误: $*" >&2; exit 1; }

ensure_dirs() {
  mkdir -p "${PAPERCLIP_RUN_DIR}" "${PAPERCLIP_LOG_DIR}" "${PAPERCLIP_NPM_CACHE}"
}

process_alive() {
  local pid="$1"
  kill -0 "$pid" 2>/dev/null
}

port_listener_pid() {
  local port="$1"
  if command -v lsof >/dev/null 2>&1; then
    lsof -tiTCP:"${port}" -sTCP:LISTEN -n -P 2>/dev/null | head -1
    return 0
  fi
  if command -v ss >/dev/null 2>&1; then
    ss -ltnp "( sport = :${port} )" 2>/dev/null | awk -F 'pid=' 'NR>1 && NF>1 {split($2,a,","); print a[1]; exit}'
    return 0
  fi
  echo ""
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
  command -v npx >/dev/null 2>&1 || die "未找到 npx；请安装包含 npm/npx 的 Node.js 发行版"
  local major
  major="$(node -p 'parseInt(process.versions.node.split(".")[0], 10)')"
  if (( major < 20 )); then
    die "需要 Node.js 20+，当前: $(node --version)"
  fi
}

require_socat() {
  command -v socat >/dev/null 2>&1 || die "需要 socat（用于将 ${PAPERCLIP_PUBLIC_BIND}:${PAPERCLIP_PUBLIC_PORT} 转发到 ${PAPERCLIP_HOST}:${PAPERCLIP_PORT}）"
}

paperclip_export_runtime_env() {
  export HOST="${PAPERCLIP_HOST}"
  export PORT="${PAPERCLIP_PORT}"
  export PAPERCLIP_HOME
  export npm_config_registry="${PAPERCLIP_NPM_REGISTRY}"
  export npm_config_cache="${PAPERCLIP_NPM_CACHE}"
}

paperclip_npx() {
  paperclip_export_runtime_env
  npx --yes --registry "${PAPERCLIP_NPM_REGISTRY}" --package "${PAPERCLIP_NPM_PACKAGE}" paperclipai "$@"
}

read_socat_pid() {
  if [[ ! -f "$SOCAT_PID_FILE" ]]; then
    echo ""
    return
  fi
  tr -d '[:space:]' <"$SOCAT_PID_FILE" || true
}

paperclip_server_pid() {
  port_listener_pid "${PAPERCLIP_PORT}"
}

paperclip_public_proxy_pid() {
  port_listener_pid "${PAPERCLIP_PUBLIC_PORT}"
}

paperclip_curl_health_http_code() {
  curl -sS -o /dev/null -w '%{http_code}' -m 3 "http://${PAPERCLIP_HOST}:${PAPERCLIP_PORT}/api/health" 2>/dev/null || echo "000"
}

paperclip_public_curl_health_http_code() {
  curl -sS -o /dev/null -w '%{http_code}' -m 3 "http://127.0.0.1:${PAPERCLIP_PUBLIC_PORT}/api/health" 2>/dev/null || echo "000"
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
  echo "---- socat 日志（${SOCAT_LOG_FILE}）----" >&2
  tail -n 40 "${SOCAT_LOG_FILE}" 2>/dev/null >&2 || true
}

paperclip_start_socat() {
  local spid
  spid="$(read_socat_pid)"
  if [[ -n "$spid" ]] && process_alive "$spid"; then
    return 0
  fi
  rm -f "$SOCAT_PID_FILE"
  nohup socat "TCP-LISTEN:${PAPERCLIP_PUBLIC_PORT},fork,reuseaddr,bind=${PAPERCLIP_PUBLIC_BIND}" "TCP:${PAPERCLIP_HOST}:${PAPERCLIP_PORT}" >>"${SOCAT_LOG_FILE}" 2>&1 &
  spid=$!
  echo "$spid" >"$SOCAT_PID_FILE"
  sleep 1
  process_alive "$spid" || die "socat 启动失败，请检查 ${SOCAT_LOG_FILE}"
}

paperclip_stop_socat() {
  local spid
  spid="$(read_socat_pid)"
  if [[ -z "$spid" ]]; then
    rm -f "$SOCAT_PID_FILE"
    return 0
  fi
  if ! process_alive "$spid"; then
    rm -f "$SOCAT_PID_FILE"
    return 0
  fi
  kill "$spid" 2>/dev/null || true
  local w=0
  while process_alive "$spid" && (( w < 10 )); do
    sleep 1
    w=$((w + 1))
  done
  if process_alive "$spid"; then
    kill -KILL "$spid" 2>/dev/null || true
  fi
  rm -f "$SOCAT_PID_FILE"
}

cmd_install() {
  require_node
  require_socat
  ensure_dirs
  echo "==> 预热 Paperclip CLI 缓存（${PAPERCLIP_NPM_PACKAGE}）…" >&2
  paperclip_npx --version >/dev/null
  echo "安装完成。首次使用可执行: $0 onboard"
}

cmd_update() {
  cmd_install
}

cmd_onboard() {
  require_node
  require_socat
  ensure_dirs
  if [[ "${NONINTERACTIVE:-}" == "1" ]]; then
    paperclip_npx onboard --yes "$@"
  else
    paperclip_npx onboard "$@"
  fi
}

cmd_start() {
  require_node
  require_socat
  ensure_dirs
  local existing
  existing="$(read_pid)"
  if [[ -n "$existing" ]] && process_alive "$existing"; then
    echo "Paperclip 已在运行（PID ${existing}）。如需重启: $0 restart" >&2
    exit 1
  fi
  local server_pid public_pid
  server_pid="$(paperclip_server_pid)"
  public_pid="$(paperclip_public_proxy_pid)"
  if [[ -n "$server_pid" ]]; then
    die "端口 ${PAPERCLIP_PORT} 已被 PID ${server_pid} 占用，请先执行 $0 stop 或手动清理。"
  fi
  if [[ -n "$public_pid" ]]; then
    die "端口 ${PAPERCLIP_PUBLIC_PORT} 已被 PID ${public_pid} 占用，请先执行 $0 stop 或手动清理。"
  fi
  rm -f "$PID_FILE"
  rm -f "$SOCAT_PID_FILE"
  echo "==> 启动 Paperclip（npx ${PAPERCLIP_NPM_PACKAGE} run），日志: ${LOG_FILE}" >&2
  echo "    内部监听: http://${PAPERCLIP_HOST}:${PAPERCLIP_PORT}" >&2
  echo "    公网映射: http://${PAPERCLIP_PUBLIC_BIND}:${PAPERCLIP_PUBLIC_PORT} -> http://${PAPERCLIP_HOST}:${PAPERCLIP_PORT}" >&2
  paperclip_export_runtime_env
  nohup npx --yes --registry "${PAPERCLIP_NPM_REGISTRY}" --package "${PAPERCLIP_NPM_PACKAGE}" paperclipai run >>"${LOG_FILE}" 2>&1 &
  local cpid=$!
  echo "$cpid" >"$PID_FILE"
  sleep 1
  if ! process_alive "$cpid"; then
    rm -f "$PID_FILE"
    paperclip_start_failure_hints
    die "进程已退出，启动失败。若是首次使用，请先执行: $0 onboard"
  fi
  paperclip_start_socat
  if [[ "${PAPERCLIP_SKIP_START_HEALTH_CHECK:-}" == "1" ]]; then
    echo "已启动 PID ${cpid}，socat PID $(read_socat_pid)（已跳过 HTTP 健康检查）"
    return 0
  fi
  echo "==> 等待 http://${PAPERCLIP_HOST}:${PAPERCLIP_PORT}/api/health 就绪（最长 ${PAPERCLIP_START_HEALTH_TIMEOUT_SEC:-60}s）…" >&2
  local wr=0
  paperclip_wait_ready "$cpid" || wr=$?
  if [[ "$wr" -eq 0 ]]; then
    local public_code
    public_code="$(paperclip_public_curl_health_http_code)"
    if [[ "$public_code" != "200" ]]; then
      paperclip_start_failure_hints
      die "内部服务已启动，但 socat 公网映射检查失败（127.0.0.1:${PAPERCLIP_PUBLIC_PORT}/api/health 返回 ${public_code}）"
    fi
    echo "已启动 PID ${cpid}，socat PID $(read_socat_pid)，内外网健康检查通过。"
    return 0
  fi
  if [[ "$wr" -eq 2 ]]; then
    rm -f "$PID_FILE"
    paperclip_stop_socat || true
    paperclip_start_failure_hints
    die "进程已退出，启动失败。若是首次使用，请先执行: $0 onboard"
  fi
  paperclip_stop_socat || true
  echo "错误: 在 ${PAPERCLIP_START_HEALTH_TIMEOUT_SEC:-60}s 内未通过健康检查；进程可能仍在运行（PID ${cpid}）。" >&2
  paperclip_start_failure_hints
  die "启动校验失败。"
}

cmd_run() {
  require_node
  require_socat
  ensure_dirs
  local existing
  existing="$(read_pid)"
  if [[ -n "$existing" ]] && process_alive "$existing"; then
    die "Paperclip 已在后台运行（PID ${existing}）。请先执行 $0 stop，再使用 run 前台调试。"
  fi
  local spid
  spid="$(read_socat_pid)"
  if [[ -n "$spid" ]] && process_alive "$spid"; then
    die "Paperclip socat 映射已在后台运行（PID ${spid}）。请先执行 $0 stop，再使用 run 前台调试。"
  fi
  echo "==> 前台启动 Paperclip（npx ${PAPERCLIP_NPM_PACKAGE} run），按 Ctrl+C 结束；不写 PID。" >&2
  echo "    内部监听: http://${PAPERCLIP_HOST}:${PAPERCLIP_PORT}" >&2
  echo "    注意: run 不启动 socat；公网映射仅在 start 模式下启用。" >&2
  exec env \
    HOST="${PAPERCLIP_HOST}" \
    PORT="${PAPERCLIP_PORT}" \
    PAPERCLIP_HOME="${PAPERCLIP_HOME}" \
    npm_config_registry="${PAPERCLIP_NPM_REGISTRY}" \
    npm_config_cache="${PAPERCLIP_NPM_CACHE}" \
    npx --yes --registry "${PAPERCLIP_NPM_REGISTRY}" --package "${PAPERCLIP_NPM_PACKAGE}" paperclipai run
}

cmd_stop() {
  local pid
  pid="$(read_pid)"
  local spid
  spid="$(read_socat_pid)"
  if [[ -n "$pid" || -n "$spid" ]] && [[ "${NONINTERACTIVE:-}" != "1" ]] && [[ -t 0 ]]; then
    _nlt_ensure_gum || exit 1
    gum confirm "停止 Paperclip（PID ${pid:--} / socat PID ${spid:--}）？" || exit 0
  fi

  local server_pid
  server_pid="$(paperclip_server_pid)"
  local proxy_pid
  proxy_pid="$(paperclip_public_proxy_pid)"

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

  paperclip_stop_socat || true
  proxy_pid="$(paperclip_public_proxy_pid)"
  if [[ -n "$proxy_pid" ]]; then
    kill "$proxy_pid" 2>/dev/null || true
  fi
  echo "已停止。"
}

cmd_restart() {
  cmd_stop || true
  cmd_start
}

cmd_status() {
  local pid
  pid="$(read_pid)"
  echo "PAPERCLIP_SERVICE_HOME=${PAPERCLIP_SERVICE_HOME}"
  echo "PAPERCLIP_HOME=${PAPERCLIP_HOME}"
  echo "PAPERCLIP_PORT=${PAPERCLIP_PORT}"
  echo "PAPERCLIP_HOST=${PAPERCLIP_HOST}"
  echo "PAPERCLIP_PUBLIC_PORT=${PAPERCLIP_PUBLIC_PORT}"
  echo "PAPERCLIP_PUBLIC_BIND=${PAPERCLIP_PUBLIC_BIND}"
  echo "PAPERCLIP_NPM_PACKAGE=${PAPERCLIP_NPM_PACKAGE}"
  echo "PAPERCLIP_NPM_REGISTRY=${PAPERCLIP_NPM_REGISTRY}"
  echo "LOG_FILE=${LOG_FILE}"
  echo "SOCAT_LOG_FILE=${SOCAT_LOG_FILE}"
  local spid
  spid="$(read_socat_pid)"
  local server_pid
  server_pid="$(paperclip_server_pid)"
  local proxy_pid
  proxy_pid="$(paperclip_public_proxy_pid)"
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
  if [[ -n "$spid" ]] && process_alive "$spid"; then
    if [[ -n "$proxy_pid" && "$proxy_pid" != "$spid" ]]; then
      echo "状态: socat 运行中 PID ${spid}（监听进程 PID ${proxy_pid}）"
    else
      echo "状态: socat 运行中 PID ${spid}"
    fi
  elif [[ -n "$proxy_pid" ]]; then
    echo "状态: socat 监听中（PID 文件缺失/失效，监听进程 PID ${proxy_pid}）"
  else
    echo "状态: socat 未运行"
    rm -f "$SOCAT_PID_FILE"
  fi
  if command -v curl >/dev/null 2>&1; then
    echo ""
    echo "==> GET 内部 http://${PAPERCLIP_HOST}:${PAPERCLIP_PORT}/api/health"
    curl -sS -m 3 "http://${PAPERCLIP_HOST}:${PAPERCLIP_PORT}/api/health" || echo "（无法连接，可能未启动或端口不同）"
    echo ""
    echo "==> GET 公网映射 http://127.0.0.1:${PAPERCLIP_PUBLIC_PORT}/api/health"
    curl -sS -m 3 "http://127.0.0.1:${PAPERCLIP_PUBLIC_PORT}/api/health" || echo "（无法连接，可能 socat 未启动或端口不同）"
    echo ""
  fi
}

cmd_uninstall() {
  cmd_stop || true
  echo "将删除目录: ${PAPERCLIP_SERVICE_HOME}" >&2
  echo "Paperclip 数据目录默认位于: ${PAPERCLIP_HOME}（不会自动删除）" >&2
  if [[ -t 0 ]]; then
    _nlt_ensure_gum || exit 1
    gum confirm "确认永久删除上述服务目录？" || exit 0
  else
    [[ "${PAPERCLIP_UNINSTALL_YES:-}" == "1" ]] || die "非交互卸载请设置 PAPERCLIP_UNINSTALL_YES=1"
  fi
  local hp ap
  hp="$(cd "$HOME" && pwd -P)"
  ap="$(cd "${PAPERCLIP_SERVICE_HOME}" 2>/dev/null && pwd -P)" || ap="${PAPERCLIP_SERVICE_HOME}"
  if [[ "$ap" == "/" || "$ap" == "$hp" ]]; then
    die "拒绝删除根目录或 \$HOME"
  fi
  rm -rf "${PAPERCLIP_SERVICE_HOME}"
  echo "已删除 ${PAPERCLIP_SERVICE_HOME}。若需彻底清理数据，请自行删除 ${PAPERCLIP_HOME}"
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
  gum style --bold --foreground 212 "Paperclip 本地服务（官方 npx 方式）"
  gum style "PAPERCLIP_HOME=${PAPERCLIP_HOME}"
  gum style "PAPERCLIP_NPM_PACKAGE=${PAPERCLIP_NPM_PACKAGE}"
  echo ""
  set +e
  while true; do
    local pick
    pick="$(gum choose --header "选择操作（取消退出）" \
      "install" "update" "onboard" "start" "run" "stop" "restart" "status" "uninstall" "help" "quit")" || break
    [[ -z "$pick" ]] && break
    case "$pick" in
      quit) break ;;
      help) usage; continue ;;
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
