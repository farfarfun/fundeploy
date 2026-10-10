#!/usr/bin/env bash
# funflix-web 一体化部署：把 funflix-api（后端，PyPI 包）与 @farfarfun/funflix-web
# （前端，私有 npm 包，bin 名 funflix-web）作为同一个服务单元来装/起/停——两者本是
# 各自独立的进程，但这里只暴露合并命令，不提供拆开单独控制前后端的子命令；
# 如需单独控制，直接用各自的 CLI（funflix-api / funflix-web）。
#
# 上游（本脚本对齐 funflix-api / funflix-web 1.0.32）:
#   后端 https://github.com/farfarfun/funflix-api     （发布到 PyPI，包名 funflix-api；
#                                                        依赖核心库 funflix）
#   前端 https://github.com/farfarfun/funflix-web     （发布到私有 npm 源，
#        包名 @farfarfun/funflix-web，bin 名 funflix-web）
#
# 前后端各自已提供健壮的生命周期命令（各自管理自己的 PID/日志），本脚本只负责
# 「装两个包 + 按依赖顺序编排调用」，不重复维护 PID 文件。两边的生命周期命令都在
# server 子分组下（server run/start/stop/restart/status），PID/日志落在：
#   后端 ~/.farfarfun/funflix/api/{server.pid,server.log}（与 --config 默认路径同一棵树）
#   前端 ~/.farfarfun/funflix/web/（FUNFLIX_WEB_STATE_DIR 可改）
# 要求后端 >= 1.0.30：server 子分组是那一版引入的，更早的版本是顶层命令。
#
# 鉴权（1.0.30 起的「整站门禁」，与旧版差别很大）：
#   除 /api/v1/auth/* 与 /healthz 外所有接口都要求登录（会话 cookie），运维区还要求
#   admin 角色。旧的 FUNFLIX_ADMIN_API_KEY 已经不存在，界面里也没有「管理密钥」入口。
#   新机器装完必须先 migrate 建表、再 user-create 建账号，否则界面只有一个登录页，
#   而且没有任何账号能登进去：
#       ./setup.sh migrate                 # funflix db upgrade
#       ./setup.sh user-create <用户名>     # funflix user create（密码交互式输入）
#   自助注册默认关闭；要开就 FUNFLIX_REGISTRATION_ENABLED=true，再用
#   `funflix invite create` 发邀请码。
#
# 安装路线（脚本自动跟着机器上已有的那一份走，不制造第二份安装）：
#   后端 uvtool 模式（默认，新机器）
#       uv tool install --upgrade funflix-api --with-executables-from funflix
#       与上游 scripts/setup.sh install-prod、以及 funflix-api 自带的 upgrade/rollback
#       同一个安装根；--with-executables-from 把核心库 funflix 的 CLI 一起暴露出来——
#       建表、建账号、邀请码都在它手里，少了它新机器根本进不去界面。
#   后端 pip 模式
#       机器上已经有一份 pip/venv 装的 funflix-api，或显式给了 FUNFLIX_WEB_PIP_BIN 时
#       走这条，并且按 PATH 上那个 funflix-api 的 shebang 定位同一个 python，确保升级/
#       卸载作用在真正会被跑到的那份安装上（`uv pip` 自己探测环境并不总是同一个）。
#   前端
#       谁装的就继续用谁（pnpm / npm），没装过时优先 pnpm。
#
# 为什么 upgrade/rollback/uninstall 不直接转调上游 CLI 的同名命令：
#   - `funflix-api upgrade` 跑的是 `uv tool install --upgrade funflix-api`，不带
#     --with-executables-from，会把 funflix 这个 CLI 从 bin 目录摘掉，迁移/建账号的
#     命令随之消失；
#   - `funflix-web upgrade` 写死了 `npm install -g`，机器上是 pnpm 全局装的话会装出
#     第二份，PATH 里仍是旧版——升级看着成功，其实没生效。
#   所以这两件事由本脚本自己做，顺带把前后端一起装、一起打印版本对比。
#
# 依赖: uv（或 pip）装后端；pnpm 或 npm 装前端。前端包在私有 npm 源上，装之前机器上
#   需要有一次性的 scope 映射：
#     npm config set @farfarfun:registry https://farfarfun-cn-hangzhou.devops.aliyuncs.com/packages/api/protocol/npm/funnpm/
#
# 用法：
#   ./setup.sh                      # gum 菜单
#   ./setup.sh install              # 装后端（uv tool / pip）+ 前端（pnpm / npm -g）
#   ./setup.sh upgrade              # 同 install（装到最新/指定版本），并打印升级前后版本对比
#   ./setup.sh rollback <版本>      # 前后端一起钉到指定版本（先停、装完按原状态恢复）
#   ./setup.sh start                # 先启动后端，再启动前端（自动把 --backend 指向后端地址）
#   ./setup.sh run                  # 启动后端后，前台运行前端；退出时顺带停掉后端
#   ./setup.sh stop                 # 先停止前端，再停止后端
#   ./setup.sh restart              # stop + start
#   ./setup.sh status               # 安装版本 + funflix-api / funflix-web 各自的状态
#   ./setup.sh migrate              # funflix db upgrade（建表/迁移，首次部署必跑）
#   ./setup.sh user-create <用户名>  # funflix user create（建登录账号，首次部署必跑）
#   ./setup.sh uninstall            # 停止两者，卸载 npm 包与 python 包
#
# 环境变量：
#   FUNFLIX_WEB_BACKEND_PACKAGE    后端 python 包名（默认 funflix-api）
#   FUNFLIX_WEB_BACKEND_VERSION    后端版本号（默认空＝最新）
#   FUNFLIX_WEB_CORE_PACKAGE       随后端一起暴露 CLI 的核心包（默认 funflix；置空则不暴露）
#   FUNFLIX_WEB_FRONTEND_PACKAGE   前端 npm 包名（默认 @farfarfun/funflix-web）
#   FUNFLIX_WEB_FRONTEND_VERSION   前端版本号（默认空＝最新）
#   FUNFLIX_WEB_PIP_BIN            指定 pip 可执行路径；给了就走 pip 模式
#   FUNFLIX_WEB_NPM_BIN            指定 npm/pnpm 可执行路径（默认自动探测）
#   FUNFLIX_WEB_BACKEND_HOST       后端监听地址（默认 127.0.0.1）
#   FUNFLIX_WEB_BACKEND_PORT       后端监听端口（默认 18810）
#   FUNFLIX_WEB_FRONTEND_HOST      前端监听地址（默认 127.0.0.1）
#   FUNFLIX_WEB_FRONTEND_PORT      前端监听端口（默认 8810）
#   FUNFLIX_WEB_BACKEND_CONFIG     后端 --config 路径（默认空＝上游默认路径）
#   FUNFLIX_WEB_FRONTEND_CONFIG    前端 --config 路径（默认空＝上游默认路径）
#   FUNFLIX_SESSION_SECRET         会话签名密钥；不设则每次重启换新的，所有人被登出
#   FUNFLIX_DATABASE_URL           后端/migrate/user-create 共用的库地址（原样继承自环境）
#   NONINTERACTIVE=1
#   FUNFLIX_WEB_UNINSTALL_YES=1    非 TTY 卸载确认
#   FUNFLIX_WEB_MIGRATE_YES=1      非 TTY 迁移确认

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../../lib/fundeploy-common.sh
source "${SCRIPT_DIR}/../../lib/fundeploy-common.sh"

FUNFLIX_WEB_BACKEND_PACKAGE="${FUNFLIX_WEB_BACKEND_PACKAGE:-funflix-api}"
FUNFLIX_WEB_BACKEND_VERSION="${FUNFLIX_WEB_BACKEND_VERSION:-}"
# 后端包的 CLI 只管 HTTP 服务；建表、建账号、邀请码都在核心库 funflix 的 CLI 里，
# uv tool 模式下必须显式要求把它的可执行文件一起装出来。
FUNFLIX_WEB_CORE_PACKAGE="${FUNFLIX_WEB_CORE_PACKAGE-funflix}"
FUNFLIX_WEB_FRONTEND_PACKAGE="${FUNFLIX_WEB_FRONTEND_PACKAGE:-@farfarfun/funflix-web}"
# 前端包从裸名 funflix-web 改成了 @farfarfun/funflix-web, bin 名没变。老机器上
# 装的还是裸名包, 它占着全局 bin/funflix-web, 新包装不进去（npm EEXIST）。
FUNFLIX_WEB_FRONTEND_LEGACY_PACKAGE="funflix-web"
FUNFLIX_WEB_FRONTEND_VERSION="${FUNFLIX_WEB_FRONTEND_VERSION:-}"
# 仅用于报错提示：pnpm 不接受 --@scope:registry= 这种命令行覆盖，所以 scope 映射
# 只能是机器上的一次性配置，这里只负责在装不上时把该敲的命令告诉用户。
FUNFLIX_WEB_FRONTEND_REGISTRY="https://farfarfun-cn-hangzhou.devops.aliyuncs.com/packages/api/protocol/npm/funnpm/"
FUNFLIX_WEB_PIP_BIN="${FUNFLIX_WEB_PIP_BIN:-}"
FUNFLIX_WEB_NPM_BIN="${FUNFLIX_WEB_NPM_BIN:-}"
FUNFLIX_WEB_BACKEND_HOST="${FUNFLIX_WEB_BACKEND_HOST:-127.0.0.1}"
FUNFLIX_WEB_BACKEND_PORT="${FUNFLIX_WEB_BACKEND_PORT:-18810}"
FUNFLIX_WEB_FRONTEND_HOST="${FUNFLIX_WEB_FRONTEND_HOST:-127.0.0.1}"
FUNFLIX_WEB_FRONTEND_PORT="${FUNFLIX_WEB_FRONTEND_PORT:-8810}"
FUNFLIX_WEB_BACKEND_CONFIG="${FUNFLIX_WEB_BACKEND_CONFIG:-}"
FUNFLIX_WEB_FRONTEND_CONFIG="${FUNFLIX_WEB_FRONTEND_CONFIG:-}"

die() { echo "错误: $*" >&2; exit 1; }

usage() {
  cat <<USAGE
用法: ./setup.sh [command]

  无参数：gum 菜单。

命令:
  install            装后端 ${FUNFLIX_WEB_BACKEND_PACKAGE}（uv tool / pip）+ 前端 ${FUNFLIX_WEB_FRONTEND_PACKAGE}（pnpm / npm -g）
  upgrade            同 install，并打印升级前后的版本对比
  rollback <版本>    前后端一起钉到指定版本（先停、装完恢复原运行状态）
  start              按序启动：先 funflix-api 后端，再 funflix-web 前端（自动带上 --backend）
  run                启动后端后，前台运行前端；Ctrl-C 退出时顺带停掉后端
  stop               按序停止：先前端，再后端（best effort，不因某一端未运行而报错）
  restart            stop + start
  status             打印已安装版本 + funflix-api / funflix-web 各自的状态
  migrate            funflix db upgrade：建表 / 执行数据库迁移（首次部署必跑）
  user-create <用户名> [选项]
                     funflix user create：建登录账号（首次部署必跑；密码省略即交互输入）
  uninstall          停止两者，卸载 npm 包与 python 包

说明:
  - 前后端是两个独立进程，各自的 PID/日志由上游 CLI 自己管理，本脚本不重复维护：
      funflix-api（后端）  ~/.farfarfun/funflix/api/{server.pid,server.log}
      funflix-web（前端）  ~/.farfarfun/funflix/web/
  - 后端: http://${FUNFLIX_WEB_BACKEND_HOST}:${FUNFLIX_WEB_BACKEND_PORT}（接口文档 /docs，健康检查 /healthz）
  - 前端: http://${FUNFLIX_WEB_FRONTEND_HOST}:${FUNFLIX_WEB_FRONTEND_PORT}/web（浏览器打开这个）
  - 鉴权是整站门禁：除 /api/v1/auth/* 与 /healthz 外都要求登录，运维区还要求 admin。
    新机器装完先 ./setup.sh migrate，再 ./setup.sh user-create <用户名>，否则没有账号能登进去。
    FUNFLIX_ADMIN_API_KEY 在 1.0.30 之后已废弃，界面里也没有「管理密钥」入口了。
  - 生产部署务必固定 FUNFLIX_SESSION_SECRET：不设的话后端每次重启都换签名密钥，
    所有已登录会话立即失效。
  - 只提供合并命令；如需单独控制某一端，直接用 funflix-api / funflix-web 各自的 CLI
    （两边都是 server 子分组：funflix-api server start/stop/status）。但包管理用本脚本的
    upgrade/rollback/uninstall，别用上游 CLI 自带的同名命令——理由见文件头注释。
  - 前端包在私有 npm 源上，装不上先确认机器上有 scope 映射：
      npm config set @farfarfun:registry ${FUNFLIX_WEB_FRONTEND_REGISTRY}

上游: https://github.com/farfarfun/funflix-api ・ https://github.com/farfarfun/funflix-web
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
    FUNFLIX_WEB_PKG_QUERY="${pkg}" python3 -c '
import json, os, sys
data = json.loads(sys.stdin.read() or "{}")
pkg = os.environ.get("FUNFLIX_WEB_PKG_QUERY", "")
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
  if [[ -n "${FUNFLIX_WEB_NPM_BIN}" ]]; then
    [[ -x "${FUNFLIX_WEB_NPM_BIN}" ]] || die "FUNFLIX_WEB_NPM_BIN 无效: ${FUNFLIX_WEB_NPM_BIN}"
    echo "${FUNFLIX_WEB_NPM_BIN}"
    return
  fi
  pnpm_bin="$(command -v pnpm 2>/dev/null || true)"
  npm_bin="$(command -v npm 2>/dev/null || true)"
  # 已经装过的那一份归谁管，就继续用谁：两个包管理器的全局 bin 目录不同，用另一个
  # 装出来的是第二份副本，PATH 里仍是旧的——upgrade 看着成功，跑的还是老版本。
  if [[ -n "${pnpm_bin}" && -n "$(_npm_global_version "${pnpm_bin}" "${FUNFLIX_WEB_FRONTEND_PACKAGE}")" ]]; then
    echo "${pnpm_bin}"
    return
  fi
  if [[ -n "${npm_bin}" && -n "$(_npm_global_version "${npm_bin}" "${FUNFLIX_WEB_FRONTEND_PACKAGE}")" ]]; then
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

# pnpm add/install -g <pkg>@latest 偶发会命中过期的 dist-tag 解析缓存，装出
# 比 latest 旧的版本（即使当场 pnpm/npm view 已经能查到新版本号）。为绕开这个
# 坑，"latest" 一律先用 view 查出具体版本号，再按精确版本号安装；查不到时才
# 退回字面量 @latest。
_npm_view_version() {
  local npm_bin="$1" pkg="$2"
  "${npm_bin}" view "${pkg}" version 2>/dev/null | tail -1
}

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

# 加 scope 之前装的裸名全局包占着 bin/funflix-web, 不先摘掉, 新的 scoped 包会在
# npm 建 bin 软链时直接 EEXIST 失败。只在目标包确实是 scoped 名时才清, 免得把
# 用户用 FUNFLIX_WEB_FRONTEND_PACKAGE 显式指回裸名的安装给卸了。
_npm_drop_legacy_unscoped() {
  local npm_bin="$1" legacy_version
  [[ "${FUNFLIX_WEB_FRONTEND_PACKAGE}" != "${FUNFLIX_WEB_FRONTEND_LEGACY_PACKAGE}" ]] || return 0
  legacy_version="$(_npm_global_version "${npm_bin}" "${FUNFLIX_WEB_FRONTEND_LEGACY_PACKAGE}")"
  [[ -n "${legacy_version}" ]] || return 0
  echo "==> 移除改 scope 前的旧前端包 ${FUNFLIX_WEB_FRONTEND_LEGACY_PACKAGE}@${legacy_version}（它占着 bin/funflix-web）"
  _npm_uninstall_global "${npm_bin}" "${FUNFLIX_WEB_FRONTEND_LEGACY_PACKAGE}" \
    || echo "警告: 旧包卸载失败, 接下来的安装可能因 bin 冲突失败" >&2
}

# ------------------------------------------------------------------ 后端包管理

_uv_tool_version() {
  command -v uv >/dev/null 2>&1 || { echo ""; return; }
  uv tool list 2>/dev/null | awk -v pkg="$1" '$1 == pkg && $2 ~ /^v/ { print substr($2, 2); exit }'
}

# PATH 上那个 funflix-api 由哪个 python 跑：console script 的 shebang 就是答案，
# uv tool 装的（指向 tool venv）和 pip 装的（指向某个 venv）都适用。
_backend_cli_python() {
  local cli shebang
  cli="$(command -v funflix-api 2>/dev/null || true)"
  [[ -n "${cli}" && -f "${cli}" ]] || return 1
  shebang="$(head -1 "${cli}" 2>/dev/null || true)"
  [[ "${shebang}" == '#!'* ]] || return 1
  shebang="${shebang#\#!}"
  # 形如 `#!/usr/bin/env python3` 的壳脚本定位不到具体环境，当作探测失败
  [[ "${shebang}" == /* && "${shebang}" != *" "* ]] || return 1
  printf '%s' "${shebang}"
}

# uvtool：uv tool 装的（与上游 install-prod 一致）。
# pip   ：显式 FUNFLIX_WEB_PIP_BIN，或机器上已有一份非 uv tool 的安装——继续用 pip
#         路线，免得 uv tool 再装出第二份、由 PATH 顺序决定跑哪个。
_backend_mode() {
  [[ -n "${FUNFLIX_WEB_PIP_BIN}" ]] && { echo pip; return; }
  command -v uv >/dev/null 2>&1 || { echo pip; return; }
  [[ -n "$(_uv_tool_version "${FUNFLIX_WEB_BACKEND_PACKAGE}")" ]] && { echo uvtool; return; }
  command -v funflix-api >/dev/null 2>&1 && { echo pip; return; }
  echo uvtool
}

# PATH 上真正会跑到的那份后端的版本。取包元数据，不用 funflix-api --version——
# 上游的 __version__ 没随发布更新，1.0.32 的包自报 0.1.0（server status 也一样）。
# 未安装时输出空字符串。
_backend_version() {
  local py
  py="$(_backend_cli_python || true)"
  [[ -n "${py}" ]] || { echo ""; return; }
  FUNFLIX_WEB_PKG_QUERY="${FUNFLIX_WEB_BACKEND_PACKAGE}" "${py}" -c '
import importlib.metadata as m, os
try:
    print(m.version(os.environ["FUNFLIX_WEB_PKG_QUERY"]))
except m.PackageNotFoundError:
    print("")
' 2>/dev/null || echo ""
}

# 本脚本这条安装路线所管理的那份的版本（uv tool 模式下未必等于 PATH 上那份）。
_backend_managed_version() {
  if [[ "$(_backend_mode)" == uvtool ]]; then
    _uv_tool_version "${FUNFLIX_WEB_BACKEND_PACKAGE}"
  else
    _backend_version
  fi
}

# $1=包规格（funflix-api 或 funflix-api==x.y.z）  $2=1 表示已钉死版本
_backend_install() {
  local spec="$1" pinned="${2:-0}" args py
  if [[ "$(_backend_mode)" == uvtool ]]; then
    if [[ "${pinned}" == 1 ]]; then
      args=(tool install --force "${spec}")
    else
      args=(tool install --upgrade "${spec}")
    fi
    [[ -n "${FUNFLIX_WEB_CORE_PACKAGE}" ]] && args+=(--with-executables-from "${FUNFLIX_WEB_CORE_PACKAGE}")
    uv "${args[@]}"
    return
  fi

  if [[ -n "${FUNFLIX_WEB_PIP_BIN}" ]]; then
    if [[ "${pinned}" == 1 ]]; then
      "${FUNFLIX_WEB_PIP_BIN}" install "${spec}"
    else
      "${FUNFLIX_WEB_PIP_BIN}" install "${spec}" -U
    fi
    return
  fi
  # 已有安装时钉住它所在的 python：`uv pip` 自己探测出来的环境未必是同一个，
  # 装进别的环境等于升级了一份没人跑的副本。
  py="$(_backend_cli_python || true)"
  if command -v uv >/dev/null 2>&1; then
    args=(pip install)
    [[ -n "${py}" ]] && args+=(--python "${py}")
    args+=("${spec}")
    [[ "${pinned}" == 1 ]] || args+=(-U)
    uv "${args[@]}"
    return
  fi
  if [[ -n "${py}" ]]; then
    if [[ "${pinned}" == 1 ]]; then "${py}" -m pip install "${spec}"; else "${py}" -m pip install "${spec}" -U; fi
    return
  fi
  command -v python3 >/dev/null 2>&1 || die "未找到 uv/python3（可先运行 fundeploy dev uv install）"
  if [[ "${pinned}" == 1 ]]; then
    python3 -m pip install --user "${spec}"
  else
    python3 -m pip install --user "${spec}" -U
  fi
}

_backend_uninstall() {
  local pkg="${FUNFLIX_WEB_BACKEND_PACKAGE}" py
  if [[ "$(_backend_mode)" == uvtool ]]; then
    uv tool uninstall "${pkg}" || echo "警告: 卸载 ${pkg} 失败" >&2
    return
  fi
  if [[ -n "${FUNFLIX_WEB_PIP_BIN}" ]]; then
    "${FUNFLIX_WEB_PIP_BIN}" uninstall -y "${pkg}" || echo "警告: 卸载 ${pkg} 失败" >&2
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

# 装完核对一次：刚装的那份必须就是 PATH 上会跑到的那份。两份安装（venv 一份、
# uv tool 一份）并存时，PATH 顺序决定跑哪个，不核对就会「升级成功、行为没变」。
_verify_backend_on_path() {
  local managed on_path cli
  cli="$(command -v funflix-api 2>/dev/null || true)"
  [[ -n "${cli}" ]] || {
    echo "警告: 后端已安装，但 funflix-api 不在 PATH 上（uv tool 装在 $(uv tool dir --bin 2>/dev/null || echo "${HOME}/.local/bin")，" >&2
    echo "      把它加进 PATH 或执行 uv tool update-shell 后重开 shell）" >&2
    return 0
  }
  managed="$(_backend_managed_version)"
  on_path="$(_backend_version)"
  [[ -z "${managed}" || -z "${on_path}" || "${managed}" == "${on_path}" ]] && return 0
  echo "警告: 刚装的 ${FUNFLIX_WEB_BACKEND_PACKAGE} 是 ${managed}，但 PATH 上的 ${cli} 是 ${on_path}——" >&2
  echo "      机器上有第二份安装把它遮住了，先卸掉多余的那份（或调整 PATH）再重试。" >&2
}

# ---------------------------------------------------------------------- 安装

cmd_install() {
  local npm_bin backend_spec frontend_spec backend_pinned=0 mode
  npm_bin="$(_resolve_npm)"
  mode="$(_backend_mode)"
  # 前端钉版本不用额外开关：npm/pnpm 装 pkg@x.y.z 本身就是升降级都适用的精确安装。
  [[ -n "${FUNFLIX_WEB_BACKEND_VERSION}" ]] && backend_pinned=1
  backend_spec="$(_pip_pkg_spec "${FUNFLIX_WEB_BACKEND_PACKAGE}" "${FUNFLIX_WEB_BACKEND_VERSION}")"
  frontend_spec="$(_npm_pkg_spec "${npm_bin}" "${FUNFLIX_WEB_FRONTEND_PACKAGE}" "${FUNFLIX_WEB_FRONTEND_VERSION}")"

  echo "==> 安装后端: ${backend_spec}（${mode} 模式）"
  _backend_install "${backend_spec}" "${backend_pinned}" || die "后端安装失败: ${backend_spec}"

  _npm_drop_legacy_unscoped "${npm_bin}"
  if _npm_is_pnpm "${npm_bin}"; then
    echo "==> 安装前端: ${npm_bin} add -g ${frontend_spec}"
  else
    echo "==> 安装前端: ${npm_bin} install -g ${frontend_spec}"
  fi
  _npm_install_global "${npm_bin}" "${frontend_spec}" || die "前端安装失败: ${frontend_spec}" \
    "（私有源 scope 映射缺失？执行: npm config set @farfarfun:registry ${FUNFLIX_WEB_FRONTEND_REGISTRY}）"

  _verify_backend_on_path
  echo "已安装。"
}

cmd_upgrade() {
  local npm_bin backend_before backend_after frontend_before frontend_after
  npm_bin="$(_resolve_npm)"
  backend_before="$(_backend_version)"
  frontend_before="$(_npm_global_version "${npm_bin}" "${FUNFLIX_WEB_FRONTEND_PACKAGE}")"

  cmd_install

  backend_after="$(_backend_version)"
  frontend_after="$(_npm_global_version "${npm_bin}" "${FUNFLIX_WEB_FRONTEND_PACKAGE}")"

  echo ""
  echo "== 版本变化 =="
  echo "后端 ${FUNFLIX_WEB_BACKEND_PACKAGE}: ${backend_before:-未安装} -> ${backend_after:-未知}"
  echo "前端 ${FUNFLIX_WEB_FRONTEND_PACKAGE}: ${frontend_before:-未安装} -> ${frontend_after:-未知}"
  if [[ -n "$(_fundeploy_listener_pid_for_port "${FUNFLIX_WEB_BACKEND_PORT}")" \
     || -n "$(_fundeploy_listener_pid_for_port "${FUNFLIX_WEB_FRONTEND_PORT}")" ]]; then
    echo ""
    echo "注意: 服务仍在用旧代码运行，执行 ./setup.sh restart 才会切到新版本。"
  fi
}

# 回退：前后端用的是 funbuild 的同一条版本线（PyPI 与 npm 同号），所以一个版本号
# 同时钉两端。先停服再装，避免运行中的进程跑在一半换掉的文件上；装完按停服前的
# 状态恢复。
cmd_rollback() {
  local version="${1:-}" was_running=0 npm_bin
  [[ -n "${version}" ]] || die "rollback 需要版本号，例如: ./setup.sh rollback 1.0.31"
  version="${version#v}"

  # 先确认 npm 源上真有这个版本。install 是「先后端、后前端」，前端装不上而后端
  # 已经降级的话，会停在前后端版本不一致的中间态上。
  npm_bin="$(_resolve_npm)"
  [[ -n "$(_npm_view_version "${npm_bin}" "${FUNFLIX_WEB_FRONTEND_PACKAGE}@${version}")" ]] \
    || die "npm 源上没有 ${FUNFLIX_WEB_FRONTEND_PACKAGE}@${version}，已中止（未改动任何安装）"

  if [[ -n "$(_fundeploy_listener_pid_for_port "${FUNFLIX_WEB_BACKEND_PORT}")" \
     || -n "$(_fundeploy_listener_pid_for_port "${FUNFLIX_WEB_FRONTEND_PORT}")" ]]; then
    was_running=1
    cmd_stop
  fi

  FUNFLIX_WEB_BACKEND_VERSION="${version}"
  FUNFLIX_WEB_FRONTEND_VERSION="${version}"
  cmd_install

  if [[ "${was_running}" == 1 ]]; then
    echo "==> 回退前服务在运行，重新启动"
    cmd_start
  else
    echo "已回退到 ${version}（回退前服务未运行，未自动启动）。"
  fi
}

# ---------------------------------------------------------------- 生命周期

# server start 只要成功 fork 出子进程就会返回 0；子进程若在几秒内因异常退出
# （比如上游包自身的 bug），start 命令本身并不会失败。这里等端口真正被监听
# 到，避免把「已启动」误报给用户。
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
  echo "警告: ${label}端口 ${port} 仍被 PID ${pid} 监听（上游 stop 只发 SIGTERM、超时不强杀）；" >&2
  echo "      确认要强停: kill -TERM ${pid}" >&2
}

_BACKEND_SERVER_ARGS=()
_FRONTEND_SERVER_ARGS=()
_build_server_args() {
  _BACKEND_SERVER_ARGS=(--host "${FUNFLIX_WEB_BACKEND_HOST}" --port "${FUNFLIX_WEB_BACKEND_PORT}")
  [[ -n "${FUNFLIX_WEB_BACKEND_CONFIG}" ]] && _BACKEND_SERVER_ARGS+=(--config "${FUNFLIX_WEB_BACKEND_CONFIG}")
  _FRONTEND_SERVER_ARGS=(
    --host "${FUNFLIX_WEB_FRONTEND_HOST}"
    --port "${FUNFLIX_WEB_FRONTEND_PORT}"
    --backend "http://${FUNFLIX_WEB_BACKEND_HOST}:${FUNFLIX_WEB_BACKEND_PORT}"
  )
  [[ -n "${FUNFLIX_WEB_FRONTEND_CONFIG}" ]] && _FRONTEND_SERVER_ARGS+=(--config "${FUNFLIX_WEB_FRONTEND_CONFIG}")
  return 0
}

# 显式 --host/--port 会盖掉配置文件里的同名字段，这是故意的：fundeploy 的状态表与
# 端口探测都以这里的变量为准。只想用配置文件里的值，就把对应变量设成相同的值。
_warn_session_secret() {
  [[ -n "${FUNFLIX_SESSION_SECRET:-}" ]] && return 0
  echo "提示: 未设置 FUNFLIX_SESSION_SECRET —— 后端每次重启都会生成新的会话签名密钥，" >&2
  echo "      所有已登录用户会被登出。生产部署请固定这个值。" >&2
}

cmd_start() {
  _require_cli funflix-api "安装 ${FUNFLIX_WEB_BACKEND_PACKAGE} 后应在 PATH 中"
  _require_cli funflix-web "npm/pnpm -g 安装 ${FUNFLIX_WEB_FRONTEND_PACKAGE} 后应在 PATH 中"
  _warn_session_secret
  _build_server_args

  # start 在目标已经运行时，上游 CLI 会返回非零退出码（"已在运行，请用 restart"）；
  # 这里先看端口是否已被监听，已在运行就跳过、不当成失败。
  if [[ -n "$(_fundeploy_listener_pid_for_port "${FUNFLIX_WEB_BACKEND_PORT}")" ]]; then
    echo "后端 ${FUNFLIX_WEB_BACKEND_HOST}:${FUNFLIX_WEB_BACKEND_PORT} 已在运行，跳过。"
  else
    echo "==> 启动后端 funflix-api，监听 ${FUNFLIX_WEB_BACKEND_HOST}:${FUNFLIX_WEB_BACKEND_PORT}"
    funflix-api server start "${_BACKEND_SERVER_ARGS[@]}" \
      || die "后端启动失败，已中止（前端未启动）"
    _wait_for_listener "${FUNFLIX_WEB_BACKEND_PORT}" \
      || die "后端启动命令已返回，但端口 ${FUNFLIX_WEB_BACKEND_PORT} 迟迟未监听（多半是启动后崩溃）。" \
             "已中止（前端未启动）。请看日志: ~/.farfarfun/funflix/api/server.log，" \
             "或跑 funflix-api server run 看前台报错（库没迁移过的话先 ./setup.sh migrate）。"
  fi

  if [[ -n "$(_fundeploy_listener_pid_for_port "${FUNFLIX_WEB_FRONTEND_PORT}")" ]]; then
    echo "前端 ${FUNFLIX_WEB_FRONTEND_HOST}:${FUNFLIX_WEB_FRONTEND_PORT} 已在运行，跳过。"
  else
    echo "==> 启动前端 funflix-web，监听 ${FUNFLIX_WEB_FRONTEND_HOST}:${FUNFLIX_WEB_FRONTEND_PORT}"
    funflix-web server start "${_FRONTEND_SERVER_ARGS[@]}" \
      || die "前端启动失败（后端已启动；如需回滚请执行 ./setup.sh stop）"
    _wait_for_listener "${FUNFLIX_WEB_FRONTEND_PORT}" \
      || die "前端启动命令已返回，但端口 ${FUNFLIX_WEB_FRONTEND_PORT} 迟迟未监听（多半是启动后崩溃）。" \
             "后端已启动；如需回滚请执行 ./setup.sh stop。日志见 ~/.farfarfun/funflix/web/。"
  fi

  echo "已启动。界面: http://${FUNFLIX_WEB_FRONTEND_HOST}:${FUNFLIX_WEB_FRONTEND_PORT}/web"
  echo "整站要求登录；还没有账号就先 ./setup.sh migrate && ./setup.sh user-create <用户名>。"
}

# 前台跑前端，后端仍是后台进程（上游没有「两个进程都前台」的形态）。退出时把自己
# 起的后端一并停掉——留一个孤儿后端在那儿比没启动更糟，所以这里不用 exec。
cmd_run() {
  _require_cli funflix-api "安装 ${FUNFLIX_WEB_BACKEND_PACKAGE} 后应在 PATH 中"
  _require_cli funflix-web "npm/pnpm -g 安装 ${FUNFLIX_WEB_FRONTEND_PACKAGE} 后应在 PATH 中"
  [[ -z "$(_fundeploy_listener_pid_for_port "${FUNFLIX_WEB_BACKEND_PORT}")" ]] \
    || die "后端端口 ${FUNFLIX_WEB_BACKEND_PORT} 已被占用；run 不能接管已有进程"
  [[ -z "$(_fundeploy_listener_pid_for_port "${FUNFLIX_WEB_FRONTEND_PORT}")" ]] \
    || die "前端端口 ${FUNFLIX_WEB_FRONTEND_PORT} 已被占用；run 不能接管已有进程"
  _warn_session_secret
  _build_server_args

  echo "==> 启动后端 funflix-api，监听 ${FUNFLIX_WEB_BACKEND_HOST}:${FUNFLIX_WEB_BACKEND_PORT}"
  funflix-api server start "${_BACKEND_SERVER_ARGS[@]}" \
    || die "后端启动失败，已中止（前端未启动）"
  _wait_for_listener "${FUNFLIX_WEB_BACKEND_PORT}" || {
    funflix-api server stop || true
    die "后端未监听端口 ${FUNFLIX_WEB_BACKEND_PORT}，前端未启动（日志: ~/.farfarfun/funflix/api/server.log）"
  }

  trap 'echo ""; echo "==> 停止后端 funflix-api"; funflix-api server stop || true' EXIT

  echo "==> 前台启动前端 funflix-web（Ctrl+C 退出，退出时自动停后端）"
  funflix-web server run "${_FRONTEND_SERVER_ARGS[@]}"
}

# best effort：某一端未安装/未运行不算错误，只提示，不阻断另一端的停止。
cmd_stop() {
  if command -v funflix-web >/dev/null 2>&1; then
    echo "==> 停止前端 funflix-web"
    funflix-web server stop || echo "警告: 前端停止失败或未在运行" >&2
  else
    echo "前端未安装，跳过。"
  fi
  if command -v funflix-api >/dev/null 2>&1; then
    echo "==> 停止后端 funflix-api"
    funflix-api server stop || echo "警告: 后端停止失败或未在运行" >&2
  else
    echo "后端未安装，跳过。"
  fi
  _warn_if_still_listening "前端" "${FUNFLIX_WEB_FRONTEND_PORT}"
  _warn_if_still_listening "后端" "${FUNFLIX_WEB_BACKEND_PORT}"
  echo "已停止（best effort）。"
}

cmd_restart() {
  cmd_stop
  cmd_start
}

cmd_status() {
  local npm_bin backend_version frontend_version
  backend_version="$(_backend_version)"
  echo "== 已安装版本 =="
  echo "后端 ${FUNFLIX_WEB_BACKEND_PACKAGE}: ${backend_version:-未安装}（$(_backend_mode) 模式）"
  if npm_bin="$(_resolve_npm 2>/dev/null)"; then
    frontend_version="$(_npm_global_version "${npm_bin}" "${FUNFLIX_WEB_FRONTEND_PACKAGE}")"
    echo "前端 ${FUNFLIX_WEB_FRONTEND_PACKAGE}: ${frontend_version:-未安装}（$(basename "${npm_bin}")）"
  else
    echo "前端 ${FUNFLIX_WEB_FRONTEND_PACKAGE}: 未知（没有可用的 npm/pnpm）"
  fi

  echo ""
  echo "== 后端 funflix-api（${FUNFLIX_WEB_BACKEND_HOST}:${FUNFLIX_WEB_BACKEND_PORT}）=="
  if command -v funflix-api >/dev/null 2>&1; then
    # 它自报的版本号来自上游未同步的 __version__，别拿它当准，看上面那行。
    funflix-api server status || true
  else
    echo "未安装（./setup.sh install）"
  fi
  echo ""
  echo "== 前端 funflix-web（${FUNFLIX_WEB_FRONTEND_HOST}:${FUNFLIX_WEB_FRONTEND_PORT}）=="
  if command -v funflix-web >/dev/null 2>&1; then
    funflix-web server status || true
  else
    echo "未安装（./setup.sh install）"
  fi
}

# ------------------------------------------------------------ 首次部署的两步

# 建表/迁移与建账号都由核心库 funflix 的 CLI 提供（funflix-api 启动时只探一次库，
# 不会自动迁移）。两条命令作用在当前 FUNFLIX_* 环境指向的库上，务必与服务同环境执行。
_require_core_cli() {
  command -v funflix >/dev/null 2>&1 && return 0
  die "未找到 funflix CLI（建表/建账号都在它手里）。" \
      "uv tool 模式下它随 --with-executables-from 一起装出来，重跑 ./setup.sh install 即可；" \
      "若 FUNFLIX_WEB_CORE_PACKAGE 被置空则需自行安装 funflix。"
}

cmd_migrate() {
  _require_core_cli
  fundeploy_confirm_destructive \
    "将对 ${FUNFLIX_DATABASE_URL:-funsecret 里配置的数据库} 执行 funflix db upgrade（建表/迁移）。确认？" \
    FUNFLIX_WEB_MIGRATE_YES || return 1
  echo "==> funflix db upgrade"
  funflix db upgrade
}

cmd_user_create() {
  (( $# >= 1 )) || die "用法: ./setup.sh user-create <用户名> [--role admin|guest]（密码省略即交互式输入）"
  _require_core_cli
  # 参数原样透传给上游命令；不要在命令行里带 --password（会进 shell 历史与 ps），
  # 省略它 funflix 会提示隐式输入。回显只打用户名，免得把密码抄进日志。
  echo "==> funflix user create $1"
  funflix user create "$@"
}

cmd_uninstall() {
  fundeploy_confirm_destructive \
    "将停止并卸载 ${FUNFLIX_WEB_BACKEND_PACKAGE}（$(_backend_mode) 模式）与 ${FUNFLIX_WEB_FRONTEND_PACKAGE}（npm -g）。确认？" \
    FUNFLIX_WEB_UNINSTALL_YES || return 1

  if command -v funflix-web >/dev/null 2>&1; then
    echo "==> 停止前端 funflix-web"
    funflix-web server stop || echo "警告: 前端停止失败或未在运行" >&2
    echo "==> 卸载前端 npm 包 ${FUNFLIX_WEB_FRONTEND_PACKAGE}"
    # 不用 `funflix-web uninstall`：它写死 npm，pnpm 全局装的那份卸不掉。
    local npm_bin
    if npm_bin="$(_resolve_npm 2>/dev/null)"; then
      _npm_uninstall_global "${npm_bin}" "${FUNFLIX_WEB_FRONTEND_PACKAGE}" \
        || echo "警告: 前端卸载失败" >&2
    else
      echo "警告: 没有可用的 npm/pnpm，前端未卸载" >&2
    fi
  fi

  if command -v funflix-api >/dev/null 2>&1; then
    echo "==> 停止后端 funflix-api"
    funflix-api server stop || echo "警告: 后端停止失败或未在运行" >&2
  fi
  echo "==> 卸载后端 python 包 ${FUNFLIX_WEB_BACKEND_PACKAGE}"
  _backend_uninstall
  echo "已卸载（数据库与 ~/.farfarfun/funflix/ 下的配置/日志保留）。"
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
    local pick arg
    pick="$(fundeploy_ui_choose "fundeploy / service / funflix-web / 选择动作" \
      "install     安装（后端 uv tool/pip + 前端 npm）" \
      "upgrade     更新到最新/指定版本，并显示升级前后版本" \
      "rollback    回退前后端到指定版本" \
      "start       启动（后端 → 前端）" \
      "run         前台运行（后端 → 前端）" \
      "stop        停止（前端 → 后端）" \
      "restart     重启" \
      "status      查看版本与状态" \
      "migrate     建表 / 数据库迁移（首次部署）" \
      "user-create 建登录账号（首次部署）" \
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
        read -r -p "回退到的版本号（如 1.0.31，留空取消）: " arg
        [[ -n "$arg" ]] && cmd_rollback "$arg"
        ;;
      start) cmd_start ;;
      run) cmd_run ;;
      stop) cmd_stop ;;
      restart) cmd_restart ;;
      status) cmd_status ;;
      migrate) cmd_migrate ;;
      user-create)
        read -r -p "用户名（留空取消）: " arg
        [[ -n "$arg" ]] && cmd_user_create "$arg"
        ;;
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
    update|upgrade) cmd_upgrade ;;
    rollback) cmd_rollback "${1:-}" ;;
    start) cmd_start ;;
    run) cmd_run ;;
    stop) cmd_stop ;;
    restart) cmd_restart ;;
    status) cmd_status ;;
    migrate) cmd_migrate ;;
    user-create) cmd_user_create "$@" ;;
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
