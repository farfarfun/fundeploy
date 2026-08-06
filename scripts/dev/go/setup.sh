#!/usr/bin/env bash
# Go：多方式安装 / 升级 / 卸载（WAR-410）
#   - pkg（默认）：通用包管理器（apt install golang-go / brew install go …）
#   - source     ：go.dev 官方预编译包，解压到 GO_INSTALL_ROOT（默认 ~/opt/go，无需 sudo）
# 用法: install（默认）| version | uninstall
# 方式选择: INSTALL_METHOD=pkg|source（不设则交互选择，默认 pkg）
set -euo pipefail

die() { echo "错误: $*" >&2; exit 1; }

# 尽力而为 source 安装辅助库（找不到时保持可独立运行）。
_GO_ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
for _c in "${_GO_ROOT_DIR}/../../lib" "${_GO_ROOT_DIR}/../../../lib"; do
  if [[ -f "${_c}/nlt-install.sh" ]]; then
    # shellcheck source=../../lib/nlt-install.sh
    source "${_c}/nlt-install.sh"
    break
  fi
done
# 无辅助库时的降级桩，保证脚本仍可运行。
if ! declare -F _nlt_say_step >/dev/null 2>&1; then
  _nlt_say_title() { printf '\n=== %s ===\n' "$*" >&2; }
  _nlt_say_step()  { printf '▸ %s\n' "$*" >&2; }
  _nlt_say_ok()    { printf '✓ %s\n' "$*" >&2; }
  _nlt_say_warn()  { printf '! %s\n' "$*" >&2; }
  _nlt_say_err()   { printf '✗ %s\n' "$*" >&2; }
  _nlt_pm_detect() { printf ''; return 1; }
  _nlt_pm_install() { return 2; }
  _nlt_pm_uninstall() { return 2; }
  _nlt_resolve_method() { printf '%s\n' "${INSTALL_METHOD:-source}"; }
fi

GO_INSTALL_ROOT="${GO_INSTALL_ROOT:-${HOME}/opt/go}"

# 各包管理器下的 Go 包名。
_go_pkg_name() {
  case "$1" in
    apt) printf 'golang-go\n' ;;
    dnf | yum) printf 'golang\n' ;;
    pacman | zypper | apk | brew) printf 'go\n' ;;
    *) printf 'go\n' ;;
  esac
}

_nlt_go_platform() {
  local os arch
  os="$(uname -s 2>/dev/null || true)"
  arch="$(uname -m 2>/dev/null || true)"
  case "${os}:${arch}" in
    Linux:x86_64) printf '%s\n' linux-amd64 ;;
    Linux:aarch64) printf '%s\n' linux-arm64 ;;
    Darwin:arm64) printf '%s\n' darwin-arm64 ;;
    Darwin:x86_64) printf '%s\n' darwin-amd64 ;;
    *) die "不支持的系统: ${os} ${arch}" ;;
  esac
}

_nlt_go_latest_version() {
  command -v curl >/dev/null 2>&1 || die "需要 curl"
  local line
  line="$(curl -fsSL "https://go.dev/VERSION?m=text" | head -1 | tr -d '\r')"
  [[ -n "$line" ]] || die "无法从 go.dev 读取版本"
  printf '%s\n' "$line"
}

# ---- pkg 方式 ----
do_install_pkg() {
  local mgr pkg
  mgr="$(_nlt_pm_detect)" || die "未探测到包管理器；请改用源码方式：INSTALL_METHOD=source $0 install"
  pkg="$(_go_pkg_name "${mgr}")"
  _nlt_say_step "使用 ${mgr} 安装 Go 包: ${pkg}"
  _nlt_pm_install "${mgr}" "${pkg}" || die "${mgr} 安装 ${pkg} 失败"
  _nlt_say_ok "已通过 ${mgr} 安装 Go：$(command -v go 2>/dev/null || echo '请重开终端确认 PATH')"
}

do_uninstall_pkg() {
  local mgr pkg
  mgr="$(_nlt_pm_detect)" || { _nlt_say_warn "未探测到包管理器，跳过 pkg 卸载。"; return 0; }
  pkg="$(_go_pkg_name "${mgr}")"
  _nlt_say_step "使用 ${mgr} 卸载 Go 包: ${pkg}"
  _nlt_pm_uninstall "${mgr}" "${pkg}" || _nlt_say_warn "${mgr} 卸载 ${pkg} 失败（可能本就未通过 ${mgr} 安装）。"
}

# ---- source 方式 ----
do_install_source() {
  command -v curl >/dev/null 2>&1 || die "需要 curl"
  local plat ver tmp url
  plat="$(_nlt_go_platform)"
  if [[ -n "${GO_VERSION:-}" ]]; then
    ver="${GO_VERSION}"
  else
    ver="$(_nlt_go_latest_version)"
  fi
  [[ "$ver" == go* ]] || ver="go${ver#go}"
  url="https://go.dev/dl/${ver}.${plat}.tar.gz"
  mkdir -p "$(dirname "${GO_INSTALL_ROOT}")"
  tmp="$(mktemp)"
  # trap 确保下载/解压失败时不残留临时包（官方包数百 MB）。
  trap 'rm -f "${tmp}"' RETURN EXIT
  _nlt_say_step "下载官方包: ${url}"
  curl -fL --proto '=https' --proto-redir '=https' --tlsv1.2 --progress-bar "${url}" -o "${tmp}"
  rm -rf "${GO_INSTALL_ROOT}"
  mkdir -p "${GO_INSTALL_ROOT}"
  # 官方 tarball 自带 go/ 顶层目录。原实现解到 dirname(GO_INSTALL_ROOT) 并依赖
  # 该目录恰好叫 go —— 一旦 GO_INSTALL_ROOT=~/opt/golang，就会先删掉 golang、
  # 再把内容写进 ~/opt/go（覆盖既有安装），且成功信息还是错的。
  tar -C "${GO_INSTALL_ROOT}" --strip-components=1 -xzf "${tmp}"
  rm -f "${tmp}"
  trap - RETURN EXIT
  _nlt_say_ok "已安装 Go ${ver} 到 ${GO_INSTALL_ROOT}"
  echo "请将下列行加入 shell 配置（若尚未配置）：" >&2
  echo "  export PATH=\"${GO_INSTALL_ROOT}/bin:\${PATH}\"" >&2
}

do_uninstall_source() {
  [[ -d "${GO_INSTALL_ROOT}" ]] || { _nlt_say_warn "目录不存在，跳过: ${GO_INSTALL_ROOT}"; return 0; }
  rm -rf "${GO_INSTALL_ROOT}"
  _nlt_say_ok "已删除源码安装目录: ${GO_INSTALL_ROOT}"
}

do_install() {
  local method
  method="$(_nlt_resolve_method Go)"
  _nlt_say_title "安装 Go（方式: ${method}）"
  case "${method}" in
    pkg) do_install_pkg ;;
    source) do_install_source ;;
    *) die "未知安装方式: ${method}" ;;
  esac
}

# 卸载：未显式指定方式时清理所有已知安装位置（pkg + source）。
do_uninstall() {
  local method="${INSTALL_METHOD:-all}"
  _nlt_say_title "卸载 Go（方式: ${method}）"
  case "${method}" in
    pkg) do_uninstall_pkg ;;
    source) do_uninstall_source ;;
    all | "") do_uninstall_pkg; do_uninstall_source ;;
    *) die "未知卸载方式: ${method}" ;;
  esac
}

do_version() {
  if [[ -x "${GO_INSTALL_ROOT}/bin/go" ]]; then
    "${GO_INSTALL_ROOT}/bin/go" version
  else
    command -v go >/dev/null 2>&1 && go version || die "未找到 go（${GO_INSTALL_ROOT}/bin/go 不存在且 PATH 中无 go）"
  fi
}

cmd="${1:-install}"
case "$cmd" in
  install | update | upgrade) do_install ;;
  version) do_version ;;
  uninstall | remove) do_uninstall ;;
  -h | --help | help)
    cat <<'EOF'
用法: go/setup.sh [install|version|uninstall]

  install / update   安装 Go；方式由 INSTALL_METHOD 决定（默认 pkg，交互时可选）
  version            打印已安装的 go version
  uninstall          卸载 Go；INSTALL_METHOD 决定范围（默认 all = pkg + source）

安装方式（INSTALL_METHOD）:
  pkg      通用包管理器（apt→golang-go / brew→go / dnf→golang …），默认、推荐
  source   go.dev 官方预编译包，解压到 GO_INSTALL_ROOT（默认 ~/opt/go）

环境变量:
  INSTALL_METHOD    pkg | source（不设则交互选择，无 gum/TTY 时默认 pkg）
  GO_INSTALL_ROOT   源码方式安装目录（GOROOT），默认 ~/opt/go
  GO_VERSION        源码方式强制版本，如 go1.22.4；不设则用 go.dev 最新稳定版
  NONINTERACTIVE=1  跳过交互，方式取 INSTALL_METHOD 或默认 pkg
EOF
    ;;
  *) die "未知子命令: $cmd（试试 install / version / uninstall）" ;;
esac
