#!/usr/bin/env bash
# cockpit-tools（https://github.com/jlcodes99/cockpit-tools）桌面工具安装器。
#
# 设计:
# - 归类为 tools，不做守护进程管理。
# - Linux 优先用官方 AppImage，避免依赖 root 安装 .deb/.rpm。
# - 安装后提供 run 子命令前台启动；并写入用户级 .desktop 入口。
#
# 用法:
#   ./setup.sh                      # gum 菜单
#   ./setup.sh install             # 下载 AppImage 到 ${COCKPIT_TOOLS_HOME}
#   ./setup.sh update              # 重新下载（同 install）
#   ./setup.sh reinstall           # 删除后重装
#   ./setup.sh uninstall           # 删除安装目录与桌面入口
#   ./setup.sh run [args...]       # 前台启动 Cockpit Tools
#
# 环境变量:
#   COCKPIT_TOOLS_HOME        默认 ~/opt/cockpit-tools
#   COCKPIT_TOOLS_VERSION     指定版本，如 1.0.4 或 v1.0.4；不设则取 GitHub latest
#   COCKPIT_TOOLS_GITHUB_REPO 默认 jlcodes99/cockpit-tools
#   COCKPIT_TOOLS_FALLBACK_TAG 默认 v1.0.4
#   NONINTERACTIVE=1
#   COCKPIT_TOOLS_UNINSTALL_YES=1

set -euo pipefail

_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
_NLT_LIB=""
if [[ -f "${_SCRIPT_DIR}/../lib/nlt-common.sh" ]]; then
  _NLT_LIB="$(cd "${_SCRIPT_DIR}/../lib" && pwd)"
elif [[ -f "${_SCRIPT_DIR}/../../lib/nlt-common.sh" ]]; then
  _NLT_LIB="$(cd "${_SCRIPT_DIR}/../../lib" && pwd)"
else
  echo "错误: 找不到 lib/nlt-common.sh（已检查 ${_SCRIPT_DIR}/../lib 与 ${_SCRIPT_DIR}/../../lib）" >&2
  exit 1
fi

# shellcheck source=../../lib/nlt-common.sh
source "${_NLT_LIB}/nlt-common.sh"
if [[ -f "${_NLT_LIB}/nlt-progress.sh" ]]; then
  # shellcheck source=../../lib/nlt-progress.sh
  source "${_NLT_LIB}/nlt-progress.sh"
fi

COCKPIT_TOOLS_HOME="${COCKPIT_TOOLS_HOME:-${HOME}/opt/cockpit-tools}"
COCKPIT_TOOLS_GITHUB_REPO="${COCKPIT_TOOLS_GITHUB_REPO:-jlcodes99/cockpit-tools}"
COCKPIT_TOOLS_FALLBACK_TAG="${COCKPIT_TOOLS_FALLBACK_TAG:-v1.0.4}"

COCKPIT_TOOLS_BIN_DIR="${COCKPIT_TOOLS_HOME}/bin"
COCKPIT_TOOLS_APPIMAGE="${COCKPIT_TOOLS_BIN_DIR}/Cockpit.Tools.AppImage"
COCKPIT_TOOLS_DESKTOP_DIR="${HOME}/.local/share/applications"
COCKPIT_TOOLS_DESKTOP_FILE="${COCKPIT_TOOLS_DESKTOP_DIR}/cockpit-tools.desktop"

_ct_usage() {
  cat <<EOF
用法: $(basename "${BASH_SOURCE[0]}") [command]

  无参数：gum 菜单。

命令:
  install / update     从 GitHub Releases 下载 Linux AppImage 到 ${COCKPIT_TOOLS_APPIMAGE}
  reinstall            删除后重新安装
  uninstall            删除 ${COCKPIT_TOOLS_HOME} 与桌面入口 ${COCKPIT_TOOLS_DESKTOP_FILE}
  run [args...]        前台启动 Cockpit Tools（终端附着）
  help                 显示本说明

上游: https://github.com/jlcodes99/cockpit-tools
EOF
}

_ct_die() { echo "错误: $*" >&2; exit 1; }

_ct_require_linux() {
  [[ "$(uname -s)" == "Linux" ]] || _ct_die "当前仅支持 Linux；官方 macOS/Windows 包请直接从上游 Releases 安装"
}

_ct_require_curl() {
  command -v curl >/dev/null 2>&1 || _ct_die "需要 curl"
}

_ct_ensure_dirs() {
  mkdir -p "${COCKPIT_TOOLS_BIN_DIR}" "${COCKPIT_TOOLS_DESKTOP_DIR}"
}

_ct_normalize_tag() {
  local v="${1:-}"
  v="${v#v}"
  [[ -n "${v}" ]] || return 1
  printf 'v%s\n' "${v}"
}

_ct_detect_arch() {
  case "$(uname -m)" in
    x86_64 | amd64) echo "amd64" ;;
    aarch64 | arm64) echo "aarch64" ;;
    *) _ct_die "不支持的架构: $(uname -m)" ;;
  esac
}

_ct_fetch_latest_tag() {
  _ct_require_curl
  local out tag
  out="$(_nlt_github_download_curl -fsSL "https://api.github.com/repos/${COCKPIT_TOOLS_GITHUB_REPO}/releases/latest")" || return 1
  tag="$(printf '%s' "${out}" | sed -n 's/.*"tag_name": *"\(v\{0,1\}[0-9][0-9.]*\)".*/\1/p' | head -1)"
  [[ -n "${tag}" ]] || return 1
  _ct_normalize_tag "${tag}"
}

_ct_resolve_tag() {
  if [[ -n "${COCKPIT_TOOLS_VERSION:-}" ]]; then
    _ct_normalize_tag "${COCKPIT_TOOLS_VERSION}" || _ct_die "无效 COCKPIT_TOOLS_VERSION: ${COCKPIT_TOOLS_VERSION}"
    return
  fi
  local tag
  tag="$(_ct_fetch_latest_tag || true)"
  if [[ -n "${tag}" ]]; then
    printf '%s\n' "${tag}"
    return
  fi
  printf '%s\n' "${COCKPIT_TOOLS_FALLBACK_TAG}"
}

_ct_asset_name_for_tag() {
  local tag="$1"
  local ver arch
  ver="${tag#v}"
  arch="$(_ct_detect_arch)"
  printf 'Cockpit.Tools_%s_%s.AppImage\n' "${ver}" "${arch}"
}

_ct_desktop_write() {
  cat > "${COCKPIT_TOOLS_DESKTOP_FILE}" <<EOF
[Desktop Entry]
Type=Application
Name=Cockpit Tools
Comment=AI IDE account manager
Exec=${COCKPIT_TOOLS_APPIMAGE}
Terminal=false
Categories=Development;Utility;
StartupNotify=true
EOF
  command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "${COCKPIT_TOOLS_DESKTOP_DIR}" >/dev/null 2>&1 || true
}

_ct_download_install() {
  _ct_require_linux
  _ct_require_curl
  _ct_ensure_dirs
  local tag asset url tmp
  tag="$(_ct_resolve_tag)"
  asset="$(_ct_asset_name_for_tag "${tag}")"
  url="https://github.com/${COCKPIT_TOOLS_GITHUB_REPO}/releases/download/${tag}/${asset}"
  echo "==> 下载 cockpit-tools ${tag}" >&2
  echo "    ${url}" >&2
  tmp="$(mktemp)"
  trap '[[ -n "${tmp-}" ]] && rm -f "${tmp}"' RETURN
  _nlt_github_download_print_accel_hint
  if declare -F nlt_pb_curl_to_file >/dev/null 2>&1; then
    NLT_PB_LABEL="cockpit-tools ${tag}" nlt_pb_curl_to_file "${url}" "${tmp}" || _ct_die "下载失败: ${url}"
  else
    _nlt_github_download_curl -fsSL "${url}" -o "${tmp}"
  fi
  install -m 0755 "${tmp}" "${COCKPIT_TOOLS_APPIMAGE}"
  rm -f "${tmp}"
  trap - RETURN
  _ct_desktop_write
  echo "已安装到 ${COCKPIT_TOOLS_APPIMAGE}（${tag} / ${asset}）"
}

cmd_install() {
  _ct_download_install
}

cmd_update() {
  echo "==> 更新 cockpit-tools（重新下载）…" >&2
  _ct_download_install
}

cmd_reinstall() {
  if [[ "${NONINTERACTIVE:-}" != "1" ]] && [[ -t 0 ]]; then
    _nlt_ensure_gum || exit 1
    gum confirm "将覆盖重装 ${COCKPIT_TOOLS_HOME} 下 Cockpit Tools，继续？" || exit 0
  fi
  rm -f "${COCKPIT_TOOLS_APPIMAGE}"
  cmd_install
}

cmd_uninstall() {
  if [[ "${COCKPIT_TOOLS_UNINSTALL_YES:-}" != "1" ]] && [[ "${NONINTERACTIVE:-}" != "1" ]] && [[ -t 0 ]]; then
    _nlt_ensure_gum || exit 1
    gum confirm "删除 ${COCKPIT_TOOLS_HOME} 与桌面入口？" || exit 0
  elif [[ "${COCKPIT_TOOLS_UNINSTALL_YES:-}" != "1" ]] && [[ ! -t 0 ]] && [[ "${NONINTERACTIVE:-}" != "1" ]]; then
    _ct_die "非交互卸载请设置 COCKPIT_TOOLS_UNINSTALL_YES=1 或 NONINTERACTIVE=1"
  fi
  rm -rf "${COCKPIT_TOOLS_HOME}"
  rm -f "${COCKPIT_TOOLS_DESKTOP_FILE}"
  echo "已卸载 cockpit-tools。"
}

cmd_run() {
  _ct_require_linux
  [[ -x "${COCKPIT_TOOLS_APPIMAGE}" ]] || _ct_die "未安装，请先执行: $0 install"
  exec "${COCKPIT_TOOLS_APPIMAGE}" "$@"
}

_ct_interactive_main() {
  while true; do
    local pick
    pick="$(gum choose --header "nlt-cockpit-tools" \
      "install" \
      "update" \
      "reinstall" \
      "run" \
      "uninstall" \
      "help" \
      "quit")" || return 0
    case "${pick}" in
      install) cmd_install ;;
      update) cmd_update ;;
      reinstall) cmd_reinstall ;;
      run) cmd_run ;;
      uninstall) cmd_uninstall ;;
      help) _ct_usage ;;
      quit) return 0 ;;
      *) _ct_usage; return 1 ;;
    esac
    echo ""
  done
}

main() {
  if [[ $# -eq 0 ]]; then
    if [[ "${NONINTERACTIVE:-}" == "1" ]]; then
      _ct_usage
      exit 0
    fi
    _nlt_ensure_gum || exit 1
    _ct_interactive_main
    exit 0
  fi

  local cmd="$1"
  shift || true
  case "${cmd}" in
    install) cmd_install "$@" ;;
    update) cmd_update "$@" ;;
    reinstall) cmd_reinstall "$@" ;;
    uninstall) cmd_uninstall "$@" ;;
    run) cmd_run "$@" ;;
    help | -h | --help) _ct_usage ;;
    *)
      _ct_die "未知子命令: ${cmd}"
      ;;
  esac
}

main "$@"
