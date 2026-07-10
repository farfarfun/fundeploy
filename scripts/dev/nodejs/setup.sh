#!/usr/bin/env bash
# Node.js：多方式安装 / 升级 / 卸载（WAR-410）
#   - pkg（默认）：通用包管理器（apt install nodejs npm / brew install node …）
#   - source     ：nodejs.org 官方预编译包，解压到 NODE_INSTALL_ROOT（默认 ~/opt/node）
# 用法: install（默认）| version | uninstall
# 方式选择: INSTALL_METHOD=pkg|source（不设则交互选择，默认 pkg）
set -euo pipefail

die() { echo "错误: $*" >&2; exit 1; }

_NODE_ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
for _c in "${_NODE_ROOT_DIR}/../../lib" "${_NODE_ROOT_DIR}/../../../lib"; do
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

NODE_VERSION="${NODE_VERSION:-22.14.0}"
NODE_INSTALL_ROOT="${NODE_INSTALL_ROOT:-${HOME}/opt/node}"

_node_pkg_names() {
  case "$1" in
    apt | apk) printf 'nodejs npm\n' ;;
    dnf | yum | pacman | zypper | brew) printf 'nodejs npm\n' ;;
    *) printf 'nodejs npm\n' ;;
  esac
}

_nlt_node_platform() {
  local os arch
  os="$(uname -s 2>/dev/null || true)"
  arch="$(uname -m 2>/dev/null || true)"
  case "${os}:${arch}" in
    Linux:x86_64) printf '%s\n' linux-x64 ;;
    Linux:aarch64) printf '%s\n' linux-arm64 ;;
    Darwin:arm64) printf '%s\n' darwin-arm64 ;;
    Darwin:x86_64) printf '%s\n' darwin-x64 ;;
    *) die "不支持的系统: ${os} ${arch}" ;;
  esac
}

do_install_pkg() {
  local mgr pkgs
  mgr="$(_nlt_pm_detect)" || die "未探测到包管理器；请改用源码方式：INSTALL_METHOD=source $0 install"
  if [[ "${mgr}" == "brew" ]]; then
    pkgs=(node)
  else
    read -r -a pkgs <<<"$(_node_pkg_names "${mgr}")"
  fi
  _nlt_say_step "使用 ${mgr} 安装 Node.js 包: ${pkgs[*]}"
  _nlt_pm_install "${mgr}" "${pkgs[@]}" || die "${mgr} 安装 Node.js 失败"
  _nlt_say_ok "已通过 ${mgr} 安装 Node.js。"
}

do_install_source() {
  command -v curl >/dev/null 2>&1 || die "需要 curl"
  local plat base name url tmp
  plat="$(_nlt_node_platform)"
  base="https://nodejs.org/dist/v${NODE_VERSION}"
  name="node-v${NODE_VERSION}-${plat}"
  url="${base}/${name}.tar.xz"
  tmp="$(mktemp)"
  mkdir -p "$(dirname "${NODE_INSTALL_ROOT}")"
  _nlt_say_step "下载官方包: ${url}"
  curl -fL --progress-bar "${url}" -o "${tmp}"
  rm -rf "${NODE_INSTALL_ROOT}"
  mkdir -p "${NODE_INSTALL_ROOT}"
  tar -C "${NODE_INSTALL_ROOT}" --strip-components=1 -xJf "${tmp}"
  rm -f "${tmp}"
  _nlt_say_ok "已安装 Node v${NODE_VERSION} 到 ${NODE_INSTALL_ROOT}"
  echo "请配置 PATH：" >&2
  echo "  export PATH=\"${NODE_INSTALL_ROOT}/bin:\${PATH}\"" >&2
}

do_version() {
  if [[ -x "${NODE_INSTALL_ROOT}/bin/node" ]]; then
    "${NODE_INSTALL_ROOT}/bin/node" -v
  else
    command -v node >/dev/null 2>&1 && node -v || die "未找到 node"
  fi
}

do_uninstall_pkg() {
  local mgr pkgs
  mgr="$(_nlt_pm_detect)" || { _nlt_say_warn "未探测到包管理器，跳过 pkg 卸载。"; return 0; }
  if [[ "${mgr}" == "brew" ]]; then
    pkgs=(node)
  else
    read -r -a pkgs <<<"$(_node_pkg_names "${mgr}")"
  fi
  _nlt_say_step "使用 ${mgr} 卸载 Node.js 包: ${pkgs[*]}"
  _nlt_pm_uninstall "${mgr}" "${pkgs[@]}" || _nlt_say_warn "${mgr} 卸载 Node.js 失败（可能本就未通过 ${mgr} 安装）。"
}

do_uninstall_source() {
  [[ -d "${NODE_INSTALL_ROOT}" ]] || { _nlt_say_warn "目录不存在，跳过: ${NODE_INSTALL_ROOT}"; return 0; }
  rm -rf "${NODE_INSTALL_ROOT}"
  _nlt_say_ok "已删除源码安装目录: ${NODE_INSTALL_ROOT}"
}

do_install() {
  local method
  method="$(_nlt_resolve_method Node.js)"
  _nlt_say_title "安装 Node.js（方式: ${method}）"
  case "${method}" in
    pkg) do_install_pkg ;;
    source) do_install_source ;;
    *) die "未知安装方式: ${method}" ;;
  esac
}

do_uninstall() {
  local method="${INSTALL_METHOD:-all}"
  _nlt_say_title "卸载 Node.js（方式: ${method}）"
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
用法: nodejs/setup.sh [install|version|uninstall]

  install / update   安装 Node.js；方式由 INSTALL_METHOD 决定（默认 pkg，交互时可选）
  version            打印 node 版本
  uninstall          卸载 Node.js；INSTALL_METHOD 决定范围（默认 all = pkg + source）

环境变量:
  INSTALL_METHOD      pkg | source（不设则交互选择，无 gum/TTY 时默认 pkg）
  NODE_VERSION         版本号，默认 22.14.0
  NODE_INSTALL_ROOT    source 方式安装根目录（含 bin/node），默认 ~/opt/node
EOF
    ;;
  *) die "未知子命令: $cmd" ;;
esac
