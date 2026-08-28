#!/usr/bin/env bash
# pnpm：多方式安装 / 升级 / 卸载（WAR-410）
#   - pkg（默认）：通用包管理器安装 pnpm（可用时）
#   - source     ：corepack 或 npm 全局前缀安装到 NPM_GLOBAL_ROOT（默认 ~/opt/npm）
# 用法: install（默认）| version | uninstall
# 方式选择: INSTALL_METHOD=pkg|source（不设则交互选择，默认 pkg）
set -euo pipefail

die() { echo "错误: $*" >&2; exit 1; }

_PNPM_ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
for _c in "${_PNPM_ROOT_DIR}/../../lib" "${_PNPM_ROOT_DIR}/../../../lib"; do
  if [[ -f "${_c}/fundeploy-install.sh" ]]; then
    # shellcheck source=../../lib/fundeploy-install.sh
    source "${_c}/fundeploy-install.sh"
    break
  fi
done
if ! declare -F _fundeploy_say_step >/dev/null 2>&1; then
  _fundeploy_say_title() { printf '\n=== %s ===\n' "$*" >&2; }
  _fundeploy_say_step()  { printf '▸ %s\n' "$*" >&2; }
  _fundeploy_say_ok()    { printf '✓ %s\n' "$*" >&2; }
  _fundeploy_say_warn()  { printf '! %s\n' "$*" >&2; }
  _fundeploy_pm_detect() { printf ''; return 1; }
  _fundeploy_pm_install() { return 2; }
  _fundeploy_pm_uninstall() { return 2; }
  _fundeploy_resolve_method() { printf '%s\n' "${INSTALL_METHOD:-source}"; }
fi

NODE_INSTALL_ROOT="${NODE_INSTALL_ROOT:-${HOME}/opt/node}"
NPM_GLOBAL_ROOT="${NPM_GLOBAL_ROOT:-${HOME}/opt/npm}"

_fundeploy_prepend_node_path() {
  if [[ -x "${NODE_INSTALL_ROOT}/bin/node" ]]; then
    export PATH="${NODE_INSTALL_ROOT}/bin:${PATH}"
  fi
}

do_install_pkg() {
  local mgr
  mgr="$(_fundeploy_pm_detect)" || die "未探测到包管理器；请改用源码方式：INSTALL_METHOD=source $0 install"
  _fundeploy_say_step "使用 ${mgr} 安装 pnpm 包"
  _fundeploy_pm_install "${mgr}" pnpm || die "${mgr} 安装 pnpm 失败；可改用 INSTALL_METHOD=source"
  _fundeploy_say_ok "已通过 ${mgr} 安装 pnpm。"
}

do_install_source() {
  _fundeploy_prepend_node_path
  command -v node >/dev/null 2>&1 || die "需要 node（可先运行 fundeploy dev nodejs install，或自行安装 Node 16.17+）"
  if [[ "${PNPM_USE_NPM_GLOBAL:-}" == "1" ]]; then
    mkdir -p "${NPM_GLOBAL_ROOT}/bin"
    export npm_config_prefix="${NPM_GLOBAL_ROOT}"
    _fundeploy_say_step "使用 npm 全局安装 pnpm 到 ${NPM_GLOBAL_ROOT}"
    command -v npm >/dev/null 2>&1 || die "需要 npm"
    npm install -g pnpm
    echo "请将 PATH 加入: ${NPM_GLOBAL_ROOT}/bin" >&2
    return 0
  fi
  node -e 'const p=process.version.slice(1).split(".").map(Number);process.exit(p[0]>16||(p[0]===16&&p[1]>=17)?0:1)' 2>/dev/null \
    || die "corepack 需要 Node 16.17+；当前版本过低或无法解析"
  _fundeploy_say_step "启用 corepack 并激活 pnpm"
  corepack enable
  corepack prepare pnpm@latest --activate
  command -v pnpm >/dev/null 2>&1 || die "pnpm 仍未在 PATH 中（尝试重新打开终端或检查 corepack 输出）"
  _fundeploy_say_ok "pnpm 已就绪: $(command -v pnpm)"
}

do_version() {
  _fundeploy_prepend_node_path
  command -v pnpm >/dev/null 2>&1 && pnpm -v || die "未找到 pnpm"
}

do_uninstall_pkg() {
  local mgr
  mgr="$(_fundeploy_pm_detect)" || { _fundeploy_say_warn "未探测到包管理器，跳过 pkg 卸载。"; return 0; }
  _fundeploy_say_step "使用 ${mgr} 卸载 pnpm 包"
  _fundeploy_pm_uninstall "${mgr}" pnpm || _fundeploy_say_warn "${mgr} 卸载 pnpm 失败（可能本就未通过 ${mgr} 安装）。"
}

do_uninstall_source() {
  _fundeploy_prepend_node_path
  if command -v corepack >/dev/null 2>&1; then
    corepack disable pnpm 2>/dev/null || true
  fi
  if [[ -d "${NPM_GLOBAL_ROOT}" ]]; then
    rm -rf "${NPM_GLOBAL_ROOT}"
    _fundeploy_say_ok "已删除 npm 全局前缀目录: ${NPM_GLOBAL_ROOT}"
  else
    _fundeploy_say_warn "目录不存在，跳过: ${NPM_GLOBAL_ROOT}"
  fi
}

do_install() {
  local method
  method="$(_fundeploy_resolve_method pnpm)"
  _fundeploy_say_title "安装 pnpm（方式: ${method}）"
  case "${method}" in
    pkg) do_install_pkg ;;
    source) do_install_source ;;
    *) die "未知安装方式: ${method}" ;;
  esac
}

do_uninstall() {
  local method="${INSTALL_METHOD:-all}"
  _fundeploy_say_title "卸载 pnpm（方式: ${method}）"
  case "${method}" in
    pkg) do_uninstall_pkg ;;
    source) do_uninstall_source ;;
    all | "") do_uninstall_pkg; do_uninstall_source ;;
    *) die "未知卸载方式: ${method}" ;;
  esac
}

cmd="${1:-install}"
case "$cmd" in
  install | update | upgrade) do_install ;;
  version) do_version ;;
  uninstall | remove) do_uninstall ;;
  -h | --help | help)
    cat <<'EOF'
用法: pnpm/setup.sh [install|version]

  install / update   安装 pnpm；方式由 INSTALL_METHOD 决定（默认 pkg，交互时可选）
  version            pnpm -v
  uninstall          卸载 pnpm；INSTALL_METHOD 决定范围（默认 all = pkg + source）

若 corepack 不可用，可设置:
  PNPM_USE_NPM_GLOBAL=1   使用 npm install -g pnpm，前缀 NPM_GLOBAL_ROOT（默认 ~/opt/npm）
  INSTALL_METHOD=source    使用 corepack/npm 路径

安装 Node 见: fundeploy dev nodejs install（默认 ~/opt/node）
EOF
    ;;
  *) die "未知子命令: $cmd" ;;
esac
