#!/usr/bin/env bash
# 本机常用 CLI 与 shell 便利项。与 README 一致可通过 curl 管道执行，不经仓库内脚本互引：
#   curl -LsSf …/scripts/tools/utils/setup.sh | bash -s -- gum      # 仅 gum
#   curl -LsSf …/scripts/tools/utils/setup.sh | bash -s -- aliases  # 仅 ll/la/lla 别名
#   curl -LsSf …/scripts/tools/utils/setup.sh | bash -s -- all      # gum + 别名
# 与 .cursor/agents/software-ops.md 一致：gum 安装在 ~/opt/gum/{bin,etc,data,log}。
#
# 用法：
#   ./setup.sh              # 交互 TTY：gum 菜单（缺 gum 时先自动安装）；否则等同 gum
#   ./setup.sh gum [--force]
#   ./setup.sh aliases      # 写入 ll / la / lla（已有标记则跳过）
#   ./setup.sh all [--force]   # gum 再 aliases
#   GUM_USE_BREW=1 ./setup.sh gum
#
# 环境变量：
#   GUM_HOME                     默认 ~/opt/gum
#   GUM_TAG / GUM_USE_BREW       见下
#   SKIP_GUM_SHELL_PROFILE=1     不写入 gum 的 PATH 片段
#   SKIP_UTILS_SHELL_ALIASES=1   不写入 ll/la/lla 别名片段
#   NONINTERACTIVE=1             跳过 gum 安装前确认（curl 管道时 stdin 非 TTY 也会跳过）

set -euo pipefail

_UTILS_INIT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ -f "${_UTILS_INIT_DIR}/../lib/fundeploy-common.sh" ]]; then
  # shellcheck source=../lib/fundeploy-common.sh
  source "${_UTILS_INIT_DIR}/../lib/fundeploy-common.sh"
elif [[ -f "${_UTILS_INIT_DIR}/../../lib/fundeploy-common.sh" ]]; then
  # shellcheck source=../../lib/fundeploy-common.sh
  source "${_UTILS_INIT_DIR}/../../lib/fundeploy-common.sh"
elif [[ -f "${_UTILS_INIT_DIR}/../lib/fundeploy-github-download.sh" ]]; then
  # shellcheck source=../lib/fundeploy-github-download.sh
  source "${_UTILS_INIT_DIR}/../lib/fundeploy-github-download.sh"
elif [[ -f "${_UTILS_INIT_DIR}/../../lib/fundeploy-github-download.sh" ]]; then
  # shellcheck source=../../lib/fundeploy-github-download.sh
  source "${_UTILS_INIT_DIR}/../../lib/fundeploy-github-download.sh"
fi
if ! declare -F _fundeploy_github_download_curl >/dev/null 2>&1; then
  _fundeploy_github_download_curl() { command curl "$@"; }
fi

GUM_HOME="${GUM_HOME:-${HOME}/opt/gum}"

usage() {
  cat <<EOF
用法: setup.sh [command [选项]]

无参数:
  交互式终端: 进入 gum 菜单（缺 gum 时先按 README 同款 curl 安装）；否则等同「gum」子命令（管道 / NONINTERACTIVE=1 等）。

命令:
  gum [--force]    安装 gum 到 ${GUM_HOME}/bin（默认命令）
  aliases          向 ~/.zshrc / ~/.bashrc / ~/.bash_profile 写入 ll、la、lla（已存在标记则跳过）
  all [--force]    依次执行 gum 与 aliases
  help             显示本说明

环境变量: GUM_HOME, GUM_TAG, GUM_USE_BREW, SKIP_GUM_SHELL_PROFILE, SKIP_UTILS_SHELL_ALIASES, NONINTERACTIVE
EOF
}

# 交互菜单前：保证 PATH 上有 gum（与 lib/fundeploy-common.sh 中 _fundeploy_ensure_gum 行为一致；单文件 curl 场景无 fundeploy-common 时内联）。
_ensure_gum_for_interactive_menu() {
  if declare -F _fundeploy_ensure_gum >/dev/null 2>&1; then
    _fundeploy_ensure_gum
    return $?
  fi
  export PATH="${HOME}/opt/gum/bin:${PATH}"
  command -v gum >/dev/null 2>&1 && return 0
  if [[ -x "${HOME}/opt/gum/bin/gum" ]]; then
    export PATH="${HOME}/opt/gum/bin:${PATH}"
    command -v gum >/dev/null 2>&1 && return 0
  fi
  command -v curl >/dev/null 2>&1 || {
    echo "错误: 需要 curl 以安装 gum。" >&2
    return 1
  }
  local _ubase _uurl
  _ubase="${FUNDEPLOY_RAW_BASE:-${fundeploy_RAW_BASE:-https://raw.githubusercontent.com/farfarfun/fundeploy/HEAD}}"
  _uurl="${_ubase}/scripts/tools/utils/setup.sh"
  echo "未检测到 gum，执行: curl -LsSf ${_uurl} | NONINTERACTIVE=1 bash -s -- gum" >&2
  NONINTERACTIVE=1 _fundeploy_github_download_curl -LsSf "${_uurl}" | NONINTERACTIVE=1 bash -s -- gum || {
    echo "错误: gum 安装失败（网络或 FUNDEPLOY_RAW_BASE / fundeploy_RAW_BASE）。" >&2
    return 1
  }
  export PATH="${HOME}/opt/gum/bin:${PATH}"
  command -v gum >/dev/null 2>&1 || {
    echo "错误: gum 仍未可用（预期 ~/opt/gum/bin）。" >&2
    return 1
  }
}

_interactive_main() {
  # 用字面量常量而非通配符匹配菜单项。原实现按 *别名*/*依次*/*gum* 依次匹配，
  # 而「gum 与别名（依次执行）」同时含「别名」和「gum」，会先命中 *别名*，
  # 于是该项只写别名、从不装 gum，cmd_all 成为死代码。
  local L_GUM L_ALIAS L_ALL L_HELP L_QUIT
  L_GUM="安装 / 更新 gum（${GUM_HOME}）"
  L_ALIAS="写入 ll / la / lla 别名"
  L_ALL="gum 与别名（依次执行）"
  L_HELP="查看帮助"
  L_QUIT="退出"

  while true; do
    local pick
    pick="$(gum choose --header "fundeploy-utils" \
      "${L_GUM}" "${L_ALIAS}" "${L_ALL}" "${L_HELP}" "${L_QUIT}")" || return 0
    [[ -z "${pick}" ]] && return 0
    case "${pick}" in
      "${L_QUIT}")  return 0 ;;
      "${L_HELP}")  usage; echo "" ;;
      "${L_ALIAS}") cmd_shell_aliases ;;
      "${L_ALL}")   cmd_all "" ;;
      "${L_GUM}")   cmd_gum "" ;;
      *)            usage; echo "" ;;
    esac
    echo ""
  done
}

# ---------- software-ops：环境感知 / 步骤提示 / 确认（有 gum 时优先 gum）----------
_path_with_gum_home() {
  echo "${GUM_HOME}/bin:${PATH}"
}

_gum_for_ui() {
  PATH="$(_path_with_gum_home)" command -v gum >/dev/null 2>&1
}

_say_step() {
  local msg="$1"
  if _gum_for_ui; then
    PATH="$(_path_with_gum_home)" gum style --foreground 212 "$msg"
  else
    echo "$msg"
  fi
}

_software_ops_confirm() {
  local prompt="$1"
  [[ ! -t 0 ]] && return 0
  [[ "${NONINTERACTIVE:-0}" == "1" ]] && return 0
  if _gum_for_ui; then
    PATH="$(_path_with_gum_home)" gum confirm "$prompt"
  else
    local a
    read -r -e -p "${prompt} [y/N] " a || return 1
    case "$a" in [yY]|[yY][eE][sS]) return 0 ;; *) return 1 ;; esac
  fi
}

_print_environment_and_plan() {
  _say_step "==> 环境: $(uname -s) $(uname -m)  用户: ${USER:-?}"
  _say_step "==> 安装根目录: ${GUM_HOME}（子目录 bin / etc / data / log，与 software-ops 一致）"
  if [[ "${GUM_USE_BREW:-0}" == "1" ]] && [[ "$(uname -s)" == "Darwin" ]]; then
    _say_step "==> 将执行: Homebrew 安装 gum → 复制二进制到 ${GUM_HOME}/bin/"
  else
    _say_step "==> 将执行: 从 GitHub Releases 下载官方包 → 安装至 ${GUM_HOME}/bin/"
  fi
}

_print_post_install_summary() {
  _say_step "==> 安装记录（software-ops）"
  echo "  可执行文件: ${GUM_HOME}/bin/gum"
  echo "  配置目录:   ${GUM_HOME}/etc"
  echo "  数据目录:   ${GUM_HOME}/data"
  echo "  日志目录:   ${GUM_HOME}/log"
  echo "  验证命令:   ${GUM_HOME}/bin/gum --version"
  echo "  PATH 提示:  export PATH=\"\${HOME}/opt/gum/bin:\${PATH}\"（或依赖本脚本写入的 profile 片段）"
}

# ---------- 通用：向 shell profile 追加带标记的块（已有标记则跳过）----------
_GUM_PATH_MARKER_BEGIN='# >>> fundeploy utils-setup: gum PATH >>>'
_GUM_PATH_MARKER_END='# <<< fundeploy utils-setup: gum PATH <<<'

_LS_ALIAS_MARKER_BEGIN='# >>> fundeploy utils-setup: ls aliases >>>'
_LS_ALIAS_MARKER_END='# <<< fundeploy utils-setup: ls aliases <<<'

_append_marked_block_to_profiles() {
  local marker_begin="$1"
  local block="$2"
  local skip="${3:-0}"
  local desc="${4:-片段}"

  [[ "$skip" == "1" ]] && return 0

  local f targets=() new_file=""
  [[ -f "${HOME}/.zshrc" ]] && targets+=("${HOME}/.zshrc")
  [[ -f "${HOME}/.bashrc" ]] && targets+=("${HOME}/.bashrc")
  [[ -f "${HOME}/.bash_profile" ]] && targets+=("${HOME}/.bash_profile")

  if [[ ${#targets[@]} -eq 0 ]]; then
    case "${SHELL:-}" in
      */zsh) new_file="${HOME}/.zshrc" ;;
      */bash) new_file="${HOME}/.bashrc" ;;
      *) new_file="${HOME}/.zshrc" ;;
    esac
    targets+=("$new_file")
  fi

  for f in "${targets[@]}"; do
    if [[ -n "$new_file" && "$f" == "$new_file" && ! -f "$f" ]]; then
      : >"$f"
      echo "已创建 ${f}"
    fi
    [[ -f "$f" ]] || continue

    if grep -qF "${marker_begin}" "$f" 2>/dev/null; then
      echo "${desc} 已存在于 $(basename "${f}")，跳过写入。"
      continue
    fi

    echo "" >>"$f"
    printf '%s\n' "$block" >>"$f"
    # 使用 ${f} 避免全角括号紧跟 $f 时在 set -u 下被误解析为变量名
    echo "已写入 ${desc} 到 ${f}（新开终端生效，或: source ${f}）"
  done
}

_ensure_gum_path_in_session() {
  local d="${GUM_HOME}/bin"
  case ":${PATH}:" in
    *":${d}:"*) ;;
    *) export PATH="${d}:${PATH}" ;;
  esac
}

_gum_path_shell_block() {
  local bindir="${GUM_HOME}/bin"
  # 写入 profile 的块：登录后若 PATH 中尚无该目录则 prepend（\ 避免此处展开 PATH）
  cat <<EOF
${_GUM_PATH_MARKER_BEGIN}
case ":\${PATH}:" in
  *":${bindir}:"*) ;;
  *) export PATH="${bindir}:\${PATH}" ;;
esac
${_GUM_PATH_MARKER_END}
EOF
}

_shell_aliases_block() {
  cat <<EOF
${_LS_ALIAS_MARKER_BEGIN}
alias ll='ls -al'
alias la='ls -A'
alias lla='ls -lA'
${_LS_ALIAS_MARKER_END}
EOF
}

_append_gum_path_to_profile_files() {
  _append_marked_block_to_profiles "${_GUM_PATH_MARKER_BEGIN}" "$(_gum_path_shell_block)" "${SKIP_GUM_SHELL_PROFILE:-0}" "gum PATH"
}

_apply_ls_aliases_in_session() {
  # 当前 bash 子进程内生效（非交互默认关闭 expand_aliases）
  shopt -s expand_aliases 2>/dev/null || true
  alias ll='ls -al' 2>/dev/null || true
  alias la='ls -A' 2>/dev/null || true
  alias lla='ls -lA' 2>/dev/null || true
}

_first_profile_ls_alias_conflict_path() {
  local f targets=() new_file=""
  [[ -f "${HOME}/.zshrc" ]] && targets+=("${HOME}/.zshrc")
  [[ -f "${HOME}/.bashrc" ]] && targets+=("${HOME}/.bashrc")
  [[ -f "${HOME}/.bash_profile" ]] && targets+=("${HOME}/.bash_profile")
  if [[ ${#targets[@]} -eq 0 ]]; then
    case "${SHELL:-}" in
      */zsh) new_file="${HOME}/.zshrc" ;;
      */bash) new_file="${HOME}/.bashrc" ;;
      *) new_file="${HOME}/.zshrc" ;;
    esac
    targets+=("$new_file")
  fi
  for f in "${targets[@]}"; do
    [[ -f "$f" ]] || continue
    if grep -qE '^[[:space:]]*alias[[:space:]]+(ll|la|lla)=' "$f" 2>/dev/null &&
      ! grep -qF "${_LS_ALIAS_MARKER_BEGIN}" "$f" 2>/dev/null; then
      printf '%s' "$f"
      return 0
    fi
  done
  return 1
}

cmd_shell_aliases() {
  _say_step "==> 配置常用 ls 别名（ll / la / lla）"

  local _cf
  if _cf="$(_first_profile_ls_alias_conflict_path)"; then
    _say_step "校验: 在 $(basename "${_cf}") 等 profile 中发现已有 ll/la/lla 类 alias，且不含本脚本标记，可能与本次写入冲突。"
    _software_ops_confirm "仍要追加本脚本别名片段？" || {
      echo "已取消。"
      exit 0
    }
  fi

  _append_marked_block_to_profiles "${_LS_ALIAS_MARKER_BEGIN}" "$(_shell_aliases_block)" "${SKIP_UTILS_SHELL_ALIASES:-0}" "ls 别名"
  _apply_ls_aliases_in_session
  echo "  ll='ls -al'   la='ls -A'   lla='ls -lA'"
  echo "  当前 shell 若为 bash 子进程已尝试启用；长期请新开终端或 source 对应 profile。"
}

cmd_all() {
  local rest="${1:-}"
  cmd_gum "${rest}"
  echo ""
  cmd_shell_aliases
}

fetch_latest_gum_tag() {
  _fundeploy_github_download_curl -fsSL https://api.github.com/repos/charmbracelet/gum/releases/latest |
    sed -n 's/^  "tag_name": "\([^"]*\)".*/\1/p' | head -1
}

resolve_gum_tag() {
  if [[ -n "${GUM_TAG:-}" ]]; then
    printf '%s' "$GUM_TAG"
    return
  fi
  local t
  t="$(fetch_latest_gum_tag)"
  if [[ -z "$t" ]]; then
    echo "错误: 无法从 GitHub 解析 gum 最新版本（需网络与 curl）。" >&2
    exit 1
  fi
  printf '%s' "$t"
}

gum_tarball_name() {
  local tag="$1"
  local ver os arch
  ver="${tag#v}"
  os=$(uname -s)
  arch=$(uname -m)
  case "$os" in
    Darwin | Linux) ;;
    *)
      echo "错误: 不支持的操作系统: ${os}" >&2
      return 1
      ;;
  esac
  case "$arch" in
    x86_64 | amd64) arch=x86_64 ;;
    arm64 | aarch64) arch=arm64 ;;
    *)
      echo "错误: 不支持的架构: ${arch}" >&2
      return 1
      ;;
  esac
  printf 'gum_%s_%s_%s.tar.gz' "$ver" "$os" "$arch"
}

# ---------- 安装前校验（避免无脑下载/覆盖）----------
_validate_gum_home_writable() {
  local parent
  parent="$(dirname "${GUM_HOME}")"
  if [[ ! -d "$parent" ]]; then
    echo "错误: 父目录不存在: ${parent}" >&2
    return 1
  fi
  if [[ ! -w "$parent" ]]; then
    echo "错误: 无法在 ${parent} 下创建 ${GUM_HOME}（无写权限）。" >&2
    return 1
  fi
  mkdir -p "${GUM_HOME}" 2>/dev/null || {
    echo "错误: 无法创建 ${GUM_HOME}。" >&2
    return 1
  }
  if [[ ! -w "${GUM_HOME}" ]]; then
    echo "错误: 目录不可写: ${GUM_HOME}" >&2
    return 1
  fi
}

_prereq_commands_for_gum_release() {
  local miss=()
  command -v curl >/dev/null 2>&1 || miss+=("curl")
  command -v tar >/dev/null 2>&1 || miss+=("tar")
  command -v mktemp >/dev/null 2>&1 || miss+=("mktemp")
  if ! command -v install >/dev/null 2>&1 && ! command -v ginstall >/dev/null 2>&1; then
    miss+=("install")
  fi
  if [[ ${#miss[@]} -gt 0 ]]; then
    echo "错误: 安装 gum（Release 方式）前缺少命令: ${miss[*]}" >&2
    return 1
  fi
}

_precheck_gum_release_downloadable() {
  local tag asset url
  tag="$(resolve_gum_tag)" || return 1
  asset="$(gum_tarball_name "$tag")" || return 1
  url="https://github.com/charmbracelet/gum/releases/download/${tag}/${asset}"
  _say_step "==> 校验: 探测 Release 包是否可访问（${asset}）…"
  if ! _fundeploy_github_download_curl -fsSIL --connect-timeout 10 --max-time 30 -o /dev/null "$url"; then
    echo "错误: 无法访问下载地址（网络、代理或 GUM_TAG 是否匹配该资源）。" >&2
    echo "       ${url}" >&2
    return 1
  fi
  _say_step "==> 校验通过: Release URL 可访问。"
}

_validate_gum_brew_prereq() {
  command -v brew >/dev/null 2>&1 || {
    echo "错误: 已设 GUM_USE_BREW=1 但未找到 brew。" >&2
    return 1
  }
  if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "错误: 本脚本 GUM_USE_BREW=1 分支仅实现 macOS（Darwin）；当前 $(uname -s) 请去掉 GUM_USE_BREW 走 Release，或自行用系统包管理器安装后再复制到 ${GUM_HOME}/bin。" >&2
    return 1
  fi
}

install_gum_brew_to_opt() {
  command -v brew &>/dev/null || {
    echo "错误: 已设置 GUM_USE_BREW=1 但未找到 brew（macOS/Linux 请安装 Homebrew）。" >&2
    return 1
  }
  _say_step "==> Homebrew 安装 gum（备选方案；长期建议仍以 ~/opt/gum 为准）…"
  brew install gum
  local src
  src="$(brew --prefix 2>/dev/null)/bin/gum"
  if [[ ! -x "$src" ]]; then
    src="$(command -v gum)"
  fi
  [[ -x "$src" ]] || {
    echo "错误: brew 安装后未找到 gum 可执行文件。" >&2
    return 1
  }
  mkdir -p "${GUM_HOME}/bin" "${GUM_HOME}/etc" "${GUM_HOME}/data" "${GUM_HOME}/log"
  rm -f "${GUM_HOME}/bin/gum"
  cp -f "$src" "${GUM_HOME}/bin/gum"
  chmod 0755 "${GUM_HOME}/bin/gum"
  _say_step "已复制 brew gum -> ${GUM_HOME}/bin/gum"
}

install_gum_release() {
  local force="${1:-}"
  _prereq_commands_for_gum_release || exit 1
  local tag asset url tmp sub
  tag="$(resolve_gum_tag)"
  asset="$(gum_tarball_name "$tag")" || exit 1
  url="https://github.com/charmbracelet/gum/releases/download/${tag}/${asset}"
  _say_step "==> 下载 gum (${tag}) …"
  echo "    ${url}"
  tmp="$(mktemp -d)"
  trap 'rm -rf "${tmp}"' EXIT
  _fundeploy_github_download_curl -fsSL "$url" -o "${tmp}/gum.tgz"
  tar -xzf "${tmp}/gum.tgz" -C "${tmp}"
  shopt -s nullglob
  sub=( "${tmp}"/gum_* )
  shopt -u nullglob
  if [[ ${#sub[@]} -ne 1 || ! -x "${sub[0]}/gum" ]]; then
    echo "错误: 解压布局异常，未找到 gum 可执行文件。" >&2
    exit 1
  fi
  mkdir -p "${GUM_HOME}/bin" "${GUM_HOME}/etc" "${GUM_HOME}/data" "${GUM_HOME}/log"
  if [[ "$force" == "--force" ]]; then
    rm -f "${GUM_HOME}/bin/gum"
  fi
  install -m 0755 "${sub[0]}/gum" "${GUM_HOME}/bin/gum"
  trap - EXIT
  rm -rf "${tmp}"
  _say_step "已安装 ${GUM_HOME}/bin/gum"
}

cmd_gum() {
  local force=""
  [[ "${1:-}" == "--force" ]] && force="--force"

  _validate_gum_home_writable || exit 1
  mkdir -p "${GUM_HOME}/bin" "${GUM_HOME}/etc" "${GUM_HOME}/data" "${GUM_HOME}/log"

  # 已有目标目录下可用 gum：直接收尾，不重复安装
  if [[ -z "$force" && -x "${GUM_HOME}/bin/gum" ]] && "${GUM_HOME}/bin/gum" --version >/dev/null 2>&1; then
    _say_step "校验通过: ${GUM_HOME}/bin/gum 已存在且可运行（传 --force 重装）"
    "${GUM_HOME}/bin/gum" --version || true
    _ensure_gum_path_in_session
    _append_gum_path_to_profile_files
    _print_post_install_summary
    return 0
  fi

  # PATH 上别处已有可用 gum：无 --force 则不再往 GUM_HOME 装一份
  if [[ -z "$force" ]]; then
    local _gp
    _gp="$(command -v gum 2>/dev/null || true)"
    if [[ -n "${_gp}" && -x "${_gp}" && "${_gp}" != "${GUM_HOME}/bin/gum" ]]; then
      if "${_gp}" --version >/dev/null 2>&1; then
        _say_step "校验通过: PATH 中已有可用 gum → ${_gp}"
        echo "未指定 --force，跳过安装到 ${GUM_HOME}。若需统一到 ~/opt/gum 请执行: $0 gum --force" >&2
        exit 0
      fi
    fi
  fi

  _print_environment_and_plan

  if [[ "${GUM_USE_BREW:-0}" == "1" ]]; then
    _validate_gum_brew_prereq || exit 1
  else
    _prereq_commands_for_gum_release || exit 1
    _precheck_gum_release_downloadable || exit 1
  fi

  local _confirm_msg="确认在 ${GUM_HOME} 安装 gum？"
  if [[ "$force" == "--force" ]] && [[ -x "${GUM_HOME}/bin/gum" ]]; then
    _confirm_msg="将覆盖 ${GUM_HOME}/bin/gum 并重新安装，是否继续？"
  fi
  _software_ops_confirm "${_confirm_msg}" || {
    echo "已取消。"
    exit 0
  }

  if [[ "${GUM_USE_BREW:-0}" == "1" ]] && [[ "$(uname -s)" == "Darwin" ]]; then
    install_gum_brew_to_opt
  else
    install_gum_release "$force"
  fi

  _say_step "==> 验证安装…"
  "${GUM_HOME}/bin/gum" --version || {
    echo "错误: ${GUM_HOME}/bin/gum --version 失败。" >&2
    exit 1
  }

  _ensure_gum_path_in_session
  _append_gum_path_to_profile_files
  echo ""
  _print_post_install_summary
  echo ""
  echo "当前会话已调整 PATH；持久化见上方 profile 提示。"
}

main() {
  if [[ $# -eq 0 ]]; then
    if [[ "${NONINTERACTIVE:-0}" == "1" ]] || [[ ! -t 0 ]]; then
      cmd_gum ""
      return 0
    fi
    _ensure_gum_for_interactive_menu || exit 1
    _interactive_main
    return 0
  fi

  case "${1:-}" in
    gum)
      shift
      cmd_gum "${1:-}"
      ;;
    aliases)
      shift
      cmd_shell_aliases
      ;;
    help | -h | --help)
      usage
      ;;
    all)
      shift
      cmd_all "${1:-}"
      ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
}

main "$@"
