#!/usr/bin/env bash
# nltdeploy：整个项目的顶层入口。
#
# 用法:
#   nltdeploy upgrade [--source github|gitee|local]   升级本安装（同步 libexec 与 bin）
#   nltdeploy uninstall                               卸载 nltdeploy（删除 NLTDEPLOY_ROOT 并清理 PATH）
#   nltdeploy tools <工具> <install|upgrade|uninstall> 透传给 nlt-tools
#   nltdeploy dev|services|ai-cli|... [args]          透传给对应 nlt-* 入口
#   nltdeploy list                                    列出顶层可用入口
#   nltdeploy help
#   nltdeploy                                         交互 TTY：gum 菜单；否则打印 help
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
#   NLTDEPLOY_UNINSTALL_YES=1 非 TTY 下允许卸载
#   NONINTERACTIVE=1          无参数时打印 help 退出，不进入菜单
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
NLTDEPLOY_ROOT="${NLTDEPLOY_ROOT:-${HOME}/.local/nltdeploy}"
NLTDEPLOY_SRC_DIR="${NLTDEPLOY_SRC_DIR:-${NLTDEPLOY_ROOT}/src/nltdeploy}"
NLTDEPLOY_GITHUB_RAW="${NLTDEPLOY_GITHUB_RAW:-https://raw.githubusercontent.com/farfarfun/nltdeploy/HEAD/install.sh}"
NLTDEPLOY_GITEE_RAW="${NLTDEPLOY_GITEE_RAW:-https://gitee.com/farfarfun/nltdeploy/raw/master/install.sh}"

die() { echo "错误: $*" >&2; exit 1; }

usage() {
  cat <<'EOF'
用法: nltdeploy <command> [args]

命令:
  upgrade [--source github|gitee|local]   升级 nltdeploy（同步 libexec 与 bin）
  uninstall                               卸载 nltdeploy（删除安装目录并清理 PATH 片段）
  tools <工具> <install|upgrade|uninstall> 透传给 nlt-tools（工具类统一入口）
  dev [args...]                           透传给 nlt-dev（开发环境统一入口）
  services [args...]                      透传给 nlt-services（服务总览与安装入口）
  ai-cli [args...]                        透传给 nlt-ai-cli（AI CLI 统一入口）
  pip-sources | python-env | utils | github-net | port-kill | download | cockpit-tools
                                           透传给对应 nlt-* 工具入口
  airflow | celery | paperclip | code-server | new-api | sub2api | open-pencil
                                           透传给对应 nlt-* 服务入口
  list                                    列出所有可路由入口
  help / -h / --help                      本说明

upgrade 源:
  github（默认公网）/ gitee（国内镜像）/ local（本地仓库 install.sh，git pull + 重新同步）
  不指定 --source 时：优先 local，其次 github，再 gitee。

示例:
  nltdeploy upgrade
  nltdeploy upgrade --source gitee
  nltdeploy uninstall
  nltdeploy tools gum install
  nltdeploy dev uv --help
  nltdeploy services status --no-http
  nltdeploy ai-cli list
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

_entry_bin() {
  case "$1" in
    tools) echo "nlt-tools" ;;
    *)     echo "nlt-$1" ;;
  esac
}

_entry_names() {
  cat <<'EOF'
tools
dev
ai-cli
pip-sources
python-env
utils
github-net
port-kill
download
cockpit-tools
services
airflow
celery
paperclip
code-server
new-api
sub2api
open-pencil
EOF
}

cmd_list() {
  echo "可用入口（nltdeploy <入口> [args...]）:"
  _entry_names | while IFS= read -r name; do
    [[ -n "$name" ]] || continue
    printf '  %-14s -> %s\n' "$name" "$(_entry_bin "$name")"
  done
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
  command -v curl >/dev/null 2>&1 || die "需要 curl 才能从 ${1:-远端} 升级"
  curl -LsSf "$@"
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
  local source=""
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --source) source="${2:-}"; shift 2 ;;
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
  sh="$(_resolve_local_install_sh)" || die "未找到 install.sh 以执行卸载（预期 ${NLTDEPLOY_SRC_DIR}/install.sh 或 libexec bundle）"
  echo "==> 卸载: bash ${sh} uninstall" >&2
  exec bash "${sh}" uninstall "$@"
}

cmd_entry() {
  local name="$1"; shift
  local rel bin c
  rel="$(_entry_rel "${name}")" || die "未知入口: ${name}（nltdeploy list 查看）"
  bin="${NLTDEPLOY_ROOT}/bin/$(_entry_bin "${name}")"
  if [[ -x "${bin}" ]]; then
    exec "${bin}" "$@"
  fi
  # 仓库内直接运行时回退到源脚本；已安装 libexec 缺 wrapper 时也可回退。
  for c in \
    "${SCRIPT_DIR}/${rel}" \
    "${SCRIPT_DIR}/../${rel}" \
    "${NLTDEPLOY_ROOT}/libexec/nltdeploy/${rel}"; do
    [[ -f "$c" ]] && exec bash "$c" "$@"
  done
  die "未找到 $(_entry_bin "${name}")（请先安装：install.sh install）"
}

interactive_main() {
  if ! command -v gum >/dev/null 2>&1 && [[ -x "${HOME}/opt/gum/bin/gum" ]]; then
    export PATH="${HOME}/opt/gum/bin:${PATH}"
  fi
  command -v gum >/dev/null 2>&1 || { usage; return 0; }
  local pick
  pick="$(printf '%s\n' "upgrade" "uninstall" "list" $(_entry_names) "help" "退出" | gum choose --header "nltdeploy")" || return 0
  case "${pick}" in
    upgrade)   cmd_upgrade ;;
    uninstall) cmd_uninstall ;;
    list)      cmd_list ;;
    tools|dev|ai-cli|pip-sources|python-env|utils|github-net|port-kill|download|cockpit-tools|services|airflow|celery|paperclip|code-server|new-api|sub2api|open-pencil)
      cmd_entry "${pick}"
      ;;
    help)      usage ;;
    *)         return 0 ;;
  esac
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
    tools|dev|ai-cli|pip-sources|python-env|utils|github-net|port-kill|download|cockpit-tools|services|airflow|celery|paperclip|code-server|new-api|sub2api|open-pencil)
      cmd="$1"; shift; cmd_entry "${cmd}" "$@"
      ;;
    help|-h|--help) usage ;;
    *) echo "未知命令: $1" >&2; usage >&2; exit 2 ;;
  esac
}

main "$@"
