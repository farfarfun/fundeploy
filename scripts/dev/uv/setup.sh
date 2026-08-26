#!/usr/bin/env bash
# uv（Astral）：多方式安装 / 升级 / 卸载（WAR-410）
#   - pkg（默认）：通用包管理器安装 uv（可用时）
#   - source     ：官方安装脚本安装到 UV_INSTALL_DIR（默认 ~/opt/uv）
# 用法: install（默认）| update | version | uninstall
# 方式选择: INSTALL_METHOD=pkg|source（不设则交互选择，默认 pkg）
# 官方文档: https://docs.astral.sh/uv/getting-started/installation/
set -euo pipefail

die() { echo "错误: $*" >&2; exit 1; }

_UV_ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
for _c in "${_UV_ROOT_DIR}/../../lib" "${_UV_ROOT_DIR}/../../../lib"; do
  if [[ -f "${_c}/nlt-install.sh" ]]; then
    # shellcheck source=../../lib/nlt-install.sh
    source "${_c}/nlt-install.sh"
    break
  fi
done
if ! declare -F _nlt_say_step >/dev/null 2>&1; then
  _nlt_say_title() { printf '\n=== %s ===\n' "$*" >&2; }
  _nlt_say_step()  { printf '▸ %s\n' "$*" >&2; }
  _nlt_say_ok()    { printf '✓ %s\n' "$*" >&2; }
  _nlt_say_warn()  { printf '! %s\n' "$*" >&2; }
  _nlt_pm_detect() { printf ''; return 1; }
  _nlt_pm_install() { return 2; }
  _nlt_pm_uninstall() { return 2; }
  _nlt_resolve_method() { printf '%s\n' "${INSTALL_METHOD:-source}"; }
fi

UV_INSTALL_URL="${UV_INSTALL_URL:-https://astral.sh/uv/install.sh}"
UV_INSTALL_DIR="${UV_INSTALL_DIR:-${HOME}/opt/uv}"

do_install_pkg() {
  local mgr
  mgr="$(_nlt_pm_detect)" || die "未探测到包管理器；请改用源码方式：INSTALL_METHOD=source $0 install"
  _nlt_say_step "使用 ${mgr} 安装 uv 包"
  _nlt_pm_install "${mgr}" uv || die "${mgr} 安装 uv 失败；可改用 INSTALL_METHOD=source"
  _nlt_say_ok "已通过 ${mgr} 安装 uv。"
}

do_install_source() {
  command -v curl >/dev/null 2>&1 || die "需要 curl"
  mkdir -p "${UV_INSTALL_DIR}"
  export UV_INSTALL_DIR
  _nlt_say_step "执行 Astral 官方 uv 安装脚本: ${UV_INSTALL_URL}"
  # 安装器识别 UV_INSTALL_DIR、INSTALLER_NO_MODIFY_PATH 等；见上游 install.sh 注释
  curl -LsSf "${UV_INSTALL_URL}" | sh
  _nlt_say_ok "uv source 安装完成。"
  echo "请配置 PATH：" >&2
  echo "  export PATH=\"${UV_INSTALL_DIR}:\${PATH}\"" >&2
}

do_update_source() {
  export PATH="${UV_INSTALL_DIR}:${PATH}"
  if command -v uv >/dev/null 2>&1; then
    _nlt_say_step "执行 uv self update"
    uv self update
  else
    _nlt_say_warn "未在 PATH 中找到 uv，改为执行 source install。"
    do_install_source
  fi
}

do_version() {
  command -v uv >/dev/null 2>&1 || die "未找到 uv（可先运行 fundeploy dev uv install）"
  uv --version
}

do_uninstall_pkg() {
  local mgr
  mgr="$(_nlt_pm_detect)" || { _nlt_say_warn "未探测到包管理器，跳过 pkg 卸载。"; return 0; }
  _nlt_say_step "使用 ${mgr} 卸载 uv 包"
  _nlt_pm_uninstall "${mgr}" uv || _nlt_say_warn "${mgr} 卸载 uv 失败（可能本就未通过 ${mgr} 安装）。"
}

do_uninstall_source() {
  export PATH="${UV_INSTALL_DIR}:${PATH}"
  command -v uv >/dev/null 2>&1 || { _nlt_say_warn "未找到 uv，跳过 uv self uninstall。"; }
  if command -v uv >/dev/null 2>&1; then
    _nlt_say_step "执行 uv self uninstall（非交互确认）"
  if printf 'y\n' | uv self uninstall; then
      _nlt_say_ok "已执行 uv self uninstall。"
  else
    die "uv self uninstall 失败，请查阅 uv 文档手动移除"
  fi
  fi
  if [[ -d "${UV_INSTALL_DIR}" ]]; then
    rm -rf "${UV_INSTALL_DIR}"
    _nlt_say_ok "已删除 source 安装目录: ${UV_INSTALL_DIR}"
  fi
}

do_install() {
  local method
  method="$(_nlt_resolve_method uv)"
  _nlt_say_title "安装 uv（方式: ${method}）"
  case "${method}" in
    pkg) do_install_pkg ;;
    source) do_install_source ;;
    *) die "未知安装方式: ${method}" ;;
  esac
}

do_update() {
  local method
  method="$(_nlt_resolve_method uv)"
  _nlt_say_title "升级 uv（方式: ${method}）"
  case "${method}" in
    pkg) do_install_pkg ;;
    source) do_update_source ;;
    *) die "未知安装方式: ${method}" ;;
  esac
}

do_uninstall() {
  local method="${INSTALL_METHOD:-all}"
  _nlt_say_title "卸载 uv（方式: ${method}）"
  case "${method}" in
    pkg) do_uninstall_pkg ;;
    source) do_uninstall_source ;;
    all | "") do_uninstall_pkg; do_uninstall_source ;;
    *) die "未知卸载方式: ${method}" ;;
  esac
}

cmd="${1:-install}"
case "$cmd" in
  install) do_install ;;
  update | upgrade) do_update ;;
  version) do_version ;;
  uninstall | remove) do_uninstall ;;
  -h | --help | help)
    cat <<'EOF'
用法: uv/setup.sh [install|update|version|uninstall]

  install（默认）  安装 uv；方式由 INSTALL_METHOD 决定（默认 pkg，交互时可选）
  update / upgrade  pkg 方式重跑包管理器安装；source 方式 uv self update
  version           uv --version
  uninstall         卸载 uv；INSTALL_METHOD 决定范围（默认 all = pkg + source）

环境变量（与上游安装器一致，常用）:
  INSTALL_METHOD           pkg | source（不设则交互选择，无 gum/TTY 时默认 pkg）
  UV_INSTALL_DIR           source 方式安装目录（默认 ~/opt/uv）
  INSTALLER_NO_MODIFY_PATH 设为 1 时不改 shell 配置，仅安装二进制
  UV_INSTALL_URL           覆盖安装脚本 URL（默认 https://astral.sh/uv/install.sh）

与 python-env 的关系:
  python-env 在创建虚拟环境前也会按需自动安装 uv；若希望**先单独装好/升级 uv**，
  请使用 fundeploy dev uv。
EOF
    ;;
  *) die "未知子命令: $cmd（见 uv/setup.sh --help）" ;;
esac
