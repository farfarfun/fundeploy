#!/usr/bin/env bash
# AI CLI 子入口共用的轻量命令解析与安全删除。

AI_CLI_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ -f "${AI_CLI_DIR}/../lib/nlt-install.sh" ]]; then
  # shellcheck source=../lib/nlt-install.sh
  source "${AI_CLI_DIR}/../lib/nlt-install.sh"
fi

if ! declare -F _nlt_say_step >/dev/null 2>&1; then
  _nlt_say_title() { printf '\n=== %s ===\n' "$*" >&2; }
  _nlt_say_step() { printf '> %s\n' "$*" >&2; }
  _nlt_say_ok() { printf 'OK: %s\n' "$*" >&2; }
  _nlt_say_warn() { printf 'WARN: %s\n' "$*" >&2; }
  _nlt_confirm() {
    local answer
    read -r -p "$1 [y/N] " answer
    [[ "${answer}" == "y" || "${answer}" == "Y" || "${answer}" == "yes" ]]
  }
fi

ai_die() { echo "错误: $*" >&2; exit 1; }
ai_has() { command -v "$1" >/dev/null 2>&1; }

ai_require() {
  ai_has "$1" || ai_die "未找到 $1；$2"
}

ai_interactive() {
  [[ -z "${NONINTERACTIVE:-}" ]] && [[ -t 0 ]] && [[ -t 2 ]]
}

ai_pick() {
  local header="$1"
  shift
  local options=("$@")
  if command -v gum >/dev/null 2>&1; then
    printf '%s\n' "${options[@]}" | gum filter \
      --header "${header}" --height 8 --limit 1 --select-if-one
    return
  fi

  local i=1 answer option
  printf '%s\n' "${header}" >&2
  for option in "${options[@]}"; do
    printf '  %d) %s\n' "${i}" "${option}" >&2
    i=$((i + 1))
  done
  read -r -p "请选择编号: " answer
  [[ "${answer}" =~ ^[0-9]+$ ]] || return 1
  (( answer >= 1 && answer <= ${#options[@]} )) || return 1
  printf '%s\n' "${options[answer - 1]}"
}

ai_safe_rm_home_path() {
  local path="$1" resolved home_resolved
  [[ -e "${path}" || -L "${path}" ]] || return 0
  resolved="$(cd "$(dirname "${path}")" && pwd -P)/$(basename "${path}")"
  home_resolved="$(cd "${HOME}" && pwd -P)"
  [[ "${resolved}" == "${home_resolved}/"* ]] || ai_die "拒绝删除非 HOME 路径: ${path}"
  if [[ "${NLT_ASSUME_YES:-}" != "1" ]]; then
    ai_interactive || ai_die "非交互卸载请设置 NLT_ASSUME_YES=1"
    _nlt_confirm "删除 ${path}？" || { _nlt_say_warn "已跳过: ${path}"; return; }
  fi
  rm -rf -- "${path}"
  _nlt_say_ok "已删除 ${path}"
}

ai_package_install() {
  local manager="$1" package="$2"
  ai_require "${manager}" "请先安装 ${manager}"
  case "${manager}" in
    npm) npm install -g "${package}@latest" ;;
    pnpm) pnpm add -g "${package}@latest" ;;
  esac
}

ai_package_uninstall() {
  local manager="$1" package="$2"
  ai_require "${manager}" "请先安装 ${manager}"
  case "${manager}" in
    npm) npm uninstall -g "${package}" ;;
    pnpm) pnpm remove -g "${package}" ;;
  esac
}

ai_method_known() {
  local wanted="$1" method
  for method in "${AI_METHODS[@]}"; do
    [[ "${method}" == "${wanted}" ]] && return 0
  done
  return 1
}

ai_main() {
  local method="${AI_DEFAULT_METHOD}" action="install" pick
  if [[ $# -eq 0 ]] && ai_interactive; then
    pick="$(ai_pick "选择 ${AI_TOOL} 安装方式" "${AI_METHOD_OPTIONS[@]}")" || ai_die "未选择安装方式"
    method="${pick%% *}"
    pick="$(ai_pick "选择 ${AI_TOOL} 动作" install update uninstall status)" || ai_die "未选择动作"
    action="${pick%% *}"
  elif [[ $# -gt 0 ]]; then
    if ai_method_known "$1"; then
      method="$1"
      shift
      action="${1:-install}"
      [[ $# -eq 0 ]] || shift
    else
      action="$1"
      shift
    fi
  fi

  case "${action}" in
    upgrade) action=update ;;
    remove) action=uninstall ;;
    version) action=status ;;
    help|-h|--help) usage; return ;;
  esac
  case "${action}" in
    install|update|uninstall|status) dispatch "${method}" "${action}" "$@" ;;
    *) usage >&2; ai_die "未知动作: ${action}" ;;
  esac
}
