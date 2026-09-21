#!/usr/bin/env bash
# funread-web 一体化部署：在本地 venv 安装 funread API，并从 GitHub 构建 funread-web。
# 上游尚未提供后台生命周期 CLI，因此 PID、日志与前后端启动顺序由本脚本管理。

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../../lib/fundeploy-common.sh
source "${SCRIPT_DIR}/../../lib/fundeploy-common.sh"

FUNREAD_WEB_SERVICE_HOME="${FUNREAD_WEB_SERVICE_HOME:-${HOME}/opt/funread-web}"
FUNREAD_WEB_BACKEND_VERSION="${FUNREAD_WEB_BACKEND_VERSION:-}"
FUNREAD_WEB_GIT_URL="${FUNREAD_WEB_GIT_URL:-https://github.com/farfarfun/funread-web.git}"
FUNREAD_WEB_GIT_REF="${FUNREAD_WEB_GIT_REF:-master}"
FUNREAD_WEB_PYTHON_BIN="${FUNREAD_WEB_PYTHON_BIN:-python3}"
FUNREAD_WEB_NPM_BIN="${FUNREAD_WEB_NPM_BIN:-}"
FUNREAD_WEB_BACKEND_HOST="${FUNREAD_WEB_BACKEND_HOST:-127.0.0.1}"
FUNREAD_WEB_BACKEND_PORT="${FUNREAD_WEB_BACKEND_PORT:-18811}"
FUNREAD_WEB_FRONTEND_HOST="${FUNREAD_WEB_FRONTEND_HOST:-127.0.0.1}"
FUNREAD_WEB_FRONTEND_PORT="${FUNREAD_WEB_FRONTEND_PORT:-8811}"

SOURCE_DIR="${FUNREAD_WEB_SERVICE_HOME}/src"
VENV_DIR="${FUNREAD_WEB_SERVICE_HOME}/venv"
RUN_DIR="${FUNREAD_WEB_SERVICE_HOME}/run"
BACKEND_PID_FILE="${RUN_DIR}/backend.pid"
FRONTEND_PID_FILE="${RUN_DIR}/frontend.pid"
BACKEND_LOG_FILE="${RUN_DIR}/backend.log"
FRONTEND_LOG_FILE="${RUN_DIR}/frontend.log"

die() { echo "错误: $*" >&2; exit 1; }

usage() {
  cat <<USAGE
用法: ./setup.sh [command]

命令:
  install / update   安装或更新 funread API，并拉取、构建 funread-web
  start              先启动后端，再启动前端
  stop               先停止前端，再停止后端
  restart            stop + start
  status             查看前后端状态
  uninstall          停止服务并删除 ${FUNREAD_WEB_SERVICE_HOME}

地址:
  后端  http://${FUNREAD_WEB_BACKEND_HOST}:${FUNREAD_WEB_BACKEND_PORT}/docs
  前端  http://${FUNREAD_WEB_FRONTEND_HOST}:${FUNREAD_WEB_FRONTEND_PORT}/

常用环境变量:
  FUNREAD_WEB_SERVICE_HOME      安装目录（默认 ~/opt/funread-web）
  FUNREAD_WEB_BACKEND_VERSION  funread 版本（默认最新）
  FUNREAD_WEB_GIT_REF          funread-web 分支或 tag（默认 master）
  FUNREAD_WEB_BACKEND_HOST / FUNREAD_WEB_BACKEND_PORT（默认 127.0.0.1:18811）
  FUNREAD_WEB_FRONTEND_HOST / FUNREAD_WEB_FRONTEND_PORT（默认 127.0.0.1:8811）
  FUNREAD_DATABASE_URL         后端数据库地址（不设时使用 funread 默认本地 SQLite）
  FUNREAD_WEB_UNINSTALL_YES=1  非交互卸载确认

上游: https://github.com/farfarfun/funread · https://github.com/farfarfun/funread-web
USAGE
}

ensure_dirs() { mkdir -p "${FUNREAD_WEB_SERVICE_HOME}" "${RUN_DIR}"; }

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "未找到 $1（$2）"
}

_resolve_npm() {
  if [[ -n "${FUNREAD_WEB_NPM_BIN}" ]]; then
    [[ -x "${FUNREAD_WEB_NPM_BIN}" ]] || die "FUNREAD_WEB_NPM_BIN 无效: ${FUNREAD_WEB_NPM_BIN}"
    echo "${FUNREAD_WEB_NPM_BIN}"
    return
  fi
  # 优先 pnpm：前端仓库多数用 only-allow pnpm 锁定包管理器，裸 npm install
  # 会在 preinstall 阶段直接失败。
  if command -v pnpm >/dev/null 2>&1; then
    command -v pnpm
    return
  fi
  command -v npm >/dev/null 2>&1 || die "未找到 npm/pnpm（可先运行 fundeploy dev nodejs install）"
  command -v npm
}

_npm_is_pnpm() {
  [[ "$(basename "$1")" == pnpm* ]]
}

wait_for_listener() {
  local port="$1" waited=0
  while (( waited < 10 )); do
    [[ -n "$(_fundeploy_listener_pid_for_port "${port}")" ]] && return 0
    sleep 1
    waited=$((waited + 1))
  done
  return 1
}

backend_spec() {
  if [[ -n "${FUNREAD_WEB_BACKEND_VERSION}" ]]; then
    printf 'funread[api]==%s' "${FUNREAD_WEB_BACKEND_VERSION#v}"
  else
    printf '%s' 'funread[api]'
  fi
}

cmd_install() {
  require_command "${FUNREAD_WEB_PYTHON_BIN}" "请安装 Python 3.12+"
  require_command git "请先安装 git"
  local npm_bin
  npm_bin="$(_resolve_npm)"
  ensure_dirs

  echo "==> 安装后端: $(backend_spec)"
  if command -v uv >/dev/null 2>&1; then
    [[ -x "${VENV_DIR}/bin/python" ]] || uv venv --python "${FUNREAD_WEB_PYTHON_BIN}" "${VENV_DIR}"
    uv pip install --python "${VENV_DIR}/bin/python" --upgrade "$(backend_spec)" || die "后端安装失败"
  else
    if [[ ! -x "${VENV_DIR}/bin/python" ]]; then
      "${FUNREAD_WEB_PYTHON_BIN}" -m venv "${VENV_DIR}" \
        || die "创建 venv 失败，请安装 python3-venv 或 uv"
    fi
    "${VENV_DIR}/bin/python" -m pip install --upgrade "$(backend_spec)" || die "后端安装失败"
  fi

  echo "==> 获取前端: ${FUNREAD_WEB_GIT_URL} (${FUNREAD_WEB_GIT_REF})"
  if [[ -d "${SOURCE_DIR}/.git" ]]; then
    git -C "${SOURCE_DIR}" fetch --depth 1 origin "${FUNREAD_WEB_GIT_REF}"
    git -C "${SOURCE_DIR}" checkout --detach FETCH_HEAD
  elif [[ -e "${SOURCE_DIR}" ]]; then
    die "${SOURCE_DIR} 已存在但不是 git 仓库"
  else
    git clone --depth 1 --branch "${FUNREAD_WEB_GIT_REF}" "${FUNREAD_WEB_GIT_URL}" "${SOURCE_DIR}"
  fi
  (
    cd "${SOURCE_DIR}"
    if _npm_is_pnpm "${npm_bin}"; then
      "${npm_bin}" install
    else
      "${npm_bin}" install --include=dev --no-package-lock
    fi
    "${npm_bin}" run build
  ) || die "前端安装或构建失败"
  echo "已安装。运行 ./setup.sh start 启动。"
}

cmd_start() {
  [[ -x "${VENV_DIR}/bin/python" ]] || die "后端未安装，请先: ./setup.sh install"
  [[ -x "${SOURCE_DIR}/node_modules/.bin/vite" && -d "${SOURCE_DIR}/dist" ]] \
    || die "前端未安装或未构建，请先: ./setup.sh install"
  ensure_dirs

  local pid occupied
  pid="$(_fundeploy_read_pid_file "${BACKEND_PID_FILE}")"
  if _fundeploy_process_alive "$pid"; then
    echo "后端已在运行（PID ${pid}），跳过。"
  else
    rm -f "${BACKEND_PID_FILE}"
    occupied="$(_fundeploy_listener_pid_for_port "${FUNREAD_WEB_BACKEND_PORT}")"
    [[ -z "$occupied" ]] || die "后端端口 ${FUNREAD_WEB_BACKEND_PORT} 已被 PID ${occupied} 占用"
    echo "==> 启动后端 ${FUNREAD_WEB_BACKEND_HOST}:${FUNREAD_WEB_BACKEND_PORT}"
    nohup "${VENV_DIR}/bin/python" -m uvicorn funread.api.app:app \
      --host "${FUNREAD_WEB_BACKEND_HOST}" --port "${FUNREAD_WEB_BACKEND_PORT}" \
      >>"${BACKEND_LOG_FILE}" 2>&1 &
    echo $! >"${BACKEND_PID_FILE}"
    if ! wait_for_listener "${FUNREAD_WEB_BACKEND_PORT}"; then
      stop_process "后端" "${BACKEND_PID_FILE}"
      die "后端未监听端口 ${FUNREAD_WEB_BACKEND_PORT}，请查看 ${BACKEND_LOG_FILE}"
    fi
  fi

  pid="$(_fundeploy_read_pid_file "${FRONTEND_PID_FILE}")"
  if _fundeploy_process_alive "$pid"; then
    echo "前端已在运行（PID ${pid}），跳过。"
  else
    rm -f "${FRONTEND_PID_FILE}"
    occupied="$(_fundeploy_listener_pid_for_port "${FUNREAD_WEB_FRONTEND_PORT}")"
    [[ -z "$occupied" ]] || die "前端端口 ${FUNREAD_WEB_FRONTEND_PORT} 已被 PID ${occupied} 占用"
    echo "==> 启动前端 ${FUNREAD_WEB_FRONTEND_HOST}:${FUNREAD_WEB_FRONTEND_PORT}"
    (
      cd "${SOURCE_DIR}"
      FUNREAD_API_BASE_URL="http://${FUNREAD_WEB_BACKEND_HOST}:${FUNREAD_WEB_BACKEND_PORT}" \
        nohup "${SOURCE_DIR}/node_modules/.bin/vite" preview \
          --host "${FUNREAD_WEB_FRONTEND_HOST}" --port "${FUNREAD_WEB_FRONTEND_PORT}" \
          >>"${FRONTEND_LOG_FILE}" 2>&1 &
      echo $! >"${FRONTEND_PID_FILE}"
    )
    if ! wait_for_listener "${FUNREAD_WEB_FRONTEND_PORT}"; then
      stop_process "前端" "${FRONTEND_PID_FILE}"
      die "前端未监听端口 ${FUNREAD_WEB_FRONTEND_PORT}，后端仍在运行；请查看 ${FRONTEND_LOG_FILE}"
    fi
  fi
  echo "已启动。界面: http://${FUNREAD_WEB_FRONTEND_HOST}:${FUNREAD_WEB_FRONTEND_PORT}/"
}

stop_process() {
  local name="$1" pid_file="$2" pid
  pid="$(_fundeploy_read_pid_file "${pid_file}")"
  if ! _fundeploy_process_alive "$pid"; then
    rm -f "${pid_file}"
    echo "${name}未运行，跳过。"
    return
  fi
  echo "==> 停止${name}（PID ${pid}）"
  _fundeploy_stop_pid "$pid" 10
  rm -f "${pid_file}"
}

cmd_stop() {
  stop_process "前端" "${FRONTEND_PID_FILE}"
  stop_process "后端" "${BACKEND_PID_FILE}"
  echo "已停止。"
}

cmd_restart() {
  cmd_stop
  cmd_start
}

component_status() {
  local name="$1" pid_file="$2" port="$3" url="$4" pid listen code=""
  pid="$(_fundeploy_read_pid_file "${pid_file}")"
  listen="$(_fundeploy_listener_pid_for_port "${port}")"
  if _fundeploy_process_alive "$pid"; then
    printf '%s: 运行中（PID %s）\n' "$name" "$pid"
  elif [[ -n "$listen" ]]; then
    printf '%s: 端口被占用（listen PID %s）\n' "$name" "$listen"
  else
    printf '%s: 未运行\n' "$name"
    rm -f "$pid_file"
  fi
  if command -v curl >/dev/null 2>&1; then
    code="$(curl -sS -m 3 -o /dev/null -w '%{http_code}' "$url" 2>/dev/null || true)"
    echo "  ${url} -> HTTP ${code:-无响应}"
  else
    echo "  ${url}"
  fi
}

cmd_status() {
  component_status "后端 funread" "${BACKEND_PID_FILE}" "${FUNREAD_WEB_BACKEND_PORT}" \
    "http://${FUNREAD_WEB_BACKEND_HOST}:${FUNREAD_WEB_BACKEND_PORT}/healthz"
  component_status "前端 funread-web" "${FRONTEND_PID_FILE}" "${FUNREAD_WEB_FRONTEND_PORT}" \
    "http://${FUNREAD_WEB_FRONTEND_HOST}:${FUNREAD_WEB_FRONTEND_PORT}/"
  echo "安装目录: ${FUNREAD_WEB_SERVICE_HOME}"
  echo "日志: ${BACKEND_LOG_FILE} / ${FRONTEND_LOG_FILE}"
}

cmd_uninstall() {
  fundeploy_confirm_destructive \
    "将停止 funread/funread-web 并删除 ${FUNREAD_WEB_SERVICE_HOME}，确认？" \
    FUNREAD_WEB_UNINSTALL_YES || return 1
  cmd_stop
  [[ ! -e "${FUNREAD_WEB_SERVICE_HOME}" ]] || fundeploy_safe_rm "${FUNREAD_WEB_SERVICE_HOME}"
  echo "已卸载。"
}

interactive_main() {
  _fundeploy_ensure_gum || exit 1
  fundeploy_ui_banner "fundeploy / service / funread-web" \
    "后端 ${FUNREAD_WEB_BACKEND_HOST}:${FUNREAD_WEB_BACKEND_PORT}  前端 ${FUNREAD_WEB_FRONTEND_HOST}:${FUNREAD_WEB_FRONTEND_PORT}"
  set +e
  while true; do
    local pick
    pick="$(fundeploy_ui_choose "fundeploy / service / funread-web / 选择动作" \
      "install    安装" "update     更新" "start      启动" "stop       停止" \
      "restart    重启" "status     查看状态" "uninstall  卸载" "help       命令帮助" "quit       返回")" || break
    [[ -n "$pick" ]] || break
    pick="${pick%% *}"
    case "$pick" in
      quit) break ;;
      help) usage ;;
      install|update) cmd_install ;;
      start) cmd_start ;;
      stop) cmd_stop ;;
      restart) cmd_restart ;;
      status) cmd_status ;;
      uninstall) cmd_uninstall ;;
    esac
    echo ""
  done
  set -e
}

main() {
  local cmd="${1:-}"
  if [[ -z "$cmd" ]]; then
    if [[ "${NONINTERACTIVE:-}" == "1" ]]; then usage >&2; exit 1; fi
    interactive_main
    return
  fi
  case "$cmd" in
    install|update) cmd_install ;;
    start) cmd_start ;;
    stop) cmd_stop ;;
    restart) cmd_restart ;;
    status) cmd_status ;;
    uninstall) cmd_uninstall ;;
    help|-h|--help) usage ;;
    *) echo "未知命令: $cmd" >&2; usage >&2; exit 2 ;;
  esac
}

main "$@"
