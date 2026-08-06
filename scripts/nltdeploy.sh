#!/usr/bin/env bash
# nltdeploy：整个项目的顶层入口。
#
# 用法:
#   nltdeploy service <服务> [动作]                   服务管理
#   nltdeploy dev <工具> [动作]                       开发环境
#   nltdeploy tool <工具> [动作]                      常用工具
#   nltdeploy ai <工具> [动作]                        AI CLI
#   nltdeploy upgrade | uninstall | list | help
#   nltdeploy                                          可搜索的交互菜单
#
# upgrade 源:
#   github   从 GitHub 拉取 install.sh 并执行 update（默认公网源）
#   gitee    从 Gitee 拉取 install.sh 并执行 update（国内镜像）
#   local    使用本地仓库/克隆的 install.sh update（git pull --ff-only + 重新同步；无需公网 raw）
#   不指定   优先 local（存在本地 install.sh 时），否则 github，再 gitee。
#
# 环境变量:
#   NLTDEPLOY_ROOT            安装根目录（默认 ~/.local/nltdeploy）
#   NLTDEPLOY_SRC_DIR         本地克隆仓库（默认 ${NLTDEPLOY_ROOT}/src/nltdeploy）
#   NLTDEPLOY_PACKAGE_MANAGER 包管理器安装标记：apt 或 brew
#   NLTDEPLOY_UNINSTALL_YES=1 非 TTY 下允许卸载
#   NONINTERACTIVE=1          无参数时打印 help 退出，不进入菜单
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
NLTDEPLOY_ROOT="${NLTDEPLOY_ROOT:-${HOME}/.local/nltdeploy}"
NLTDEPLOY_SRC_DIR="${NLTDEPLOY_SRC_DIR:-${NLTDEPLOY_ROOT}/src/nltdeploy}"
NLTDEPLOY_GITHUB_RAW="${NLTDEPLOY_GITHUB_RAW:-https://raw.githubusercontent.com/farfarfun/nltdeploy/HEAD/install.sh}"
NLTDEPLOY_GITEE_RAW="${NLTDEPLOY_GITEE_RAW:-https://gitee.com/farfarfun/nltdeploy/raw/master/install.sh}"

# 可选加载统一交互主题（banner / 主题化菜单）；缺失不致命，菜单会降级为朴素 gum/文本。
for _cand in \
  "${SCRIPT_DIR}/lib/nlt-ui.sh" \
  "${SCRIPT_DIR}/../lib/nlt-ui.sh"; do
  if [[ -f "${_cand}" ]]; then
    # shellcheck source=/dev/null
    source "${_cand}"
    break
  fi
done

die() { echo "错误: $*" >&2; exit 1; }

usage() {
  cat <<'EOF'
用法: nltdeploy <领域> [模块] [动作]

命令:
  service <服务> [动作]                    服务状态、安装和生命周期管理
  dev <工具> [动作]                        Python、Go、Rust、Node.js 等开发环境
  tool <工具> [动作]                       brew、gum、网络诊断、下载等工具
  ai <工具> [动作]                         Claude Code、Codex、Cursor
  upgrade [--source github|gitee|local]    升级 nltdeploy
  uninstall                                卸载 nltdeploy
  list                                     显示领域入口
  help / -h / --help                       本说明

无参数时打开可搜索菜单。nltdeploy 是唯一命令入口。

示例:
  nltdeploy service status
  nltdeploy service code-server official install
  nltdeploy service sub2api official install
  nltdeploy service sub2api official restart
  nltdeploy dev nodejs install
  nltdeploy tool github-net doctor
  nltdeploy ai codex update
  nltdeploy upgrade
EOF
}

_entry_rel() {
  case "$1" in
    tools)          echo "tools/nlt-tools.sh" ;;
    dev)            echo "dev/setup.sh" ;;
    ai-cli)         echo "ai-cli/setup.sh" ;;
    pip-sources)    echo "pip-sources/setup.sh" ;;
    python-env)     echo "python-env/setup.sh" ;;
    utils)          echo "utils/setup.sh" ;;
    github-net)     echo "github-net/setup.sh" ;;
    port-kill)      echo "port-kill/setup.sh" ;;
    download)       echo "download/setup.sh" ;;
    cockpit-tools)  echo "cockpit-tools/setup.sh" ;;
    services)       echo "services/nlt-services.sh" ;;
    airflow)        echo "airflow/setup.sh" ;;
    celery)         echo "celery/setup.sh" ;;
    paperclip)      echo "paperclip/setup.sh" ;;
    code-server)    echo "code-server/setup.sh" ;;
    new-api)        echo "new-api/setup.sh" ;;
    sub2api)        echo "sub2api/setup.sh" ;;
    open-pencil)    echo "open-pencil/setup.sh" ;;
    *)              return 1 ;;
  esac
}

cmd_list() {
  cat <<'EOF'
可用领域:
  service   服务管理（nltdeploy service list）
  dev       开发环境（nltdeploy dev --help）
  tool      常用工具（nltdeploy tool list）
  ai        AI CLI（nltdeploy ai list）

项目操作: upgrade / uninstall
EOF
}

# 解析可用的本地 install.sh（仓库内运行 / 已安装的 src 克隆 / libexec bundle）。
_resolve_local_install_sh() {
  local c
  for c in \
    "${NLTDEPLOY_SRC_DIR}/install.sh" \
    "${SCRIPT_DIR}/../install.sh" \
    "${SCRIPT_DIR}/install.sh" \
    "${NLTDEPLOY_ROOT}/libexec/nltdeploy/nltdeploy-install.sh"; do
    [[ -f "$c" ]] && { printf '%s\n' "$c"; return 0; }
  done
  return 1
}

_curl() {
  local source="$1"
  shift
  command -v curl >/dev/null 2>&1 || die "需要 curl 才能从 ${source} 升级"
  curl --proto '=https' --proto-redir '=https' --tlsv1.2 -LsSf "$@"
}

_managed_install_action() {
  local action="$1" manager="${NLTDEPLOY_PACKAGE_MANAGER:-}"
  [[ -n "$manager" ]] || return 1
  case "${manager}:${action}" in
    apt:upgrade)
      echo "nltdeploy 由 APT 管理，请运行:"
      echo "  sudo apt update && sudo apt install --only-upgrade nltdeploy"
      ;;
    apt:uninstall)
      echo "nltdeploy 由 APT 管理，请运行:"
      echo "  sudo apt remove nltdeploy"
      ;;
    brew:upgrade)
      echo "nltdeploy 由 Homebrew 管理，请运行:"
      echo "  brew upgrade nltdeploy"
      ;;
    brew:uninstall)
      echo "nltdeploy 由 Homebrew 管理，请运行:"
      echo "  brew uninstall nltdeploy"
      ;;
    *) die "未知包管理器标记: ${manager}" ;;
  esac
}

upgrade_local() {
  local sh
  sh="$(_resolve_local_install_sh)" || die "未找到本地 install.sh（可改用 --source github/gitee）"
  echo "==> 本地升级: bash ${sh} update" >&2
  exec bash "${sh}" update
}

upgrade_github() {
  echo "==> 从 GitHub 升级: ${NLTDEPLOY_GITHUB_RAW}" >&2
  _curl GitHub "${NLTDEPLOY_GITHUB_RAW}" | bash -s -- update
}

upgrade_gitee() {
  echo "==> 从 Gitee 升级: ${NLTDEPLOY_GITEE_RAW}" >&2
  _curl Gitee "${NLTDEPLOY_GITEE_RAW}" | bash -s -- update
}

cmd_upgrade() {
  _managed_install_action upgrade && return 0
  local source=""
  while [[ $# -gt 0 ]]; do
    case "$1" in
      # `shift 2` 在只剩 --source 一个参数时返回 1，在 set -e 下会静默 exit 1，
      # 使下面精心准备的错误提示永远打不出来。先校验再 shift。
      --source)
        [[ $# -ge 2 ]] || die "--source 缺少取值（github|gitee|local）"
        source="$2"; shift 2
        ;;
      --source=*) source="${1#*=}"; shift ;;
      github|gitee|local) source="$1"; shift ;;
      -h|--help|help) usage; exit 0 ;;
      *) die "未知参数: $1（nltdeploy upgrade --source github|gitee|local）" ;;
    esac
  done

  case "${source}" in
    local)  upgrade_local ;;
    github) upgrade_github ;;
    gitee)  upgrade_gitee ;;
    "")
      # 自动：优先本地，其次 github，再 gitee。
      if _resolve_local_install_sh >/dev/null 2>&1; then
        upgrade_local
      elif upgrade_github; then
        :
      else
        echo "GitHub 升级失败，改用 Gitee …" >&2
        upgrade_gitee
      fi
      ;;
    *) die "未知 --source: ${source}（支持 github|gitee|local）" ;;
  esac
}

cmd_uninstall() {
  local sh
  _managed_install_action uninstall && return 0
  sh="$(_resolve_local_install_sh)" || die "未找到 install.sh 以执行卸载（预期 ${NLTDEPLOY_SRC_DIR}/install.sh 或 libexec bundle）"
  echo "==> 卸载: bash ${sh} uninstall" >&2
  exec bash "${sh}" uninstall "$@"
}

_resolve_entry() {
  local name="$1" rel c
  rel="$(_entry_rel "${name}")" || die "未知入口: ${name}（nltdeploy list 查看）"
  for c in \
    "${SCRIPT_DIR}/${rel}" \
    "${SCRIPT_DIR}/../${rel}" \
    "${NLTDEPLOY_ROOT}/libexec/nltdeploy/${rel}"; do
    [[ -f "$c" ]] && { printf '%s\n' "$c"; return 0; }
  done
  die "未找到入口脚本: ${rel}（请先安装：install.sh install）"
}

cmd_entry() {
  local name="$1" target
  shift
  target="$(_resolve_entry "$name")"
  exec bash "$target" "$@"
}

open_entry() {
  local target
  target="$(_resolve_entry "$1")"
  bash "$target"
}

interactive_main() {
  if ! command -v gum >/dev/null 2>&1 && [[ -x "${HOME}/opt/gum/bin/gum" ]]; then
    export PATH="${HOME}/opt/gum/bin:${PATH}"
  fi

  if declare -F nlt_ui_banner >/dev/null 2>&1; then
    nlt_ui_banner "nltdeploy" "开发环境、工具与服务管理"
  fi

  command -v gum >/dev/null 2>&1 || { usage; return 0; }

  local -a labels=(
    "service    服务管理与运行状态"
    "dev        语言与开发环境"
    "tool       本机工具与网络诊断"
    "ai         AI 编程 CLI"
    "upgrade    升级 nltdeploy"
    "uninstall  卸载 nltdeploy"
    "help       命令帮助"
    "quit       退出"
  )

  local pick key
  while true; do
    if declare -F nlt_ui_choose >/dev/null 2>&1; then
      pick="$(nlt_ui_choose "nltdeploy / 选择领域" "${labels[@]}")" || return 0
    else
      pick="$(printf '%s\n' "${labels[@]}" | gum filter --header "nltdeploy / 选择领域" --limit 1)" || return 0
    fi
    [[ -n "${pick}" ]] || return 0
    key="${pick%% *}"

    case "${key}" in
      service)   open_entry services ;;
      dev)       open_entry dev ;;
      tool)      open_entry tools ;;
      ai)        open_entry ai-cli ;;
      upgrade)   cmd_upgrade ;;
      uninstall) cmd_uninstall ;;
      help)      usage ;;
      *)         return 0 ;;
    esac
    echo ""
  done
}

main() {
  if [[ $# -eq 0 ]]; then
    if [[ "${NONINTERACTIVE:-0}" == "1" || ! -t 0 ]]; then
      usage
      return 0
    fi
    interactive_main
    return 0
  fi

  case "$1" in
    upgrade|update) shift; cmd_upgrade "$@" ;;
    uninstall|remove) shift; cmd_uninstall "$@" ;;
    list|--list) cmd_list ;;
    service|services) shift; cmd_entry services "$@" ;;
    tool|tools) shift; cmd_entry tools "$@" ;;
    ai|ai-cli) shift; cmd_entry ai-cli "$@" ;;
    dev) shift; cmd_entry dev "$@" ;;
    pip-sources|python-env|utils|github-net|port-kill|download|cockpit-tools|airflow|celery|paperclip|code-server|new-api|sub2api|open-pencil)
      cmd="$1"; shift; cmd_entry "${cmd}" "$@"
      ;;
    help|-h|--help) usage ;;
    *) echo "未知命令: $1" >&2; usage >&2; exit 2 ;;
  esac
}

main "$@"
