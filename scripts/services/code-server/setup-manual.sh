#!/usr/bin/env bash
# code-server 手动模式：从官方 GitHub Releases 下载独立发行包并以本地 PID 运行。
#
# 依赖：curl、tar；无需单独安装 Node（发行包已内置）。
#
# 用法：
#   ./setup-manual.sh              # gum 菜单
#   ./setup-manual.sh install      # 下载并解压到 ${CODE_SERVER_SERVICE_HOME}
#   ./setup-manual.sh update       # 重新下载安装（同 install）
#   ./setup-manual.sh start        # 后台 code-server --bind-addr …
#   ./setup-manual.sh run          # 前台附着（PASSWORD 与 start 一致；不写 PID；后台已在跑时拒绝）
#   ./setup-manual.sh stop | restart | status | uninstall
#
# 环境变量：
#   CODE_SERVER_SERVICE_HOME  安装根（默认 ~/opt/code-server），内含 bin/code-server
#   CODE_SERVER_VERSION       版本号如 4.112.0；不设置则从 GitHub latest 解析
#   CODE_SERVER_BIND          监听地址（默认 127.0.0.1:8080）
#   PASSWORD                  登录密码（可选；不设则见日志中随机密码提示）
#   NONINTERACTIVE=1
#   CODE_SERVER_UNINSTALL_YES=1   非 TTY 卸载确认

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

if [[ -f "${SCRIPT_DIR}/../lib/nlt-progress.sh" ]]; then
  # shellcheck source=../lib/nlt-progress.sh
  source "${SCRIPT_DIR}/../lib/nlt-progress.sh"
elif [[ -f "${SCRIPT_DIR}/../../lib/nlt-progress.sh" ]]; then
  # shellcheck source=../../lib/nlt-progress.sh
  source "${SCRIPT_DIR}/../../lib/nlt-progress.sh"
fi

CODE_SERVER_SERVICE_HOME="${CODE_SERVER_SERVICE_HOME:-${HOME}/opt/code-server}"
CODE_SERVER_BIND="${CODE_SERVER_BIND:-127.0.0.1:8080}"
# status 探测用；未单独设置时从 BIND 取端口
CODE_SERVER_PORT="${CODE_SERVER_PORT:-${CODE_SERVER_BIND##*:}}"

CODE_SERVER_RUN_DIR="${CODE_SERVER_SERVICE_HOME}/run"
CODE_SERVER_LOG_DIR="${CODE_SERVER_SERVICE_HOME}/log"
PID_FILE="${CODE_SERVER_RUN_DIR}/code-server.pid"
LOG_FILE="${CODE_SERVER_LOG_DIR}/code-server.log"
CODE_SERVER_BIN="${CODE_SERVER_SERVICE_HOME}/bin/code-server"

usage() {
  cat <<USAGE
用法: setup-manual.sh [command]

  无参数：gum 菜单。

命令:
  install / update   从 GitHub Releases 下载 standalone 包并解压到 ${CODE_SERVER_SERVICE_HOME}
  start              后台启动（日志 ${LOG_FILE}，默认绑定 ${CODE_SERVER_BIND}）
  run                前台启动（同绑定与 PASSWORD 规则；不写 PID；后台已在跑时拒绝）
  stop / restart / status
  uninstall          停止并删除 ${CODE_SERVER_SERVICE_HOME}（配置目录 ~/.config/code-server 需自行处理）

上游文档: https://github.com/coder/code-server
USAGE
}

ensure_dirs() {
  mkdir -p "${CODE_SERVER_RUN_DIR}" "${CODE_SERVER_LOG_DIR}"
}

die() { echo "错误: $*" >&2; exit 1; }

process_alive() {
  kill -0 "$1" 2>/dev/null
}

listener_pid_for_port() {
  _nlt_listener_pid_for_port "$1"
}

read_pid() {
  if [[ ! -f "$PID_FILE" ]]; then
    echo ""
    return
  fi
  tr -d '[:space:]' <"$PID_FILE" || true
}

require_curl() {
  command -v curl >/dev/null 2>&1 || die "需要 curl"
}

# 输出如 linux-amd64、macos-arm64
_detect_platform() {
  local os arch
  case "$(uname -s)" in
    Linux) os=linux ;;
    Darwin) os=macos ;;
    *) die "不支持的操作系统: $(uname -s)" ;;
  esac
  case "$(uname -m)" in
    x86_64 | amd64) arch=amd64 ;;
    aarch64 | arm64) arch=arm64 ;;
    *) die "不支持的架构: $(uname -m)" ;;
  esac
  printf '%s-%s\n' "$os" "$arch"
}

_fetch_latest_version() {
  require_curl
  local out ver
  out="$(_nlt_github_download_curl -fsSL "https://api.github.com/repos/coder/code-server/releases/latest")" || return 1
  ver="$(printf '%s' "$out" | sed -n 's/.*"tag_name": *"v\([0-9][0-9.]*\)".*/\1/p' | head -1)"
  if [[ -n "$ver" ]]; then
    echo "$ver"
    return 0
  fi
  return 1
}

_resolve_version() {
  if [[ -n "${CODE_SERVER_VERSION:-}" ]]; then
    echo "${CODE_SERVER_VERSION}"
    return
  fi
  local v
  v="$(_fetch_latest_version)" || true
  if [[ -n "$v" ]]; then
    echo "$v"
    return
  fi
  echo "4.112.0"
}

_download_install() {
  require_curl
  command -v tar >/dev/null 2>&1 || die "需要 tar"
  local plat ver url tmpdir
  plat="$(_detect_platform)"
  ver="$(_resolve_version)"
  url="https://github.com/coder/code-server/releases/download/v${ver}/code-server-${ver}-${plat}.tar.gz"
  echo "==> 下载 code-server v${ver} (${plat})" >&2
  echo "    ${url}" >&2
  tmpdir="$(mktemp -d)"
  trap '[[ -n "${tmpdir-}" ]] && rm -rf "${tmpdir}"' RETURN
  _nlt_github_download_print_accel_hint
  if declare -F nlt_pb_curl_to_file >/dev/null 2>&1; then
    NLT_PB_LABEL="code-server v${ver}" nlt_pb_curl_to_file "$url" "${tmpdir}/code-server.tgz" || die "下载失败: ${url}"
  else
    _nlt_github_download_curl -fsSL "$url" -o "${tmpdir}/code-server.tgz"
  fi
  rm -rf "${CODE_SERVER_SERVICE_HOME}/lib" "${CODE_SERVER_SERVICE_HOME}/bin" 2>/dev/null || true
  mkdir -p "${CODE_SERVER_SERVICE_HOME}"
  tar -xzf "${tmpdir}/code-server.tgz" -C "${CODE_SERVER_SERVICE_HOME}" --strip-components=1
  rm -rf "${tmpdir}"
  trap - RETURN
  [[ -x "${CODE_SERVER_BIN}" ]] || die "解压后未找到可执行文件: ${CODE_SERVER_BIN}"
  echo "已安装到 ${CODE_SERVER_SERVICE_HOME}（v${ver}）"
}

cmd_install() {
  ensure_dirs
  _download_install
}

cmd_update() {
  ensure_dirs
  echo "==> 更新 code-server（重新下载）…" >&2
  _download_install
}

cmd_start() {
  [[ -x "${CODE_SERVER_BIN}" ]] || die "未安装，请先: $0 install"
  ensure_dirs
  local existing
  existing="$(read_pid)"
  if [[ -n "$existing" ]] && process_alive "$existing"; then
    echo "code-server 已在运行（PID ${existing}）。重启请: $0 restart" >&2
    exit 1
  fi
  local listener_pid
  listener_pid="$(listener_pid_for_port "${CODE_SERVER_PORT}")"
  if [[ -n "$listener_pid" ]]; then
    die "端口 ${CODE_SERVER_PORT} 已被 PID ${listener_pid} 占用，请先执行 $0 stop 或手动清理。"
  fi
  rm -f "$PID_FILE"
  echo "==> 启动 code-server，绑定 ${CODE_SERVER_BIND}，日志: ${LOG_FILE}" >&2
  pushd "${HOME}" >/dev/null
  if [[ -n "${PASSWORD:-}" ]]; then
    nohup env PASSWORD="${PASSWORD}" "${CODE_SERVER_BIN}" --bind-addr "${CODE_SERVER_BIND}" >>"${LOG_FILE}" 2>&1 &
  else
    nohup "${CODE_SERVER_BIN}" --bind-addr "${CODE_SERVER_BIND}" >>"${LOG_FILE}" 2>&1 &
  fi
  local cpid=$!
  echo "$cpid" >"$PID_FILE"
  popd >/dev/null
  sleep 1
  existing="$(read_pid)"
  if [[ -n "$existing" ]] && process_alive "$existing"; then
    echo "已启动 PID ${existing}（密码见日志或未设置 PASSWORD 时终端提示）"
  else
    echo "警告: 进程可能已退出，请查看: tail -80 ${LOG_FILE}" >&2
  fi
}

cmd_run() {
  [[ -x "${CODE_SERVER_BIN}" ]] || die "未安装，请先: $0 install"
  ensure_dirs
  local existing
  existing="$(read_pid)"
  if [[ -n "$existing" ]] && process_alive "$existing"; then
    echo "code-server 已在后台运行（PID ${existing}）。请先 $0 stop，再使用 run。" >&2
    exit 1
  fi
  echo "==> 前台启动 code-server，绑定 ${CODE_SERVER_BIND}（Ctrl+C 退出；不写 PID）" >&2
  pushd "${HOME}" >/dev/null
  if [[ -n "${PASSWORD:-}" ]]; then
    exec env PASSWORD="${PASSWORD}" "${CODE_SERVER_BIN}" --bind-addr "${CODE_SERVER_BIND}"
  fi
  exec "${CODE_SERVER_BIN}" --bind-addr "${CODE_SERVER_BIND}"
}

cmd_stop() {
  local pid
  pid="$(read_pid)"
  local listener_pid
  listener_pid="$(listener_pid_for_port "${CODE_SERVER_PORT}")"
  if [[ -z "$pid" ]]; then
    echo "未找到 PID，视为未运行。" >&2
    rm -f "$PID_FILE"
  elif ! process_alive "$pid"; then
    rm -f "$PID_FILE"
  fi
  if [[ -n "$pid" || -n "$listener_pid" ]] && [[ "${NONINTERACTIVE:-}" != "1" ]] && [[ -t 0 ]]; then
    gum confirm "停止 code-server（PID ${pid}）？" || exit 0
  fi
  if [[ -n "$pid" ]] && process_alive "$pid"; then
    kill -TERM "$pid" 2>/dev/null || true
    local w=0
    while process_alive "$pid" && (( w < 20 )); do
      sleep 1
      w=$((w + 1))
    done
    process_alive "$pid" && kill -KILL "$pid" 2>/dev/null || true
  fi
  if [[ -n "$listener_pid" ]] && { [[ -z "$pid" ]] || [[ "$listener_pid" != "$pid" ]]; }; then
    kill -TERM "$listener_pid" 2>/dev/null || true
  fi
  rm -f "$PID_FILE"
  echo "已停止。"
}

cmd_restart() {
  cmd_stop || true
  cmd_start
}

cmd_status() {
  local pid
  pid="$(read_pid)"
  local listener_pid
  listener_pid="$(listener_pid_for_port "${CODE_SERVER_PORT}")"
  echo "CODE_SERVER_SERVICE_HOME=${CODE_SERVER_SERVICE_HOME}"
  echo "CODE_SERVER_BIND=${CODE_SERVER_BIND}"
  if [[ -n "$pid" ]] && process_alive "$pid"; then
    if [[ -n "$listener_pid" && "$listener_pid" != "$pid" ]]; then
      echo "状态: 运行中 PID ${pid}（监听进程 PID ${listener_pid}）"
    else
      echo "状态: 运行中 PID ${pid}"
    fi
  elif [[ -n "$listener_pid" ]]; then
    echo "状态: 监听中（PID 文件缺失/失效，监听进程 PID ${listener_pid}）"
  else
    echo "状态: 未运行"
    rm -f "$PID_FILE"
  fi
  if [[ -x "${CODE_SERVER_BIN}" ]]; then
    echo "二进制: $("${CODE_SERVER_BIN}" --version 2>/dev/null | head -1 || echo ok)"
  fi
  if command -v curl >/dev/null 2>&1; then
    echo ""
    echo "==> 探测 http://127.0.0.1:${CODE_SERVER_PORT}/"
    curl -sS -m 3 -o /dev/null -w "HTTP %{http_code}\n" "http://127.0.0.1:${CODE_SERVER_PORT}/" || echo "（无法连接）"
  fi
}

cmd_uninstall() {
  cmd_stop || true
  echo "将删除: ${CODE_SERVER_SERVICE_HOME}" >&2
  if [[ -t 0 ]]; then
    gum confirm "确认删除？" || exit 0
  else
    [[ "${CODE_SERVER_UNINSTALL_YES:-}" == "1" ]] || die "非交互请设 CODE_SERVER_UNINSTALL_YES=1"
  fi
  local hp ap
  hp="$(cd "$HOME" && pwd -P)"
  ap="$(cd "${CODE_SERVER_SERVICE_HOME}" 2>/dev/null && pwd -P)" || ap="${CODE_SERVER_SERVICE_HOME}"
  if [[ "$ap" == "/" || "$ap" == "$hp" ]]; then
    die "拒绝删除根目录或 \$HOME"
  fi
  rm -rf "${CODE_SERVER_SERVICE_HOME}"
  echo "已删除。用户配置可能在 ~/.config/code-server"
}

dispatch() {
  local cmd="${1:-}"
  shift || true
  case "$cmd" in
    install) cmd_install ;;
    update) cmd_update ;;
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
  _nlt_ensure_gum || exit 1
  declare -F nlt_ui_apply_theme >/dev/null 2>&1 && nlt_ui_apply_theme
  if declare -F nlt_ui_banner >/dev/null 2>&1; then
    nlt_ui_banner "nltdeploy / service / code-server / manual" "本地安装 · ${CODE_SERVER_SERVICE_HOME} · ${CODE_SERVER_BIND}"
  else
    gum style --bold --foreground 212 "code-server 本地服务（coder/code-server）"
    gum style "安装目录: ${CODE_SERVER_SERVICE_HOME}"
    gum style "绑定: ${CODE_SERVER_BIND}"
  fi
  echo ""
  set +e
  while true; do
    local pick
    pick="$(nlt_ui_choose "nltdeploy / service / code-server / manual / 选择动作" \
      "install" "update" "start" "run" "stop" "restart" "status" "uninstall" "help" "quit")" || break
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
    interactive_main
    return 0
  fi
  dispatch "$@"
}

main "$@"
