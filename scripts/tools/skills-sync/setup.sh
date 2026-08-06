#!/usr/bin/env bash
# 同步 Gitee farfarfun-skills 组织中名称含 "--" 的仓库。
set -euo pipefail

ORG="${SKILLS_SYNC_ORG:-farfarfun-skills}"
TARGET_DIR="${SKILLS_SYNC_DIR:-${HOME}/workspace/github/farfarfun-skills}"
API_URL="${SKILLS_SYNC_API_URL:-https://gitee.com/api/v5/orgs/${ORG}/repos}"
PER_PAGE=100
REMOTE_NAMES=()
REMOTE_URLS=()
LOCAL_NAMES=()

die() { printf '错误: %s\n' "$*" >&2; exit 1; }

usage() {
  cat <<EOF
用法: nltdeploy tool skills-sync [命令] [仓库名]

命令:
  list                         查询远端并显示本地状态
  install [all|仓库名]         安装全部未安装仓库，或安装单个仓库
  update [all|仓库名]          更新全部已安装仓库，或更新单个仓库
  delete [all|仓库名] [--yes]  删除全部或单个已安装仓库
  selftest                     运行本地 Git 自检

无参数时打开交互菜单。目标目录: ${TARGET_DIR}
依赖: git、curl、jq
环境变量: SKILLS_SYNC_DIR、SKILLS_SYNC_ORG、SKILLS_SYNC_API_URL、NONINTERACTIVE
EOF
}

_need() { command -v "$1" >/dev/null 2>&1 || die "需要命令: $1"; }

_valid_name() {
  [[ "$1" == *--* && "$1" =~ ^[A-Za-z0-9._-]+$ ]]
}

_guard_target() {
  local create="${1:-0}" depth
  [[ "${TARGET_DIR}" == /* ]] || die "SKILLS_SYNC_DIR 必须是绝对路径: ${TARGET_DIR}"
  [[ "${create}" == "1" ]] && mkdir -p "${TARGET_DIR}"
  [[ ! -d "${TARGET_DIR}" ]] || TARGET_DIR="$(cd "${TARGET_DIR}" && pwd -P)"
  case "${TARGET_DIR}" in
    /|"${HOME}"|/bin|/boot|/dev|/etc|/home|/lib|/lib64|/opt|/proc|/root|/run|/sbin|/srv|/sys|/tmp|/usr|/var|/Users)
      die "拒绝使用系统目录或 \$HOME: ${TARGET_DIR}" ;;
  esac
  depth="${TARGET_DIR//[^\/]/}"
  (( ${#depth} >= 3 )) || die "目标目录层级过浅: ${TARGET_DIR}"
}

_fetch_remote() {
  local page=1 json count name
  REMOTE_NAMES=()
  REMOTE_URLS=()
  _need curl
  _need jq
  _need git

  while true; do
    json="$(curl --proto '=https' --proto-redir '=https' --tlsv1.2 \
      -fsSL --connect-timeout 10 --max-time 30 --retry 2 \
      "${API_URL}?type=all&page=${page}&per_page=${PER_PAGE}")" \
      || die "查询 Gitee 仓库失败"
    jq -e 'type == "array"' >/dev/null <<<"${json}" || die "Gitee 返回了无效数据"
    count="$(jq 'length' <<<"${json}")"
    while IFS= read -r name; do
      _valid_name "${name}" || continue
      REMOTE_NAMES+=("${name}")
      REMOTE_URLS+=("https://gitee.com/${ORG}/${name}.git")
    done < <(jq -r '.[] | select(.name | contains("--")) | .name' <<<"${json}")
    (( count < PER_PAGE )) && break
    page=$((page + 1))
  done
}

_remote_url() {
  local wanted="$1" i
  for ((i = 0; i < ${#REMOTE_NAMES[@]}; i++)); do
    [[ "${REMOTE_NAMES[$i]}" == "${wanted}" ]] && { printf '%s\n' "${REMOTE_URLS[$i]}"; return 0; }
  done
  return 1
}

_is_managed_repo() {
  local name="$1" dir="${TARGET_DIR}/$1" actual expected
  [[ -d "${dir}" && ! -L "${dir}" ]] || return 1
  actual="$(git -C "${dir}" remote get-url origin 2>/dev/null)" || return 1
  if expected="$(_remote_url "${name}" 2>/dev/null)"; then
    [[ "${actual%.git}" == "${expected%.git}" ]] && return 0
  fi
  actual="${actual%.git}"
  [[ "${actual}" == "https://gitee.com/${ORG}/${name}" ||
     "${actual}" == "git@gitee.com:${ORG}/${name}" ||
     "${actual}" == "ssh://git@gitee.com/${ORG}/${name}" ]]
}

_is_dirty() {
  [[ -n "$(git -C "${TARGET_DIR}/$1" status --porcelain 2>/dev/null)" ]]
}

_state() {
  local name="$1" dir="${TARGET_DIR}/$1"
  if [[ ! -e "${dir}" && ! -L "${dir}" ]]; then
    printf '未安装\n'
  elif _is_managed_repo "${name}"; then
    if _is_dirty "${name}"; then printf '已安装，有修改\n'; else printf '已安装\n'; fi
  else
    printf '目录冲突\n'
  fi
}

_collect_local() {
  local dir name
  LOCAL_NAMES=()
  [[ -d "${TARGET_DIR}" ]] || return 0
  shopt -s nullglob
  for dir in "${TARGET_DIR}"/*--*; do
    name="${dir##*/}"
    _valid_name "${name}" && _is_managed_repo "${name}" && LOCAL_NAMES+=("${name}")
  done
  shopt -u nullglob
}

cmd_list() {
  local name state
  printf '远端仓库: %d 个  目标目录: %s\n' "${#REMOTE_NAMES[@]}" "${TARGET_DIR}"
  for name in "${REMOTE_NAMES[@]}"; do
    state="$(_state "${name}")"
    printf '[%s] %s\n' "${state}" "${name}"
  done
  _collect_local
  for name in "${LOCAL_NAMES[@]}"; do
    _remote_url "${name}" >/dev/null 2>&1 && continue
    if _is_dirty "${name}"; then state='仅本地，有修改'; else state='仅本地'; fi
    printf '[%s] %s\n' "${state}" "${name}"
  done
}

_install_one() {
  local name="$1" url dir
  _valid_name "${name}" || { printf '无效仓库名: %s\n' "${name}" >&2; return 1; }
  url="$(_remote_url "${name}")" || { printf '远端不存在: %s\n' "${name}" >&2; return 1; }
  _guard_target 1
  dir="${TARGET_DIR}/${name}"
  if _is_managed_repo "${name}"; then
    printf '[跳过] 已安装: %s\n' "${name}"
    return 0
  fi
  [[ ! -e "${dir}" && ! -L "${dir}" ]] || { printf '[失败] 同名目录不是受管仓库: %s\n' "${dir}" >&2; return 1; }
  printf '[安装] %s\n' "${name}"
  git clone --depth 1 "${url}" "${dir}"
}

_update_one() {
  local name="$1"
  _valid_name "${name}" || { printf '无效仓库名: %s\n' "${name}" >&2; return 1; }
  _remote_url "${name}" >/dev/null 2>&1 || { printf '远端不存在: %s\n' "${name}" >&2; return 1; }
  _is_managed_repo "${name}" || { printf '[失败] 未安装或不是受管仓库: %s\n' "${name}" >&2; return 1; }
  _is_dirty "${name}" && { printf '[跳过] 有本地修改: %s\n' "${name}" >&2; return 1; }
  printf '[更新] %s\n' "${name}"
  git -C "${TARGET_DIR}/${name}" pull --ff-only
}

_confirm() {
  local prompt="$1" answer
  if command -v gum >/dev/null 2>&1; then
    gum confirm "${prompt}"
  elif [[ -t 0 ]]; then
    read -r -p "${prompt} [y/N] " answer || return 1
    [[ "${answer}" == y || "${answer}" == Y || "${answer}" == yes || "${answer}" == YES ]]
  else
    return 1
  fi
}

_delete_one() {
  local name="$1" confirmed="${2:-no}" dir
  _valid_name "${name}" || { printf '无效仓库名: %s\n' "${name}" >&2; return 1; }
  _guard_target 0
  dir="${TARGET_DIR}/${name}"
  _is_managed_repo "${name}" || { printf '[失败] 未安装或不是受管仓库: %s\n' "${name}" >&2; return 1; }
  _is_dirty "${name}" && { printf '[跳过] 有本地修改，拒绝删除: %s\n' "${name}" >&2; return 1; }
  [[ "${confirmed}" == "yes" ]] || _confirm "确认删除 ${dir}？" || { printf '已取消。\n'; return 0; }
  printf '[删除] %s\n' "${name}"
  rm -rf -- "${dir}"
}

_run_many() {
  local action="$1" name ok=0 failed=0
  case "${action}" in
    install)
      for name in "${REMOTE_NAMES[@]}"; do
        if _install_one "${name}"; then ok=$((ok + 1)); else failed=$((failed + 1)); fi
      done
      ;;
    update)
      for name in "${REMOTE_NAMES[@]}"; do
        _is_managed_repo "${name}" || continue
        if _update_one "${name}"; then ok=$((ok + 1)); else failed=$((failed + 1)); fi
      done
      ;;
    delete)
      _collect_local
      (( ${#LOCAL_NAMES[@]} > 0 )) || { printf '没有可删除的受管仓库。\n'; return 0; }
      [[ "${2:-}" == "--yes" ]] || _confirm "确认删除 ${#LOCAL_NAMES[@]} 个已安装仓库？" || { printf '已取消。\n'; return 0; }
      for name in "${LOCAL_NAMES[@]}"; do
        if _delete_one "${name}" yes; then ok=$((ok + 1)); else failed=$((failed + 1)); fi
      done
      ;;
  esac
  printf '完成: %d，失败/跳过: %d\n' "${ok}" "${failed}"
  (( failed == 0 ))
}

_choose() {
  local prompt="$1" choice i
  shift
  if command -v gum >/dev/null 2>&1; then
    printf '%s\n' "$@" | gum choose --header "${prompt}"
    return
  fi
  printf '%s\n' "${prompt}" >&2
  i=1
  for choice in "$@"; do printf '  %d) %s\n' "${i}" "${choice}" >&2; i=$((i + 1)); done
  read -r -p "请选择 [1-$#]: " choice || return 1
  [[ "${choice}" =~ ^[0-9]+$ && "${choice}" -ge 1 && "${choice}" -le "$#" ]] || return 1
  local options=("$@")
  printf '%s\n' "${options[$((choice - 1))]}"
}

_choose_repo() {
  local labels=() name pick
  for name in "${REMOTE_NAMES[@]}"; do labels+=("${name}  [$(_state "${name}")]"); done
  _collect_local
  for name in "${LOCAL_NAMES[@]}"; do
    _remote_url "${name}" >/dev/null 2>&1 || labels+=("${name}  [仅本地]")
  done
  (( ${#labels[@]} > 0 )) || return 1
  if command -v gum >/dev/null 2>&1; then
    pick="$(printf '%s\n' "${labels[@]}" | gum filter --header '选择仓库' --limit 1)" || return 1
  else
    pick="$(_choose '选择仓库' "${labels[@]}")" || return 1
  fi
  printf '%s\n' "${pick%% *}"
}

_single_menu() {
  local name action state
  name="$(_choose_repo)" || return 0
  state="$(_state "${name}")"
  if [[ "${state}" == 未安装 ]]; then
    action="$(_choose "${name}" '安装' '返回')" || return 0
  elif _is_managed_repo "${name}" && _remote_url "${name}" >/dev/null 2>&1; then
    action="$(_choose "${name}" '更新' '删除' '返回')" || return 0
  elif _is_managed_repo "${name}"; then
    action="$(_choose "${name}" '删除' '返回')" || return 0
  else
    printf '同名目录不是受管仓库，无法操作: %s/%s\n' "${TARGET_DIR}" "${name}" >&2
    return 0
  fi
  case "${action}" in
    安装) _install_one "${name}" || true ;;
    更新) _update_one "${name}" || true ;;
    删除) _delete_one "${name}" || true ;;
  esac
}

interactive_main() {
  local action
  while true; do
    printf '\n'
    cmd_list
    action="$(_choose 'skills-sync' '安装全部未安装仓库' '更新全部已安装仓库' '删除全部已安装仓库' '选择单个仓库' '刷新远端列表' '退出')" || return 0
    case "${action}" in
      安装*) _run_many install || true ;;
      更新*) _run_many update || true ;;
      删除*) _run_many delete || true ;;
      选择*) _single_menu ;;
      刷新*) _fetch_remote ;;
      *) return 0 ;;
    esac
  done
}

_selftest() {
  local tmp seed upstream name='demo--skills'
  tmp="$(mktemp -d)"
  trap 'rm -rf -- "${tmp}"' EXIT
  seed="${tmp}/seed"
  upstream="${tmp}/upstream.git"
  git init -q --bare "${upstream}"
  git init -q -b main "${seed}"
  git -C "${seed}" config user.name test
  git -C "${seed}" config user.email test@example.com
  printf 'one\n' >"${seed}/version"
  git -C "${seed}" add version
  git -C "${seed}" commit -qm one
  git -C "${seed}" remote add origin "${upstream}"
  git -C "${seed}" push -qu origin main
  git --git-dir="${upstream}" symbolic-ref HEAD refs/heads/main
  TARGET_DIR="${tmp}/target/repos"
  REMOTE_NAMES=("${name}")
  REMOTE_URLS=("${upstream}")
  _install_one "${name}" >/dev/null
  printf 'two\n' >"${seed}/version"
  git -C "${seed}" commit -qam two
  git -C "${seed}" push -qu
  _update_one "${name}" >/dev/null
  [[ "$(<"${TARGET_DIR}/${name}/version")" == two ]]
  printf 'dirty\n' >>"${TARGET_DIR}/${name}/version"
  ! _delete_one "${name}" yes >/dev/null 2>&1
  git -C "${TARGET_DIR}/${name}" reset --hard -q
  _delete_one "${name}" yes >/dev/null
  [[ ! -e "${TARGET_DIR}/${name}" ]]
  rm -rf -- "${tmp}"
  trap - EXIT
  printf 'skills-sync selftest: OK\n'
}

main() {
  local action="${1:-}" target="${2:-all}" confirm="${3:-}"
  (( $# <= 3 )) || die "参数过多"
  [[ "${ORG}" =~ ^[A-Za-z0-9._-]+$ ]] || die "无效组织名: ${ORG}"
  case "${action}" in
    help|-h|--help) usage; return 0 ;;
    selftest) _need git; _selftest; return 0 ;;
  esac
  if [[ -z "${action}" && ( "${NONINTERACTIVE:-0}" == 1 || ! -t 0 ) ]]; then usage; return 0; fi
  [[ -z "${confirm}" || "${confirm}" == --yes ]] || die "第三个参数只支持 --yes"
  [[ -z "${confirm}" || "${action}" == delete || "${action}" == remove || "${action}" == uninstall ]] \
    || die "--yes 只用于删除"
  _guard_target 0
  _fetch_remote
  case "${action}" in
    "") interactive_main ;;
    list|status) cmd_list ;;
    install)
      if [[ "${target}" == all || "${target}" == --all ]]; then _run_many install; else _install_one "${target}"; fi
      ;;
    update|upgrade)
      if [[ "${target}" == all || "${target}" == --all ]]; then _run_many update; else _update_one "${target}"; fi
      ;;
    delete|remove|uninstall)
      if [[ "${target}" == all || "${target}" == --all ]]; then
        _run_many delete "${confirm}"
      else
        _delete_one "${target}" "$([[ "${confirm}" == --yes ]] && printf yes || printf no)"
      fi
      ;;
    *) printf '未知命令: %s\n' "${action}" >&2; usage >&2; return 2 ;;
  esac
}

main "$@"
