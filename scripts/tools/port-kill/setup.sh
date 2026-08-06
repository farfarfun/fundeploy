#!/usr/bin/env bash
# port-kill — 根据端口号查找并终止进程的工具，可被其他服务脚本 source 使用。
#
# 用法（直接执行）：
#   ./setup.sh                        # 无参：gum 交互模式
#   ./setup.sh kill <port> [port…]    # 杀掉占用指定端口的进程（SIGTERM → SIGKILL）
#   ./setup.sh list [port]            # 列出占用端口的进程（不杀）
#   ./setup.sh install                # 无需单独安装，清理历史包装脚本
#   ./setup.sh uninstall              # 清理历史包装脚本
#   NONINTERACTIVE=1 ./setup.sh kill 8080   # 非交互，无需确认直接杀
#
# 作为库使用（source）：
#   source /path/to/port-kill/setup.sh --lib
#   nlt_kill_port 8080          # 终止占用 8080 的进程
#   nlt_list_port 8080          # 列出占用 8080 的进程信息
#   nlt_kill_ports 8080 8443    # 批量终止

# 本文件同时用作可 source 的库（见头部 `--lib` 用法）。直接执行时才收紧 shell 选项——
# 原先无条件 `set -euo pipefail` 会把 nounset/pipefail 泄漏进调用方 shell，
# 让未按 set -u 编写的宿主脚本在无关位置崩溃。
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  set -euo pipefail
else
  set -o pipefail
fi

# ── 路径解析 & 公共库 ────────────────────────────────────────────────────────
_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
_NLT_LIB=""
if [[ -f "${_SCRIPT_DIR}/../lib/nlt-common.sh" ]]; then
  _NLT_LIB="$(cd "${_SCRIPT_DIR}/../lib" && pwd)"
elif [[ -f "${_SCRIPT_DIR}/../../lib/nlt-common.sh" ]]; then
  _NLT_LIB="$(cd "${_SCRIPT_DIR}/../../lib" && pwd)"
fi

if [[ -n "${_NLT_LIB}" ]]; then
  # shellcheck source=../../lib/nlt-common.sh
  source "${_NLT_LIB}/nlt-common.sh"
fi

# ── 历史独立入口（仅用于清理）─────────────────────────────────────────────────
NLT_BIN_DIR="${NLT_BIN_DIR:-${HOME}/opt/nlt/bin}"
INSTALL_NAME="nlt-port-kill"

# ── 基础输出工具 ──────────────────────────────────────────────────────────────
_pk_say()  { printf '%s\n' "$*"; }
_pk_info() {
  if command -v gum >/dev/null 2>&1; then
    gum style --foreground 212 "$*"
  else
    printf '[INFO] %s\n' "$*"
  fi
}
_pk_warn() {
  if command -v gum >/dev/null 2>&1; then
    gum style --foreground 214 "$*" >&2
  else
    printf '[WARN] %s\n' "$*" >&2
  fi
}
_pk_err()  { printf '[ERROR] %s\n' "$*" >&2; }

# 确认：优先用 lib 的统一实现，独立运行（无 lib）时回退 read y/N。
# 关键：绝不以「有没有 gum」作为是否询问的条件——原实现在无 gum 时直接跳过确认，
# 于是 kill 会在完全没有提示的情况下执行。
_pk_confirm() {
  local prompt="$1"
  if declare -F nlt_ui_confirm >/dev/null 2>&1; then
    nlt_ui_confirm "${prompt}"
    return $?
  fi
  local a
  read -r -p "${prompt} [y/N] " a || return 1
  case "$a" in [yY] | [yY][eE][sS]) return 0 ;; *) return 1 ;; esac
}

_pk_header() {
  if command -v gum >/dev/null 2>&1; then
    gum style --bold --foreground 212 "$1"
  else
    printf '\n==> %s\n' "$1"
  fi
}

# ── 核心库函数（可被其他脚本 source 使用）────────────────────────────────────

# nlt_list_port <port>
# 列出占用指定端口的进程，返回 "pid:comm" 行列表；没有进程时返回空。
nlt_list_port() {
  local port="$1"
  local result=()

  # 优先 lsof（macOS / Linux 均可用）
  if command -v lsof >/dev/null 2>&1; then
    while IFS= read -r line; do
      [[ -z "$line" ]] && continue
      result+=("$line")
    done < <(lsof -ti :"${port}" 2>/dev/null | sort -un | while read -r pid; do
      local comm
      comm="$(ps -p "$pid" -o comm= 2>/dev/null || echo '?')"
      printf '%s:%s\n' "$pid" "$comm"
    done)
  # 备用：ss（Linux）
  elif command -v ss >/dev/null 2>&1; then
    while IFS= read -r pid; do
      [[ -z "$pid" ]] && continue
      local comm
      comm="$(ps -p "$pid" -o comm= 2>/dev/null || echo '?')"
      result+=("${pid}:${comm}")
    done < <(ss -tlnp "sport = :${port}" 2>/dev/null |
              sed -nE 's/.*pid=([0-9]+).*/\1/p' | sort -nu || true)
  # 备用：netstat（老系统）
  elif command -v netstat >/dev/null 2>&1; then
    while IFS= read -r pid; do
      [[ -z "$pid" ]] && continue
      local comm
      comm="$(ps -p "$pid" -o comm= 2>/dev/null || echo '?')"
      result+=("${pid}:${comm}")
    done < <(netstat -tlnp 2>/dev/null |
              awk -v p=":${port}" '$4 ~ p" " || $4 ~ p"$" {print $7}' |
              grep -oE '^[0-9]+' || true)
  fi

  printf '%s\n' "${result[@]+"${result[@]}"}"
}

# nlt_kill_port <port> [signal]
# 终止占用 <port> 的所有进程。
# signal 默认 TERM；15s 后若还存在则升级 KILL。
# 返回 0 = 成功（含"本来就没进程"），1 = 仍有进程未被杀死。
nlt_kill_port() {
  local port="$1"
  local sig="${2:-TERM}"
  local entries=() line
  while IFS= read -r line || [[ -n "${line}" ]]; do
    [[ -z "${line}" ]] && continue
    entries+=("$line")
  done < <(nlt_list_port "${port}")

  if [[ ${#entries[@]} -eq 0 ]]; then
    _pk_info "端口 ${port}：无占用进程。"
    return 0
  fi

  _pk_header "端口 ${port} — 发现 ${#entries[@]} 个进程"
  local pids=()
  for entry in "${entries[@]}"; do
    local pid comm
    pid="${entry%%:*}"
    comm="${entry##*:}"
    _pk_say "  PID ${pid}  (${comm})"
    pids+=("$pid")
  done

  # 交互确认（非 NONINTERACTIVE、有 TTY）。不再要求 gum：缺 gum 时回退 read y/N，
  # 而不是跳过确认直接开杀。
  if [[ "${NONINTERACTIVE:-0}" != "1" ]] && [[ -t 0 ]]; then
    _pk_confirm "确认终止以上 ${#pids[@]} 个进程（端口 ${port}）？" || {
      _pk_warn "已取消（端口 ${port}）。"
      return 0
    }
  fi

  # 第一轮：SIGTERM（或调用方指定信号）
  for pid in "${pids[@]}"; do
    if kill -"${sig}" "${pid}" 2>/dev/null; then
      _pk_say "  已发送 SIG${sig} → PID ${pid}"
    else
      _pk_warn "  无法发送信号给 PID ${pid}（可能已退出或权限不足）"
    fi
  done

  # 若首轮是 TERM，等待并升级 KILL
  if [[ "${sig}" == "TERM" || "${sig}" == "15" ]]; then
    local deadline=$(( $(date +%s) + 15 ))
    local remaining=("${pids[@]}")
    while [[ ${#remaining[@]} -gt 0 ]] && [[ $(date +%s) -lt $deadline ]]; do
      sleep 1
      local still=()
      for pid in "${remaining[@]}"; do
        kill -0 "${pid}" 2>/dev/null && still+=("$pid") || true
      done
      remaining=("${still[@]+"${still[@]}"}")
    done

    if [[ ${#remaining[@]} -gt 0 ]]; then
      _pk_warn "以下进程在 15s 内未退出，升级 SIGKILL：${remaining[*]}"
      for pid in "${remaining[@]}"; do
        kill -KILL "${pid}" 2>/dev/null || true
      done
      sleep 1
    fi
  fi

  # 最终校验
  local still_alive=()
  for pid in "${pids[@]}"; do
    kill -0 "${pid}" 2>/dev/null && still_alive+=("$pid") || true
  done

  if [[ ${#still_alive[@]} -eq 0 ]]; then
    _pk_info "端口 ${port}：所有进程已终止。"
    return 0
  else
    _pk_err "端口 ${port}：以下进程仍在运行：${still_alive[*]}"
    return 1
  fi
}

# nlt_kill_ports <port> [port…]
# 批量终止多个端口的进程，收集失败并在末尾汇报。
nlt_kill_ports() {
  local failed=()
  for port in "$@"; do
    nlt_kill_port "${port}" || failed+=("${port}")
  done
  if [[ ${#failed[@]} -gt 0 ]]; then
    _pk_err "以下端口的进程未能完全终止：${failed[*]}"
    return 1
  fi
  return 0
}

_pk_uninstall_wrapper() {
  local target="${NLT_BIN_DIR}/${INSTALL_NAME}"
  if [[ -f "${target}" ]]; then
    rm -f "${target}"
    _pk_info "已移除: ${target}"
  else
    _pk_warn "未找到已安装的 ${target}，无需卸载。"
  fi
}

# ── 子命令：list ──────────────────────────────────────────────────────────────

cmd_list() {
  if [[ $# -eq 0 ]]; then
    if command -v gum >/dev/null 2>&1; then
      local port
      port="$(gum input --placeholder "输入端口号（如 8080）")" || { _pk_warn "已取消。"; return 0; }
      [[ -z "$port" ]] && { _pk_warn "端口号不能为空。"; return 1; }
      set -- "${port}"
    else
      _pk_err "用法: list <port>"
      return 1
    fi
  fi

  for port in "$@"; do
    _pk_header "端口 ${port} 占用情况"
    local entries=() line
    while IFS= read -r line || [[ -n "${line}" ]]; do
      [[ -z "${line}" ]] && continue
      entries+=("$line")
    done < <(nlt_list_port "${port}")
    if [[ ${#entries[@]} -eq 0 ]]; then
      _pk_say "  (无占用进程)"
    else
      for e in "${entries[@]}"; do
        local pid comm
        pid="${e%%:*}"; comm="${e##*:}"
        _pk_say "  PID ${pid}  ${comm}"
      done
    fi
  done
}

# ── 子命令：kill ──────────────────────────────────────────────────────────────

cmd_kill() {
  if [[ $# -eq 0 ]]; then
    if command -v gum >/dev/null 2>&1; then
      local ports_str
      ports_str="$(gum input --placeholder "输入端口号，多个用空格分隔（如 8080 8443）")" || {
        _pk_warn "已取消。"; return 0
      }
      [[ -z "${ports_str}" ]] && { _pk_warn "端口号不能为空。"; return 1; }
      read -ra _ports <<< "${ports_str}"
      nlt_kill_ports "${_ports[@]}"
    else
      _pk_err "用法: kill <port> [port…]"
      return 1
    fi
  else
    nlt_kill_ports "$@"
  fi
}

# ── Tool 标准子命令 ───────────────────────────────────────────────────────────

cmd_install() {
  _pk_uninstall_wrapper
  _pk_info "port-kill 已由 nltdeploy 提供，无需单独安装。"
  _pk_say "  用法: nltdeploy tool port-kill kill <port> [port…]"
  _pk_say "  用法: nltdeploy tool port-kill list [port]"
}

cmd_update() {
  cmd_install
}

cmd_reinstall() {
  cmd_install
}

cmd_uninstall() {
  _pk_header "卸载 ${INSTALL_NAME}"
  if [[ "${NONINTERACTIVE:-0}" != "1" ]] && [[ -t 0 ]]; then
    _pk_confirm "将移除 ${NLT_BIN_DIR}/${INSTALL_NAME}，继续？" || {
      _pk_warn "已取消。"; return 0
    }
  fi
  _pk_uninstall_wrapper
}

# ── 无参交互主菜单 ────────────────────────────────────────────────────────────

_interactive_main() {
  if [[ -n "${_NLT_LIB}" ]]; then
    _nlt_ensure_gum || true
  fi
  command -v gum >/dev/null 2>&1 || {
    _pk_err "gum 未安装，无法进入交互模式。请传入子命令，如: $0 kill <port>"
    return 1
  }

  while true; do
    local pick
    pick="$(gum choose --header "port-kill — 选择操作" \
      "kill  — 终止占用指定端口的进程" \
      "list  — 列出占用指定端口的进程" \
      "退出")" || { _pk_warn "已取消。"; return 0; }

    case "${pick}" in
      kill*)
        local ports_str
        ports_str="$(gum input --placeholder "输入端口号，多个用空格分隔")" || { _pk_warn "已取消。"; continue; }
        [[ -z "${ports_str}" ]] && continue
        read -ra _ports <<< "${ports_str}"
        nlt_kill_ports "${_ports[@]}" || true
        ;;
      list*)
        local port
        port="$(gum input --placeholder "输入端口号")" || { _pk_warn "已取消。"; continue; }
        [[ -z "${port}" ]] && continue
        cmd_list "${port}"
        ;;
      "退出") _pk_say "已退出。"; return 0 ;;
      *) _pk_warn "无效选项。" ;;
    esac
    _pk_say ""
  done
}

# ── 帮助 ──────────────────────────────────────────────────────────────────────

_usage() {
  cat <<EOF
用法: $(basename "${BASH_SOURCE[0]}") <子命令> [参数…]

核心子命令:
  kill <port> [port…]   终止占用指定端口的进程（SIGTERM → 超时后 SIGKILL）
  list [port]           列出占用端口的进程（不终止）

Tool 管理子命令:
  install/update        无需单独安装；清理旧版 ${INSTALL_NAME} 包装脚本
  uninstall             清理旧版 ${INSTALL_NAME} 包装脚本

库 source 模式:
  source setup.sh --lib
  nlt_kill_port <port>        # 终止单个端口
  nlt_kill_ports <port>…      # 批量终止
  nlt_list_port <port>        # 列出进程（返回 "pid:comm" 行）

环境变量:
  NONINTERACTIVE=1      跳过所有 gum 确认
  NLT_BIN_DIR           旧版入口清理目录（默认 ~/opt/nlt/bin）
EOF
}

# ── 入口 ──────────────────────────────────────────────────────────────────────

main() {
  # source 模式：仅导出库函数，不执行任何动作
  if [[ "${1:-}" == "--lib" ]]; then
    return 0
  fi

  if [[ $# -eq 0 ]]; then
    _interactive_main
    return 0
  fi

  local cmd="$1"; shift
  case "${cmd}" in
    kill)       cmd_kill "$@" ;;
    list)       cmd_list "$@" ;;
    install)    cmd_install ;;
    update|upgrade) cmd_update ;;
    reinstall)  cmd_reinstall ;;
    uninstall)  cmd_uninstall ;;
    help|-h|--help) _usage ;;
    *)
      _pk_err "未知子命令: ${cmd}"
      _usage >&2
      exit 2
      ;;
  esac
}

main "$@"
