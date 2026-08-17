#!/usr/bin/env bash
# 一键安装 / 更新 / 卸载 nltdeploy 到 ~/.local/nltdeploy（可通过 NLTDEPLOY_ROOT 覆盖）。
# 用法见下方 usage；无参数且为交互式终端时，会先选择「安装」「更新」或「卸载」，不直接写盘。
# 管道非 TTY 时必须显式传入子命令，例如: curl … | bash -s -- install
set -euo pipefail

NLTDEPLOY_ROOT="${NLTDEPLOY_ROOT:-${HOME}/.local/nltdeploy}"
NLTDEPLOY_WRAPPER_ROOT="${NLTDEPLOY_WRAPPER_ROOT:-${NLTDEPLOY_ROOT}}"
NLTDEPLOY_GITHUB_REPO="${NLTDEPLOY_GITHUB_REPO:-https://github.com/farfarfun/nltdeploy.git}"
NLTDEPLOY_GITEE_REPO="${NLTDEPLOY_GITEE_REPO:-https://gitee.com/farfarfun/nltdeploy.git}"
NLTDEPLOY_SRC_DIR="${NLTDEPLOY_SRC_DIR:-${NLTDEPLOY_ROOT}/src/nltdeploy}"
NLTDEPLOY_SOURCE_FILE="${NLTDEPLOY_ROOT}/etc/nltdeploy/source"
_NLTDEPLOY_SOURCE_REQUESTED=""
_NLTDEPLOY_SOURCE="auto"

die() { echo "错误: $*" >&2; exit 1; }

_parse_source_args() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --source)
        [[ $# -ge 2 ]] || die "--source 缺少取值（github|gitee）"
        _NLTDEPLOY_SOURCE_REQUESTED="$2"
        shift 2
        ;;
      --source=*) _NLTDEPLOY_SOURCE_REQUESTED="${1#*=}"; shift ;;
      *) die "未知参数: $1（支持 --source github|gitee）" ;;
    esac
  done
  case "${_NLTDEPLOY_SOURCE_REQUESTED}" in
    "" | github | gitee) ;;
    *) die "未知 --source: ${_NLTDEPLOY_SOURCE_REQUESTED}（支持 github|gitee）" ;;
  esac
}

# 首次安装时公共库尚不存在，因此安装根目录必须在本文件内守卫。
_guard_nltdeploy_root() {
  local create="${1:-0}" root hp depth
  [[ "${NLTDEPLOY_ROOT}" == /* ]] || die "NLTDEPLOY_ROOT 必须是绝对路径: ${NLTDEPLOY_ROOT}"
  [[ "${create}" == "1" ]] && mkdir -p "${NLTDEPLOY_ROOT}"
  [[ -d "${NLTDEPLOY_ROOT}" ]] || die "安装目录不存在: ${NLTDEPLOY_ROOT}"

  root="$(cd "${NLTDEPLOY_ROOT}" && pwd -P)"
  hp="$(cd "${HOME}" && pwd -P)"
  case "${root}" in
    / | "${hp}" | /bin | /boot | /dev | /etc | /home | /lib* | /opt | /proc | /root | /run | /sbin | /srv | /sys | /tmp | /usr | /var | /private | /private/etc | /private/tmp | /private/var | /Users | /Applications | /System | /Library)
      die "拒绝使用系统目录或 \$HOME 作为 NLTDEPLOY_ROOT: ${root}" ;;
  esac

  depth="${root//[!\/]/}"
  (( ${#depth} >= 2 )) || die "NLTDEPLOY_ROOT 路径层级过浅: ${root}"
}

usage() {
  cat <<'EOF'
用法: install.sh [install|update] [--source github|gitee]
      install.sh uninstall

  install / update   同步 libexec 与 bin；若 scripts 所在目录为 git 仓库则先 git pull --ff-only（可跳过）
  uninstall / remove 删除 NLTDEPLOY_ROOT，并从 shell 配置中移除本安装器写入的 PATH 片段

无参数:
  交互式终端下会先询问「安装」「更新」或「卸载」（有 gum 则用 gum）。
  非交互（管道、无 TTY）或无参数且 NONINTERACTIVE=1 时必须写明子命令，例如:
    curl -LsSf https://raw.githubusercontent.com/farfarfun/nltdeploy/HEAD/install.sh | bash -s -- install

远端安装:
  GitHub:
    curl -LsSf https://raw.githubusercontent.com/farfarfun/nltdeploy/HEAD/install.sh | bash -s -- install --source github
  Gitee:
    curl -LsSf https://gitee.com/farfarfun/nltdeploy/raw/master/install.sh | bash -s -- install --source gitee
  更新:
    nltdeploy upgrade

环境变量:
  NLTDEPLOY_ROOT              安装根目录（默认 ~/.local/nltdeploy）
  NLTDEPLOY_WRAPPER_ROOT      包装命令中的运行根目录（默认同 NLTDEPLOY_ROOT，发行包构建专用）
  NLTDEPLOY_PACKAGE_MANAGER   写入包装命令的包管理器标记：apt 或 brew（发行包构建专用）
  NLTDEPLOY_SKIP_GIT_PULL     设为 1 时不执行 git pull（仍同步文件）
  NLTDEPLOY_SKIP_PROFILE_HINT 设为 1 时不写入 PATH、不打印 PATH 说明（适合 CI）
  NLTDEPLOY_AUTO_EXEC_ZSH_AFTER_INSTALL  默认 1：安装结束且为交互 TTY、且检测到由 zsh 启动本脚本时，执行 exec zsh -l 以加载 ~/.zshrc（无法改父进程环境时的折中）。设为 0 则只提示手动 source。
  NLTDEPLOY_UNINSTALL_YES     设为 1 时非 TTY 也可执行 uninstall（确认删除）
  NLTDEPLOY_GITHUB_REPO / NLTDEPLOY_GITEE_REPO / NLTDEPLOY_SRC_DIR  见 README
  NLTDEPLOY_GIT_CLONE_REF   管道安装时 git clone 的分支或 tag（可选）。raw 用 …/master/… 而仓库默认分支是 main 时，可设为 master 与脚本版本一致；不设则克隆远程默认分支。
EOF
}

# 若从仓库根执行（install.sh 为普通文件且旁侧有 scripts/），返回该 scripts 绝对路径；否则返回非 0。
_resolve_scripts_from_install_sh() {
  local _src="${BASH_SOURCE[0]-}" _dir
  if [[ -n "$_src" && "$_src" != "-" && -f "$_src" ]]; then
    _dir="$(cd "$(dirname "$_src")" && pwd)" || return 1
    if [[ -d "${_dir}/scripts" ]]; then
      echo "${_dir}/scripts"
      return 0
    fi
  fi
  return 1
}

# 浅克隆；若设置了 NLTDEPLOY_GIT_CLONE_REF 则固定该分支/tag（与 raw 脚本 URL 中的 ref 对齐）。
_nlt_git_clone_shallow() {
  local url="$1" dest="$2"
  if [[ -n "${NLTDEPLOY_GIT_CLONE_REF:-}" ]]; then
    git clone --depth 1 --branch "${NLTDEPLOY_GIT_CLONE_REF}" "${url}" "${dest}"
  else
    git clone --depth 1 "${url}" "${dest}"
  fi
}

_repo_source() {
  local url
  url="$(git -C "$1" config --get remote.origin.url 2>/dev/null)" || return 1
  case "${url}" in
    *github.com/* | git@github.com:*) echo github ;;
    *gitee.com/* | git@gitee.com:*) echo gitee ;;
    *) return 1 ;;
  esac
}

_resolve_install_source() {
  local saved=""
  if [[ -n "${_NLTDEPLOY_SOURCE_REQUESTED}" ]]; then
    _NLTDEPLOY_SOURCE="${_NLTDEPLOY_SOURCE_REQUESTED}"
  elif [[ -f "${NLTDEPLOY_SOURCE_FILE}" ]]; then
    saved="$(tr -d '[:space:]' <"${NLTDEPLOY_SOURCE_FILE}")"
    case "${saved}" in
      github | gitee) _NLTDEPLOY_SOURCE="${saved}" ;;
    esac
  elif [[ -d "${NLTDEPLOY_SRC_DIR}/.git" ]]; then
    _NLTDEPLOY_SOURCE="$(_repo_source "${NLTDEPLOY_SRC_DIR}" || echo auto)"
  fi
}

_set_managed_clone_source() {
  local repo="$1" source="$2" url actual=""
  actual="$(_repo_source "${repo}" || true)"
  [[ "${actual}" == "${source}" ]] && return 0
  case "${source}" in
    github) url="${NLTDEPLOY_GITHUB_REPO}" ;;
    gitee) url="${NLTDEPLOY_GITEE_REPO}" ;;
    *) return 0 ;;
  esac
  echo "正在将更新源切换为 ${source}: ${url}" >&2
  if git -C "${repo}" remote get-url origin >/dev/null 2>&1; then
    git -C "${repo}" remote set-url origin "${url}"
  else
    git -C "${repo}" remote add origin "${url}"
  fi
}

_persist_install_source() {
  [[ -z "${NLTDEPLOY_PACKAGE_MANAGER:-}" ]] || return 0
  case "${_NLTDEPLOY_SOURCE}" in
    github | gitee)
      mkdir -p "$(dirname "${NLTDEPLOY_SOURCE_FILE}")"
      printf '%s\n' "${_NLTDEPLOY_SOURCE}" >"${NLTDEPLOY_SOURCE_FILE}"
      ;;
  esac
}

# 将仓库克隆到 NLTDEPLOY_SRC_DIR（若尚不存在）。
_ensure_clone_for_scripts() {
  command -v git >/dev/null 2>&1 || die "通过管道安装需要 git。请安装 git 或在克隆后的仓库根目录执行 ./install.sh"
  mkdir -p "${NLTDEPLOY_ROOT}" "${NLTDEPLOY_ROOT}/src"
  local repo="${NLTDEPLOY_SRC_DIR}"
  if [[ -d "${repo}/.git" ]]; then
    [[ "${_NLTDEPLOY_SOURCE}" == "auto" ]] && _NLTDEPLOY_SOURCE="$(_repo_source "${repo}" || echo auto)"
    _set_managed_clone_source "${repo}" "${_NLTDEPLOY_SOURCE}"
  elif [[ -e "${repo}" ]]; then
    die "路径已存在但不是 git 仓库，请删除或移走后重试: ${repo}"
  else
    case "${_NLTDEPLOY_SOURCE}" in
      github)
        echo "正在从 GitHub 克隆 farfarfun/nltdeploy …" >&2
        _nlt_git_clone_shallow "${NLTDEPLOY_GITHUB_REPO}" "${repo}" || die "GitHub 克隆失败，请检查网络与代理"
        ;;
      gitee)
        echo "正在从 Gitee 克隆 farfarfun/nltdeploy …" >&2
        _nlt_git_clone_shallow "${NLTDEPLOY_GITEE_REPO}" "${repo}" || die "Gitee 克隆失败，请检查网络与代理"
        ;;
      *)
        echo "正在从 GitHub 克隆 farfarfun/nltdeploy …" >&2
        if _nlt_git_clone_shallow "${NLTDEPLOY_GITHUB_REPO}" "${repo}"; then
          _NLTDEPLOY_SOURCE="github"
        else
          echo "GitHub 不可用，正在从 Gitee 克隆 farfarfun/nltdeploy …" >&2
          _nlt_git_clone_shallow "${NLTDEPLOY_GITEE_REPO}" "${repo}" || die "GitHub 与 Gitee 克隆均失败，请检查网络与代理"
          _NLTDEPLOY_SOURCE="gitee"
        fi
        ;;
    esac
  fi
  [[ -d "${repo}/scripts" ]] || die "克隆完成但未找到 scripts 目录: ${repo}"
}

# scripts 的父目录若为 git 仓库，则拉取最新（可跳过）。
_sync_git_upstream_for_scripts() {
  local scripts_dir="$1"
  local root current before after source=""
  root="$(cd "$(dirname "$scripts_dir")" && pwd)"
  [[ -d "${root}/.git" ]] || return 0
  source="$(_repo_source "${root}" || true)"
  if [[ "${_NLTDEPLOY_SOURCE}" == "auto" ]]; then
    _NLTDEPLOY_SOURCE="${source:-auto}"
  elif [[ -n "${source}" && "${source}" != "${_NLTDEPLOY_SOURCE}" ]]; then
    die "当前源码仓库 origin 与 --source ${_NLTDEPLOY_SOURCE} 不一致: ${root}"
  fi
  [[ "${NLTDEPLOY_SKIP_GIT_PULL:-}" == "1" ]] && return 0
  command -v git >/dev/null 2>&1 || die "发现 git 仓库但未安装 git，无法更新: ${root}"
  echo "正在拉取最新脚本: ${root}" >&2
  before="$(git -C "${root}" rev-parse HEAD)"
  git -C "${root}" pull --ff-only || die "git pull 失败: ${root}"
  after="$(git -C "${root}" rev-parse HEAD)"
  current="${BASH_SOURCE[0]:-}"
  if [[ -f "${root}/install.sh" ]] && \
    [[ "${before}" != "${after}" || ! -f "${current}" || ! "${current}" -ef "${root}/install.sh" ]]; then
    echo "安装器已更新，正在使用新版本继续…" >&2
    case "${_NLTDEPLOY_SOURCE}" in
      github | gitee)
        exec env NLTDEPLOY_SKIP_GIT_PULL=1 bash "${root}/install.sh" "${_CMD}" --source "${_NLTDEPLOY_SOURCE}"
        ;;
      *) exec env NLTDEPLOY_SKIP_GIT_PULL=1 bash "${root}/install.sh" "${_CMD}" ;;
    esac
  fi
}

# 规范路径，便于去重与写入 rc
_nlt_canonical_bin_dir() {
  (cd "${NLTDEPLOY_ROOT}/bin" && pwd -P)
}

_nlt_rc_has_managed_block() {
  local f="$1"
  [[ -f "$f" ]] && grep -Fq -e '--- nltdeploy PATH' "$f"
}

_nlt_rc_path_mentions_bin() {
  local f="$1" bin="$2"
  [[ -f "$f" ]] && grep -qF "${bin}" "$f"
}

_nlt_append_nlt_path_block() {
  local rc="$1"
  local bin="$2"
  local line marker_top marker_bot
  line="export PATH=\"${bin}:\${PATH}\""
  marker_top='# --- nltdeploy PATH (github.com/farfarfun/nltdeploy install.sh) ---'
  marker_bot='# --- end nltdeploy PATH ---'

  if _nlt_rc_has_managed_block "$rc"; then
    echo "PATH 已配置（存在 nltdeploy 标记块）: ${rc}" >&2
    return 0
  fi
  if _nlt_rc_path_mentions_bin "$rc" "$bin"; then
    echo "跳过写入 ${rc}：文件中已出现 ${bin}（请确认 PATH 已包含 nltdeploy bin）" >&2
    return 0
  fi

  {
    echo ""
    echo "${marker_top}"
    echo "${line}"
    echo "${marker_bot}"
  } >>"$rc"
  echo "已追加 PATH 到: ${rc}" >&2
}

_nlt_collect_profile_targets() {
  local -a out=()
  local p t

  add_unique() {
    p="$1"
    [[ -z "$p" ]] && return
    for t in "${out[@]:-}"; do
      [[ "$t" == "$p" ]] && return
    done
    out+=("$p")
  }

  if [[ -f "${HOME}/.zshrc" ]] || [[ "${SHELL:-}" == *zsh* ]]; then
    add_unique "${HOME}/.zshrc"
  fi
  if [[ -f "${HOME}/.bashrc" ]] || [[ "${SHELL:-}" == *bash* ]]; then
    add_unique "${HOME}/.bashrc"
  fi
  if [[ "${SHELL:-}" == *bash* ]] && [[ ! -f "${HOME}/.bashrc" ]] && [[ -f "${HOME}/.bash_profile" ]]; then
    add_unique "${HOME}/.bash_profile"
  fi
  if [[ ${#out[@]} -eq 0 ]]; then
    add_unique "${HOME}/.zshrc"
  fi

  for t in "${out[@]}"; do
    printf '%s\n' "$t"
  done
}

# 当前进程的父进程命令名是否像 zsh（用于安装后是否 exec zsh -l）
_nlt_parent_shell_looks_like_zsh() {
  local c
  c="$(ps -p "$PPID" -o comm= 2>/dev/null || true)"
  c="${c##-}"
  c="${c##*/}"
  [[ "$c" == *zsh* ]]
}

# 安装后立即生效：当前 install 进程 PATH + 可选 exec 登录 zsh 以加载 ~/.zshrc
_nlt_post_install_refresh_shell() {
  [[ "${NLTDEPLOY_SKIP_PROFILE_HINT:-}" == "1" ]] && return 0
  local bin
  bin="$(_nlt_canonical_bin_dir)" || return 0
  case ":${PATH}:" in
    *":${bin}:"*) ;;
    *) export PATH="${bin}:${PATH}" ;;
  esac

  if
    [[ "${NLTDEPLOY_AUTO_EXEC_ZSH_AFTER_INSTALL:-1}" == "1" ]] &&
      [[ -t 1 ]] &&
      _nlt_parent_shell_looks_like_zsh &&
      [[ -f "${HOME}/.zshrc" ]] &&
      command -v zsh >/dev/null 2>&1
  then
    echo "检测到由 zsh 启动安装：将 exec zsh -l 以加载 ~/.zshrc（nltdeploy 立即可用；exit 回到上一层 shell）。" >&2
    exec zsh -l
  fi

  echo "已将 ${bin} 加入当前 shell 的 PATH（本进程内可直接使用 nlt-*）。"
  echo "新开终端或手动执行: source ~/.zshrc（zsh）或 source ~/.bashrc（bash）"
}

_nlt_install_path_to_profiles() {
  local bin
  bin="$(_nlt_canonical_bin_dir)" || die "无法解析 ${NLTDEPLOY_ROOT}/bin 为绝对路径"
  local rc
  while IFS= read -r rc; do
    [[ -n "$rc" ]] || continue
    touch "$rc" 2>/dev/null || {
      echo "无法写入 ${rc}，跳过。" >&2
      continue
    }
    _nlt_append_nlt_path_block "$rc" "$bin"
  done < <(_nlt_collect_profile_targets)
}

_remove_managed_path_block_from_file() {
  local f="$1"
  [[ -f "$f" ]] || return 0
  grep -Fq -e '--- nltdeploy PATH' "$f" || return 0
  local start end tmp
  # `|| true`：标记缺失时 grep 返回 1，在 set -e + pipefail 下会直接终止整个
  # install.sh —— 于是用户手工编辑过 rc 文件后，uninstall 什么都没删就退出 1，
  # 下面那行本该兜底的守卫永远走不到。
  start="$(grep -nF '# --- nltdeploy PATH (github.com/farfarfun/nltdeploy install.sh) ---' "$f" | head -1 | cut -d: -f1 || true)"
  end="$(grep -nF '# --- end nltdeploy PATH ---' "$f" | head -1 | cut -d: -f1 || true)"
  if [[ -z "$start" || -z "$end" || "$end" -lt "$start" ]]; then
    echo "警告: ${f} 中的 nltdeploy PATH 标记不完整，跳过自动清理（请手动检查）。" >&2
    return 0
  fi
  tmp="$(mktemp)"
  awk -v s="$start" -v e="$end" 'NR < s || NR > e' "$f" >"$tmp" && mv "$tmp" "$f"
  echo "已从 ${f} 移除 nltdeploy PATH 片段" >&2
}

# 输出所选子命令；用户选择「退出」或取消时输出 quit。
# 注意：本函数总是在 `$( )` 命令替换里被调用，因此 **不能用 exit 表达退出** ——
# exit 只会终止那层子 shell，调用方拿到空串后落进 `case *)` 报「未知命令: 」并以 1 退出。
_pick_cmd_interactive() {
  if command -v gum >/dev/null 2>&1; then
    local p
    p="$(gum choose --header "nltdeploy" "安装" "更新" "卸载" "退出")" || { echo "quit"; return 0; }
    case "$p" in
      安装) echo "install" ;;
      更新) echo "update" ;;
      卸载) echo "uninstall" ;;
      *) echo "quit" ;;
    esac
  else
    echo "请选择:" >&2
    echo "  1) 安装   2) 更新   3) 卸载   4) 退出" >&2
    read -r -p "输入 1-4: " sel || { echo "quit"; return 0; }
    case "$sel" in
      1) echo "install" ;;
      2) echo "update" ;;
      3) echo "uninstall" ;;
      *) echo "quit" ;;
    esac
  fi
}

_emit_wrapper() {
  local name="$1"
  shift
  local rel="$1"
  shift
  local bin_path="${NLTDEPLOY_ROOT}/bin/${name}"
  {
    printf '%s\n' '#!/usr/bin/env bash'
    printf '%s\n' 'set -euo pipefail'
    # 必须 export：wrapper 用 exec 启动真正的入口脚本，只有环境变量能传递过去。
    # 漏掉 export 时 deb(/usr) 与 brew(opt_prefix) 安装下的入口会回落到
    # ~/.local/nltdeploy，导致 _resolve_local_install_sh / NLTDEPLOY_SRC_DIR 找错位置。
    printf 'export NLTDEPLOY_ROOT=${NLTDEPLOY_ROOT:-%q}\n' "${NLTDEPLOY_WRAPPER_ROOT}"
    if [[ -n "${NLTDEPLOY_PACKAGE_MANAGER:-}" ]]; then
      printf 'export NLTDEPLOY_PACKAGE_MANAGER=%q\n' "${NLTDEPLOY_PACKAGE_MANAGER}"
    fi
    if [[ $# -gt 0 ]]; then
      printf 'exec "${NLTDEPLOY_ROOT}/libexec/nltdeploy/%s"' "$rel"
      local a
      for a in "$@"; do
        printf ' %q' "$a"
      done
      printf ' "$@"\n'
    else
      printf 'exec "${NLTDEPLOY_ROOT}/libexec/nltdeploy/%s" "$@"\n' "$rel"
    fi
  } > "${bin_path}"
  chmod 0755 "${bin_path}"
}

# 从候选路径中选第一个存在的文件复制到 dest 并 chmod 0755（兼容 _lib→lib、扁平 scripts 与 tools/services 分层）。
_nlt_cp_first() {
  local dest="$1"
  shift
  local f
  for f in "$@"; do
    if [[ -f "$f" ]]; then
      cp -f "$f" "$dest"
      chmod 0755 "$dest"
      return 0
    fi
  done
  die "找不到源文件，已尝试: $*"
}

do_install_or_update() {
  local SCRIPTS LIBEXEC
  _guard_nltdeploy_root 1
  _resolve_install_source
  SCRIPTS=""
  if SCRIPTS="$(_resolve_scripts_from_install_sh)"; then
    :
  else
    _ensure_clone_for_scripts
    SCRIPTS="${NLTDEPLOY_SRC_DIR}/scripts"
  fi
  [[ -d "$SCRIPTS" ]] || die "找不到 scripts 目录: ${SCRIPTS}"

  _sync_git_upstream_for_scripts "$SCRIPTS"
  _persist_install_source

  LIBEXEC="${NLTDEPLOY_ROOT}/libexec/nltdeploy"
  mkdir -p "${NLTDEPLOY_ROOT}/bin" "${LIBEXEC}" \
    "${NLTDEPLOY_ROOT}/share/nltdeploy" "${NLTDEPLOY_ROOT}/etc/nltdeploy"
  mkdir -p "${LIBEXEC}/pip-sources" "${LIBEXEC}/python-env" "${LIBEXEC}/port-kill" \
    "${LIBEXEC}/ai-cli/claude" "${LIBEXEC}/ai-cli/codex" "${LIBEXEC}/ai-cli/cursor" \
    "${LIBEXEC}/dev/go" "${LIBEXEC}/dev/rust" "${LIBEXEC}/dev/nodejs" "${LIBEXEC}/dev/pnpm" "${LIBEXEC}/dev/uv" \
    "${LIBEXEC}/brew" "${LIBEXEC}/download" "${LIBEXEC}/cockpit-tools" "${LIBEXEC}/skills-sync" \
    "${LIBEXEC}/airflow" "${LIBEXEC}/celery" "${LIBEXEC}/utils" "${LIBEXEC}/github-net" \
    "${LIBEXEC}/paperclip" "${LIBEXEC}/code-server" "${LIBEXEC}/new-api" "${LIBEXEC}/sub2api" \
    "${LIBEXEC}/open-pencil" \
    "${LIBEXEC}/services" \
    "${LIBEXEC}/tools" \
    "${LIBEXEC}/lib"

  _nlt_cp_first "${LIBEXEC}/lib/nlt-ui.sh" \
    "${SCRIPTS}/lib/nlt-ui.sh"

  _nlt_cp_first "${LIBEXEC}/lib/nlt-install.sh" \
    "${SCRIPTS}/lib/nlt-install.sh"

  _nlt_cp_first "${LIBEXEC}/lib/nlt-github-download.sh" \
    "${SCRIPTS}/lib/nlt-github-download.sh"

  _nlt_cp_first "${LIBEXEC}/lib/nlt-progress.sh" \
    "${SCRIPTS}/lib/nlt-progress.sh"

  _nlt_cp_first "${LIBEXEC}/lib/nlt-common.sh" \
    "${SCRIPTS}/lib/nlt-common.sh" \
    "${SCRIPTS}/_lib/nlt-common.sh"

  _nlt_cp_first "${LIBEXEC}/dev/setup.sh" \
    "${SCRIPTS}/dev/setup.sh"

  _nlt_cp_first "${LIBEXEC}/dev/go/setup.sh" \
    "${SCRIPTS}/dev/go/setup.sh"

  _nlt_cp_first "${LIBEXEC}/dev/rust/setup.sh" \
    "${SCRIPTS}/dev/rust/setup.sh"

  _nlt_cp_first "${LIBEXEC}/dev/nodejs/setup.sh" \
    "${SCRIPTS}/dev/nodejs/setup.sh"

  _nlt_cp_first "${LIBEXEC}/dev/pnpm/setup.sh" \
    "${SCRIPTS}/dev/pnpm/setup.sh"

  _nlt_cp_first "${LIBEXEC}/dev/uv/setup.sh" \
    "${SCRIPTS}/dev/uv/setup.sh"

  _nlt_cp_first "${LIBEXEC}/ai-cli/common.sh" \
    "${SCRIPTS}/ai-cli/common.sh"

  _nlt_cp_first "${LIBEXEC}/ai-cli/setup.sh" \
    "${SCRIPTS}/ai-cli/setup.sh"

  _nlt_cp_first "${LIBEXEC}/ai-cli/claude/setup.sh" \
    "${SCRIPTS}/ai-cli/claude/setup.sh"

  _nlt_cp_first "${LIBEXEC}/ai-cli/codex/setup.sh" \
    "${SCRIPTS}/ai-cli/codex/setup.sh"

  _nlt_cp_first "${LIBEXEC}/ai-cli/cursor/setup.sh" \
    "${SCRIPTS}/ai-cli/cursor/setup.sh"

  _nlt_cp_first "${LIBEXEC}/pip-sources/setup.sh" \
    "${SCRIPTS}/tools/pip-sources/setup.sh" \
    "${SCRIPTS}/pip-sources/setup.sh"

  _nlt_cp_first "${LIBEXEC}/python-env/setup.sh" \
    "${SCRIPTS}/tools/python-env/setup.sh" \
    "${SCRIPTS}/python-env/setup.sh"

  _nlt_cp_first "${LIBEXEC}/airflow/setup.sh" \
    "${SCRIPTS}/services/airflow/setup.sh" \
    "${SCRIPTS}/airflow/setup.sh"

  _nlt_cp_first "${LIBEXEC}/celery/setup.sh" \
    "${SCRIPTS}/services/celery/setup.sh" \
    "${SCRIPTS}/celery/setup.sh" \
    "${SCRIPTS}/celery/celery-setup.sh"

  _nlt_cp_first "${LIBEXEC}/utils/setup.sh" \
    "${SCRIPTS}/tools/utils/setup.sh" \
    "${SCRIPTS}/utils/setup.sh" \
    "${SCRIPTS}/utils/utils-setup.sh"

  _nlt_cp_first "${LIBEXEC}/github-net/setup.sh" \
    "${SCRIPTS}/tools/github-net/setup.sh" \
    "${SCRIPTS}/github-net/setup.sh"

  _nlt_cp_first "${LIBEXEC}/port-kill/setup.sh" \
    "${SCRIPTS}/tools/port-kill/setup.sh"

  _nlt_cp_first "${LIBEXEC}/brew/setup.sh" \
    "${SCRIPTS}/tools/brew/setup.sh"

  _nlt_cp_first "${LIBEXEC}/download/setup.sh" \
    "${SCRIPTS}/tools/download/setup.sh"

  _nlt_cp_first "${LIBEXEC}/download/selftest.sh" \
    "${SCRIPTS}/tools/download/selftest.sh"

  _nlt_cp_first "${LIBEXEC}/cockpit-tools/setup.sh" \
    "${SCRIPTS}/tools/cockpit-tools/setup.sh"

  _nlt_cp_first "${LIBEXEC}/skills-sync/setup.sh" \
    "${SCRIPTS}/tools/skills-sync/setup.sh"

  _nlt_cp_first "${LIBEXEC}/paperclip/setup.sh" \
    "${SCRIPTS}/services/paperclip/setup.sh" \
    "${SCRIPTS}/paperclip/setup.sh" \
    "${SCRIPTS}/paperclip/paperclip-setup.sh"

  _nlt_cp_first "${LIBEXEC}/code-server/setup.sh" \
    "${SCRIPTS}/services/code-server/setup.sh" \
    "${SCRIPTS}/code-server/setup.sh" \
    "${SCRIPTS}/code-server/code-server-setup.sh"

  _nlt_cp_first "${LIBEXEC}/code-server/setup-manual.sh" \
    "${SCRIPTS}/services/code-server/setup-manual.sh"

  _nlt_cp_first "${LIBEXEC}/code-server/setup-offical.sh" \
    "${SCRIPTS}/services/code-server/setup-offical.sh"

  _nlt_cp_first "${LIBEXEC}/new-api/setup.sh" \
    "${SCRIPTS}/services/new-api/setup.sh" \
    "${SCRIPTS}/new-api/setup.sh" \
    "${SCRIPTS}/new-api/new-api-setup.sh"

  _nlt_cp_first "${LIBEXEC}/sub2api/setup.sh" \
    "${SCRIPTS}/services/sub2api/setup.sh" \
    "${SCRIPTS}/sub2api/setup.sh"

  _nlt_cp_first "${LIBEXEC}/sub2api/setup-manual.sh" \
    "${SCRIPTS}/services/sub2api/setup-manual.sh"

  _nlt_cp_first "${LIBEXEC}/sub2api/setup-offical.sh" \
    "${SCRIPTS}/services/sub2api/setup-offical.sh"

  _nlt_cp_first "${LIBEXEC}/open-pencil/setup.sh" \
    "${SCRIPTS}/services/open-pencil/setup.sh" \
    "${SCRIPTS}/open-pencil/setup.sh"

  _nlt_cp_first "${LIBEXEC}/services/nlt-services.sh" \
    "${SCRIPTS}/services/nlt-services.sh" \
    "${SCRIPTS}/services/services.sh" \
    "${SCRIPTS}/10-services/services.sh"

  # 顶层与工具统一入口（WAR-402）。
  _nlt_cp_first "${LIBEXEC}/tools/nlt-tools.sh" \
    "${SCRIPTS}/tools/nlt-tools.sh"

  _nlt_cp_first "${LIBEXEC}/nltdeploy.sh" \
    "${SCRIPTS}/nltdeploy.sh"

  # 供 nltdeploy uninstall / --source local 离线复用（无需公网 raw）。
  _nlt_cp_first "${LIBEXEC}/nltdeploy-install.sh" \
    "${SCRIPTS}/../install.sh"

  _emit_wrapper nltdeploy nltdeploy.sh

  rm -rf "${LIBEXEC}/build"
  # 更新旧版本时只清理本项目曾安装过的包装器，保留用户的其它命令。
  rm -f \
    "${NLTDEPLOY_ROOT}/bin/nlt" \
    "${NLTDEPLOY_ROOT}/bin/nlt-build" \
    "${NLTDEPLOY_ROOT}/bin/nlt-dev" \
    "${NLTDEPLOY_ROOT}/bin/nlt-ai-cli" \
    "${NLTDEPLOY_ROOT}/bin/nlt-pip-sources" \
    "${NLTDEPLOY_ROOT}/bin/nlt-python-env" \
    "${NLTDEPLOY_ROOT}/bin/nlt-utils" \
    "${NLTDEPLOY_ROOT}/bin/nlt-github-net" \
    "${NLTDEPLOY_ROOT}/bin/nlt-port-kill" \
    "${NLTDEPLOY_ROOT}/bin/nlt-download" \
    "${NLTDEPLOY_ROOT}/bin/nlt-cockpit-tools" \
    "${NLTDEPLOY_ROOT}/bin/nlt-services" \
    "${NLTDEPLOY_ROOT}/bin/nlt-tools" \
    "${NLTDEPLOY_ROOT}/bin/nlt-airflow" \
    "${NLTDEPLOY_ROOT}/bin/nlt-celery" \
    "${NLTDEPLOY_ROOT}/bin/nlt-paperclip" \
    "${NLTDEPLOY_ROOT}/bin/nlt-code-server" \
    "${NLTDEPLOY_ROOT}/bin/nlt-new-api" \
    "${NLTDEPLOY_ROOT}/bin/nlt-sub2api" \
    "${NLTDEPLOY_ROOT}/bin/nlt-open-pencil" \
    "${NLTDEPLOY_ROOT}/bin/nlt-airflow-install" \
    "${NLTDEPLOY_ROOT}/bin/nlt-celery-install" \
    "${NLTDEPLOY_ROOT}/bin/nlt-celery-update" \
    "${NLTDEPLOY_ROOT}/bin/nlt-paperclip-install" \
    "${NLTDEPLOY_ROOT}/bin/nlt-code-server-install" \
    "${NLTDEPLOY_ROOT}/bin/nlt-new-api-install" \
    "${NLTDEPLOY_ROOT}/bin/nlt-sub2api-install"
  if [[ -z "${NLTDEPLOY_PACKAGE_MANAGER:-}" ]]; then
    rm -f "${HOME}/opt/nlt/bin/nlt-port-kill"
  fi

  if [[ "${NLTDEPLOY_SKIP_PROFILE_HINT:-}" != "1" ]]; then
    echo ""
    echo "已安装到: ${NLTDEPLOY_ROOT}"
    echo "统一入口: ${NLTDEPLOY_ROOT}/bin/nltdeploy"
    echo "PATH 生效后可运行: nltdeploy"
    _nlt_install_path_to_profiles
    echo ""
    echo "若不想自动写入 shell 配置，可设置 NLTDEPLOY_SKIP_PROFILE_HINT=1"
    echo "若不想安装结束后自动 exec 登录 zsh，可设置 NLTDEPLOY_AUTO_EXEC_ZSH_AFTER_INSTALL=0"
    _nlt_post_install_refresh_shell
  fi
}

do_uninstall() {
  local ap rc
  if [[ ! -d "${NLTDEPLOY_ROOT}" ]]; then
    echo "未找到安装目录，跳过: ${NLTDEPLOY_ROOT}" >&2
    exit 0
  fi
  _guard_nltdeploy_root

  if [[ "${NLTDEPLOY_UNINSTALL_YES:-}" != "1" ]]; then
    if [[ -t 0 ]]; then
      if command -v gum >/dev/null 2>&1; then
        gum confirm "将删除整个 ${NLTDEPLOY_ROOT}（含 bin、libexec、克隆仓库），并从 shell 配置中移除 nltdeploy PATH 片段。确认？" || exit 0
      else
        read -r -p "确认删除 ${NLTDEPLOY_ROOT}？[y/N] " ap
        [[ "$ap" == "y" || "$ap" == "Y" ]] || exit 0
      fi
    else
      die "非交互卸载请设置 NLTDEPLOY_UNINSTALL_YES=1"
    fi
  fi

  while IFS= read -r rc; do
    [[ -n "$rc" ]] || continue
    _remove_managed_path_block_from_file "$rc"
  done < <(_nlt_collect_profile_targets)

  echo "正在删除: ${NLTDEPLOY_ROOT}" >&2
  rm -rf "${NLTDEPLOY_ROOT}"
  echo "已卸载 nltdeploy。" >&2
}

# ---- 入口 ----
_CMD=""
if [[ $# -eq 0 ]]; then
  if [[ ! -t 0 ]] || [[ "${NONINTERACTIVE:-}" == "1" ]]; then
    usage >&2
    die "请指定子命令 install（或 update）或 uninstall。管道示例: curl …/install.sh | bash -s -- install"
  fi
  _CMD="$(_pick_cmd_interactive)"
else
  _CMD="$1"
  shift
fi

case "${_CMD}" in
  quit)
    echo "已退出。" >&2
    exit 0
    ;;
  install | update)
    _parse_source_args "$@"
    do_install_or_update
    ;;
  uninstall | remove)
    [[ $# -eq 0 ]] || die "uninstall 不支持额外参数: $*"
    do_uninstall
    ;;
  help | -h | --help)
    usage
    exit 0
    ;;
  *)
    usage >&2
    die "未知命令: ${_CMD}"
    ;;
esac
