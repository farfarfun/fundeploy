#!/usr/bin/env bash
# OpenPencil（https://github.com/open-pencil/open-pencil）本机部署：
#   - 通过 npm 全局安装 CLI（@open-pencil/cli）与 MCP 服务器（@open-pencil/mcp）
#   - 可选：从 GitHub Releases 下载桌面 Tauri 安装包（Linux AppImage / macOS dmg / Windows msi）
#
# 依赖：npm（≥ Node 18 建议）；可选 curl（下载桌面版）。
#
# 用法：
#   ./setup.sh                    # gum 菜单
#   ./setup.sh install            # 安装 CLI + MCP（npm -g）
#   ./setup.sh update             # 重新安装最新版（同 install）
#   ./setup.sh install-desktop    # 下载桌面安装包到 ${OPEN_PENCIL_SERVICE_HOME}/dist
#   ./setup.sh mcp-config         # 打印 Claude Code / Cursor 的 MCP 集成示例
#   ./setup.sh status             # 显示 CLI/MCP 版本、桌面包下载情况
#   ./setup.sh uninstall          # 卸载 npm 包 + 清理 ${OPEN_PENCIL_SERVICE_HOME}
#
# 环境变量：
#   OPEN_PENCIL_SERVICE_HOME     数据目录（默认 ~/opt/open-pencil），存放桌面安装包、日志
#   OPEN_PENCIL_CLI_PACKAGE      CLI 包名（默认 @open-pencil/cli）
#   OPEN_PENCIL_MCP_PACKAGE      MCP 包名（默认 @open-pencil/mcp）
#   OPEN_PENCIL_VERSION          npm 包版本（默认 latest；同时应用于 CLI 与 MCP）
#   OPEN_PENCIL_NPM_BIN          npm 可执行路径（默认从 PATH 查找）
#   OPEN_PENCIL_DESKTOP_VERSION  桌面包版本 tag（默认取 GitHub latest；如 v0.13.2）
#   OPEN_PENCIL_GITHUB_REPO      owner/repo（默认 open-pencil/open-pencil）
#   OPEN_PENCIL_DESKTOP_ASSET    覆盖桌面包资源名（自动检测失败时使用）
#   NONINTERACTIVE=1
#   OPEN_PENCIL_UNINSTALL_YES=1  非 TTY 卸载确认

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ -f "${SCRIPT_DIR}/../lib/nlt-common.sh" ]]; then
  # shellcheck source=../lib/nlt-common.sh
  source "${SCRIPT_DIR}/../lib/nlt-common.sh"
elif [[ -f "${SCRIPT_DIR}/../../lib/nlt-common.sh" ]]; then
  # shellcheck source=../../lib/nlt-common.sh
  source "${SCRIPT_DIR}/../../lib/nlt-common.sh"
else
  echo "错误: 找不到 lib/nlt-common.sh（已检查 ${SCRIPT_DIR}/../lib 与 ${SCRIPT_DIR}/../../lib）" >&2
  exit 1
fi

if [[ -f "${SCRIPT_DIR}/../lib/nlt-progress.sh" ]]; then
  # shellcheck source=../lib/nlt-progress.sh
  source "${SCRIPT_DIR}/../lib/nlt-progress.sh"
elif [[ -f "${SCRIPT_DIR}/../../lib/nlt-progress.sh" ]]; then
  # shellcheck source=../../lib/nlt-progress.sh
  source "${SCRIPT_DIR}/../../lib/nlt-progress.sh"
fi

OPEN_PENCIL_GITHUB_REPO="${OPEN_PENCIL_GITHUB_REPO:-open-pencil/open-pencil}"
OPEN_PENCIL_SERVICE_HOME="${OPEN_PENCIL_SERVICE_HOME:-${HOME}/opt/open-pencil}"
OPEN_PENCIL_CLI_PACKAGE="${OPEN_PENCIL_CLI_PACKAGE:-@open-pencil/cli}"
OPEN_PENCIL_MCP_PACKAGE="${OPEN_PENCIL_MCP_PACKAGE:-@open-pencil/mcp}"
OPEN_PENCIL_VERSION="${OPEN_PENCIL_VERSION:-latest}"
OPEN_PENCIL_NPM_BIN="${OPEN_PENCIL_NPM_BIN:-}"

OPEN_PENCIL_DIST_DIR="${OPEN_PENCIL_SERVICE_HOME}/dist"
OPEN_PENCIL_LOG_DIR="${OPEN_PENCIL_SERVICE_HOME}/log"
OPEN_PENCIL_FALLBACK_TAG="${OPEN_PENCIL_FALLBACK_TAG:-v0.13.2}"

usage() {
  cat <<USAGE
用法: ./setup.sh [command]

  无参数：gum 菜单。

命令:
  install / update      通过 npm 全局安装 ${OPEN_PENCIL_CLI_PACKAGE} 与 ${OPEN_PENCIL_MCP_PACKAGE}
                        版本由 OPEN_PENCIL_VERSION 控制（默认 latest）
  install-desktop       从 GitHub Releases 下载桌面 Tauri 安装包到 ${OPEN_PENCIL_DIST_DIR}
                        Linux 默认 AppImage，macOS dmg，Windows msi；可用
                        OPEN_PENCIL_DESKTOP_ASSET / OPEN_PENCIL_DESKTOP_VERSION 覆盖
  mcp-config            打印 Claude Code / Cursor 的 MCP 集成配置样例
  status                展示 CLI/MCP 版本，与 dist 目录已下载资源
  uninstall             卸载 npm 包并删除 ${OPEN_PENCIL_SERVICE_HOME}

上游: https://github.com/open-pencil/open-pencil
文档: https://openpencil.dev
USAGE
}

die() { echo "错误: $*" >&2; exit 1; }

ensure_dirs() {
  mkdir -p "${OPEN_PENCIL_DIST_DIR}" "${OPEN_PENCIL_LOG_DIR}"
}

_resolve_npm() {
  if [[ -n "${OPEN_PENCIL_NPM_BIN}" ]]; then
    [[ -x "${OPEN_PENCIL_NPM_BIN}" ]] || die "OPEN_PENCIL_NPM_BIN 无效: ${OPEN_PENCIL_NPM_BIN}"
    echo "${OPEN_PENCIL_NPM_BIN}"
    return
  fi
  command -v npm >/dev/null 2>&1 || die "未找到 npm（可先运行 fundeploy dev nodejs install）"
  command -v npm
}

_pkg_spec() {
  local pkg="$1"
  if [[ "${OPEN_PENCIL_VERSION}" == "latest" || -z "${OPEN_PENCIL_VERSION}" ]]; then
    printf '%s@latest' "$pkg"
  else
    # 允许 0.13.2 或 v0.13.2 或语义化 range
    local v="${OPEN_PENCIL_VERSION#v}"
    printf '%s@%s' "$pkg" "$v"
  fi
}

_npm_global_ls() {
  local npm_bin
  npm_bin="$(_resolve_npm)"
  "${npm_bin}" ls -g --depth=0 --json 2>/dev/null || true
}

_pkg_installed_version() {
  local pkg="$1" json
  json="$(_npm_global_ls)"
  [[ -n "$json" ]] || { echo ""; return; }
  if command -v python3 >/dev/null 2>&1; then
    OPEN_PENCIL_PKG_QUERY="$pkg" python3 -c '
import json, os, sys
data = json.loads(sys.stdin.read() or "{}")
pkg = os.environ.get("OPEN_PENCIL_PKG_QUERY", "")
deps = data.get("dependencies") or {}
info = deps.get(pkg) or {}
ver = info.get("version") or ""
print(ver)
' <<<"$json"
  else
    # 无 python3 时的兜底：从字符串中抓取，格式相对宽松。
    # 分隔符必须用 # 而非 /：包名形如 @open-pencil/cli，其中的 / 会提前终止
    # s/// 表达式，报 "sed: unknown option to 's'"，并因 pipefail 让调用方
    # 的 cli_ver="$(…)" 失败、set -e 中止整个脚本。
    printf '%s' "$json" | sed -n "s#.*\"${pkg}\": *{[^}]*\"version\": *\"\\([^\"]*\\)\".*#\\1#p" | head -1
  fi
}

_npm_install() {
  local npm_bin pkg spec
  npm_bin="$(_resolve_npm)"
  ensure_dirs
  echo "==> npm 全局安装 OpenPencil（${OPEN_PENCIL_VERSION}）"
  for pkg in "${OPEN_PENCIL_CLI_PACKAGE}" "${OPEN_PENCIL_MCP_PACKAGE}"; do
    spec="$(_pkg_spec "$pkg")"
    echo "    → ${npm_bin} install -g ${spec}"
    if ! "${npm_bin}" install -g "${spec}"; then
      echo "    npm 全局安装失败：${spec}" >&2
      echo "    权限不足时可考虑：sudo ${npm_bin} install -g ${spec}" >&2
      echo "    或改用 nvm/volta 等无 sudo 的 Node 版本管理器" >&2
      exit 1
    fi
  done
  echo ""
  echo "已安装："
  cmd_status_brief
}

cmd_install() { _npm_install; }
cmd_update() { _npm_install; }

# ---- 桌面 Tauri 安装包下载 ----
_detect_desktop_asset_default() {
  local os arch
  case "$(uname -s)" in
    Linux) os=linux ;;
    Darwin) os=darwin ;;
    MINGW* | MSYS* | CYGWIN* | Windows_NT) os=windows ;;
    *) die "未知操作系统: $(uname -s)（可指定 OPEN_PENCIL_DESKTOP_ASSET）" ;;
  esac
  case "$(uname -m)" in
    x86_64 | amd64) arch=amd64 ;;
    aarch64 | arm64) arch=arm64 ;;
    *) die "未知架构: $(uname -m)（可指定 OPEN_PENCIL_DESKTOP_ASSET）" ;;
  esac
  # 输出 os-arch，让上层拼接资源名
  printf '%s-%s\n' "$os" "$arch"
}

_asset_name_for_tag() {
  local tag="$1" plat ver
  ver="${tag#v}"
  if [[ -n "${OPEN_PENCIL_DESKTOP_ASSET:-}" ]]; then
    printf '%s' "${OPEN_PENCIL_DESKTOP_ASSET}"
    return
  fi
  plat="$(_detect_desktop_asset_default)"
  case "$plat" in
    linux-amd64) printf 'OpenPencil_%s_amd64.AppImage' "$ver" ;;
    linux-arm64)
      # 上游 v0.13.2 未提供 linux-arm64 资源；提示用户覆盖
      die "上游 Release 未提供 linux-arm64 资源；请设置 OPEN_PENCIL_DESKTOP_ASSET 显式指定文件名"
      ;;
    darwin-amd64) printf 'OpenPencil_%s_x64.dmg' "$ver" ;;
    darwin-arm64) printf 'OpenPencil_%s_aarch64.dmg' "$ver" ;;
    windows-amd64) printf 'OpenPencil_%s_x64_en-US.msi' "$ver" ;;
    windows-arm64) printf 'OpenPencil_%s_arm64_en-US.msi' "$ver" ;;
    *) die "无法为平台 ${plat} 选择资源；请设置 OPEN_PENCIL_DESKTOP_ASSET" ;;
  esac
}

_fetch_latest_desktop_tag() {
  command -v curl >/dev/null 2>&1 || die "需要 curl"
  local json tag
  json="$(_nlt_github_download_curl -fsSL "https://api.github.com/repos/${OPEN_PENCIL_GITHUB_REPO}/releases/latest")" || json=""
  if [[ -n "$json" ]]; then
    tag="$(printf '%s' "$json" | sed -n 's/.*"tag_name": *"\(v[0-9][0-9.]*\)".*/\1/p' | head -1)"
    [[ -n "$tag" ]] && { printf '%s\n' "$tag"; return; }
  fi
  printf '%s\n' "${OPEN_PENCIL_FALLBACK_TAG}"
}

cmd_install_desktop() {
  command -v curl >/dev/null 2>&1 || die "需要 curl"
  ensure_dirs
  local tag asset url dest
  tag="${OPEN_PENCIL_DESKTOP_VERSION:-$(_fetch_latest_desktop_tag)}"
  case "$tag" in
    v*) : ;;
    [0-9]*) tag="v${tag}" ;;
    *) die "无效版本 tag: ${tag}" ;;
  esac
  asset="$(_asset_name_for_tag "$tag")"
  url="https://github.com/${OPEN_PENCIL_GITHUB_REPO}/releases/download/${tag}/${asset}"
  dest="${OPEN_PENCIL_DIST_DIR}/${asset}"
  echo "==> 下载桌面安装包 ${tag}"
  echo "    ${url}"
  echo "    → ${dest}"
  _nlt_github_download_print_accel_hint
  if declare -F nlt_pb_curl_to_file >/dev/null 2>&1; then
    NLT_PB_LABEL="open-pencil ${tag}" nlt_pb_curl_to_file "$url" "$dest" || die "下载失败: ${url}"
  else
    _nlt_github_download_curl -fsSL "$url" -o "$dest" || die "下载失败: ${url}"
  fi
  case "$asset" in
    *.AppImage)
      chmod +x "$dest"
      echo "已下载 AppImage，可直接执行： ${dest}"
      ;;
    *.deb)
      echo "已下载 deb，可执行： sudo dpkg -i ${dest}"
      ;;
    *.rpm)
      echo "已下载 rpm，可执行： sudo rpm -ivh ${dest}"
      ;;
    *.dmg)
      echo "已下载 dmg，请在 Finder 中挂载安装： ${dest}"
      ;;
    *.msi)
      echo "已下载 msi，请双击执行： ${dest}"
      ;;
    *)
      echo "已下载： ${dest}"
      ;;
  esac
}

cmd_mcp_config() {
  local mcp_bin cli_ver mcp_ver
  cli_ver="$(_pkg_installed_version "${OPEN_PENCIL_CLI_PACKAGE}")"
  mcp_ver="$(_pkg_installed_version "${OPEN_PENCIL_MCP_PACKAGE}")"
  mcp_bin="$(command -v openpencil-mcp 2>/dev/null || true)"
  cat <<CFG
==> OpenPencil MCP 集成说明

已安装：
  ${OPEN_PENCIL_CLI_PACKAGE}  ${cli_ver:-（未安装，先执行 install）}
  ${OPEN_PENCIL_MCP_PACKAGE}  ${mcp_ver:-（未安装，先执行 install）}
  openpencil-mcp 可执行文件： ${mcp_bin:-（PATH 中未找到，请检查 npm 全局 bin 是否在 PATH）}

Claude Code（用户级 settings.json）：
  {
    "permissions": {
      "allow": ["mcp__open-pencil__*"]
    },
    "mcpServers": {
      "open-pencil": {
        "command": "openpencil-mcp"
      }
    }
  }

Cursor / Windsurf / 其他 MCP 客户端：
  {
    "mcpServers": {
      "open-pencil": {
        "command": "openpencil-mcp"
      }
    }
  }

或者用 Claude CLI 一键添加（安装完 CLI 后）：
  claude mcp add --scope user open-pencil -- openpencil-mcp

更多： https://openpencil.dev/reference/mcp-tools
CFG
}

cmd_status_brief() {
  local cli_ver mcp_ver
  cli_ver="$(_pkg_installed_version "${OPEN_PENCIL_CLI_PACKAGE}")"
  mcp_ver="$(_pkg_installed_version "${OPEN_PENCIL_MCP_PACKAGE}")"
  printf '  %s  %s\n' "${OPEN_PENCIL_CLI_PACKAGE}" "${cli_ver:-（未安装）}"
  printf '  %s  %s\n' "${OPEN_PENCIL_MCP_PACKAGE}" "${mcp_ver:-（未安装）}"
}

cmd_status() {
  local npm_bin
  npm_bin="$(command -v npm 2>/dev/null || true)"
  echo "OPEN_PENCIL_SERVICE_HOME=${OPEN_PENCIL_SERVICE_HOME}"
  echo "OPEN_PENCIL_VERSION=${OPEN_PENCIL_VERSION}"
  echo "OPEN_PENCIL_GITHUB_REPO=${OPEN_PENCIL_GITHUB_REPO}"
  echo "npm: ${npm_bin:-（未找到）}"
  local cli_bin mcp_bin
  cli_bin="$(command -v openpencil 2>/dev/null || true)"
  mcp_bin="$(command -v openpencil-mcp 2>/dev/null || true)"
  echo "openpencil CLI: ${cli_bin:-（未在 PATH 中）}"
  echo "openpencil-mcp: ${mcp_bin:-（未在 PATH 中）}"
  echo ""
  echo "npm 全局包："
  cmd_status_brief
  echo ""
  if [[ -d "${OPEN_PENCIL_DIST_DIR}" ]]; then
    echo "桌面安装包目录: ${OPEN_PENCIL_DIST_DIR}"
    if compgen -G "${OPEN_PENCIL_DIST_DIR}/*" >/dev/null; then
      ls -lh "${OPEN_PENCIL_DIST_DIR}" | tail -n +2 || true
    else
      echo "  （空；如需下载执行 install-desktop）"
    fi
  else
    echo "桌面安装包目录未创建；如需下载执行 install-desktop"
  fi
}

cmd_uninstall() {
  local npm_bin pkg
  npm_bin="$(_resolve_npm)"
  nlt_confirm_destructive \
    "将卸载 npm -g ${OPEN_PENCIL_CLI_PACKAGE} 与 ${OPEN_PENCIL_MCP_PACKAGE}，并删除 ${OPEN_PENCIL_SERVICE_HOME}。确认？" \
    OPEN_PENCIL_UNINSTALL_YES || return 1
  for pkg in "${OPEN_PENCIL_MCP_PACKAGE}" "${OPEN_PENCIL_CLI_PACKAGE}"; do
    echo "==> ${npm_bin} uninstall -g ${pkg}"
    "${npm_bin}" uninstall -g "${pkg}" || echo "（忽略）卸载 ${pkg} 失败"
  done
  if [[ -d "${OPEN_PENCIL_SERVICE_HOME}" ]]; then
    nlt_safe_rm "${OPEN_PENCIL_SERVICE_HOME}" || return 1
    echo "已删除 ${OPEN_PENCIL_SERVICE_HOME}"
  fi
  echo "已卸载。"
}

dispatch() {
  local cmd="${1:-}"
  shift || true
  case "$cmd" in
    install) cmd_install ;;
    update) cmd_update ;;
    install-desktop) cmd_install_desktop ;;
    mcp-config | mcp) cmd_mcp_config ;;
    status) cmd_status ;;
    uninstall) cmd_uninstall ;;
    help | -h | --help) usage ;;
    *)
      echo "未知命令: ${cmd}" >&2
      usage >&2
      exit 2
      ;;
  esac
}

interactive_main() {
  declare -F nlt_ui_apply_theme >/dev/null 2>&1 && nlt_ui_apply_theme
  if declare -F nlt_ui_banner >/dev/null 2>&1; then
    nlt_ui_banner "OpenPencil 本地部署（open-pencil/open-pencil）" "安装目录: ${OPEN_PENCIL_SERVICE_HOME}" "CLI: ${OPEN_PENCIL_CLI_PACKAGE}  MCP: ${OPEN_PENCIL_MCP_PACKAGE}  版本: ${OPEN_PENCIL_VERSION}"
  else
    gum style --bold --foreground 212 "OpenPencil 本地部署（open-pencil/open-pencil）"
    gum style "安装目录: ${OPEN_PENCIL_SERVICE_HOME}"
    gum style "CLI: ${OPEN_PENCIL_CLI_PACKAGE}  MCP: ${OPEN_PENCIL_MCP_PACKAGE}  版本: ${OPEN_PENCIL_VERSION}"
  fi
  echo ""
  set +e
  while true; do
    local pick
    pick="$(nlt_ui_choose "fundeploy / service / open-pencil / 选择动作" \
      "install" "update" "install-desktop" "mcp-config" "status" "uninstall" "help" "quit")" || break
    [[ -z "$pick" ]] && break
    case "$pick" in
      quit) break ;;
      help) usage; continue ;;
    esac
    ( dispatch "$pick" )
    echo ""
  done
  set -e
}

main() {
  if [[ $# -gt 0 ]]; then
    case "$1" in
      help | -h | --help)
        dispatch "$@"
        return 0
        ;;
    esac
  fi
  _nlt_ensure_gum || exit 1
  if [[ $# -eq 0 ]]; then
    interactive_main
    return 0
  fi
  dispatch "$@"
}

main "$@"
