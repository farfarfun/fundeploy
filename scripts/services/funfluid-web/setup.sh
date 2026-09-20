#!/usr/bin/env bash
# funfluid-web 一体化部署：把 funfluid-api（后端，私有 PyPI 包）与
# @farfarfun/funfluid-web（前端，私有 npm 包，bin 名 funfluid-web）作为同一个
# 服务单元来装/起/停——两者本是各自独立的进程，但这里只暴露合并命令，不提供
# 拆开单独控制前后端的子命令；如需单独控制，直接用各自的 CLI
# （funfluid-api / funfluid-web）。
#
# 上游:
#   后端 https://github.com/farfarfun/funfluid（apps/funfluid-api，包名 funfluid-api）
#   前端 https://github.com/farfarfun/funfluid（apps/funfluid-web，
#        包名 @farfarfun/funfluid-web，bin 名 funfluid-web）
#
# 前后端各自已提供健壮的生命周期命令（各自管理自己的 PID/日志），本脚本只负责
# 「装两个包 + 按依赖顺序编排调用」，不重复维护 PID 文件。两边 CLI 都是顶层
# 命令（没有 server 子分组），但语法不同：funfluid-api 是
# `start/stop/restart/status <service>`（stop 恒传 all，否则自愈 watch 循环
# 会把进程重新拉起来）；funfluid-web 是 `start/restart --port/--host -d`、
# 不带参数的 `stop`/`status`。
#
# 依赖: python3 + pip（或 uv）装后端；npm 装前端。
#
# 用法：
#   ./setup.sh                 # gum 菜单
#   ./setup.sh install         # pip/uv 装 funfluid-api + npm -g 装 funfluid-web
#   ./setup.sh update          # 同 install（重新安装到最新/指定版本），并打印升级前后版本对比
#   ./setup.sh start           # 先启动后端，再启动前端
#   ./setup.sh stop            # 先停止前端，再停止后端
#   ./setup.sh restart         # stop + start
#   ./setup.sh status          # 依次打印 funfluid-api / funfluid-web 各自的状态
#   ./setup.sh uninstall       # 停止两者，卸载 npm 包与 pip 包
#
# 环境变量：
#   FUNFLUID_WEB_BACKEND_PACKAGE     后端 pip 包名（默认 funfluid-api）
#   FUNFLUID_WEB_BACKEND_VERSION     后端版本号（默认空＝最新）
#   FUNFLUID_WEB_BACKEND_SERVICE     后端目标 service（默认 gunicorn；如需连带
#                                    celery/beat/flower 可设为 all）
#   FUNFLUID_WEB_BACKEND_HOST        后端展示/探测用地址（默认 127.0.0.1）
#   FUNFLUID_WEB_BACKEND_PORT        后端展示/探测用端口（默认 18806，与后端
#                                    config.yml 里 HTTP_LISTEN_PORT 的默认值
#                                    一致；不会作为参数传给 CLI）
#   FUNFLUID_WEB_FRONTEND_PACKAGE    前端 npm 包名（默认 @farfarfun/funfluid-web）
#   FUNFLUID_WEB_FRONTEND_VERSION    前端版本号（默认空＝最新）
#   FUNFLUID_WEB_FRONTEND_HOST       前端监听地址（默认 0.0.0.0，会传给 --host）
#   FUNFLUID_WEB_FRONTEND_PORT       前端监听端口（默认 8806，会传给 --port）
#   FUNFLUID_WEB_PIP_BIN             指定 pip/uv 可执行路径（默认自动探测：优先 uv，否则 python3 -m pip）
#   FUNFLUID_WEB_NPM_BIN             指定 npm 可执行路径（默认从 PATH 查找）
#   NONINTERACTIVE=1
#   FUNFLUID_WEB_UNINSTALL_YES=1     非 TTY 卸载确认

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../../lib/fundeploy-common.sh
source "${SCRIPT_DIR}/../../lib/fundeploy-common.sh"

FUNFLUID_WEB_BACKEND_PACKAGE="${FUNFLUID_WEB_BACKEND_PACKAGE:-funfluid-api}"
FUNFLUID_WEB_BACKEND_VERSION="${FUNFLUID_WEB_BACKEND_VERSION:-}"
FUNFLUID_WEB_BACKEND_SERVICE="${FUNFLUID_WEB_BACKEND_SERVICE:-gunicorn}"
FUNFLUID_WEB_BACKEND_HOST="${FUNFLUID_WEB_BACKEND_HOST:-127.0.0.1}"
FUNFLUID_WEB_BACKEND_PORT="${FUNFLUID_WEB_BACKEND_PORT:-18806}"
FUNFLUID_WEB_FRONTEND_PACKAGE="${FUNFLUID_WEB_FRONTEND_PACKAGE:-@farfarfun/funfluid-web}"
FUNFLUID_WEB_FRONTEND_VERSION="${FUNFLUID_WEB_FRONTEND_VERSION:-}"
FUNFLUID_WEB_FRONTEND_HOST="${FUNFLUID_WEB_FRONTEND_HOST:-0.0.0.0}"
FUNFLUID_WEB_FRONTEND_PORT="${FUNFLUID_WEB_FRONTEND_PORT:-8806}"
FUNFLUID_WEB_PIP_BIN="${FUNFLUID_WEB_PIP_BIN:-}"
FUNFLUID_WEB_NPM_BIN="${FUNFLUID_WEB_NPM_BIN:-}"

die() { echo "错误: $*" >&2; exit 1; }

usage() {
  cat <<USAGE
用法: ./setup.sh [command]

  无参数：gum 菜单。

命令:
  install            pip/uv 装 ${FUNFLUID_WEB_BACKEND_PACKAGE}，npm -g 装 ${FUNFLUID_WEB_FRONTEND_PACKAGE}
  update             同 install，并打印升级前后的版本对比
  start              按序启动：先 funfluid-api 后端，再 funfluid-web 前端
  stop               按序停止：先前端，再后端（best effort，不因某一端未运行而报错）
  restart            stop + start
  status             依次打印 funfluid-api / funfluid-web 的状态
  uninstall          停止两者，卸载 npm 包与 pip 包

说明:
  - 前后端是两个独立进程，各自的 PID/日志由上游 CLI 自己管理，本脚本不重复维护。
  - 后端: http://${FUNFLUID_WEB_BACKEND_HOST}:${FUNFLUID_WEB_BACKEND_PORT}（接口文档 /api/docs/swagger/）
    端口来自安装后 ~/.farfarfun/funfluid/backend/config.yml 的 HTTP_LISTEN_PORT，
    不是 CLI 参数；改端口需要编辑该配置文件后 restart。
  - 前端: http://${FUNFLUID_WEB_FRONTEND_HOST}:${FUNFLUID_WEB_FRONTEND_PORT}（浏览器打开这个）
  - 只提供合并命令；如需单独控制某一端，直接用 funfluid-api / funfluid-web 各自的 CLI
    （两者都是顶层命令 start/stop/restart/status，没有 server 子分组；
    funfluid-api 的 stop 恒传 all，否则自愈 watch 循环会把进程重新拉起来）

上游: https://github.com/farfarfun/funfluid
USAGE
}

_require_cli() {
  local bin="$1" hint="$2"
  command -v "${bin}" >/dev/null 2>&1 || die "未找到 ${bin}（${hint}），请先: ./setup.sh install"
}

_resolve_npm() {
  if [[ -n "${FUNFLUID_WEB_NPM_BIN}" ]]; then
    [[ -x "${FUNFLUID_WEB_NPM_BIN}" ]] || die "FUNFLUID_WEB_NPM_BIN 无效: ${FUNFLUID_WEB_NPM_BIN}"
    echo "${FUNFLUID_WEB_NPM_BIN}"
    return
  fi
  command -v npm >/dev/null 2>&1 || die "未找到 npm（可先运行 fundeploy dev nodejs install）"
  command -v npm
}

_npm_pkg_spec() {
  local pkg="$1" version="$2"
  if [[ -z "${version}" ]]; then
    printf '%s@latest' "${pkg}"
  else
    printf '%s@%s' "${pkg}" "${version#v}"
  fi
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
  if [[ -n "${FUNFLUID_WEB_PIP_BIN}" ]]; then
    "${FUNFLUID_WEB_PIP_BIN}" install "${spec}"
    return
  fi
  if command -v uv >/dev/null 2>&1; then
    uv pip install "${spec}"
    return
  fi
  command -v python3 >/dev/null 2>&1 || die "未找到 python3/pip/uv（可先运行 fundeploy dev uv install）"
  python3 -m pip install --user "${spec}"
}

_pip_uninstall_pkg() {
  local pkg="$1"
  if [[ -n "${FUNFLUID_WEB_PIP_BIN}" ]]; then
    "${FUNFLUID_WEB_PIP_BIN}" uninstall -y "${pkg}" || echo "警告: 卸载 ${pkg} 失败" >&2
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
# （FUNFLUID_WEB_PIP_BIN > uv > python3 -m pip），未安装时输出空字符串。
_pip_pkg_version() {
  local pkg="$1" out
  if [[ -n "${FUNFLUID_WEB_PIP_BIN}" ]]; then
    out="$("${FUNFLUID_WEB_PIP_BIN}" show "${pkg}" 2>/dev/null || true)"
  elif command -v uv >/dev/null 2>&1; then
    out="$(uv pip show "${pkg}" 2>/dev/null || true)"
  elif command -v python3 >/dev/null 2>&1; then
    out="$(python3 -m pip show "${pkg}" 2>/dev/null || true)"
  else
    out=""
  fi
  printf '%s' "${out}" | sed -n 's/^Version: *//p' | head -1
}

# 已安装的 npm 全局包版本，未安装时输出空字符串。
_npm_pkg_version() {
  local pkg="$1" npm_bin json
  npm_bin="$(_resolve_npm 2>/dev/null)" || { echo ""; return; }
  json="$("${npm_bin}" ls -g "${pkg}" --depth=0 --json 2>/dev/null || true)"
  [[ -n "${json}" ]] || { echo ""; return; }
  if command -v python3 >/dev/null 2>&1; then
    FUNFLUID_WEB_PKG_QUERY="${pkg}" python3 -c '
import json, os, sys
data = json.loads(sys.stdin.read() or "{}")
pkg = os.environ.get("FUNFLUID_WEB_PKG_QUERY", "")
deps = data.get("dependencies") or {}
info = deps.get(pkg) or {}
print(info.get("version") or "")
' <<<"${json}"
  else
    # 无 python3 时的兜底：包名可能形如 @scope/name，分隔符须用 # 而非 /
    printf '%s' "${json}" | sed -n "s#.*\"${pkg}\": *{[^}]*\"version\": *\"\\([^\"]*\\)\".*#\\1#p" | head -1
  fi
}

cmd_install() {
  local npm_bin backend_spec frontend_spec
  npm_bin="$(_resolve_npm)"
  backend_spec="$(_pip_pkg_spec "${FUNFLUID_WEB_BACKEND_PACKAGE}" "${FUNFLUID_WEB_BACKEND_VERSION}")"
  frontend_spec="$(_npm_pkg_spec "${FUNFLUID_WEB_FRONTEND_PACKAGE}" "${FUNFLUID_WEB_FRONTEND_VERSION}")"
  echo "==> 安装后端: ${backend_spec}"
  _pip_install_pkg "${backend_spec}" || die "后端安装失败: ${backend_spec}"
  echo "==> 安装前端: ${npm_bin} install -g ${frontend_spec}"
  "${npm_bin}" install -g "${frontend_spec}" || die "前端安装失败: ${frontend_spec}"
  echo "已安装。"
}

cmd_update() {
  local backend_before backend_after frontend_before frontend_after
  backend_before="$(_pip_pkg_version "${FUNFLUID_WEB_BACKEND_PACKAGE}")"
  frontend_before="$(_npm_pkg_version "${FUNFLUID_WEB_FRONTEND_PACKAGE}")"

  cmd_install

  backend_after="$(_pip_pkg_version "${FUNFLUID_WEB_BACKEND_PACKAGE}")"
  frontend_after="$(_npm_pkg_version "${FUNFLUID_WEB_FRONTEND_PACKAGE}")"

  echo ""
  echo "== 版本变化 =="
  echo "后端 ${FUNFLUID_WEB_BACKEND_PACKAGE}: ${backend_before:-未安装} -> ${backend_after:-未知}"
  echo "前端 ${FUNFLUID_WEB_FRONTEND_PACKAGE}: ${frontend_before:-未安装} -> ${frontend_after:-未知}"
}

# start 只要成功 fork 出子进程就会返回 0；子进程若在几秒内因异常退出（比如
# 上游包自身的 bug），start 命令本身并不会失败。这里等端口真正被监听到，
# 避免把「已启动」误报给用户。
_wait_for_listener() {
  local host="$1" port="$2" timeout="${3:-8}" waited=0
  while (( waited < timeout )); do
    [[ -n "$(_fundeploy_listener_pid_for_port "${port}")" ]] && return 0
    sleep 1
    waited=$((waited + 1))
  done
  return 1
}

cmd_start() {
  _require_cli funfluid-api "pip/uv 安装 ${FUNFLUID_WEB_BACKEND_PACKAGE} 后应在 PATH 中"
  _require_cli funfluid-web "npm -g 安装 ${FUNFLUID_WEB_FRONTEND_PACKAGE} 后应在 PATH 中"

  if [[ -n "$(_fundeploy_listener_pid_for_port "${FUNFLUID_WEB_BACKEND_PORT}")" ]]; then
    echo "后端 ${FUNFLUID_WEB_BACKEND_HOST}:${FUNFLUID_WEB_BACKEND_PORT} 已在运行，跳过。"
  else
    echo "==> 启动后端 funfluid-api（service=${FUNFLUID_WEB_BACKEND_SERVICE}），预期监听 ${FUNFLUID_WEB_BACKEND_HOST}:${FUNFLUID_WEB_BACKEND_PORT}"
    funfluid-api start "${FUNFLUID_WEB_BACKEND_SERVICE}" -d \
      || die "后端启动失败，已中止（前端未启动）"
    _wait_for_listener "${FUNFLUID_WEB_BACKEND_HOST}" "${FUNFLUID_WEB_BACKEND_PORT}" \
      || die "后端启动命令已返回，但端口 ${FUNFLUID_WEB_BACKEND_PORT} 迟迟未监听（多半是启动后崩溃，或 config.yml 里 HTTP_LISTEN_PORT 与 FUNFLUID_WEB_BACKEND_PORT 不一致）。" \
             "已中止（前端未启动）。请看日志: ~/.farfarfun/funfluid/backend/，或跑 funfluid-api run ${FUNFLUID_WEB_BACKEND_SERVICE} 看前台报错。"
  fi

  if [[ -n "$(_fundeploy_listener_pid_for_port "${FUNFLUID_WEB_FRONTEND_PORT}")" ]]; then
    echo "前端 ${FUNFLUID_WEB_FRONTEND_HOST}:${FUNFLUID_WEB_FRONTEND_PORT} 已在运行，跳过。"
  else
    echo "==> 启动前端 funfluid-web，监听 ${FUNFLUID_WEB_FRONTEND_HOST}:${FUNFLUID_WEB_FRONTEND_PORT}"
    funfluid-web start --port "${FUNFLUID_WEB_FRONTEND_PORT}" --host "${FUNFLUID_WEB_FRONTEND_HOST}" -d \
      || die "前端启动失败（后端已启动；如需回滚请执行 ./setup.sh stop）"
    _wait_for_listener "127.0.0.1" "${FUNFLUID_WEB_FRONTEND_PORT}" \
      || die "前端启动命令已返回，但端口 ${FUNFLUID_WEB_FRONTEND_PORT} 迟迟未监听（多半是启动后崩溃）。" \
             "后端已启动；如需回滚请执行 ./setup.sh stop。"
  fi

  echo "已启动。界面: http://127.0.0.1:${FUNFLUID_WEB_FRONTEND_PORT}"
}

# best effort：某一端未安装/未运行不算错误，只提示，不阻断另一端的停止。
cmd_stop() {
  if command -v funfluid-web >/dev/null 2>&1; then
    echo "==> 停止前端 funfluid-web"
    funfluid-web stop || echo "警告: 前端停止失败或未在运行" >&2
  else
    echo "前端未安装，跳过。"
  fi
  if command -v funfluid-api >/dev/null 2>&1; then
    # 恒传 all：funfluid-api 的 stop 只有目标包含 "all" 时才会一并杀掉自我
    # daemonize 的 watch 循环本身；否则 watch 循环存活，~30s 后会把刚 stop
    # 掉的服务自动拉起来（自愈设计），造成"stop 了但过一会儿又活了"的假象。
    echo "==> 停止后端 funfluid-api"
    funfluid-api stop all || echo "警告: 后端停止失败或未在运行" >&2
  else
    echo "后端未安装，跳过。"
  fi
  echo "已停止（best effort）。"
}

cmd_restart() {
  cmd_stop
  cmd_start
}

cmd_status() {
  echo "== 后端 funfluid-api（${FUNFLUID_WEB_BACKEND_HOST}:${FUNFLUID_WEB_BACKEND_PORT}）=="
  if command -v funfluid-api >/dev/null 2>&1; then
    funfluid-api status "${FUNFLUID_WEB_BACKEND_SERVICE}" || true
  else
    echo "未安装（./setup.sh install）"
  fi
  echo ""
  echo "== 前端 funfluid-web（${FUNFLUID_WEB_FRONTEND_HOST}:${FUNFLUID_WEB_FRONTEND_PORT}）=="
  if command -v funfluid-web >/dev/null 2>&1; then
    funfluid-web status || true
  else
    echo "未安装（./setup.sh install）"
  fi
}

cmd_uninstall() {
  fundeploy_confirm_destructive \
    "将停止并卸载 ${FUNFLUID_WEB_BACKEND_PACKAGE}（pip/uv）与 ${FUNFLUID_WEB_FRONTEND_PACKAGE}（npm -g）。确认？" \
    FUNFLUID_WEB_UNINSTALL_YES || return 1

  if command -v funfluid-web >/dev/null 2>&1; then
    echo "==> funfluid-web uninstall（自带 stop + 卸载 npm 包）"
    funfluid-web uninstall || {
      echo "警告: funfluid-web 自带卸载失败，尝试手动 npm uninstall" >&2
      local npm_bin
      npm_bin="$(_resolve_npm 2>/dev/null)" && "${npm_bin}" uninstall -g "${FUNFLUID_WEB_FRONTEND_PACKAGE}" 2>/dev/null || true
    }
  fi

  if command -v funfluid-api >/dev/null 2>&1; then
    echo "==> 停止后端 funfluid-api"
    funfluid-api stop all || echo "警告: 后端停止失败或未在运行" >&2
  fi
  echo "==> 卸载后端 ${FUNFLUID_WEB_BACKEND_PACKAGE}（pip/uv）"
  _pip_uninstall_pkg "${FUNFLUID_WEB_BACKEND_PACKAGE}"
  echo "已卸载。"
}

interactive_main() {
  _fundeploy_ensure_gum || exit 1
  declare -F fundeploy_ui_apply_theme >/dev/null 2>&1 && fundeploy_ui_apply_theme
  if declare -F fundeploy_ui_banner >/dev/null 2>&1; then
    fundeploy_ui_banner "fundeploy / service / funfluid-web" \
      "后端 ${FUNFLUID_WEB_BACKEND_HOST}:${FUNFLUID_WEB_BACKEND_PORT}  前端 ${FUNFLUID_WEB_FRONTEND_HOST}:${FUNFLUID_WEB_FRONTEND_PORT}"
  fi
  set +e
  while true; do
    local pick
    pick="$(fundeploy_ui_choose "fundeploy / service / funfluid-web / 选择动作" \
      "install    安装（后端 pip + 前端 npm）" \
      "update     更新到最新/指定版本，并显示升级前后版本" \
      "start      启动（后端 → 前端）" \
      "stop       停止（前端 → 后端）" \
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
      update) cmd_update ;;
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
    update) cmd_update ;;
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
