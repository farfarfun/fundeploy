#!/usr/bin/env bash
# funread-web 一体化部署：把 funread-api（后端，PyPI 包）和 @farfarfun/funread-web
# （前端，Codeup 私有 npm 包）当成一个服务单元来装/起/停。两者本是各自独立的进程，
# 这里只暴露合并命令；要单独控制某一端，直接用它们各自的 CLI。
#
# 上游:
#   后端 https://github.com/farfarfun/funread-api  （PyPI，包名与 CLI 名都是 funread-api）
#   前端 https://github.com/farfarfun/funread-web  （Codeup 私有 npm 源，包名
#        @farfarfun/funread-web，CLI 名 funread-web —— scope 在包名上，不在命令名上）
#   核心 https://github.com/farfarfun/funread      （funread-api 的依赖，自己没有 CLI）
#
# 前后端各自已提供 server start|run|restart|stop|status，PID 与日志都写在自己实际
# 解析到的 config 同目录下。所以本脚本只做「装两个正式包 + 按依赖顺序编排两个 CLI」，
# 不 nohup、不写 PID 文件、不管日志 —— 那些都是上游 CLI 的责任。
#
# 为什么 install/upgrade/rollback 不直接转调上游 CLI 的同名命令：
#   - funread-api 的 upgrade 跑 `uv tool install --reinstall --upgrade funread-api`，
#     机器上若是 pip 装的，这会另装一份 uv tool 副本，跑哪个由 PATH 顺序决定；
#   - funread-web 的 upgrade/rollback 写死了 `npm install -g`，pnpm 全局装的机器上
#     会装出第二份，PATH 里仍是旧版 —— 升级看着成功，其实没生效。
#   所以这两件事由本脚本自己做（顺带把前后端一起装、一起打印版本对比）。
#   start/run/stop/status 没有这个问题，一律直接转发给上游 CLI。
#
# 建管理员账号与发注册邀请码不在本脚本里（funread 的注册由邀请码控制）：那是运维动作
# 不是部署动作，装完直接敲 `funread-api accounts create-admin <用户名>` 和
# `funread-api accounts invite issue`。
#
# 前端包在私有源上，装之前机器上需要一次性的 scope 映射（pnpm 不接受
# --@scope:registry= 这种命令行覆盖，只能是机器级配置）：
#   npm config set @farfarfun:registry https://farfarfun-cn-hangzhou.devops.aliyuncs.com/packages/api/protocol/npm/funnpm/
#
# 用法：
#   ./setup.sh                      # gum 菜单
#   ./setup.sh install              # 装后端（uv tool / pip 自适应）+ 前端（pnpm / npm -g）
#   ./setup.sh upgrade              # 同 install，额外打印升级前后的版本对比
#   ./setup.sh rollback <版本>      # 前后端一起钉到指定版本（先停，装完恢复原运行状态）
#   ./setup.sh start                # 先启动后端，再启动前端（自动把 --backend 指向后端）
#   ./setup.sh run                  # 后端后台 + 前端前台（Ctrl+C 退出时连后端一起停）
#   ./setup.sh stop                 # 先停前端，再停后端
#   ./setup.sh restart              # stop + start
#   ./setup.sh status               # 两个包的已安装版本 + 两个 CLI 各自的 server status
#   ./setup.sh uninstall            # 停止两者，卸载 npm 包与 python 包
#
# 环境变量：
#   FUNREAD_WEB_BACKEND_PACKAGE    后端 python 包名（默认 funread-api）
#   FUNREAD_WEB_BACKEND_VERSION    后端版本（默认空＝最新）
#   FUNREAD_WEB_FRONTEND_PACKAGE   前端 npm 包名（默认 @farfarfun/funread-web）
#   FUNREAD_WEB_FRONTEND_VERSION   前端版本（默认空＝最新）
#   FUNREAD_WEB_PIP_BIN            指定 pip 可执行文件；给了就走 pip 模式
#   FUNREAD_WEB_NPM_BIN            指定 npm/pnpm 可执行文件（默认自动探测）
#   FUNREAD_WEB_BACKEND_HOST/PORT  后端监听地址与端口（默认 127.0.0.1 / 18811）
#   FUNREAD_WEB_FRONTEND_HOST/PORT 前端监听地址与端口（默认 127.0.0.1 / 8811）
#   FUNREAD_WEB_BACKEND_CONFIG     透传给后端的 --config（默认空＝上游默认路径）
#   FUNREAD_WEB_FRONTEND_CONFIG    透传给前端的 --config（默认空＝上游默认路径）
#   FUNREAD_DATABASE_URL           后端库地址，原样继承自环境，本脚本不设默认值
#   NONINTERACTIVE=1               无参数调用时直接报错而不进菜单
#   FUNREAD_WEB_UNINSTALL_YES=1    非 TTY 下的卸载确认

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../../lib/fundeploy-common.sh
source "${SCRIPT_DIR}/../../lib/fundeploy-common.sh"

FUNREAD_WEB_BACKEND_PACKAGE="${FUNREAD_WEB_BACKEND_PACKAGE:-funread-api}"
FUNREAD_WEB_BACKEND_VERSION="${FUNREAD_WEB_BACKEND_VERSION:-}"
FUNREAD_WEB_FRONTEND_PACKAGE="${FUNREAD_WEB_FRONTEND_PACKAGE:-@farfarfun/funread-web}"
FUNREAD_WEB_FRONTEND_VERSION="${FUNREAD_WEB_FRONTEND_VERSION:-}"
# 只用于装不上时的提示文案，脚本自己不会去配它。
FUNREAD_WEB_FRONTEND_REGISTRY="https://farfarfun-cn-hangzhou.devops.aliyuncs.com/packages/api/protocol/npm/funnpm/"
FUNREAD_WEB_PIP_BIN="${FUNREAD_WEB_PIP_BIN:-}"
FUNREAD_WEB_NPM_BIN="${FUNREAD_WEB_NPM_BIN:-}"
FUNREAD_WEB_BACKEND_HOST="${FUNREAD_WEB_BACKEND_HOST:-127.0.0.1}"
FUNREAD_WEB_BACKEND_PORT="${FUNREAD_WEB_BACKEND_PORT:-18811}"
FUNREAD_WEB_FRONTEND_HOST="${FUNREAD_WEB_FRONTEND_HOST:-127.0.0.1}"
FUNREAD_WEB_FRONTEND_PORT="${FUNREAD_WEB_FRONTEND_PORT:-8811}"
FUNREAD_WEB_BACKEND_CONFIG="${FUNREAD_WEB_BACKEND_CONFIG:-}"
FUNREAD_WEB_FRONTEND_CONFIG="${FUNREAD_WEB_FRONTEND_CONFIG:-}"

die() {
  local line
  for line in "$@"; do echo "错误: ${line}" >&2; done
  exit 1
}

# 两个 CLI 都把 PID 与日志放在实际解析到的 config 同目录下。这里只用于报错时指路，
# 脚本自己不读写这些文件。
_backend_state_dir() {
  if [[ -n "${FUNREAD_WEB_BACKEND_CONFIG}" ]]; then
    dirname "${FUNREAD_WEB_BACKEND_CONFIG}"
  else
    printf '%s/farfarfun/funread-api' "${XDG_CONFIG_HOME:-${HOME}/.config}"
  fi
}

_frontend_state_dir() {
  if [[ -n "${FUNREAD_WEB_FRONTEND_CONFIG}" ]]; then
    dirname "${FUNREAD_WEB_FRONTEND_CONFIG}"
  else
    printf '%s/farfarfun/funread-web' "${XDG_CONFIG_HOME:-${HOME}/.config}"
  fi
}

usage() {
  cat <<USAGE
用法: ./setup.sh [command]

  无参数：进 gum 菜单。

命令:
  install            装 ${FUNREAD_WEB_BACKEND_PACKAGE}（uv tool / pip 自适应）+ ${FUNREAD_WEB_FRONTEND_PACKAGE}（pnpm / npm -g）
  upgrade            同 install，额外打印升级前后的版本对比
  rollback <版本>    前后端一起钉到指定版本（先停，装完恢复原运行状态）
  start              先启动后端 funread-api，再启动前端 funread-web
  run                后端后台 + 前端前台（Ctrl+C 退出时连后端一起停）
  stop               先停前端，再停后端（best effort，某一端没跑不算失败）
  restart            stop + start
  status             两个包的已安装版本 + 两个 CLI 各自的 server status
  uninstall          停止两者，卸载 npm 包与 python 包

地址:
  前端  http://${FUNREAD_WEB_FRONTEND_HOST}:${FUNREAD_WEB_FRONTEND_PORT}/web （阅读端；管理端 /admin）
  后端  http://${FUNREAD_WEB_BACKEND_HOST}:${FUNREAD_WEB_BACKEND_PORT}/docs
  浏览器只该访问前端端口：前端同时把 /api 与 /healthz 反代到后端，直连后端会撞 CORS
  （funread-api 刻意不挂 CORS 中间件）。

说明:
  - PID 与日志归两个 CLI 自己管，各自写在实际解析到的 config 同目录下：
      后端 $(_backend_state_dir)/funread-api.{pid,log}
      前端 $(_frontend_state_dir)/funread-web.{pid,log}
    fundeploy 不另记一份。
  - install/upgrade/rollback 由本脚本做，不要用上游 CLI 自带的同名命令 —— 理由见文件头注释。
  - 前端包在私有源上，装不上先确认机器上有 scope 映射：
      npm config set @farfarfun:registry ${FUNREAD_WEB_FRONTEND_REGISTRY}
  - FUNREAD_DATABASE_URL 不设时，后端按 env > funsecret（funread/cache/source/db_url）
    > 本地 SQLite 的顺序解析。机器上配了 funsecret 就会连到 funsecret 里那个库，
    想用本地库请显式设这个变量。
  - 建管理员与发邀请码不在本脚本里，装完直接敲：
      funread-api accounts create-admin <用户名>
      funread-api accounts invite issue
  - 只提供合并命令；要单独控制某一端，直接用 funread-api / funread-web 各自的 CLI。

上游: https://github.com/farfarfun/funread-api · https://github.com/farfarfun/funread-web · https://github.com/farfarfun/funread
USAGE
}

_require_cli() {
  local bin="$1" hint="$2"
  command -v "${bin}" >/dev/null 2>&1 || die "未找到 ${bin}（${hint}），请先: ./setup.sh install"
}

# ---------------------------------------------------------------- 前端包管理器

_npm_is_pnpm() {
  [[ "$(basename "$1")" == pnpm* ]]
}

# 指定包管理器下的全局包版本，未安装时输出空字符串。不自己探测包管理器，
# 避免与 _resolve_npm 互相调用。
_npm_global_version() {
  local npm_bin="$1" pkg="$2" json
  if _npm_is_pnpm "${npm_bin}"; then
    json="$("${npm_bin}" ls -g "${pkg}" --json 2>/dev/null || true)"
  else
    json="$("${npm_bin}" ls -g "${pkg}" --depth=0 --json 2>/dev/null || true)"
  fi
  [[ -n "${json}" ]] || { echo ""; return; }
  if command -v python3 >/dev/null 2>&1; then
    FUNREAD_WEB_PKG_QUERY="${pkg}" python3 -c '
import json, os, sys
data = json.loads(sys.stdin.read() or "{}")
pkg = os.environ.get("FUNREAD_WEB_PKG_QUERY", "")
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

_resolve_npm() {
  local pnpm_bin npm_bin
  if [[ -n "${FUNREAD_WEB_NPM_BIN}" ]]; then
    [[ -x "${FUNREAD_WEB_NPM_BIN}" ]] || die "FUNREAD_WEB_NPM_BIN 无效: ${FUNREAD_WEB_NPM_BIN}"
    echo "${FUNREAD_WEB_NPM_BIN}"
    return
  fi
  pnpm_bin="$(command -v pnpm 2>/dev/null || true)"
  npm_bin="$(command -v npm 2>/dev/null || true)"
  # 已经装过的那一份归谁管，就继续用谁：两个包管理器的全局 bin 目录不同，用另一个
  # 装出来的是第二份副本，PATH 里仍是旧的——upgrade 看着成功，跑的还是老版本。
  if [[ -n "${pnpm_bin}" && -n "$(_npm_global_version "${pnpm_bin}" "${FUNREAD_WEB_FRONTEND_PACKAGE}")" ]]; then
    echo "${pnpm_bin}"
    return
  fi
  if [[ -n "${npm_bin}" && -n "$(_npm_global_version "${npm_bin}" "${FUNREAD_WEB_FRONTEND_PACKAGE}")" ]]; then
    echo "${npm_bin}"
    return
  fi
  # 没装过：优先 pnpm——前端包多数用 only-allow 锁定包管理器，裸 npm install -g
  # 会在 preinstall 阶段直接失败。
  [[ -n "${pnpm_bin}" ]] && { echo "${pnpm_bin}"; return; }
  [[ -n "${npm_bin}" ]] && { echo "${npm_bin}"; return; }
  die "未找到 npm/pnpm（可先运行 fundeploy dev nodejs install）"
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

_npm_view_version() {
  local npm_bin="$1" pkg="$2"
  "${npm_bin}" view "${pkg}" version 2>/dev/null | tail -1
}

# pnpm add/install -g <pkg>@latest 偶发会命中过期的 dist-tag 解析缓存，装出比
# latest 旧的版本（即使当场 view 已经能查到新版本号）。所以 "最新" 一律先用 view
# 查出具体版本号再按精确版本号装；查不到才退回字面量 @latest。
_npm_pkg_spec() {
  local npm_bin="$1" pkg="$2" version="$3" resolved
  if [[ -z "${version}" ]]; then
    resolved="$(_npm_view_version "${npm_bin}" "${pkg}")"
    if [[ -n "${resolved}" ]]; then
      printf '%s@%s' "${pkg}" "${resolved}"
    else
      echo "警告: 查不到 ${pkg} 的最新版本号（私有源 scope 映射缺失？），退回 @latest" >&2
      printf '%s@latest' "${pkg}"
    fi
  else
    printf '%s@%s' "${pkg}" "${version#v}"
  fi
}

# ------------------------------------------------------------------ 后端包管理

_uv_tool_version() {
  command -v uv >/dev/null 2>&1 || { echo ""; return; }
  uv tool list 2>/dev/null | awk -v pkg="$1" '$1 == pkg && $2 ~ /^v/ { print substr($2, 2); exit }'
}

# PATH 上那个 funread-api 由哪个 python 跑：console script 的 shebang 就是答案，
# uv tool 装的（指向 tool venv）和 pip 装的（指向某个 venv）都适用。
_backend_cli_python() {
  local cli shebang
  cli="$(command -v funread-api 2>/dev/null || true)"
  [[ -n "${cli}" && -f "${cli}" ]] || return 1
  shebang="$(head -1 "${cli}" 2>/dev/null || true)"
  [[ "${shebang}" == '#!'* ]] || return 1
  shebang="${shebang#\#!}"
  # 形如 `#!/usr/bin/env python3` 的壳脚本定位不到具体环境，当探测失败处理
  [[ "${shebang}" == /* && "${shebang}" != *" "* ]] || return 1
  printf '%s' "${shebang}"
}

# uvtool：用 uv tool 装（与上游 funread-api upgrade 自己用的方式一致）。
# pip   ：显式给了 FUNREAD_WEB_PIP_BIN，或机器上已有一份非 uv tool 的安装 ——
#         继续走 pip 路线，免得 uv tool 再装出第二份、跑哪个由 PATH 顺序决定。
_backend_mode() {
  [[ -n "${FUNREAD_WEB_PIP_BIN}" ]] && { echo pip; return; }
  command -v uv >/dev/null 2>&1 || { echo pip; return; }
  [[ -n "$(_uv_tool_version "${FUNREAD_WEB_BACKEND_PACKAGE}")" ]] && { echo uvtool; return; }
  command -v funread-api >/dev/null 2>&1 && { echo pip; return; }
  echo uvtool
}

# PATH 上真正会跑到的那份后端的版本，取包元数据而不是 --version。未安装时输出空串。
_backend_version() {
  local py
  py="$(_backend_cli_python || true)"
  [[ -n "${py}" ]] || { echo ""; return; }
  FUNREAD_WEB_PKG_QUERY="${FUNREAD_WEB_BACKEND_PACKAGE}" "${py}" -c '
import importlib.metadata as m, os
try:
    print(m.version(os.environ["FUNREAD_WEB_PKG_QUERY"]))
except m.PackageNotFoundError:
    print("")
' 2>/dev/null || echo ""
}

# 本脚本这条安装路线所管理的那份的版本（uv tool 模式下未必等于 PATH 上那份）。
_backend_managed_version() {
  if [[ "$(_backend_mode)" == uvtool ]]; then
    _uv_tool_version "${FUNREAD_WEB_BACKEND_PACKAGE}"
  else
    _backend_version
  fi
}

_backend_install() {
  local spec="$1" pinned="${2:-0}" args=() py
  if [[ "$(_backend_mode)" == uvtool ]]; then
    # --force 而不是 --upgrade：钉版本时要允许装成比现在更旧的版本
    if [[ "${pinned}" == 1 ]]; then
      uv tool install --force "${spec}"
    else
      uv tool install --upgrade "${spec}"
    fi
    return
  fi

  if [[ -n "${FUNREAD_WEB_PIP_BIN}" ]]; then
    if [[ "${pinned}" == 1 ]]; then
      "${FUNREAD_WEB_PIP_BIN}" install "${spec}"
    else
      "${FUNREAD_WEB_PIP_BIN}" install --upgrade "${spec}"
    fi
    return
  fi

  # 已有安装时钉住它所在的 python：`uv pip` 自己探测出来的环境未必是同一个，
  # 装进别的环境等于升级了一份没人跑的副本。
  py="$(_backend_cli_python || true)"
  if command -v uv >/dev/null 2>&1; then
    args=(pip install)
    [[ -n "${py}" ]] && args+=(--python "${py}")
    [[ "${pinned}" == 1 ]] || args+=(--upgrade)
    args+=("${spec}")
    uv "${args[@]}"
    return
  fi
  if [[ -n "${py}" ]]; then
    if [[ "${pinned}" == 1 ]]; then
      "${py}" -m pip install "${spec}"
    else
      "${py}" -m pip install --upgrade "${spec}"
    fi
    return
  fi
  command -v python3 >/dev/null 2>&1 || die "未找到 uv/python3（可先运行 fundeploy dev uv install）"
  if [[ "${pinned}" == 1 ]]; then
    python3 -m pip install --user "${spec}"
  else
    python3 -m pip install --user --upgrade "${spec}"
  fi
}

_backend_uninstall() {
  local pkg="${FUNREAD_WEB_BACKEND_PACKAGE}" py
  if [[ "$(_backend_mode)" == uvtool ]]; then
    uv tool uninstall "${pkg}" || echo "警告: 卸载 ${pkg} 失败" >&2
    return
  fi
  if [[ -n "${FUNREAD_WEB_PIP_BIN}" ]]; then
    "${FUNREAD_WEB_PIP_BIN}" uninstall -y "${pkg}" || echo "警告: 卸载 ${pkg} 失败" >&2
    return
  fi
  py="$(_backend_cli_python || true)"
  if [[ -n "${py}" ]]; then
    if command -v uv >/dev/null 2>&1; then
      uv pip uninstall --python "${py}" "${pkg}" || echo "警告: 卸载 ${pkg} 失败" >&2
    else
      "${py}" -m pip uninstall -y "${pkg}" || echo "警告: 卸载 ${pkg} 失败" >&2
    fi
    return
  fi
  echo "警告: 未找到已安装的 ${pkg}，跳过卸载" >&2
}

_pip_pkg_spec() {
  local pkg="$1" version="$2"
  if [[ -z "${version}" ]]; then
    printf '%s' "${pkg}"
  else
    printf '%s==%s' "${pkg}" "${version#v}"
  fi
}

# 装完核对一次：刚装的那份必须就是 PATH 上会跑到的那份。两份安装并存时跑哪个由
# PATH 顺序决定，不核对就会出现「升级成功、行为没变」。
_verify_backend_on_path() {
  local managed on_path cli
  cli="$(command -v funread-api 2>/dev/null || true)"
  if [[ -z "${cli}" ]]; then
    echo "警告: 后端已安装，但 funread-api 不在 PATH 上（uv tool 装在 $(uv tool dir --bin 2>/dev/null || echo "${HOME}/.local/bin")）；" >&2
    echo "      把它加进 PATH，或执行 uv tool update-shell 后重开 shell。" >&2
    return 0
  fi
  managed="$(_backend_managed_version)"
  on_path="$(_backend_version)"
  [[ -z "${managed}" || -z "${on_path}" || "${managed}" == "${on_path}" ]] && return 0
  echo "警告: 刚装的 ${FUNREAD_WEB_BACKEND_PACKAGE} 是 ${managed}，但 PATH 上的 ${cli} 是 ${on_path} ——" >&2
  echo "      机器上有第二份安装把它遮住了，先卸掉多余的那份（或调整 PATH）再重试。" >&2
}

# ------------------------------------------------------------------ 安装与版本

cmd_install() {
  local npm_bin backend_spec frontend_spec backend_pinned=0
  npm_bin="$(_resolve_npm)"
  # 前端钉版本不用额外开关：npm/pnpm 装 pkg@x.y.z 本身升降级都适用。
  [[ -n "${FUNREAD_WEB_BACKEND_VERSION}" ]] && backend_pinned=1
  backend_spec="$(_pip_pkg_spec "${FUNREAD_WEB_BACKEND_PACKAGE}" "${FUNREAD_WEB_BACKEND_VERSION}")"
  frontend_spec="$(_npm_pkg_spec "${npm_bin}" "${FUNREAD_WEB_FRONTEND_PACKAGE}" "${FUNREAD_WEB_FRONTEND_VERSION}")"

  echo "==> 安装后端: ${backend_spec}（$(_backend_mode) 模式）"
  _backend_install "${backend_spec}" "${backend_pinned}" || die "后端安装失败: ${backend_spec}"

  echo "==> 安装前端: $(basename "${npm_bin}") -g ${frontend_spec}"
  _npm_install_global "${npm_bin}" "${frontend_spec}" \
    || die "前端安装失败: ${frontend_spec}" \
           "私有源 scope 映射缺失？执行一次:" \
           "  npm config set @farfarfun:registry ${FUNREAD_WEB_FRONTEND_REGISTRY}"

  _verify_backend_on_path
  echo "已安装。接下来: ./setup.sh start"
}

cmd_upgrade() {
  local npm_bin backend_before backend_after frontend_before frontend_after
  npm_bin="$(_resolve_npm)"
  backend_before="$(_backend_version)"
  frontend_before="$(_npm_global_version "${npm_bin}" "${FUNREAD_WEB_FRONTEND_PACKAGE}")"

  cmd_install

  # 重新探一次包管理器：首装可能刚把包装到另一个全局 bin 目录下。
  npm_bin="$(_resolve_npm)"
  backend_after="$(_backend_version)"
  frontend_after="$(_npm_global_version "${npm_bin}" "${FUNREAD_WEB_FRONTEND_PACKAGE}")"

  echo ""
  echo "== 版本变化 =="
  echo "后端 ${FUNREAD_WEB_BACKEND_PACKAGE}: ${backend_before:-未安装} -> ${backend_after:-未知}"
  echo "前端 ${FUNREAD_WEB_FRONTEND_PACKAGE}: ${frontend_before:-未安装} -> ${frontend_after:-未知}"
}

cmd_rollback() {
  local version="${1:-}" npm_bin was_running=0
  [[ -n "${version}" ]] || die "rollback 需要版本号，例如: ./setup.sh rollback 1.1.109"
  version="${version#v}"

  # 先确认 npm 源上真有这个版本。install 的顺序是先后端后前端，前端装不上而后端
  # 已经降级的话，会停在前后端版本不一致的中间态上。
  npm_bin="$(_resolve_npm)"
  [[ -n "$(_npm_view_version "${npm_bin}" "${FUNREAD_WEB_FRONTEND_PACKAGE}@${version}")" ]] \
    || die "npm 源上没有 ${FUNREAD_WEB_FRONTEND_PACKAGE}@${version}，已中止（没有改动任何安装）。" \
           "注意 npm 上的版本号是跳着的：1.1.106–1.1.108 三次发版都因私有源链路丢包失败，从未发布成功。"

  if [[ -n "$(_fundeploy_listener_pid_for_port "${FUNREAD_WEB_BACKEND_PORT}")" \
     || -n "$(_fundeploy_listener_pid_for_port "${FUNREAD_WEB_FRONTEND_PORT}")" ]]; then
    was_running=1
    cmd_stop
  fi

  FUNREAD_WEB_BACKEND_VERSION="${version}"
  FUNREAD_WEB_FRONTEND_VERSION="${version}"
  cmd_install

  if [[ "${was_running}" == 1 ]]; then
    echo "==> 回退前服务在运行，重新启动"
    cmd_start
  else
    echo "已回退到 ${version}（回退前服务未运行，未自动启动）。"
  fi
}

# ---------------------------------------------------------------- 生命周期

# server start 只要成功 fork 出子进程就返回 0；子进程若在几秒内崩掉，start 本身
# 并不失败。所以这里等端口真被监听上，避免把「已启动」误报给用户。
_wait_for_listener() {
  local port="$1" timeout="${2:-8}" waited=0
  while (( waited < timeout )); do
    [[ -n "$(_fundeploy_listener_pid_for_port "${port}")" ]] && return 0
    sleep 1
    waited=$((waited + 1))
  done
  return 1
}

_warn_if_still_listening() {
  local label="$1" port="$2" pid
  pid="$(_fundeploy_listener_pid_for_port "${port}")"
  [[ -z "${pid}" ]] && return 0
  echo "警告: ${label}端口 ${port} 仍被 PID ${pid} 监听（上游 stop 只发 SIGTERM、不强杀）；" >&2
  echo "      确认要强停: kill -TERM ${pid}" >&2
}

# 显式 --host/--port 会盖掉配置文件里的同名字段，这是故意的：fundeploy 的状态表与
# 端口探测都以这里的变量为准。只想用配置文件里的值，就把对应变量设成相同的值。
#
# stop/status 不接受 --host（上游只给了 --config 与 --port）：--config 用来定位 PID
# 文件，--port 是 PID 文件丢了之后的兜底，所以这两个都要传。
_BACKEND_SERVER_ARGS=()
_FRONTEND_SERVER_ARGS=()
_BACKEND_STOP_ARGS=()
_FRONTEND_STOP_ARGS=()
_build_server_args() {
  _BACKEND_SERVER_ARGS=(--host "${FUNREAD_WEB_BACKEND_HOST}" --port "${FUNREAD_WEB_BACKEND_PORT}")
  _BACKEND_STOP_ARGS=(--port "${FUNREAD_WEB_BACKEND_PORT}")
  if [[ -n "${FUNREAD_WEB_BACKEND_CONFIG}" ]]; then
    _BACKEND_SERVER_ARGS+=(--config "${FUNREAD_WEB_BACKEND_CONFIG}")
    _BACKEND_STOP_ARGS+=(--config "${FUNREAD_WEB_BACKEND_CONFIG}")
  fi
  _FRONTEND_SERVER_ARGS=(
    --host "${FUNREAD_WEB_FRONTEND_HOST}"
    --port "${FUNREAD_WEB_FRONTEND_PORT}"
    --backend "http://${FUNREAD_WEB_BACKEND_HOST}:${FUNREAD_WEB_BACKEND_PORT}"
  )
  _FRONTEND_STOP_ARGS=(--port "${FUNREAD_WEB_FRONTEND_PORT}")
  if [[ -n "${FUNREAD_WEB_FRONTEND_CONFIG}" ]]; then
    _FRONTEND_SERVER_ARGS+=(--config "${FUNREAD_WEB_FRONTEND_CONFIG}")
    _FRONTEND_STOP_ARGS+=(--config "${FUNREAD_WEB_FRONTEND_CONFIG}")
  fi
  return 0
}

_die_backend_never_listened() {
  die "后端启动命令已返回，但端口 ${FUNREAD_WEB_BACKEND_PORT} 迟迟未监听（多半是启动后崩溃）。" \
      "已中止（前端未启动）。日志见 $(_backend_state_dir)/funread-api.log，或跑" \
      "  funread-api server run --port ${FUNREAD_WEB_BACKEND_PORT}" \
      "看前台报错 —— 连不上库是最常见的原因，检查 FUNREAD_DATABASE_URL。"
}

cmd_start() {
  _require_cli funread-api "uv tool / pip 装了 ${FUNREAD_WEB_BACKEND_PACKAGE} 后应在 PATH 中"
  _require_cli funread-web "npm/pnpm -g 装了 ${FUNREAD_WEB_FRONTEND_PACKAGE} 后应在 PATH 中"
  _build_server_args

  # start 在目标已经运行时，上游 CLI 会返回非零退出码；这里先看端口是否已被监听，
  # 已在运行就跳过、不当成失败。
  if [[ -n "$(_fundeploy_listener_pid_for_port "${FUNREAD_WEB_BACKEND_PORT}")" ]]; then
    echo "后端 ${FUNREAD_WEB_BACKEND_HOST}:${FUNREAD_WEB_BACKEND_PORT} 已在运行，跳过。"
  else
    echo "==> 启动后端 funread-api，监听 ${FUNREAD_WEB_BACKEND_HOST}:${FUNREAD_WEB_BACKEND_PORT}"
    funread-api server start "${_BACKEND_SERVER_ARGS[@]}" \
      || die "后端启动失败，已中止（前端未启动）"
    _wait_for_listener "${FUNREAD_WEB_BACKEND_PORT}" || _die_backend_never_listened
  fi

  if [[ -n "$(_fundeploy_listener_pid_for_port "${FUNREAD_WEB_FRONTEND_PORT}")" ]]; then
    echo "前端 ${FUNREAD_WEB_FRONTEND_HOST}:${FUNREAD_WEB_FRONTEND_PORT} 已在运行，跳过。"
  else
    echo "==> 启动前端 funread-web，监听 ${FUNREAD_WEB_FRONTEND_HOST}:${FUNREAD_WEB_FRONTEND_PORT}"
    funread-web server start "${_FRONTEND_SERVER_ARGS[@]}" \
      || die "前端启动失败（后端已启动；如需回滚请执行 ./setup.sh stop）"
    _wait_for_listener "${FUNREAD_WEB_FRONTEND_PORT}" \
      || die "前端启动命令已返回，但端口 ${FUNREAD_WEB_FRONTEND_PORT} 迟迟未监听（多半是启动后崩溃）。" \
             "后端已启动；如需回滚请执行 ./setup.sh stop。日志见 $(_frontend_state_dir)/funread-web.log。"
  fi

  echo "已启动。界面: http://${FUNREAD_WEB_FRONTEND_HOST}:${FUNREAD_WEB_FRONTEND_PORT}/web （管理端 /admin）"
  echo "还没有账号就先: funread-api accounts create-admin <用户名>"
}

# 前台跑前端，后端仍是后台进程（上游没有「两个进程都前台」的形态）。退出时把自己
# 起的后端一并停掉 —— 留一个孤儿后端在那儿比没启动更糟，所以这里不用 exec。
cmd_run() {
  _require_cli funread-api "uv tool / pip 装了 ${FUNREAD_WEB_BACKEND_PACKAGE} 后应在 PATH 中"
  _require_cli funread-web "npm/pnpm -g 装了 ${FUNREAD_WEB_FRONTEND_PACKAGE} 后应在 PATH 中"
  [[ -z "$(_fundeploy_listener_pid_for_port "${FUNREAD_WEB_BACKEND_PORT}")" ]] \
    || die "后端端口 ${FUNREAD_WEB_BACKEND_PORT} 已被占用；run 不能接管已有进程"
  [[ -z "$(_fundeploy_listener_pid_for_port "${FUNREAD_WEB_FRONTEND_PORT}")" ]] \
    || die "前端端口 ${FUNREAD_WEB_FRONTEND_PORT} 已被占用；run 不能接管已有进程"
  _build_server_args

  echo "==> 启动后端 funread-api，监听 ${FUNREAD_WEB_BACKEND_HOST}:${FUNREAD_WEB_BACKEND_PORT}"
  funread-api server start "${_BACKEND_SERVER_ARGS[@]}" \
    || die "后端启动失败，已中止（前端未启动）"
  _wait_for_listener "${FUNREAD_WEB_BACKEND_PORT}" || {
    funread-api server stop "${_BACKEND_STOP_ARGS[@]}" || true
    _die_backend_never_listened
  }

  trap 'echo ""; echo "==> 停止后端 funread-api"; funread-api server stop "${_BACKEND_STOP_ARGS[@]}" || true' EXIT

  echo "==> 前台启动前端 funread-web（Ctrl+C 退出，退出时自动停后端）"
  funread-web server run "${_FRONTEND_SERVER_ARGS[@]}"
}

# best effort：某一端未安装/未运行不算错误，只提示，不阻断另一端的停止。
cmd_stop() {
  _build_server_args
  if command -v funread-web >/dev/null 2>&1; then
    echo "==> 停止前端 funread-web"
    funread-web server stop "${_FRONTEND_STOP_ARGS[@]}" || echo "警告: 前端停止失败或未在运行" >&2
  else
    echo "前端未安装，跳过。"
  fi
  if command -v funread-api >/dev/null 2>&1; then
    echo "==> 停止后端 funread-api"
    funread-api server stop "${_BACKEND_STOP_ARGS[@]}" || echo "警告: 后端停止失败或未在运行" >&2
  else
    echo "后端未安装，跳过。"
  fi
  _warn_if_still_listening "前端" "${FUNREAD_WEB_FRONTEND_PORT}"
  _warn_if_still_listening "后端" "${FUNREAD_WEB_BACKEND_PORT}"
  echo "已停止（best effort）。"
}

cmd_restart() {
  cmd_stop
  cmd_start
}

cmd_status() {
  local npm_bin backend_version frontend_version
  _build_server_args
  backend_version="$(_backend_version)"
  echo "== 已安装版本 =="
  echo "后端 ${FUNREAD_WEB_BACKEND_PACKAGE}: ${backend_version:-未安装}（$(_backend_mode) 模式）"
  if npm_bin="$(_resolve_npm 2>/dev/null)"; then
    frontend_version="$(_npm_global_version "${npm_bin}" "${FUNREAD_WEB_FRONTEND_PACKAGE}")"
    echo "前端 ${FUNREAD_WEB_FRONTEND_PACKAGE}: ${frontend_version:-未安装}（$(basename "${npm_bin}")）"
  else
    echo "前端 ${FUNREAD_WEB_FRONTEND_PACKAGE}: 未知（没有可用的 npm/pnpm）"
  fi

  echo ""
  echo "== 后端 funread-api（${FUNREAD_WEB_BACKEND_HOST}:${FUNREAD_WEB_BACKEND_PORT}）=="
  if command -v funread-api >/dev/null 2>&1; then
    funread-api server status "${_BACKEND_STOP_ARGS[@]}" || true
  else
    echo "未安装（./setup.sh install）"
  fi
  echo ""
  echo "== 前端 funread-web（${FUNREAD_WEB_FRONTEND_HOST}:${FUNREAD_WEB_FRONTEND_PORT}）=="
  if command -v funread-web >/dev/null 2>&1; then
    funread-web server status "${_FRONTEND_STOP_ARGS[@]}" || true
  else
    echo "未安装（./setup.sh install）"
  fi
}

cmd_uninstall() {
  local npm_bin
  fundeploy_confirm_destructive \
    "将停止并卸载 ${FUNREAD_WEB_BACKEND_PACKAGE}（$(_backend_mode) 模式）与 ${FUNREAD_WEB_FRONTEND_PACKAGE}（全局 npm 包），确认？" \
    FUNREAD_WEB_UNINSTALL_YES || return 1

  # 先停再卸：绝不把包从活着的进程底下抽掉。
  cmd_stop

  if npm_bin="$(_resolve_npm 2>/dev/null)"; then
    echo "==> 卸载前端 ${FUNREAD_WEB_FRONTEND_PACKAGE}"
    _npm_uninstall_global "${npm_bin}" "${FUNREAD_WEB_FRONTEND_PACKAGE}" \
      || echo "警告: 卸载 ${FUNREAD_WEB_FRONTEND_PACKAGE} 失败" >&2
  else
    echo "警告: 未找到 npm/pnpm，跳过前端卸载" >&2
  fi

  echo "==> 卸载后端 ${FUNREAD_WEB_BACKEND_PACKAGE}"
  _backend_uninstall

  echo "已卸载。两个 CLI 的 config / 日志没动:"
  echo "  $(_backend_state_dir)"
  echo "  $(_frontend_state_dir)"
}

interactive_main() {
  _fundeploy_ensure_gum || exit 1
  fundeploy_ui_apply_theme
  fundeploy_ui_banner "fundeploy / service / funread-web" \
    "后端 ${FUNREAD_WEB_BACKEND_HOST}:${FUNREAD_WEB_BACKEND_PORT}  前端 ${FUNREAD_WEB_FRONTEND_HOST}:${FUNREAD_WEB_FRONTEND_PORT}"
  set +e
  while true; do
    local pick arg
    pick="$(fundeploy_ui_choose "fundeploy / service / funread-web / 选择动作" \
      "install     安装后端与前端正式包" \
      "upgrade     升级到最新版并打印版本对比" \
      "rollback    回退前后端到指定版本" \
      "start       启动（先后端，再前端）" \
      "run         前台运行（Ctrl+C 连后端一起停）" \
      "stop        停止（先前端，再后端）" \
      "restart     重启" \
      "status      查看版本与状态" \
      "uninstall   卸载" \
      "help        命令帮助" \
      "quit        返回")" || break
    [[ -z "$pick" ]] && break
    pick="${pick%% *}"
    case "$pick" in
      quit) break ;;
      help) usage ;;
      install) cmd_install ;;
      upgrade) cmd_upgrade ;;
      rollback)
        read -r -p "回退到的版本号（如 1.1.109，留空取消）: " arg
        [[ -n "$arg" ]] && cmd_rollback "$arg"
        ;;
      start) cmd_start ;;
      run) cmd_run ;;
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
  shift
  case "$cmd" in
    install) cmd_install ;;
    update | upgrade) cmd_upgrade ;;
    rollback) cmd_rollback "${1:-}" ;;
    start) cmd_start ;;
    run) cmd_run ;;
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
