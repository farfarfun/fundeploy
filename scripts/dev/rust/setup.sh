#!/usr/bin/env bash
# Rust：多方式安装 / 升级 / 卸载（WAR-410）
#   - pkg（默认）：通用包管理器（apt install rustc cargo / brew install rust …）
#   - source     ：rustup 安装到 RUST_INSTALL_ROOT（默认 ~/opt/rust，无需 sudo）
# 用法: install（默认）| update | uninstall
# 方式选择: INSTALL_METHOD=pkg|source（不设则交互选择，默认 pkg）
set -euo pipefail

die() { echo "错误: $*" >&2; exit 1; }

_RUST_ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
for _c in "${_RUST_ROOT_DIR}/../../lib" "${_RUST_ROOT_DIR}/../../../lib"; do
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

RUST_INSTALL_ROOT="${RUST_INSTALL_ROOT:-${HOME}/opt/rust}"

_rust_pkg_names() {
  case "$1" in
    apt | dnf | yum | pacman | zypper | apk) printf 'rustc cargo\n' ;;
    brew) printf 'rust\n' ;;
    *) printf 'rustc cargo\n' ;;
  esac
}

do_install_pkg() {
  local mgr pkgs
  mgr="$(_fundeploy_pm_detect)" || die "未探测到包管理器；请改用源码方式：INSTALL_METHOD=source $0 install"
  read -r -a pkgs <<<"$(_rust_pkg_names "${mgr}")"
  _fundeploy_say_step "使用 ${mgr} 安装 Rust 包: ${pkgs[*]}"
  _fundeploy_pm_install "${mgr}" "${pkgs[@]}" || die "${mgr} 安装 Rust 失败"
  _fundeploy_say_ok "已通过 ${mgr} 安装 Rust。"
}

do_install_source() {
  command -v curl >/dev/null 2>&1 || die "需要 curl"
  export RUSTUP_HOME="${RUSTUP_HOME:-${RUST_INSTALL_ROOT}/rustup}"
  export CARGO_HOME="${CARGO_HOME:-${RUST_INSTALL_ROOT}/cargo}"
  mkdir -p "${RUSTUP_HOME}" "${CARGO_HOME}"
  _fundeploy_say_step "执行 rustup 安装（stable）到 ${RUST_INSTALL_ROOT}"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
  _fundeploy_say_ok "Rust source 安装完成。"
  echo "请配置 PATH：" >&2
  echo "  export PATH=\"${CARGO_HOME}/bin:\${PATH}\"" >&2
}

do_update_source() {
  export RUSTUP_HOME="${RUSTUP_HOME:-${RUST_INSTALL_ROOT}/rustup}"
  export CARGO_HOME="${CARGO_HOME:-${RUST_INSTALL_ROOT}/cargo}"
  export PATH="${CARGO_HOME}/bin:${PATH}"
  if command -v rustup >/dev/null 2>&1; then
    rustup self update || true
    rustup update stable
  else
    _fundeploy_say_warn "未找到 rustup，改为执行 source install。"
    do_install_source
  fi
}

do_uninstall_pkg() {
  local mgr pkgs
  mgr="$(_fundeploy_pm_detect)" || { _fundeploy_say_warn "未探测到包管理器，跳过 pkg 卸载。"; return 0; }
  read -r -a pkgs <<<"$(_rust_pkg_names "${mgr}")"
  _fundeploy_say_step "使用 ${mgr} 卸载 Rust 包: ${pkgs[*]}"
  _fundeploy_pm_uninstall "${mgr}" "${pkgs[@]}" || _fundeploy_say_warn "${mgr} 卸载 Rust 失败（可能本就未通过 ${mgr} 安装）。"
}

do_uninstall_source() {
  export RUSTUP_HOME="${RUSTUP_HOME:-${RUST_INSTALL_ROOT}/rustup}"
  export CARGO_HOME="${CARGO_HOME:-${RUST_INSTALL_ROOT}/cargo}"
  export PATH="${CARGO_HOME}/bin:${PATH}"
  if [[ -x "${CARGO_HOME}/bin/rustup" ]]; then
    rustup self uninstall -y || true
  fi
  if [[ -d "${RUST_INSTALL_ROOT}" ]]; then
    rm -rf "${RUST_INSTALL_ROOT}"
    _fundeploy_say_ok "已删除源码安装目录: ${RUST_INSTALL_ROOT}"
  else
    _fundeploy_say_warn "目录不存在，跳过: ${RUST_INSTALL_ROOT}"
  fi
}

do_install() {
  local method
  method="$(_fundeploy_resolve_method Rust)"
  _fundeploy_say_title "安装 Rust（方式: ${method}）"
  case "${method}" in
    pkg) do_install_pkg ;;
    source) do_install_source ;;
    *) die "未知安装方式: ${method}" ;;
  esac
}

do_update() {
  local method
  method="$(_fundeploy_resolve_method Rust)"
  _fundeploy_say_title "升级 Rust（方式: ${method}）"
  case "${method}" in
    pkg) do_install_pkg ;;
    source) do_update_source ;;
    *) die "未知安装方式: ${method}" ;;
  esac
}

do_uninstall() {
  local method="${INSTALL_METHOD:-all}"
  _fundeploy_say_title "卸载 Rust（方式: ${method}）"
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
  uninstall | remove) do_uninstall ;;
  -h | --help | help)
    cat <<'EOF'
用法: rust/setup.sh [install|update|uninstall]

  install     安装 Rust；方式由 INSTALL_METHOD 决定（默认 pkg，交互时可选）
  update      pkg 方式重跑包管理器安装；source 方式 rustup update stable
  uninstall   卸载 Rust；INSTALL_METHOD 决定范围（默认 all = pkg + source）

可选环境变量:
  INSTALL_METHOD      pkg | source（不设则交互选择，无 gum/TTY 时默认 pkg）
  RUST_INSTALL_ROOT   source 方式安装根目录，默认 ~/opt/rust
  RUSTUP_HOME / CARGO_HOME  显式覆盖 rustup/cargo 目录
EOF
    ;;
  *) die "未知子命令: $cmd" ;;
esac
