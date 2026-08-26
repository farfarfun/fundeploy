#!/usr/bin/env bash
# fundeploy tool：工具类内部路由。
#
# 用法:
#   fundeploy tool <tool> install      # 检测是否已安装，未安装则安装
#   fundeploy tool <tool> upgrade      # 升级到最新
#   fundeploy tool <tool> uninstall    # 卸载
#   fundeploy tool <tool> <其它子命令> # 原样透传给工具脚本
#   fundeploy tool list                # 列出可用工具
#   fundeploy tool help                # 显示本说明
#   fundeploy tool                     # 交互菜单
#
# 供服务脚本调用（服务内部依赖工具时，不自行检测，直接）:
#   fundeploy tool gum install
#   fundeploy tool python-env install
#
# 设计约定:
#   - install = 幂等「检测→未装则装」，由各工具 setup.sh 的 install 子命令负责。
#   - upgrade 为规范动词；各工具历史上用 update，这里做归一化映射。
#   - 支持 linux 与 mac（macOS/Darwin），具体平台分支在各工具内实现。
#   - 仓库内（scripts/tools/）与安装后（libexec/fundeploy/tools/）两种布局自适应。
#
# 环境变量:
#   NONINTERACTIVE=1   无参数时不进入 gum 菜单，直接打印 help 退出。
#   GUM_HOME           gum 安装根目录（默认 ~/opt/gum），用于 gum uninstall。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# 可选加载公共片段（用于 _nlt_ensure_gum 驱动菜单）；缺失不致命。
for _cand in \
  "${SCRIPT_DIR}/../lib/nlt-common.sh" \
  "${SCRIPT_DIR}/../../lib/nlt-common.sh" \
  "${SCRIPT_DIR}/lib/nlt-common.sh"; do
  if [[ -f "${_cand}" ]]; then
    # shellcheck source=/dev/null
    source "${_cand}"
    break
  fi
done

# 可选加载统一交互主题（banner / 主题化菜单）；缺失不致命。
for _cand in \
  "${SCRIPT_DIR}/../lib/nlt-ui.sh" \
  "${SCRIPT_DIR}/../../lib/nlt-ui.sh" \
  "${SCRIPT_DIR}/lib/nlt-ui.sh"; do
  if [[ -f "${_cand}" ]]; then
    # shellcheck source=/dev/null
    source "${_cand}"
    break
  fi
done

die() { echo "错误: $*" >&2; exit 1; }

# 工具注册表：name -> 相对 setup.sh 路径尾部（相对 tools/ 或 libexec 根）。
# gum 为一等工具，映射到 utils/setup.sh 的 gum 子命令。
_nlt_tools_tail() {
  case "$1" in
    brew)          echo "brew/setup.sh" ;;
    gum)           echo "utils/setup.sh" ;;
    aliases)       echo "utils/setup.sh" ;;
    download)      echo "download/setup.sh" ;;
    pip-sources)   echo "pip-sources/setup.sh" ;;
    python-env)    echo "python-env/setup.sh" ;;
    github-net)    echo "github-net/setup.sh" ;;
    skills-sync)   echo "skills-sync/setup.sh" ;;
    port-kill)     echo "port-kill/setup.sh" ;;
    cockpit-tools) echo "cockpit-tools/setup.sh" ;;
    utils)         echo "utils/setup.sh" ;;
    go)            echo "dev/go/setup.sh" ;;
    rust)          echo "dev/rust/setup.sh" ;;
    nodejs)        echo "dev/nodejs/setup.sh" ;;
    pnpm)          echo "dev/pnpm/setup.sh" ;;
    uv)            echo "dev/uv/setup.sh" ;;
    *)             return 1 ;;
  esac
}

# 已知工具名（供 list / 菜单 / 校验），顺序即展示顺序。
_NLT_TOOLS_NAMES=(
  brew gum download github-net skills-sync port-kill cockpit-tools
)

# 解析某工具 setup.sh 的绝对路径（自适应仓库内 / libexec 两种布局）。
_nlt_tools_resolve() {
  local tail="$1" b p
  tail="$(_nlt_tools_tail "$tail")" || return 1
  for b in "${SCRIPT_DIR}" "${SCRIPT_DIR}/.." "${SCRIPT_DIR}/../.."; do
    p="${b}/${tail}"
    if [[ -f "${p}" ]]; then
      (cd "$(dirname "${p}")" && printf '%s/%s\n' "$(pwd)" "$(basename "${p}")")
      return 0
    fi
  done
  return 1
}

usage() {
  cat <<'EOF'
用法: fundeploy tool <工具> <install|upgrade|uninstall> [args...]
      fundeploy tool list
      fundeploy tool help

动作:
  install     检测是否已安装，未安装则安装（幂等）。服务依赖工具时统一走此入口。
  upgrade     升级到最新（映射到各工具的 update / 强制重装）。
  uninstall   卸载该工具安装的内容。
  <其它>      原样透传给该工具 setup.sh（如 reinstall、status 等）。

工具:
  brew gum download github-net skills-sync port-kill cockpit-tools

示例:
  fundeploy tool brew install
  fundeploy tool gum install
  fundeploy tool gum upgrade
  fundeploy tool github-net doctor
  fundeploy tool cockpit-tools uninstall

无参数（交互 TTY）: 进入 gum 菜单选择工具与动作；NONINTERACTIVE=1 或非 TTY 时打印本说明。
支持 linux 与 macOS。
EOF
}

cmd_list() {
  local n tail resolved
  echo "可用工具（fundeploy tool <工具> <install|upgrade|uninstall>）:"
  for n in "${_NLT_TOOLS_NAMES[@]}"; do
    tail="$(_nlt_tools_tail "$n" || true)"
    if resolved="$(_nlt_tools_resolve "$n" 2>/dev/null)"; then
      printf '  %-14s -> %s\n' "$n" "${resolved}"
    else
      printf '  %-14s -> (未找到 setup.sh: %s)\n' "$n" "${tail}"
    fi
  done
}

# gum 卸载（utils 无 uninstall 子命令，这里就地移除 GUM_HOME）。
_nlt_tools_gum_uninstall() {
  local gum_home="${GUM_HOME:-${HOME}/opt/gum}"
  local root hp
  [[ -d "${gum_home}" ]] || { echo "gum 未安装（${gum_home} 不存在），跳过。"; return 0; }
  root="$(cd "${gum_home}" && pwd -P)"
  hp="$(cd "$HOME" && pwd -P)"
  [[ "${root}" == "/" ]] && die "拒绝删除根目录"
  [[ "${root}" == "${hp}" ]] && die "拒绝删除 \$HOME"
  case "${root}" in
    "${hp}"/*) ;;
    *) die "gum 安装目录不在 \$HOME 下，出于安全拒绝自动删除: ${root}（请手动清理）" ;;
  esac
  if [[ "${NONINTERACTIVE:-0}" != "1" && -t 0 ]]; then
    # nlt_ui_confirm 自身已内建 gum/read 双路径，无需在此再分叉一次。
    nlt_ui_confirm "将删除 gum 安装目录 ${root}，确认？" || { echo "已取消。"; return 0; }
  fi
  rm -rf "${root}"
  echo "已卸载 gum（删除 ${root}）。PATH 中的 gum 片段请按需手动清理。"
}

# 分发：将规范动词映射到具体工具 setup.sh 的子命令后 exec。
dispatch() {
  local tool="$1"; shift
  local verb="${1:-}"; [[ $# -gt 0 ]] && shift

  local setup expected
  expected="$(_nlt_tools_tail "${tool}")"
  setup="$(_nlt_tools_resolve "${tool}" 2>/dev/null)" \
    || die "安装不完整：缺少 ${expected}。请运行 fundeploy upgrade 后重试"

  # gum 一等工具：映射到 utils setup.sh 的 gum 子命令。
  if [[ "${tool}" == "gum" ]]; then
    case "${verb}" in
      install|"")   exec bash "${setup}" gum "$@" ;;
      upgrade|update) exec bash "${setup}" gum --force "$@" ;;
      uninstall|remove) _nlt_tools_gum_uninstall; return $? ;;
      reinstall)    exec bash "${setup}" gum --force "$@" ;;
      *)            exec bash "${setup}" gum "${verb}" "$@" ;;
    esac
  fi

  # 其它工具：upgrade 归一化为各脚本支持的动词。
  case "${verb}" in
    "")
      [[ "${tool}" == "skills-sync" ]] && exec bash "${setup}"
      exec bash "${setup}" help
      ;;
    help|-h|--help)
      exec bash "${setup}" help
      ;;
    install)
      exec bash "${setup}" install "$@"
      ;;
    upgrade)
      # dev 工具接受 upgrade；tools/* 历史用 update。二者都先试 upgrade 再回退 update 不便，
      # 这里按类别选择：dev 系列传 upgrade，其余传 update（均与各脚本 case 分支一致）。
      case "${tool}" in
        go|rust|nodejs|pnpm|uv) exec bash "${setup}" upgrade "$@" ;;
        *)                      exec bash "${setup}" update "$@" ;;
      esac
      ;;
    update|uninstall|reinstall|remove|status|check|doctor)
      exec bash "${setup}" "${verb}" "$@"
      ;;
    *)
      # 未知动词原样透传（工具自身负责校验）。
      exec bash "${setup}" "${verb}" "$@"
      ;;
  esac
}

# 工具一行描述（交互菜单用）。
_nlt_tool_desc() {
  case "$1" in
    brew)          echo "Homebrew 包管理器" ;;
    gum)           echo "终端交互 UI（菜单/输入/确认）" ;;
    download)      echo "GitHub 下载加速" ;;
    pip-sources)   echo "pip 镜像源切换" ;;
    python-env)    echo "Python 虚拟环境管理" ;;
    github-net)    echo "GitHub 网络诊断" ;;
    skills-sync)   echo "Gitee skills 仓库同步" ;;
    port-kill)     echo "按端口杀进程" ;;
    cockpit-tools) echo "Cockpit 运维面板工具" ;;
    go)            echo "Go 工具链" ;;
    rust)          echo "Rust（rustup）" ;;
    nodejs)        echo "Node.js" ;;
    pnpm)          echo "pnpm 包管理器" ;;
    uv)            echo "uv（Astral Python 安装器）" ;;
    *)             echo "" ;;
  esac
}

# 无参交互菜单（gum 驱动）。
interactive_main() {
  if declare -F _nlt_ensure_gum >/dev/null 2>&1; then
    _nlt_ensure_gum || { usage; return 0; }
  elif ! command -v gum >/dev/null 2>&1; then
    usage
    return 0
  fi

  if declare -F nlt_ui_banner >/dev/null 2>&1; then
    nlt_ui_banner "fundeploy / tool" "本机工具安装、升级与卸载"
  fi

  # 组装带描述的工具项：key 为首个 token，label 为 "key — 描述"。
  local -a labels=()
  local n desc
  for n in "${_NLT_TOOLS_NAMES[@]}"; do
    desc="$(_nlt_tool_desc "$n")"
    labels+=("$(printf '%-14s— %s' "$n" "$desc")")
  done
  labels+=("退出")

  local pick tool action
  if declare -F nlt_ui_choose >/dev/null 2>&1; then
    pick="$(nlt_ui_choose "fundeploy / tool / 选择工具" "${labels[@]}")" || return 0
  else
    pick="$(printf '%s\n' "${labels[@]}" | gum choose --header "fundeploy / tool / 选择工具")" || return 0
  fi
  [[ -n "${pick}" ]] || return 0
  tool="${pick%% *}"
  [[ "${tool}" == "退出" ]] && return 0

  if declare -F nlt_ui_choose >/dev/null 2>&1; then
    action="$(nlt_ui_choose "对 ${tool} 执行" \
      "install    — 检测并安装（幂等）" \
      "upgrade    — 升级到最新" \
      "uninstall  — 卸载" \
      "back       — 返回")" || return 0
  else
    action="$(printf '%s\n' "install" "upgrade" "uninstall" "back" | gum choose --header "对 ${tool} 执行")" || return 0
  fi
  action="${action%% *}"
  [[ -z "${action}" || "${action}" == "back" ]] && return 0
  dispatch "${tool}" "${action}"
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
    list|--list)
      cmd_list
      ;;
    help|-h|--help)
      usage
      ;;
    *)
      # 第一个参数为工具名 → 分发。
      if _nlt_tools_tail "$1" >/dev/null 2>&1; then
        dispatch "$@"
      else
        echo "未知命令或工具: $1" >&2
        usage >&2
        exit 2
      fi
      ;;
  esac
}

main "$@"
