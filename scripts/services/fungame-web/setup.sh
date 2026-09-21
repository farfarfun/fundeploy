#!/usr/bin/env bash
# fungame-web 一体化部署：把 fungame-backend（后端，私有 PyPI 包）、
# @fungame/admin-cli（B 端管理台，私有 npm 包，bin 名 fungame-admin）与
# @fungame/client-cli（C 端游戏客户端，私有 npm 包，bin 名 fungame-client）作为
# 同一个服务单元来装/起/停——三者本是各自独立的进程，但这里只暴露合并命令，不
# 提供拆开单独控制某一端的子命令；如需单独控制，直接用各自的 CLI
# （fungame-backend / fungame-admin / fungame-client）。
#
# 上游:
#   后端   https://github.com/farfarfun/fungame（apps/fungame-api，包名 fungame-backend）
#   admin  https://github.com/farfarfun/fungame（apps/fungame-admin，
#          包名 @fungame/admin-cli，bin 名 fungame-admin）
#   client https://github.com/farfarfun/fungame（apps/fungame-client，
#          包名 @fungame/client-cli，bin 名 fungame-client）
#
# 三端各自已提供健壮的生命周期命令（各自管理自己的 PID/日志），本脚本只负责
# 「装三个包 + 按依赖顺序编排调用」，不重复维护 PID 文件。三者语法一致，都是
# `server run/start/stop/status` 子命令组：backend 的 `server start`/`run` 支持
# `--host`/`--port`（不传时退回其自身 config.toml，无需额外 -d，`start` 本身即
# 后台 daemonize）；admin/client 除了同样的 `--host`/`--port`，还需要
# `--api-upstream-host`/`--api-upstream-port` 把同源 `/api` 反代到本地 backend。
#
# 依赖: python3 + pip（或 uv）装后端；npm 或 pnpm 装 admin 与 client（优先
# pnpm——前端包多数用 only-allow 锁定只能 pnpm 安装，裸 npm install -g 会在
# preinstall 阶段直接失败）。
#
# 用法：
#   ./setup.sh                 # gum 菜单
#   ./setup.sh install         # pip/uv 装 fungame-backend + npm -g 装 admin/client 两个包
#   ./setup.sh upgrade         # 同 install（重新安装到最新/指定版本），并打印升级前后版本对比
#   ./setup.sh start           # 按序启动：backend -> admin -> client
#   ./setup.sh stop            # 按序停止：client -> admin -> backend
#   ./setup.sh restart         # stop + start
#   ./setup.sh status          # 依次打印 fungame-backend / fungame-admin / fungame-client 各自状态
#   ./setup.sh uninstall       # 停止三者，卸载 npm 包与 pip 包
#
# 环境变量：
#   FUNGAME_WEB_BACKEND_PACKAGE   后端 pip 包名（默认 fungame-backend）
#   FUNGAME_WEB_BACKEND_VERSION   后端版本号（默认空＝最新）
#   FUNGAME_WEB_BACKEND_HOST      后端监听地址（默认 127.0.0.1，会传给 --host）
#   FUNGAME_WEB_BACKEND_PORT      后端监听端口（默认 8809，会传给 --port）
#   FUNGAME_WEB_ADMIN_PACKAGE     admin(B端) npm 包名（默认 @fungame/admin-cli）
#   FUNGAME_WEB_ADMIN_VERSION     admin(B端) 版本号（默认空＝最新）
#   FUNGAME_WEB_ADMIN_HOST        admin(B端) 监听地址（默认 127.0.0.1，会传给 --host）
#   FUNGAME_WEB_ADMIN_PORT        admin(B端) 监听端口（默认 8807，会传给 --port）
#   FUNGAME_WEB_CLIENT_PACKAGE    client(C端) npm 包名（默认 @fungame/client-cli）
#   FUNGAME_WEB_CLIENT_VERSION    client(C端) 版本号（默认空＝最新）
#   FUNGAME_WEB_CLIENT_HOST       client(C端) 监听地址（默认 127.0.0.1，会传给 --host）
#   FUNGAME_WEB_CLIENT_PORT       client(C端) 监听端口（默认 8808，会传给 --port）
#   FUNGAME_WEB_PIP_BIN           指定 pip/uv 可执行路径（默认自动探测：优先 uv，否则 python3 -m pip）
#   FUNGAME_WEB_NPM_BIN           指定 npm/pnpm 可执行路径（默认自动探测：优先 pnpm，否则 npm）
#   NONINTERACTIVE=1
#   FUNGAME_WEB_UNINSTALL_YES=1   非 TTY 卸载确认

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../../lib/fundeploy-common.sh
source "${SCRIPT_DIR}/../../lib/fundeploy-common.sh"

FUNGAME_WEB_BACKEND_PACKAGE="${FUNGAME_WEB_BACKEND_PACKAGE:-fungame-backend}"
FUNGAME_WEB_BACKEND_VERSION="${FUNGAME_WEB_BACKEND_VERSION:-}"
FUNGAME_WEB_BACKEND_HOST="${FUNGAME_WEB_BACKEND_HOST:-127.0.0.1}"
FUNGAME_WEB_BACKEND_PORT="${FUNGAME_WEB_BACKEND_PORT:-8809}"
FUNGAME_WEB_ADMIN_PACKAGE="${FUNGAME_WEB_ADMIN_PACKAGE:-@fungame/admin-cli}"
FUNGAME_WEB_ADMIN_VERSION="${FUNGAME_WEB_ADMIN_VERSION:-}"
FUNGAME_WEB_ADMIN_HOST="${FUNGAME_WEB_ADMIN_HOST:-127.0.0.1}"
FUNGAME_WEB_ADMIN_PORT="${FUNGAME_WEB_ADMIN_PORT:-8807}"
FUNGAME_WEB_CLIENT_PACKAGE="${FUNGAME_WEB_CLIENT_PACKAGE:-@fungame/client-cli}"
FUNGAME_WEB_CLIENT_VERSION="${FUNGAME_WEB_CLIENT_VERSION:-}"
FUNGAME_WEB_CLIENT_HOST="${FUNGAME_WEB_CLIENT_HOST:-127.0.0.1}"
FUNGAME_WEB_CLIENT_PORT="${FUNGAME_WEB_CLIENT_PORT:-8808}"
FUNGAME_WEB_PIP_BIN="${FUNGAME_WEB_PIP_BIN:-}"
FUNGAME_WEB_NPM_BIN="${FUNGAME_WEB_NPM_BIN:-}"

die() { echo "错误: $*" >&2; exit 1; }

usage() {
  cat <<USAGE
用法: ./setup.sh [command]

  无参数：gum 菜单。

命令:
  install            pip/uv 装 ${FUNGAME_WEB_BACKEND_PACKAGE}，npm -g 装 ${FUNGAME_WEB_ADMIN_PACKAGE} 与 ${FUNGAME_WEB_CLIENT_PACKAGE}
  upgrade            同 install，并打印升级前后的版本对比
  start              按序启动：backend -> admin(B端) -> client(C端)
  stop               按序停止：client -> admin -> backend（best effort，不因某一端未运行而报错）
  restart            stop + start
  status             依次打印 fungame-backend / fungame-admin / fungame-client 的状态
  uninstall          停止三者，卸载 npm 包与 pip 包

说明:
  - 三端是三个独立进程，各自的 PID/日志由上游 CLI 自己管理，本脚本不重复维护。
  - backend: http://${FUNGAME_WEB_BACKEND_HOST}:${FUNGAME_WEB_BACKEND_PORT}（健康检查 /health）
  - admin(B端): http://${FUNGAME_WEB_ADMIN_HOST}:${FUNGAME_WEB_ADMIN_PORT}
  - client(C端): http://${FUNGAME_WEB_CLIENT_HOST}:${FUNGAME_WEB_CLIENT_PORT}（浏览器打开这个）
  - admin/client 启动时会带上 --api-upstream-host 127.0.0.1 --api-upstream-port ${FUNGAME_WEB_BACKEND_PORT}，
    把各自的同源 /api 请求反代到本地 backend。
  - 只提供合并命令；如需单独控制某一端，直接用 fungame-backend / fungame-admin / fungame-client
    各自的 CLI（三者都是 server run/start/stop/status 子命令组）。

上游: https://github.com/farfarfun/fungame
USAGE
}

_require_cli() {
  local bin="$1" hint="$2"
  command -v "${bin}" >/dev/null 2>&1 || die "未找到 ${bin}（${hint}），请先: ./setup.sh install"
}

_resolve_npm() {
  if [[ -n "${FUNGAME_WEB_NPM_BIN}" ]]; then
    [[ -x "${FUNGAME_WEB_NPM_BIN}" ]] || die "FUNGAME_WEB_NPM_BIN 无效: ${FUNGAME_WEB_NPM_BIN}"
    echo "${FUNGAME_WEB_NPM_BIN}"
    return
  fi
  # 优先 pnpm：前端包多数用 only-allow pnpm 锁定包管理器，裸 npm install -g
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

_npm_install_global() {
  local npm_bin="$1" spec="$2"
  if _npm_is_pnpm "${npm_bin}"; then
    "${npm_bin}" add -g "${spec}"
  else
    "${npm_bin}" install -g "${spec}"
  fi
}

_npm_uninstall_global() {
  local npm_bin="$1" pkg="$2"
  if _npm_is_pnpm "${npm_bin}"; then
    "${npm_bin}" remove -g "${pkg}"
  else
    "${npm_bin}" uninstall -g "${pkg}"
  fi
}

# pnpm add/install -g <pkg>@latest 偶发会命中过期的 dist-tag 解析缓存，装出
# 比 latest 旧的版本（即使当场 pnpm/npm view 已经能查到新版本号）。为绕开这个
# 坑，"latest" 一律先用 view 查出具体版本号，再按精确版本号安装；查不到时才
# 退回字面量 @latest。
_npm_resolve_latest_version() {
  local npm_bin="$1" pkg="$2"
  "${npm_bin}" view "${pkg}" version 2>/dev/null | tail -1
}

_npm_pkg_spec() {
  local npm_bin="$1" pkg="$2" version="$3" resolved
  if [[ -z "${version}" ]]; then
    resolved="$(_npm_resolve_latest_version "${npm_bin}" "${pkg}")"
    if [[ -n "${resolved}" ]]; then
      printf '%s@%s' "${pkg}" "${resolved}"
    else
      printf '%s@latest' "${pkg}"
    fi
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
  if [[ -n "${FUNGAME_WEB_PIP_BIN}" ]]; then
    "${FUNGAME_WEB_PIP_BIN}" install "${spec}"
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
  if [[ -n "${FUNGAME_WEB_PIP_BIN}" ]]; then
    "${FUNGAME_WEB_PIP_BIN}" uninstall -y "${pkg}" || echo "警告: 卸载 ${pkg} 失败" >&2
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
# （FUNGAME_WEB_PIP_BIN > uv > python3 -m pip），未安装时输出空字符串。
_pip_pkg_version() {
  local pkg="$1" out
  if [[ -n "${FUNGAME_WEB_PIP_BIN}" ]]; then
    out="$("${FUNGAME_WEB_PIP_BIN}" show "${pkg}" 2>/dev/null || true)"
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
  if _npm_is_pnpm "${npm_bin}"; then
    json="$("${npm_bin}" ls -g "${pkg}" --json 2>/dev/null || true)"
  else
    json="$("${npm_bin}" ls -g "${pkg}" --depth=0 --json 2>/dev/null || true)"
  fi
  [[ -n "${json}" ]] || { echo ""; return; }
  if command -v python3 >/dev/null 2>&1; then
    FUNGAME_WEB_PKG_QUERY="${pkg}" python3 -c '
import json, os, sys
data = json.loads(sys.stdin.read() or "{}")
pkg = os.environ.get("FUNGAME_WEB_PKG_QUERY", "")
if isinstance(data, list):
    data = data[0] if data else {}
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
  local npm_bin backend_spec admin_spec client_spec
  npm_bin="$(_resolve_npm)"
  backend_spec="$(_pip_pkg_spec "${FUNGAME_WEB_BACKEND_PACKAGE}" "${FUNGAME_WEB_BACKEND_VERSION}")"
  admin_spec="$(_npm_pkg_spec "${npm_bin}" "${FUNGAME_WEB_ADMIN_PACKAGE}" "${FUNGAME_WEB_ADMIN_VERSION}")"
  client_spec="$(_npm_pkg_spec "${npm_bin}" "${FUNGAME_WEB_CLIENT_PACKAGE}" "${FUNGAME_WEB_CLIENT_VERSION}")"
  echo "==> 安装后端: ${backend_spec}"
  _pip_install_pkg "${backend_spec}" || die "后端安装失败: ${backend_spec}"
  if _npm_is_pnpm "${npm_bin}"; then
    echo "==> 安装 admin(B端): ${npm_bin} add -g ${admin_spec}"
  else
    echo "==> 安装 admin(B端): ${npm_bin} install -g ${admin_spec}"
  fi
  _npm_install_global "${npm_bin}" "${admin_spec}" || die "admin(B端) 安装失败: ${admin_spec}"
  if _npm_is_pnpm "${npm_bin}"; then
    echo "==> 安装 client(C端): ${npm_bin} add -g ${client_spec}"
  else
    echo "==> 安装 client(C端): ${npm_bin} install -g ${client_spec}"
  fi
  _npm_install_global "${npm_bin}" "${client_spec}" || die "client(C端) 安装失败: ${client_spec}"
  echo "已安装。"
}

cmd_upgrade() {
  local backend_before backend_after admin_before admin_after client_before client_after
  backend_before="$(_pip_pkg_version "${FUNGAME_WEB_BACKEND_PACKAGE}")"
  admin_before="$(_npm_pkg_version "${FUNGAME_WEB_ADMIN_PACKAGE}")"
  client_before="$(_npm_pkg_version "${FUNGAME_WEB_CLIENT_PACKAGE}")"

  cmd_install

  backend_after="$(_pip_pkg_version "${FUNGAME_WEB_BACKEND_PACKAGE}")"
  admin_after="$(_npm_pkg_version "${FUNGAME_WEB_ADMIN_PACKAGE}")"
  client_after="$(_npm_pkg_version "${FUNGAME_WEB_CLIENT_PACKAGE}")"

  echo ""
  echo "== 版本变化 =="
  echo "后端 ${FUNGAME_WEB_BACKEND_PACKAGE}: ${backend_before:-未安装} -> ${backend_after:-未知}"
  echo "admin ${FUNGAME_WEB_ADMIN_PACKAGE}: ${admin_before:-未安装} -> ${admin_after:-未知}"
  echo "client ${FUNGAME_WEB_CLIENT_PACKAGE}: ${client_before:-未安装} -> ${client_after:-未知}"
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
  _require_cli fungame-backend "pip/uv 安装 ${FUNGAME_WEB_BACKEND_PACKAGE} 后应在 PATH 中"
  _require_cli fungame-admin "npm -g 安装 ${FUNGAME_WEB_ADMIN_PACKAGE} 后应在 PATH 中"
  _require_cli fungame-client "npm -g 安装 ${FUNGAME_WEB_CLIENT_PACKAGE} 后应在 PATH 中"

  if [[ -n "$(_fundeploy_listener_pid_for_port "${FUNGAME_WEB_BACKEND_PORT}")" ]]; then
    echo "backend ${FUNGAME_WEB_BACKEND_HOST}:${FUNGAME_WEB_BACKEND_PORT} 已在运行，跳过。"
  else
    echo "==> 启动 backend fungame-backend，预期监听 ${FUNGAME_WEB_BACKEND_HOST}:${FUNGAME_WEB_BACKEND_PORT}"
    fungame-backend server start --host "${FUNGAME_WEB_BACKEND_HOST}" --port "${FUNGAME_WEB_BACKEND_PORT}" \
      || die "backend 启动失败，已中止（admin/client 未启动）"
    _wait_for_listener "${FUNGAME_WEB_BACKEND_HOST}" "${FUNGAME_WEB_BACKEND_PORT}" \
      || die "backend 启动命令已返回，但端口 ${FUNGAME_WEB_BACKEND_PORT} 迟迟未监听（多半是启动后崩溃）。" \
             "已中止（admin/client 未启动）。请看日志: ~/.config/farfarfun/fungame-backend/，或跑" \
             "fungame-backend server run --host ${FUNGAME_WEB_BACKEND_HOST} --port ${FUNGAME_WEB_BACKEND_PORT} 看前台报错。"
  fi

  if [[ -n "$(_fundeploy_listener_pid_for_port "${FUNGAME_WEB_ADMIN_PORT}")" ]]; then
    echo "admin(B端) ${FUNGAME_WEB_ADMIN_HOST}:${FUNGAME_WEB_ADMIN_PORT} 已在运行，跳过。"
  else
    echo "==> 启动 admin(B端) fungame-admin，监听 ${FUNGAME_WEB_ADMIN_HOST}:${FUNGAME_WEB_ADMIN_PORT}"
    fungame-admin server start --host "${FUNGAME_WEB_ADMIN_HOST}" --port "${FUNGAME_WEB_ADMIN_PORT}" \
      --api-upstream-host 127.0.0.1 --api-upstream-port "${FUNGAME_WEB_BACKEND_PORT}" \
      || die "admin(B端) 启动失败（backend 已启动；如需回滚请执行 ./setup.sh stop）"
    _wait_for_listener "127.0.0.1" "${FUNGAME_WEB_ADMIN_PORT}" \
      || die "admin(B端) 启动命令已返回，但端口 ${FUNGAME_WEB_ADMIN_PORT} 迟迟未监听（多半是启动后崩溃）。" \
             "backend 已启动；如需回滚请执行 ./setup.sh stop。"
  fi

  if [[ -n "$(_fundeploy_listener_pid_for_port "${FUNGAME_WEB_CLIENT_PORT}")" ]]; then
    echo "client(C端) ${FUNGAME_WEB_CLIENT_HOST}:${FUNGAME_WEB_CLIENT_PORT} 已在运行，跳过。"
  else
    echo "==> 启动 client(C端) fungame-client，监听 ${FUNGAME_WEB_CLIENT_HOST}:${FUNGAME_WEB_CLIENT_PORT}"
    fungame-client server start --host "${FUNGAME_WEB_CLIENT_HOST}" --port "${FUNGAME_WEB_CLIENT_PORT}" \
      --api-upstream-host 127.0.0.1 --api-upstream-port "${FUNGAME_WEB_BACKEND_PORT}" \
      || die "client(C端) 启动失败（backend/admin 已启动；如需回滚请执行 ./setup.sh stop）"
    _wait_for_listener "127.0.0.1" "${FUNGAME_WEB_CLIENT_PORT}" \
      || die "client(C端) 启动命令已返回，但端口 ${FUNGAME_WEB_CLIENT_PORT} 迟迟未监听（多半是启动后崩溃）。" \
             "backend/admin 已启动；如需回滚请执行 ./setup.sh stop。"
  fi

  echo "已启动。C 端: http://127.0.0.1:${FUNGAME_WEB_CLIENT_PORT}  B 端: http://127.0.0.1:${FUNGAME_WEB_ADMIN_PORT}"
}

# best effort：某一端未安装/未运行不算错误，只提示，不阻断其它端的停止。
cmd_stop() {
  if command -v fungame-client >/dev/null 2>&1; then
    echo "==> 停止 client(C端) fungame-client"
    fungame-client server stop || echo "警告: client(C端) 停止失败或未在运行" >&2
  else
    echo "client(C端) 未安装，跳过。"
  fi
  if command -v fungame-admin >/dev/null 2>&1; then
    echo "==> 停止 admin(B端) fungame-admin"
    fungame-admin server stop || echo "警告: admin(B端) 停止失败或未在运行" >&2
  else
    echo "admin(B端) 未安装，跳过。"
  fi
  if command -v fungame-backend >/dev/null 2>&1; then
    echo "==> 停止 backend fungame-backend"
    fungame-backend server stop || echo "警告: backend 停止失败或未在运行" >&2
  else
    echo "backend 未安装，跳过。"
  fi
  echo "已停止（best effort）。"
}

cmd_restart() {
  cmd_stop
  cmd_start
}

cmd_status() {
  echo "== backend fungame-backend（${FUNGAME_WEB_BACKEND_HOST}:${FUNGAME_WEB_BACKEND_PORT}）=="
  if command -v fungame-backend >/dev/null 2>&1; then
    fungame-backend server status || true
  else
    echo "未安装（./setup.sh install）"
  fi
  echo ""
  echo "== admin(B端) fungame-admin（${FUNGAME_WEB_ADMIN_HOST}:${FUNGAME_WEB_ADMIN_PORT}）=="
  if command -v fungame-admin >/dev/null 2>&1; then
    fungame-admin server status || true
  else
    echo "未安装（./setup.sh install）"
  fi
  echo ""
  echo "== client(C端) fungame-client（${FUNGAME_WEB_CLIENT_HOST}:${FUNGAME_WEB_CLIENT_PORT}）=="
  if command -v fungame-client >/dev/null 2>&1; then
    fungame-client server status || true
  else
    echo "未安装（./setup.sh install）"
  fi
}

cmd_uninstall() {
  fundeploy_confirm_destructive \
    "将停止并卸载 ${FUNGAME_WEB_BACKEND_PACKAGE}（pip/uv）与 ${FUNGAME_WEB_ADMIN_PACKAGE}、${FUNGAME_WEB_CLIENT_PACKAGE}（npm -g）。确认？" \
    FUNGAME_WEB_UNINSTALL_YES || return 1

  if command -v fungame-client >/dev/null 2>&1; then
    echo "==> fungame-client uninstall（自带 stop + 卸载 npm 包）"
    fungame-client uninstall || {
      echo "警告: client(C端) 自带卸载失败，尝试手动 npm uninstall" >&2
      local npm_bin
      npm_bin="$(_resolve_npm 2>/dev/null)" && _npm_uninstall_global "${npm_bin}" "${FUNGAME_WEB_CLIENT_PACKAGE}" 2>/dev/null || true
    }
  fi

  if command -v fungame-admin >/dev/null 2>&1; then
    echo "==> fungame-admin uninstall（自带 stop + 卸载 npm 包）"
    fungame-admin uninstall || {
      echo "警告: admin(B端) 自带卸载失败，尝试手动 npm uninstall" >&2
      local npm_bin
      npm_bin="$(_resolve_npm 2>/dev/null)" && _npm_uninstall_global "${npm_bin}" "${FUNGAME_WEB_ADMIN_PACKAGE}" 2>/dev/null || true
    }
  fi

  if command -v fungame-backend >/dev/null 2>&1; then
    echo "==> 停止 backend fungame-backend"
    fungame-backend server stop || echo "警告: backend 停止失败或未在运行" >&2
  fi
  echo "==> 卸载后端 ${FUNGAME_WEB_BACKEND_PACKAGE}（pip/uv）"
  _pip_uninstall_pkg "${FUNGAME_WEB_BACKEND_PACKAGE}"
  echo "已卸载。"
}

interactive_main() {
  _fundeploy_ensure_gum || exit 1
  declare -F fundeploy_ui_apply_theme >/dev/null 2>&1 && fundeploy_ui_apply_theme
  if declare -F fundeploy_ui_banner >/dev/null 2>&1; then
    fundeploy_ui_banner "fundeploy / service / fungame-web" \
      "backend ${FUNGAME_WEB_BACKEND_HOST}:${FUNGAME_WEB_BACKEND_PORT}  admin ${FUNGAME_WEB_ADMIN_HOST}:${FUNGAME_WEB_ADMIN_PORT}  client ${FUNGAME_WEB_CLIENT_HOST}:${FUNGAME_WEB_CLIENT_PORT}"
  fi
  set +e
  while true; do
    local pick
    pick="$(fundeploy_ui_choose "fundeploy / service / fungame-web / 选择动作" \
      "install    安装（后端 pip + admin/client npm）" \
      "upgrade    更新到最新/指定版本，并显示升级前后版本" \
      "start      启动（backend → admin → client）" \
      "stop       停止（client → admin → backend）" \
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
