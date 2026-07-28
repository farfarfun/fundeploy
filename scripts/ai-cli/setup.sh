#!/usr/bin/env bash
# AI CLI unified entry: install / update / uninstall terminal AI coding tools.
# Scope: official package or installer paths only; no source builds.
set -euo pipefail

die() { echo "错误: $*" >&2; exit 1; }

_AI_ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
for _c in "${_AI_ROOT_DIR}/../lib" "${_AI_ROOT_DIR}/../../lib"; do
  if [[ -f "${_c}/nlt-install.sh" ]]; then
    # shellcheck source=../lib/nlt-install.sh
    source "${_c}/nlt-install.sh"
    break
  fi
done
if ! declare -F _nlt_say_step >/dev/null 2>&1; then
  _nlt_say_title() { printf '\n=== %s ===\n' "$*" >&2; }
  _nlt_say_step() { printf '> %s\n' "$*" >&2; }
  _nlt_say_ok() { printf 'OK: %s\n' "$*" >&2; }
  _nlt_say_warn() { printf 'WARN: %s\n' "$*" >&2; }
  _nlt_confirm() {
    local ans
    read -r -p "$* [y/N] " ans
    [[ "${ans,,}" == "y" || "${ans,,}" == "yes" ]]
  }
fi

_TOOLS=(claude codex cursor)

usage() {
  cat <<'EOF'
用法: nltdeploy ai <工具|all> <install|update|uninstall|status|version> [args...]
      nltdeploy ai                    # 交互选择工具和动作
      nltdeploy ai list
      nltdeploy ai help

工具:
  claude   Claude Code CLI（npm global package: @anthropic-ai/claude-code）
  codex    OpenAI Codex CLI（官方 standalone installer）
  cursor   Cursor CLI（官方 installer）
  all      依次处理 claude、codex、cursor

动作:
  install    安装；已安装则跳过或按官方安装器幂等执行
  update     升级到最新
  uninstall  卸载本入口可识别的官方安装产物
  status     打印是否可用和版本
  version    等同 status

环境变量:
  NLT_ASSUME_YES=1       非交互卸载时确认删除本入口识别的用户目录文件
  NLT_AI_SKIP_NODE=1     claude 安装缺少 npm 时不尝试调用 nlt-dev nodejs install
  CODEX_INSTALL_URL      默认 https://chatgpt.com/codex/install.sh
  CURSOR_INSTALL_URL     默认 https://cursor.com/install

说明:
  本入口只支持官方 package / installer 路径，不支持源码安装。
EOF
}

_has() { command -v "$1" >/dev/null 2>&1; }

_resolve_nlt_dev() {
  local root="${NLTDEPLOY_ROOT:-${HOME}/.local/nltdeploy}" cand
  for cand in \
    "${root}/bin/nlt-dev" \
    "${_AI_ROOT_DIR}/../dev/setup.sh" \
    "${_AI_ROOT_DIR}/../../dev/setup.sh"; do
    [[ -x "${cand}" || -f "${cand}" ]] && { printf '%s\n' "${cand}"; return 0; }
  done
  command -v nlt-dev >/dev/null 2>&1 && { command -v nlt-dev; return 0; }
  return 1
}

_ensure_npm() {
  if _has npm; then
    return 0
  fi
  [[ "${NLT_AI_SKIP_NODE:-}" == "1" ]] && die "未找到 npm；请先安装 Node.js/npm"
  local nlt_dev
  nlt_dev="$(_resolve_nlt_dev 2>/dev/null)" || die "未找到 npm，也找不到 nlt-dev；请先安装 Node.js/npm"
  _nlt_say_step "未找到 npm，执行 nltdeploy dev nodejs install"
  if [[ -x "${nlt_dev}" ]]; then
    "${nlt_dev}" nodejs install
  else
    bash "${nlt_dev}" nodejs install
  fi
  _has npm || die "Node.js 安装后仍未找到 npm"
}

_safe_rm_home_path() {
  local p="$1" real home_real
  [[ -e "${p}" || -L "${p}" ]] || return 0
  real="$(cd "$(dirname "${p}")" && pwd -P)/$(basename "${p}")"
  home_real="$(cd "${HOME}" && pwd -P)"
  [[ "${real}" == "${home_real}/"* ]] || die "拒绝删除非 HOME 路径: ${p}"
  if [[ "${NLT_ASSUME_YES:-}" != "1" && ( ! -t 0 || ! -t 1 ) ]]; then
    die "非交互卸载请设置 NLT_ASSUME_YES=1"
  fi
  if [[ "${NLT_ASSUME_YES:-}" != "1" && -t 0 ]]; then
    if ! _nlt_confirm "删除 ${p}？"; then
      _nlt_say_warn "已跳过: ${p}"
      return 0
    fi
  fi
  rm -rf "${p}"
  _nlt_say_ok "已删除 ${p}"
}

_npm_global_uninstall() {
  local pkg="$1"
  _has npm || { _nlt_say_warn "未找到 npm，跳过 ${pkg}"; return 0; }
  npm uninstall -g "${pkg}"
}

install_claude() {
  _ensure_npm
  _nlt_say_title "安装 Claude Code CLI"
  npm install -g @anthropic-ai/claude-code@latest
  _nlt_say_ok "Claude Code 安装完成。运行: claude --version"
}

update_claude() { install_claude; }

uninstall_claude() {
  _nlt_say_title "卸载 Claude Code CLI"
  _npm_global_uninstall @anthropic-ai/claude-code
}

status_claude() {
  if _has claude; then
    claude --version
  else
    echo "claude: 未安装"
  fi
}

install_codex() {
  _has curl || die "需要 curl"
  local url="${CODEX_INSTALL_URL:-https://chatgpt.com/codex/install.sh}"
  _nlt_say_title "安装 OpenAI Codex CLI"
  _nlt_say_step "执行官方安装器: ${url}"
  curl -fsSL "${url}" | sh
  _nlt_say_ok "Codex 安装完成。运行: codex --version"
}

update_codex() { install_codex; }

uninstall_codex() {
  _nlt_say_title "卸载 OpenAI Codex CLI"
  _safe_rm_home_path "${HOME}/.local/bin/codex"
  _safe_rm_home_path "${HOME}/.local/share/codex"
}

status_codex() {
  if _has codex; then
    codex --version
  else
    echo "codex: 未安装"
  fi
}

install_cursor() {
  _has curl || die "需要 curl"
  local url="${CURSOR_INSTALL_URL:-https://cursor.com/install}"
  _nlt_say_title "安装 Cursor CLI"
  _nlt_say_step "执行官方安装器: ${url}"
  curl "${url}" -fsS | bash
  _nlt_say_ok "Cursor CLI 安装完成。运行: cursor-agent --version"
}

update_cursor() { install_cursor; }

uninstall_cursor() {
  _nlt_say_title "卸载 Cursor CLI"
  _safe_rm_home_path "${HOME}/.local/bin/cursor-agent"
  _safe_rm_home_path "${HOME}/.local/bin/agent"
  _safe_rm_home_path "${HOME}/.local/share/cursor-agent"
}

status_cursor() {
  if _has cursor-agent; then
    cursor-agent --version
  elif _has agent; then
    agent --version
  else
    echo "cursor: 未安装"
  fi
}

cmd_list() {
  printf '%s\n' "${_TOOLS[@]}"
}

dispatch_one() {
  local tool="$1" action="$2"
  case "${tool}:${action}" in
    claude:install) install_claude ;;
    claude:update | claude:upgrade) update_claude ;;
    claude:uninstall | claude:remove) uninstall_claude ;;
    claude:status | claude:version) status_claude ;;
    codex:install) install_codex ;;
    codex:update | codex:upgrade) update_codex ;;
    codex:uninstall | codex:remove) uninstall_codex ;;
    codex:status | codex:version) status_codex ;;
    cursor:install) install_cursor ;;
    cursor:update | cursor:upgrade) update_cursor ;;
    cursor:uninstall | cursor:remove) uninstall_cursor ;;
    cursor:status | cursor:version) status_cursor ;;
    *) die "未知工具或动作: ${tool} ${action}" ;;
  esac
}

_ai_menu_interactive() {
  [[ -z "${NONINTERACTIVE:-}" ]] && [[ -t 0 ]] && [[ -t 2 ]]
}

_ai_pick_menu() {
  local header="$1"
  shift
  local options=("$@")
  if _nlt_has_gum && _ai_menu_interactive; then
    local sel
    sel="$(printf '%s\n' "${options[@]}" | gum filter --header "${header}" --height 8 --limit 1 --select-if-one)" || return 1
    [[ -n "${sel}" ]] || return 1
    printf '%s\n' "${sel}"
    return 0
  fi

  local i=1 ans
  printf '%s\n' "${header}" >&2
  for ans in "${options[@]}"; do
    printf '  %d) %s\n' "${i}" "${ans}" >&2
    i=$((i + 1))
  done
  read -r -p "请选择编号: " ans
  [[ "${ans}" =~ ^[0-9]+$ ]] || return 1
  (( ans >= 1 && ans <= ${#options[@]} )) || return 1
  printf '%s\n' "${options[ans - 1]}"
}

interactive_main() {
  _nlt_interactive || { usage >&2; die "无参数模式需要交互式 TTY；非交互请显式传入工具和动作"; }

  local tool action
  tool="$(_ai_pick_menu "选择 AI CLI 工具" "${_TOOLS[@]}" all)" || die "未选择工具"
  action="$(_ai_pick_menu "选择 ${tool} 动作" install upgrade uninstall status)" || die "未选择动作"

  dispatch_one "${tool}" "${action}"
}

main() {
  if [[ "$#" -eq 0 ]]; then
    interactive_main
    return 0
  fi

  local tool="${1:-help}"
  case "${tool}" in
    help | -h | --help) usage; return 0 ;;
    list | --list) cmd_list; return 0 ;;
  esac
  shift || true
  local action="${1:-install}"
  case "${tool}" in
    all)
      local t
      for t in "${_TOOLS[@]}"; do
        dispatch_one "${t}" "${action}"
      done
      ;;
    claude | codex | cursor)
      dispatch_one "${tool}" "${action}"
      ;;
    *)
      usage >&2
      die "未知工具: ${tool}"
      ;;
  esac
}

main "$@"
