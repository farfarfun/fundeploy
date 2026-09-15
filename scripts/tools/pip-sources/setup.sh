#!/usr/bin/env bash
# Probe pip indexes by latency and configure pip through its native CLI.

if [[ -z "${BASH_VERSION:-}" ]]; then
  exec /usr/bin/env bash "$0" "$@"
fi
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
for common in "${SCRIPT_DIR}/../../lib/fundeploy-common.sh" "${SCRIPT_DIR}/../lib/fundeploy-common.sh"; do
  if [[ -f "$common" ]]; then
    # shellcheck source=../../lib/fundeploy-common.sh
    source "$common"
    break
  fi
done

get_pip_source_info() {
  case "$1" in
    tsinghua) echo "https://pypi.tuna.tsinghua.edu.cn/simple/|清华大学镜像源" ;;
    aliyun) echo "https://mirrors.aliyun.com/pypi/simple/|阿里云镜像源" ;;
    douban) echo "https://pypi.douban.com/simple/|豆瓣镜像源" ;;
    tencent) echo "https://mirrors.cloud.tencent.com/pypi/simple/|腾讯云镜像源" ;;
    huawei) echo "https://mirrors.huaweicloud.com/repository/pypi/simple|华为云镜像源" ;;
    ustc) echo "https://pypi.mirrors.ustc.edu.cn/simple/|中科大镜像源" ;;
    bfsu) echo "https://mirrors.bfsu.edu.cn/pypi/web/simple/|北京外国语大学镜像源" ;;
    sjtu) echo "https://mirror.sjtu.edu.cn/pypi/web/simple/|上海交通大学镜像源" ;;
    hust) echo "http://pypi.hustunique.com/|华中科技大学镜像源" ;;
    artlab-visable) echo "https://artlab.alibaba-inc.com/1/pypi/visable|artlab-visable" ;;
    artlab-pai) echo "https://artlab.alibaba-inc.com/1/pypi/pai|artlab-pai" ;;
    artlab-aop) echo "https://artlab.alibaba-inc.com/1/pypi/aop|artlab-aop" ;;
    tbsite) echo "http://yum.tbsite.net/pypi/simple|淘宝内部源" ;;
    tbsite_aliyun) echo "http://yum.tbsite.net/aliyun-pypi/simple|淘宝内部阿里云源" ;;
    antfin) echo "https://pypi.antfin-inc.com/simple|蚂蚁内部源" ;;
    official) echo "https://pypi.org/simple|官方源" ;;
    *) echo "|" ;;
  esac
}

get_pip_source_url() {
  case "$1" in
    http://*|https://*) printf '%s\n' "$1" ;;
    *) local info; info="$(get_pip_source_info "$1")"; printf '%s\n' "${info%%|*}" ;;
  esac
}

_pip_source_is_insecure() {
  local url
  url="$(get_pip_source_url "$1")"
  [[ "$url" == http://* ]]
}

PIP_PREDEFINED_SOURCES=(
  tsinghua aliyun douban tencent huawei ustc bfsu sjtu hust
  artlab-visable artlab-pai artlab-aop tbsite tbsite_aliyun antfin official
)
PIP_PYTHON="${PIP_SOURCES_PYTHON:-python3}"
TEST_TIMEOUT="${PIP_SOURCES_TEST_TIMEOUT:-5}"
CONNECT_TIMEOUT="${PIP_SOURCES_CONNECT_TIMEOUT:-2}"
PARALLEL_JOBS="${PIP_SOURCES_PARALLEL_JOBS:-8}"
TEST_PACKAGE="${PIP_SOURCES_TEST_PACKAGE:-setuptools}"
VERBOSE=0

info() { printf '[INFO] %s\n' "$*"; }
warn() { printf '[WARN] %s\n' "$*" >&2; }
die() { printf '[ERROR] %s\n' "$*" >&2; exit 1; }

usage() {
  cat <<EOF
用法: $0 [install|update|reinstall] [-v]
      $0 status
      $0 uninstall

默认动作是检测镜像延迟并写入用户级 pip 配置。

选项:
  -v, --verbose   输出每个探测地址
  -h, --help      显示帮助

环境变量:
  PIP_SOURCES_PYTHON=python3        用于执行 python -m pip
  PIP_SOURCES_TEST_TIMEOUT=5        单次探测总超时
  PIP_SOURCES_CONNECT_TIMEOUT=2     连接超时
  PIP_SOURCES_PARALLEL_JOBS=8       并行探测数
  PIP_SOURCES_ALLOW_INSECURE=1      明确允许 HTTP 源
EOF
}

_require_tools() {
  command -v curl >/dev/null 2>&1 || die "需要 curl"
  command -v "$PIP_PYTHON" >/dev/null 2>&1 || die "需要 ${PIP_PYTHON}"
  "$PIP_PYTHON" -m pip --version >/dev/null 2>&1 || die "${PIP_PYTHON} 未安装 pip"
  [[ "$PARALLEL_JOBS" =~ ^[1-9][0-9]*$ ]] || die "PIP_SOURCES_PARALLEL_JOBS 必须是正整数"
}

_pip_get() {
  "$PIP_PYTHON" -m pip config --user get "$1" 2>/dev/null || true
}

_pip_unset() {
  "$PIP_PYTHON" -m pip config --user unset "$1" >/dev/null 2>&1 || true
}

_display_url() {
  printf '%s' "$1" | sed -E 's#(https?://[^:/]+):[^@]+@#\1:***@#'
}

CANDIDATE_NAMES=()
CANDIDATE_URLS=()
CANDIDATE_LABELS=()
CANDIDATE_KEEP=()

_add_source() {
  local name="$1" url="$2" label="$3" keep="${4:-0}" i
  [[ "$url" == http://* || "$url" == https://* ]] || return 0
  if [[ "$url" == http://* && "${PIP_SOURCES_ALLOW_INSECURE:-0}" != "1" ]]; then
    warn "跳过明文 HTTP 源 $(_display_url "$url")（如确需使用请设置 PIP_SOURCES_ALLOW_INSECURE=1）"
    return 0
  fi
  for ((i=0; i<${#CANDIDATE_URLS[@]}; i++)); do
    if [[ "${CANDIDATE_URLS[i]}" == "$url" ]]; then
      [[ "$keep" == "1" ]] && CANDIDATE_KEEP[i]=1
      return 0
    fi
  done
  CANDIDATE_NAMES+=("$name")
  CANDIDATE_URLS+=("$url")
  CANDIDATE_LABELS+=("$label")
  CANDIDATE_KEEP+=("$keep")
}

_load_sources() {
  local name info_line url label value index=0
  for name in "${PIP_PREDEFINED_SOURCES[@]}"; do
    info_line="$(get_pip_source_info "$name")"
    url="${info_line%%|*}"
    label="${info_line#*|}"
    _add_source "$name" "$url" "$label"
  done

  for value in "$(_pip_get global.index-url)" "$(_pip_get global.extra-index-url)"; do
    for url in $value; do
      index=$((index + 1))
      _add_source "existing-${index}" "$url" "现有配置: $(_display_url "$url")" 1
    done
  done
}

_probe_one() {
  local name="$1" url="$2" label="$3" keep="$4" result_file="$5"
  local test_url="${url%/}/${TEST_PACKAGE}/" measured code seconds latency
  [[ "$VERBOSE" == "1" ]] && printf '[DEBUG] %s -> %s\n' "$name" "$(_display_url "$test_url")" >&2
  if measured="$(curl --proto '=https,http' --proto-redir '=https,http' -L -sS \
      --connect-timeout "$CONNECT_TIMEOUT" --max-time "$TEST_TIMEOUT" \
      -o /dev/null -w '%{http_code}|%{time_total}' "$test_url" 2>/dev/null)"; then
    code="${measured%%|*}"
    seconds="${measured#*|}"
    if [[ "$code" =~ ^[23][0-9][0-9]$ ]]; then
      latency="$(awk -v seconds="$seconds" 'BEGIN {printf "%.0f", seconds * 1000}')"
      printf '%09d|ok|%s|%s|%s|%s|%s\n' "$latency" "$name" "$keep" "$latency" "$url" "$label" >"$result_file"
      return
    fi
  fi
  printf '999999999|fail|%s|%s|-|%s|%s\n' "$name" "$keep" "$url" "$label" >"$result_file"
}

AVAILABLE_NAMES=()
AVAILABLE_URLS=()

_probe_sources() {
  local tmp i active=0 result sort_key status name keep latency url label
  tmp="$(mktemp -d)"
  for ((i=0; i<${#CANDIDATE_NAMES[@]}; i++)); do
    _probe_one "${CANDIDATE_NAMES[i]}" "${CANDIDATE_URLS[i]}" \
      "${CANDIDATE_LABELS[i]}" "${CANDIDATE_KEEP[i]}" "${tmp}/${i}" &
    active=$((active + 1))
    if (( active >= PARALLEL_JOBS )); then
      wait || true
      active=0
    fi
  done
  wait || true

  printf '%-18s %-10s %-8s %s\n' "源" "状态" "延迟" "地址"
  while IFS='|' read -r sort_key status name keep latency url label; do
    if [[ "$status" == "ok" ]]; then
      printf '%-18s %-10s %6sms %s\n' "$name" "可用" "$latency" "$(_display_url "$url")"
      AVAILABLE_NAMES+=("$name")
      AVAILABLE_URLS+=("$url")
    else
      printf '%-18s %-10s %-8s %s\n' "$name" "不可用" "-" "$(_display_url "$url")"
      if [[ "$keep" == "1" ]]; then
        warn "现有源暂时不可用，仍保留在 extra-index-url: $(_display_url "$url")"
        AVAILABLE_NAMES+=("$name")
        AVAILABLE_URLS+=("$url")
      fi
    fi
  done < <(sort -t '|' -k1,1n "${tmp}"/*)
  rm -rf "$tmp"
}

_confirm() {
  local prompt="$1"
  [[ "${NONINTERACTIVE:-0}" == "1" || ! -t 0 ]] && return 0
  if declare -F fundeploy_ui_confirm >/dev/null 2>&1; then
    fundeploy_ui_confirm "$prompt"
    return
  fi
  local answer
  read -r -p "${prompt} [y/N] " answer
  [[ "$answer" == [yY] || "$answer" == [yY][eE][sS] ]]
}

_configure_pip() {
  local first_url source_name source_url extra_urls=() trusted_hosts=() host i
  (("${#AVAILABLE_URLS[@]}" > 0)) || die "没有可配置的 pip 源"
  _confirm "将最快镜像写入用户级 pip 配置，继续？" || { info "已取消。"; return 0; }

  first_url="${AVAILABLE_URLS[0]}"
  "$PIP_PYTHON" -m pip config --user set global.index-url "$first_url"
  if (("${#AVAILABLE_URLS[@]}" > 1)); then
    extra_urls=("${AVAILABLE_URLS[@]:1}")
    "$PIP_PYTHON" -m pip config --user set global.extra-index-url "${extra_urls[*]}"
  else
    _pip_unset global.extra-index-url
  fi

  for ((i=0; i<${#AVAILABLE_NAMES[@]}; i++)); do
    source_url="${AVAILABLE_URLS[i]}"
    source_name="$source_url"
    _pip_source_is_insecure "$source_name" || continue
    host="${source_url#*://}"
    host="${host#*@}"
    host="${host%%/*}"
    trusted_hosts+=("${host%%:*}")
  done
  if (("${#trusted_hosts[@]}" > 0)); then
    "$PIP_PYTHON" -m pip config --user set global.trusted-host "${trusted_hosts[*]}"
  else
    _pip_unset global.trusted-host
  fi
  info "已配置 index-url: $(_display_url "$first_url")"
}

configure() {
  while (($#)); do
    case "$1" in
      -v|--verbose) VERBOSE=1 ;;
      -h|--help) usage; return 0 ;;
      *) die "未知选项: $1" ;;
    esac
    shift
  done
  _require_tools
  _load_sources
  _probe_sources
  _configure_pip
}

status() {
  _require_tools
  "$PIP_PYTHON" -m pip config --user list
}

uninstall_config() {
  _require_tools
  if [[ "${NONINTERACTIVE:-0}" != "1" && -t 0 ]]; then
    _confirm "将清除用户级 pip 镜像配置，继续？" || { info "已取消。"; return 0; }
  fi
  _pip_unset global.index-url
  _pip_unset global.extra-index-url
  _pip_unset global.trusted-host
  info "已清除用户级 pip 镜像配置。"
}

main() {
  case "${1:-install}" in
    install|update|upgrade|reinstall)
      (($#)) && shift
      configure "$@"
      ;;
    status|info)
      status
      ;;
    uninstall|remove)
      uninstall_config
      ;;
    help|-h|--help)
      usage
      ;;
    -v|--verbose)
      configure "$@"
      ;;
    *)
      die "未知命令: $1"
      ;;
  esac
}

main "$@"
