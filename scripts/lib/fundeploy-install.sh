#!/usr/bin/env bash
# fundeploy 安装辅助库（WAR-410）：多安装方式 + 包管理器探测 + gum 交互 UI。
# 由各 dev 工具 setup.sh 以「尽力而为」方式 source（找不到时脚本仍可独立运行）。
#
# 提供能力：
#   1. 安装方式选择：INSTALL_METHOD=pkg|source（默认 pkg；无包管理器时回退 source）。
#   2. 包管理器探测与安装/卸载（apt/brew/dnf/yum/pacman/zypper/apk）。
#   3. gum 感知的 UI：已装 gum 则美化（标题/步骤/选择/确认），否则回退纯文本；
#      非交互（NONINTERACTIVE=1 或无 TTY）时选择返回默认项、确认走 FUNDEPLOY_ASSUME_YES。
#
# 设计取舍（爆炸半径 / 回滚路径）：
#   - 本库只“使用”已存在的 gum，不主动安装 gum，避免为一次 go 安装引入重依赖。
#   - 所有函数对 `set -euo pipefail` 友好：失败以返回码表达，由调用方决定回退。
[[ -n "${_FUNDEPLOY_INSTALL_LOADED:-}" ]] && return 0
_FUNDEPLOY_INSTALL_LOADED=1

# ---------------------------------------------------------------------------
# UI：gum 感知
# ---------------------------------------------------------------------------
_fundeploy_has_gum() { command -v gum >/dev/null 2>&1; }

# 是否可交互：有 TTY 且未显式声明非交互。
_fundeploy_interactive() {
  [[ -z "${NONINTERACTIVE:-}" ]] && [[ -t 0 ]] && [[ -t 1 ]]
}

_fundeploy_say_title() {
  local msg="$*"
  if _fundeploy_has_gum; then
    gum style --border rounded --padding "0 2" --margin "1 0" \
      --border-foreground 212 --foreground 212 "${msg}" >&2
  else
    printf '\n=== %s ===\n' "${msg}" >&2
  fi
}

_fundeploy_say_step() {
  if _fundeploy_has_gum; then
    gum style --foreground 39 "▸ $*" >&2
  else
    printf '▸ %s\n' "$*" >&2
  fi
}

_fundeploy_say_ok() {
  if _fundeploy_has_gum; then
    gum style --foreground 42 "✓ $*" >&2
  else
    printf '✓ %s\n' "$*" >&2
  fi
}

_fundeploy_say_warn() {
  if _fundeploy_has_gum; then
    gum style --foreground 214 "! $*" >&2
  else
    printf '! %s\n' "$*" >&2
  fi
}

_fundeploy_say_err() {
  if _fundeploy_has_gum; then
    gum style --foreground 196 "✗ $*" >&2
  else
    printf '✗ %s\n' "$*" >&2
  fi
}

# _fundeploy_choose <header> <default> <opt...>
# 交互且有 gum：弹出选择菜单（默认项置顶）；否则回显 <default>。
_fundeploy_choose() {
  local header="$1" default="$2"
  shift 2
  if _fundeploy_has_gum && _fundeploy_interactive; then
    local sel
    if sel="$(gum choose --header "${header}" --selected "${default}" "$@")"; then
      printf '%s\n' "${sel}"
      return 0
    fi
    return 1
  fi
  printf '%s\n' "${default}"
}

# _fundeploy_confirm <prompt>：交互+gum → gum confirm；否则按 FUNDEPLOY_ASSUME_YES 决定（默认否）。
_fundeploy_confirm() {
  local prompt="$1"
  if _fundeploy_has_gum && _fundeploy_interactive; then
    gum confirm "${prompt}"
    return $?
  fi
  if [[ "${FUNDEPLOY_ASSUME_YES:-}" == "1" ]]; then
    return 0
  fi
  return 1
}

# ---------------------------------------------------------------------------
# 包管理器探测
# ---------------------------------------------------------------------------
# 输出首个可用的包管理器名；找不到输出空串、返回 1。
_fundeploy_pm_detect() {
  local os
  os="$(uname -s 2>/dev/null || true)"
  if [[ "${os}" == "Darwin" ]]; then
    if command -v brew >/dev/null 2>&1; then printf 'brew\n'; return 0; fi
    printf ''; return 1
  fi
  local m
  for m in apt-get dnf yum pacman zypper apk brew; do
    if command -v "${m}" >/dev/null 2>&1; then
      case "${m}" in
        apt-get) printf 'apt\n' ;;
        *) printf '%s\n' "${m}" ;;
      esac
      return 0
    fi
  done
  printf ''
  return 1
}

# 需要时给出提权前缀（brew 从不用 sudo；root 无需 sudo）。
_fundeploy_sudo_prefix() {
  local mgr="$1"
  [[ "${mgr}" == "brew" ]] && { printf ''; return 0; }
  [[ "$(id -u 2>/dev/null || echo 0)" == "0" ]] && { printf ''; return 0; }
  if command -v sudo >/dev/null 2>&1; then printf 'sudo\n'; else printf ''; fi
}

# _fundeploy_pm_install <mgr> <pkg...>
_fundeploy_pm_install() {
  local mgr="$1"; shift
  local sudo_p; sudo_p="$(_fundeploy_sudo_prefix "${mgr}")"
  case "${mgr}" in
    apt)    ${sudo_p} apt-get update -y && ${sudo_p} apt-get install -y "$@" ;;
    dnf)    ${sudo_p} dnf install -y "$@" ;;
    yum)    ${sudo_p} yum install -y "$@" ;;
    pacman) ${sudo_p} pacman -S --noconfirm "$@" ;;
    zypper) ${sudo_p} zypper install -y "$@" ;;
    apk)    ${sudo_p} apk add "$@" ;;
    brew)   brew install "$@" ;;
    *) return 2 ;;
  esac
}

# _fundeploy_pm_uninstall <mgr> <pkg...>
_fundeploy_pm_uninstall() {
  local mgr="$1"; shift
  local sudo_p; sudo_p="$(_fundeploy_sudo_prefix "${mgr}")"
  case "${mgr}" in
    apt)    ${sudo_p} apt-get remove -y "$@" ;;
    dnf)    ${sudo_p} dnf remove -y "$@" ;;
    yum)    ${sudo_p} yum remove -y "$@" ;;
    pacman) ${sudo_p} pacman -R --noconfirm "$@" ;;
    zypper) ${sudo_p} zypper remove -y "$@" ;;
    apk)    ${sudo_p} apk del "$@" ;;
    brew)   brew uninstall "$@" ;;
    *) return 2 ;;
  esac
}

# ---------------------------------------------------------------------------
# 安装方式选择
# ---------------------------------------------------------------------------
# _fundeploy_resolve_method <tool-label>
#   优先级：INSTALL_METHOD 环境变量 > 交互选择 > 默认 pkg。
#   pkg 但探测不到包管理器时回退 source（并 warn）。
#   输出规范化后的方法：pkg | source
_fundeploy_resolve_method() {
  local label="${1:-该工具}" method="${INSTALL_METHOD:-}"

  if [[ -z "${method}" ]]; then
    if _fundeploy_has_gum && _fundeploy_interactive; then
      local pick
      pick="$(_fundeploy_choose "为 ${label} 选择安装方式" \
        "通用包管理器（推荐，默认）" \
        "通用包管理器（推荐，默认）" \
        "源码/官方包安装到 ~/opt")" || pick="通用包管理器（推荐，默认）"
      case "${pick}" in
        通用*) method="pkg" ;;
        源码*) method="source" ;;
        *) method="pkg" ;;
      esac
    else
      method="pkg"
    fi
  fi

  case "${method}" in
    pkg | package | system | apt | brew) method="pkg" ;;
    source | src | tarball | official) method="source" ;;
    *) method="pkg" ;;
  esac

  if [[ "${method}" == "pkg" ]] && ! _fundeploy_pm_detect >/dev/null 2>&1; then
    _fundeploy_say_warn "未探测到可用包管理器，回退为源码/官方包安装（~/opt）。"
    method="source"
  fi

  printf '%s\n' "${method}"
}
