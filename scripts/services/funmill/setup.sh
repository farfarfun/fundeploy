#!/usr/bin/env bash
# funmill：任务执行 API（PyPI 包 funmill-api，命令 funmill），可选编排 Dagu 后端。
#
# 上游: https://github.com/farfarfun/funmill-dev（apps/funmill-api）
#
# funmill 自身的 CLI 已经管理好自己的后台生命周期（start/stop/status/restart
# 都会把 PID/日志写到 ~/.farfarfun/funmill/services/<name>/），本脚本不重复
# 维护 PID 文件，只负责「装包 + 按依赖顺序编排调用 + 端口探测判断是否已在运
# 行」。funmill 的端口固定（API 8812、第三方后端 8813，见其 README），CLI 不
# 支持 --host/--port 覆盖，所以这里也不做 host/port 透传。
#
# 用法：
#   ./setup.sh                 # gum 菜单
#   ./setup.sh install         # pip/uv 装 funmill-api；FUNMILL_BACKEND != none 时再装该后端
#   ./setup.sh upgrade         # 同 install（重新安装到最新/指定版本），并打印升级前后版本对比
#   ./setup.sh start           # 先启动后端（默认 dagu），再启动 Funmill API
#   ./setup.sh stop            # 先停止 API，再停止后端
#   ./setup.sh restart         # stop + start
#   ./setup.sh status          # 依次打印 API / 后端各自的状态
#   ./setup.sh uninstall       # 停止两者，卸载 pip 包（不清理 ~/.farfarfun/funmill/）
#
# 环境变量：
#   FUNMILL_PACKAGE          pip 包名（默认 funmill-api）
#   FUNMILL_VERSION          版本号（默认空＝最新）
#   FUNMILL_PIP_BIN          指定 pip/uv 可执行路径（默认自动探测：优先 uv，否则 python3 -m pip）
#   FUNMILL_BACKEND          编排的第三方后端（默认 dagu；设为 none 跳过后端编排，
#                            适用于已用 windmill 或后端由别处管理的场景）
#   FUNMILL_API_PORT         Funmill API 端口，仅用于本脚本探测（默认 8812，与上游固定值一致）
#   FUNMILL_BACKEND_PORT     第三方后端端口，仅用于本脚本探测（默认 8813，与上游固定值一致）
#   FUNMILL_API_KEY          必需；Funmill 强制要求 X-API-Key 鉴权，start 前必须 export
#   NONINTERACTIVE=1
#   FUNMILL_UNINSTALL_YES=1  非 TTY 卸载确认

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../../lib/fundeploy-common.sh
source "${SCRIPT_DIR}/../../lib/fundeploy-common.sh"

FUNMILL_PACKAGE="${FUNMILL_PACKAGE:-funmill-api}"
FUNMILL_VERSION="${FUNMILL_VERSION:-}"
FUNMILL_PIP_BIN="${FUNMILL_PIP_BIN:-}"
FUNMILL_BACKEND="${FUNMILL_BACKEND:-dagu}"
FUNMILL_API_PORT="${FUNMILL_API_PORT:-8812}"
FUNMILL_BACKEND_PORT="${FUNMILL_BACKEND_PORT:-8813}"

die() { echo "错误: $*" >&2; exit 1; }

usage() {
  cat <<USAGE
用法: ./setup.sh [command]

  无参数：gum 菜单。

命令:
  install            pip/uv 装 ${FUNMILL_PACKAGE}；FUNMILL_BACKEND=${FUNMILL_BACKEND} 时再装该后端
  upgrade            同 install，并打印升级前后的版本对比
  start              按序启动：先后端（${FUNMILL_BACKEND}），再 Funmill API
  stop               按序停止：先 API，再后端（best effort，不因某一端未运行而报错）
  restart            stop + start
  status             依次打印 API / 后端各自的状态
  uninstall          停止两者，卸载 pip 包（不清理 ~/.farfarfun/funmill/，需要手动删除）

说明:
  - API 与后端各自的 PID/日志由 funmill 自己的 CLI 管理，见
    ~/.farfarfun/funmill/services/<api|${FUNMILL_BACKEND}>/，本脚本不重复维护。
  - API: http://127.0.0.1:${FUNMILL_API_PORT}（接口文档 /docs）
  - 后端: http://127.0.0.1:${FUNMILL_BACKEND_PORT}
  - 鉴权: start 前必须 export FUNMILL_API_KEY=...，没有默认值。
  - 只提供合并命令；如需单独控制某一端，直接用 funmill 的 CLI
    （funmill start/stop/status [${FUNMILL_BACKEND}]）。

上游: https://github.com/farfarfun/funmill-dev
USAGE
}

_require_cli() {
  local bin="$1" hint="$2"
  command -v "${bin}" >/dev/null 2>&1 || die "未找到 ${bin}（${hint}），请先: ./setup.sh install"
}

_pip_pkg_spec() {
  local pkg="$1" version="$2"
  if [[ -z "${version}" ]]; then
    printf '%s' "${pkg}"
  else
    printf '%s==%s' "${pkg}" "${version#v}"
  fi
}

_pip_install_pkg() {
  local spec="$1"
  if [[ -n "${FUNMILL_PIP_BIN}" ]]; then
    "${FUNMILL_PIP_BIN}" install "${spec}"
    return
  fi
  if command -v uv >/dev/null 2>&1; then
    uv pip install "${spec}" -U
    return
  fi
  command -v python3 >/dev/null 2>&1 || die "未找到 python3/pip/uv（可先运行 fundeploy dev uv install）"
  python3 -m pip install --user "${spec}" -U
}

_pip_uninstall_pkg() {
  local pkg="$1"
  if [[ -n "${FUNMILL_PIP_BIN}" ]]; then
    "${FUNMILL_PIP_BIN}" uninstall -y "${pkg}" || echo "警告: 卸载 ${pkg} 失败" >&2
    return
  fi
  if command -v uv >/dev/null 2>&1; then
    uv pip uninstall "${pkg}" || echo "警告: 卸载 ${pkg} 失败" >&2
    return
  fi
  if command -v python3 >/dev/null 2>&1; then
    python3 -m pip uninstall -y "${pkg}" || echo "警告: 卸载 ${pkg} 失败" >&2
    return
  fi
  echo "警告: 未找到 pip/uv，无法卸载 ${pkg}" >&2
}

# 已安装的 pip 包版本；镜像 _pip_install_pkg/_pip_uninstall_pkg 的探测优先级
# （FUNMILL_PIP_BIN > uv > python3 -m pip），未安装时输出空字符串。
_pip_pkg_version() {
  local pkg="$1" out
  if [[ -n "${FUNMILL_PIP_BIN}" ]]; then
    out="$("${FUNMILL_PIP_BIN}" show "${pkg}" 2>/dev/null || true)"
  elif command -v uv >/dev/null 2>&1; then
    out="$(uv pip show "${pkg}" 2>/dev/null || true)"
  elif command -v python3 >/dev/null 2>&1; then
    out="$(python3 -m pip show "${pkg}" 2>/dev/null || true)"
  else
    out=""
  fi
  printf '%s' "${out}" | sed -n 's/^Version: *//p' | head -1
}

cmd_install() {
  local spec
  spec="$(_pip_pkg_spec "${FUNMILL_PACKAGE}" "${FUNMILL_VERSION}")"
  echo "==> 安装 ${spec}"
  _pip_install_pkg "${spec}" || die "安装失败: ${spec}"
  if [[ "${FUNMILL_BACKEND}" != "none" ]]; then
    echo "==> 安装后端 ${FUNMILL_BACKEND}（funmill install ${FUNMILL_BACKEND}）"
    funmill install "${FUNMILL_BACKEND}" || die "后端 ${FUNMILL_BACKEND} 安装失败"
  fi
  echo "已安装。"
}

cmd_upgrade() {
  local before after
  before="$(_pip_pkg_version "${FUNMILL_PACKAGE}")"

  cmd_install

  after="$(_pip_pkg_version "${FUNMILL_PACKAGE}")"

  echo ""
  echo "== 版本变化 =="
  echo "${FUNMILL_PACKAGE}: ${before:-未安装} -> ${after:-未知}"
}

# start 在目标已经运行时，上游 CLI 会返回非零退出码（"already running"）；这
# 里先看端口是否已被监听，已在运行就跳过、不当成失败。
_wait_for_listener() {
  local port="$1" timeout="${2:-8}" waited=0
  while (( waited < timeout )); do
    [[ -n "$(_fundeploy_listener_pid_for_port "${port}")" ]] && return 0
    sleep 1
    waited=$((waited + 1))
  done
  return 1
}

cmd_start() {
  _require_cli funmill "pip/uv 安装 ${FUNMILL_PACKAGE} 后应在 PATH 中"
  : "${FUNMILL_API_KEY:?请先 export FUNMILL_API_KEY=...（Funmill 强制要求 X-API-Key 鉴权，不提供默认值）}"

  if [[ "${FUNMILL_BACKEND}" != "none" ]]; then
    if [[ -n "$(_fundeploy_listener_pid_for_port "${FUNMILL_BACKEND_PORT}")" ]]; then
      echo "后端 ${FUNMILL_BACKEND}（端口 ${FUNMILL_BACKEND_PORT}）已在运行，跳过。"
    else
      echo "==> 启动后端 ${FUNMILL_BACKEND}"
      funmill start "${FUNMILL_BACKEND}" || die "后端 ${FUNMILL_BACKEND} 启动失败，已中止（Funmill API 未启动）"
      _wait_for_listener "${FUNMILL_BACKEND_PORT}" \
        || die "后端 ${FUNMILL_BACKEND} 启动命令已返回，但端口 ${FUNMILL_BACKEND_PORT} 迟迟未监听（多半是启动后崩溃）。已中止（Funmill API 未启动）。日志见 ~/.farfarfun/funmill/services/${FUNMILL_BACKEND}/。"
    fi
  fi

  if [[ -n "$(_fundeploy_listener_pid_for_port "${FUNMILL_API_PORT}")" ]]; then
    echo "Funmill API（端口 ${FUNMILL_API_PORT}）已在运行，跳过。"
  else
    echo "==> 启动 Funmill API"
    FUNMILL_BACKEND="${FUNMILL_BACKEND}" funmill start \
      || die "Funmill API 启动失败（后端已启动；如需回滚请执行 ./setup.sh stop）"
    _wait_for_listener "${FUNMILL_API_PORT}" \
      || die "Funmill API 启动命令已返回，但端口 ${FUNMILL_API_PORT} 迟迟未监听（多半是启动后崩溃）。日志见 ~/.farfarfun/funmill/services/api/，或跑 funmill run 看前台报错。"
  fi

  echo "已启动。接口: http://127.0.0.1:${FUNMILL_API_PORT}（文档 /docs）"
}

# best effort：某一端未安装/未运行不算错误，只提示，不阻断另一端的停止。
cmd_stop() {
  if command -v funmill >/dev/null 2>&1; then
    echo "==> 停止 Funmill API"
    funmill stop || echo "警告: Funmill API 停止失败或未在运行" >&2
    if [[ "${FUNMILL_BACKEND}" != "none" ]]; then
      echo "==> 停止后端 ${FUNMILL_BACKEND}"
      funmill stop "${FUNMILL_BACKEND}" || echo "警告: 后端 ${FUNMILL_BACKEND} 停止失败或未在运行" >&2
    fi
  else
    echo "未安装，跳过。"
  fi
  echo "已停止（best effort）。"
}

cmd_restart() {
  cmd_stop
  cmd_start
}

cmd_status() {
  echo "== Funmill API（127.0.0.1:${FUNMILL_API_PORT}）=="
  if command -v funmill >/dev/null 2>&1; then
    funmill status || true
  else
    echo "未安装（./setup.sh install 或: pip/uv install ${FUNMILL_PACKAGE}）"
  fi
  if [[ "${FUNMILL_BACKEND}" != "none" ]]; then
    echo ""
    echo "== 后端 ${FUNMILL_BACKEND}（127.0.0.1:${FUNMILL_BACKEND_PORT}）=="
    if command -v funmill >/dev/null 2>&1; then
      funmill status "${FUNMILL_BACKEND}" || true
    else
      echo "未安装"
    fi
  fi
}

cmd_uninstall() {
  fundeploy_confirm_destructive \
    "将停止并卸载 ${FUNMILL_PACKAGE}（pip/uv）。不会清理 ~/.farfarfun/funmill/（含已下载的 ${FUNMILL_BACKEND} 二进制/数据），如需彻底清理请手动删除该目录。确认？" \
    FUNMILL_UNINSTALL_YES || return 1

  cmd_stop
  echo "==> 卸载 pip 包 ${FUNMILL_PACKAGE}"
  _pip_uninstall_pkg "${FUNMILL_PACKAGE}"
  echo "已卸载。"
}

interactive_main() {
  _fundeploy_ensure_gum || exit 1
  declare -F fundeploy_ui_apply_theme >/dev/null 2>&1 && fundeploy_ui_apply_theme
  if declare -F fundeploy_ui_banner >/dev/null 2>&1; then
    fundeploy_ui_banner "fundeploy / service / funmill" \
      "API 127.0.0.1:${FUNMILL_API_PORT}  后端 ${FUNMILL_BACKEND} 127.0.0.1:${FUNMILL_BACKEND_PORT}"
  fi
  set +e
  while true; do
    local pick
    pick="$(fundeploy_ui_choose "fundeploy / service / funmill / 选择动作" \
      "install    安装（pip + 可选后端）" \
      "upgrade    更新到最新/指定版本，并显示升级前后版本" \
      "start      启动（后端 → API）" \
      "stop       停止（API → 后端）" \
      "restart    重启" \
      "status     查看状态" \
      "uninstall  卸载" \
      "help       命令帮助" \
      "quit       返回")" || break
    [[ -z "$pick" ]] && break
    pick="${pick%% *}"
    case "$pick" in
      quit) break ;;
      help) usage ;;
      install) cmd_install ;;
      upgrade) cmd_upgrade ;;
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
    if [[ "${NONINTERACTIVE:-}" == "1" ]]; then
      usage >&2
      exit 1
    fi
    interactive_main
    return 0
  fi
  case "$cmd" in
    install) cmd_install ;;
    update|upgrade) cmd_upgrade ;;
    start) cmd_start ;;
    stop) cmd_stop ;;
    restart) cmd_restart ;;
    status) cmd_status ;;
    uninstall) cmd_uninstall ;;
    help | -h | --help) usage ;;
    *)
      echo "未知命令: $cmd" >&2
      usage >&2
      exit 2
      ;;
  esac
}

main "$@"
