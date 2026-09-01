#!/usr/bin/env bash
# funflix-web 一体化部署：把 funflix（后端，PyPI 包）与 funflix-web（前端，私有 npm 包）
# 作为同一个服务单元来装/起/停——两者本是各自独立的进程，但这里只暴露合并命令，
# 不提供拆开单独控制前后端的子命令；如需单独控制，直接用各自的 CLI（funflix / funflix-web）。
#
# 上游:
#   后端 https://github.com/farfarfun/funflix        （发布到 PyPI，包名 funflix）
#   前端 https://github.com/farfarfun/funflix-web     （发布到私有 npm 源，包名 funflix-web）
#
# 前后端各自已提供健壮的 server start/stop/restart/status（各自管理自己的 PID/日志），
# 本脚本只负责「装两个包 + 按依赖顺序编排调用」，不重复维护 PID 文件。
#
# 依赖: python3 + pip（或 uv）装后端；npm 装前端。
#
# 用法：
#   ./setup.sh                 # gum 菜单
#   ./setup.sh install         # pip/uv 装 funflix + npm -g 装 funflix-web
#   ./setup.sh update          # 同 install（重新安装到最新/指定版本），并打印升级前后版本对比
#   ./setup.sh start           # 先启动后端，再启动前端（自动把 --backend 指向后端地址）
#   ./setup.sh stop            # 先停止前端，再停止后端
#   ./setup.sh restart         # stop + start
#   ./setup.sh status          # 依次打印 funflix / funflix-web 各自的 server status
#   ./setup.sh uninstall       # 停止两者，卸载 npm 包与 pip 包
#
# 环境变量：
#   FUNFLIX_WEB_BACKEND_PACKAGE    后端 pip 包名（默认 funflix）
#   FUNFLIX_WEB_BACKEND_VERSION    后端版本号（默认空＝最新）
#   FUNFLIX_WEB_FRONTEND_PACKAGE   前端 npm 包名（默认 funflix-web）
#   FUNFLIX_WEB_FRONTEND_VERSION   前端版本号（默认空＝最新）
#   FUNFLIX_WEB_PIP_BIN            指定 pip/uv 可执行路径（默认自动探测：优先 uv，否则 python3 -m pip）
#   FUNFLIX_WEB_NPM_BIN            指定 npm 可执行路径（默认从 PATH 查找）
#   FUNFLIX_WEB_BACKEND_HOST       后端监听地址（默认 127.0.0.1）
#   FUNFLIX_WEB_BACKEND_PORT       后端监听端口（默认 18810）
#   FUNFLIX_WEB_FRONTEND_HOST      前端监听地址（默认 127.0.0.1）
#   FUNFLIX_WEB_FRONTEND_PORT      前端监听端口（默认 8810）
#   FUNFLIX_ADMIN_API_KEY          管理接口密钥；原样继承自环境，随 start 一起传给后端
#   NONINTERACTIVE=1
#   FUNFLIX_WEB_UNINSTALL_YES=1    非 TTY 卸载确认

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

FUNFLIX_WEB_BACKEND_PACKAGE="${FUNFLIX_WEB_BACKEND_PACKAGE:-funflix}"
FUNFLIX_WEB_BACKEND_VERSION="${FUNFLIX_WEB_BACKEND_VERSION:-}"
FUNFLIX_WEB_FRONTEND_PACKAGE="${FUNFLIX_WEB_FRONTEND_PACKAGE:-funflix-web}"
FUNFLIX_WEB_FRONTEND_VERSION="${FUNFLIX_WEB_FRONTEND_VERSION:-}"
FUNFLIX_WEB_PIP_BIN="${FUNFLIX_WEB_PIP_BIN:-}"
FUNFLIX_WEB_NPM_BIN="${FUNFLIX_WEB_NPM_BIN:-}"
FUNFLIX_WEB_BACKEND_HOST="${FUNFLIX_WEB_BACKEND_HOST:-127.0.0.1}"
FUNFLIX_WEB_BACKEND_PORT="${FUNFLIX_WEB_BACKEND_PORT:-18810}"
FUNFLIX_WEB_FRONTEND_HOST="${FUNFLIX_WEB_FRONTEND_HOST:-127.0.0.1}"
FUNFLIX_WEB_FRONTEND_PORT="${FUNFLIX_WEB_FRONTEND_PORT:-8810}"

die() { echo "错误: $*" >&2; exit 1; }

usage() {
  cat <<USAGE
用法: ./setup.sh [command]

  无参数：gum 菜单。

命令:
  install            pip/uv 装 ${FUNFLIX_WEB_BACKEND_PACKAGE}，npm -g 装 ${FUNFLIX_WEB_FRONTEND_PACKAGE}
  update             同 install，并打印升级前后的版本对比
  start              按序启动：先 funflix 后端，再 funflix-web 前端（自动带上 --backend）
  stop               按序停止：先前端，再后端（best effort，不因某一端未运行而报错）
  restart            stop + start
  status             依次打印 funflix / funflix-web 的 server status
  uninstall          停止两者，卸载 npm 包与 pip 包

说明:
  - 前后端是两个独立进程，各自的 PID/日志由上游 CLI 自己管理，本脚本不重复维护：
      funflix（后端）      PID/日志见 \${XDG_CONFIG_HOME:-~/.config}/farfarfun/funflix/
      funflix-web（前端）  PID/日志见 ~/.cache/farfarfun/funflix-web/run/
  - 后端: http://${FUNFLIX_WEB_BACKEND_HOST}:${FUNFLIX_WEB_BACKEND_PORT}（接口文档 /docs）
  - 前端: http://${FUNFLIX_WEB_FRONTEND_HOST}:${FUNFLIX_WEB_FRONTEND_PORT}/web（浏览器打开这个）
  - 管理密钥: 提前 export FUNFLIX_ADMIN_API_KEY=... 后再 start，会自动带给后端；
    界面左下角「管理密钥」里填入同一个值即可解锁写操作
  - 只提供合并命令；如需单独控制某一端，直接用 funflix / funflix-web 各自的 CLI

上游: https://github.com/farfarfun/funflix ・ https://github.com/farfarfun/funflix-web
USAGE
}

_require_cli() {
  local bin="$1" hint="$2"
  command -v "${bin}" >/dev/null 2>&1 || die "未找到 ${bin}（${hint}），请先: ./setup.sh install"
}

_resolve_npm() {
  if [[ -n "${FUNFLIX_WEB_NPM_BIN}" ]]; then
    [[ -x "${FUNFLIX_WEB_NPM_BIN}" ]] || die "FUNFLIX_WEB_NPM_BIN 无效: ${FUNFLIX_WEB_NPM_BIN}"
    echo "${FUNFLIX_WEB_NPM_BIN}"
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
  if [[ -n "${FUNFLIX_WEB_PIP_BIN}" ]]; then
    "${FUNFLIX_WEB_PIP_BIN}" install "${spec}"
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
  if [[ -n "${FUNFLIX_WEB_PIP_BIN}" ]]; then
    "${FUNFLIX_WEB_PIP_BIN}" uninstall -y "${pkg}" || echo "警告: 卸载 ${pkg} 失败" >&2
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

cmd_install() {
  local npm_bin backend_spec frontend_spec
  npm_bin="$(_resolve_npm)"
  backend_spec="$(_pip_pkg_spec "${FUNFLIX_WEB_BACKEND_PACKAGE}" "${FUNFLIX_WEB_BACKEND_VERSION}")"
  frontend_spec="$(_npm_pkg_spec "${FUNFLIX_WEB_FRONTEND_PACKAGE}" "${FUNFLIX_WEB_FRONTEND_VERSION}")"
  echo "==> 安装后端: ${backend_spec}"
  _pip_install_pkg "${backend_spec}" || die "后端安装失败: ${backend_spec}"
  echo "==> 安装前端: ${npm_bin} install -g ${frontend_spec}"
  "${npm_bin}" install -g "${frontend_spec}" || die "前端安装失败: ${frontend_spec}"
  echo "已安装。"
}

# 已安装的 pip 包版本；镜像 _pip_install_pkg/_pip_uninstall_pkg 的探测优先级
# （FUNFLIX_WEB_PIP_BIN > uv > python3 -m pip），未安装时输出空字符串。
_pip_pkg_version() {
  local pkg="$1" out
  if [[ -n "${FUNFLIX_WEB_PIP_BIN}" ]]; then
    out="$("${FUNFLIX_WEB_PIP_BIN}" show "${pkg}" 2>/dev/null || true)"
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
    FUNFLIX_WEB_PKG_QUERY="${pkg}" python3 -c '
import json, os, sys
data = json.loads(sys.stdin.read() or "{}")
pkg = os.environ.get("FUNFLIX_WEB_PKG_QUERY", "")
deps = data.get("dependencies") or {}
info = deps.get(pkg) or {}
print(info.get("version") or "")
' <<<"${json}"
  else
    # 无 python3 时的兜底：包名可能形如 @scope/name，分隔符须用 # 而非 /
    printf '%s' "${json}" | sed -n "s#.*\"${pkg}\": *{[^}]*\"version\": *\"\\([^\"]*\\)\".*#\\1#p" | head -1
  fi
}

cmd_update() {
  local backend_before backend_after frontend_before frontend_after
  backend_before="$(_pip_pkg_version "${FUNFLIX_WEB_BACKEND_PACKAGE}")"
  frontend_before="$(_npm_pkg_version "${FUNFLIX_WEB_FRONTEND_PACKAGE}")"

  cmd_install

  backend_after="$(_pip_pkg_version "${FUNFLIX_WEB_BACKEND_PACKAGE}")"
  frontend_after="$(_npm_pkg_version "${FUNFLIX_WEB_FRONTEND_PACKAGE}")"

  echo ""
  echo "== 版本变化 =="
  echo "后端 ${FUNFLIX_WEB_BACKEND_PACKAGE}: ${backend_before:-未安装} -> ${backend_after:-未知}"
  echo "前端 ${FUNFLIX_WEB_FRONTEND_PACKAGE}: ${frontend_before:-未安装} -> ${frontend_after:-未知}"
}

# server start 只要成功 fork 出子进程就会返回 0；子进程若在几秒内因异常退出
# （比如上游包自身的 bug），start 命令本身并不会失败。这里等端口真正被监听
# 到，避免把「已启动」误报给用户。
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
  _require_cli funflix "pip/uv 安装 ${FUNFLIX_WEB_BACKEND_PACKAGE} 后应在 PATH 中"
  _require_cli funflix-web "npm -g 安装 ${FUNFLIX_WEB_FRONTEND_PACKAGE} 后应在 PATH 中"

  # start 在目标已经运行时，上游 CLI 会返回非零退出码（"已在运行，请用 restart"）；
  # 这里先看端口是否已被监听，已在运行就跳过、不当成失败。
  if [[ -n "$(_fundeploy_listener_pid_for_port "${FUNFLIX_WEB_BACKEND_PORT}")" ]]; then
    echo "后端 ${FUNFLIX_WEB_BACKEND_HOST}:${FUNFLIX_WEB_BACKEND_PORT} 已在运行，跳过。"
  else
    echo "==> 启动后端 funflix，监听 ${FUNFLIX_WEB_BACKEND_HOST}:${FUNFLIX_WEB_BACKEND_PORT}"
    funflix server start --host "${FUNFLIX_WEB_BACKEND_HOST}" --port "${FUNFLIX_WEB_BACKEND_PORT}" \
      || die "后端启动失败，已中止（前端未启动）"
    _wait_for_listener "${FUNFLIX_WEB_BACKEND_HOST}" "${FUNFLIX_WEB_BACKEND_PORT}" \
      || die "后端启动命令已返回，但端口 ${FUNFLIX_WEB_BACKEND_PORT} 迟迟未监听（多半是启动后崩溃）。" \
             "已中止（前端未启动）。请看日志: \${XDG_CONFIG_HOME:-~/.config}/farfarfun/funflix/server.log，" \
             "或跑 funflix server run 看前台报错。"
  fi

  if [[ -n "$(_fundeploy_listener_pid_for_port "${FUNFLIX_WEB_FRONTEND_PORT}")" ]]; then
    echo "前端 ${FUNFLIX_WEB_FRONTEND_HOST}:${FUNFLIX_WEB_FRONTEND_PORT} 已在运行，跳过。"
  else
    echo "==> 启动前端 funflix-web，监听 ${FUNFLIX_WEB_FRONTEND_HOST}:${FUNFLIX_WEB_FRONTEND_PORT}"
    funflix-web server start \
      --host "${FUNFLIX_WEB_FRONTEND_HOST}" \
      --port "${FUNFLIX_WEB_FRONTEND_PORT}" \
      --backend "http://${FUNFLIX_WEB_BACKEND_HOST}:${FUNFLIX_WEB_BACKEND_PORT}" \
      || die "前端启动失败（后端已启动；如需回滚请执行 ./setup.sh stop）"
    _wait_for_listener "${FUNFLIX_WEB_FRONTEND_HOST}" "${FUNFLIX_WEB_FRONTEND_PORT}" \
      || die "前端启动命令已返回，但端口 ${FUNFLIX_WEB_FRONTEND_PORT} 迟迟未监听（多半是启动后崩溃）。" \
             "后端已启动；如需回滚请执行 ./setup.sh stop。日志见 ~/.cache/farfarfun/funflix-web/run/。"
  fi

  echo "已启动。界面: http://${FUNFLIX_WEB_FRONTEND_HOST}:${FUNFLIX_WEB_FRONTEND_PORT}/web"
}

# best effort：某一端未安装/未运行不算错误，只提示，不阻断另一端的停止。
cmd_stop() {
  if command -v funflix-web >/dev/null 2>&1; then
    echo "==> 停止前端 funflix-web"
    funflix-web server stop || echo "警告: 前端停止失败或未在运行" >&2
  else
    echo "前端未安装，跳过。"
  fi
  if command -v funflix >/dev/null 2>&1; then
    echo "==> 停止后端 funflix"
    funflix server stop || echo "警告: 后端停止失败或未在运行" >&2
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
  echo "== 后端 funflix（${FUNFLIX_WEB_BACKEND_HOST}:${FUNFLIX_WEB_BACKEND_PORT}）=="
  if command -v funflix >/dev/null 2>&1; then
    funflix server status || true
  else
    echo "未安装（./setup.sh install 或: pip/uv install ${FUNFLIX_WEB_BACKEND_PACKAGE}）"
  fi
  echo ""
  echo "== 前端 funflix-web（${FUNFLIX_WEB_FRONTEND_HOST}:${FUNFLIX_WEB_FRONTEND_PORT}）=="
  if command -v funflix-web >/dev/null 2>&1; then
    funflix-web server status || true
  else
    echo "未安装（./setup.sh install 或: npm install -g ${FUNFLIX_WEB_FRONTEND_PACKAGE}）"
  fi
}

cmd_uninstall() {
  fundeploy_confirm_destructive \
    "将停止并卸载 ${FUNFLIX_WEB_BACKEND_PACKAGE}（pip/uv）与 ${FUNFLIX_WEB_FRONTEND_PACKAGE}（npm -g）。确认？" \
    FUNFLIX_WEB_UNINSTALL_YES || return 1

  if command -v funflix-web >/dev/null 2>&1; then
    echo "==> funflix-web uninstall（自带 stop + 卸载 npm 包）"
    funflix-web uninstall || {
      echo "警告: funflix-web 自带卸载失败，尝试手动 npm uninstall" >&2
      local npm_bin
      npm_bin="$(_resolve_npm 2>/dev/null)" && "${npm_bin}" uninstall -g "${FUNFLIX_WEB_FRONTEND_PACKAGE}" 2>/dev/null || true
    }
  fi

  if command -v funflix >/dev/null 2>&1; then
    echo "==> 停止后端 funflix"
    funflix server stop || echo "警告: 后端停止失败或未在运行" >&2
  fi
  echo "==> 卸载后端 pip 包 ${FUNFLIX_WEB_BACKEND_PACKAGE}"
  _pip_uninstall_pkg "${FUNFLIX_WEB_BACKEND_PACKAGE}"
  echo "已卸载。"
}

interactive_main() {
  _fundeploy_ensure_gum || exit 1
  declare -F fundeploy_ui_apply_theme >/dev/null 2>&1 && fundeploy_ui_apply_theme
  if declare -F fundeploy_ui_banner >/dev/null 2>&1; then
    fundeploy_ui_banner "fundeploy / service / funflix-web" \
      "后端 ${FUNFLIX_WEB_BACKEND_HOST}:${FUNFLIX_WEB_BACKEND_PORT}  前端 ${FUNFLIX_WEB_FRONTEND_HOST}:${FUNFLIX_WEB_FRONTEND_PORT}"
  fi
  set +e
  while true; do
    local pick
    pick="$(fundeploy_ui_choose "fundeploy / service / funflix-web / 选择动作" \
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
