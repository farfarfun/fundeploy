#!/usr/bin/env bash
# Sub2API（https://github.com/Wei-Shaw/sub2api）本机服务：从 GitHub Releases 下载预编译二进制并运行。
#
# 依赖：curl、tar、gzip；无需本机 Go。
#
# 用法：
#   ./setup.sh              # gum 菜单
#   ./setup.sh install      # 下载二进制与 deploy 资料到 ${SUB2API_SERVICE_HOME}
#   ./setup.sh update       # 重新下载（同 install）
#   ./setup.sh install -v v0.1.144
#   ./setup.sh rollback -v v0.1.143
#   ./setup.sh list-versions
#   ./setup.sh start        # 后台启动（默认绑定 0.0.0.0:8802）
#   ./setup.sh run          # 前台启动（不写 PID；后台已在跑时拒绝）
#   ./setup.sh stop | restart | status | uninstall
#
# 环境变量：
#   SUB2API_SERVICE_HOME   安装根（默认 ~/opt/sub2api），内含 bin/sub2api、deploy/、data/
#   SUB2API_DATA_DIR       数据目录（默认 ${SUB2API_SERVICE_HOME}/data）；会向程序导出 DATA_DIR
#   SUB2API_HOST           监听地址（默认 0.0.0.0）
#   SUB2API_PORT           监听端口（默认 8802）
#   SUB2API_VERSION        指定版本，如 0.1.144 / v0.1.144；不设则取 GitHub latest
#   SUB2API_GITHUB_REPO    owner/repo（默认 Wei-Shaw/sub2api）
#   SUB2API_ENV_FILE       可选环境变量文件（默认 ${SUB2API_DATA_DIR}/sub2api.env）
#   SUB2API_CONFIG_EXAMPLE_DEST  install 时示例配置落盘路径（默认 ${SUB2API_DATA_DIR}/config.example.yaml）
#   SUB2API_SKIP_CHECKSUM=1     跳过 sha256 校验（默认不跳过；若 checksums.txt 缺失则自动降级）
#   NONINTERACTIVE=1
#   SUB2API_UNINSTALL_YES=1

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ -f "${SCRIPT_DIR}/../lib/nlt-common.sh" ]]; then
  # shellcheck source=../lib/nlt-common.sh
  source "${SCRIPT_DIR}/../lib/nlt-common.sh"
elif [[ -f "${SCRIPT_DIR}/../../lib/nlt-common.sh" ]]; then
  # shellcheck source=../../lib/nlt-common.sh
  source "${SCRIPT_DIR}/../../lib/nlt-common.sh"
else
  echo "错误: 找不到 lib/nlt-common.sh（已检查 ${SCRIPT_DIR}/../lib 与 ${SCRIPT_DIR}/../../lib）" >&2
  exit 1
fi

if [[ -f "${SCRIPT_DIR}/../lib/nlt-progress.sh" ]]; then
  # shellcheck source=../lib/nlt-progress.sh
  source "${SCRIPT_DIR}/../lib/nlt-progress.sh"
elif [[ -f "${SCRIPT_DIR}/../../lib/nlt-progress.sh" ]]; then
  # shellcheck source=../../lib/nlt-progress.sh
  source "${SCRIPT_DIR}/../../lib/nlt-progress.sh"
fi

SUB2API_GITHUB_REPO="${SUB2API_GITHUB_REPO:-Wei-Shaw/sub2api}"
SUB2API_SERVICE_HOME="${SUB2API_SERVICE_HOME:-${HOME}/opt/sub2api}"
SUB2API_DATA_DIR="${SUB2API_DATA_DIR:-${SUB2API_SERVICE_HOME}/data}"
SUB2API_HOST="${SUB2API_HOST:-0.0.0.0}"
SUB2API_PORT="${SUB2API_PORT:-8802}"
SUB2API_ENV_FILE="${SUB2API_ENV_FILE:-${SUB2API_DATA_DIR}/sub2api.env}"
SUB2API_CONFIG_EXAMPLE_DEST="${SUB2API_CONFIG_EXAMPLE_DEST:-${SUB2API_DATA_DIR}/config.example.yaml}"

SUB2API_RUN_DIR="${SUB2API_SERVICE_HOME}/run"
SUB2API_LOG_DIR="${SUB2API_SERVICE_HOME}/log"
PID_FILE="${SUB2API_RUN_DIR}/sub2api.pid"
LOG_FILE="${SUB2API_LOG_DIR}/sub2api.log"
SUB2API_BIN="${SUB2API_SERVICE_HOME}/bin/sub2api"
SUB2API_DEPLOY_DIR="${SUB2API_SERVICE_HOME}/deploy"
SUB2API_FALLBACK_TAG="${SUB2API_FALLBACK_TAG:-v0.1.144}"

usage() {
  cat <<USAGE
用法: ./setup.sh [command] [-v VERSION|--version VERSION]

  无参数：gum 菜单。

命令:
  install / update   从 GitHub Releases 下载二进制到 ${SUB2API_SERVICE_HOME}
                     可配合 -v/--version 指定版本；默认 latest
  start              后台启动（日志 ${LOG_FILE}；默认 ${SUB2API_HOST}:${SUB2API_PORT}）
  run                前台启动（同环境；不写 PID；后台已在跑时拒绝）
  stop / restart / status
  rollback          回滚到指定版本；未指定时列出可用版本
  list-versions      列出最近的 release tag
  uninstall          停止并删除 ${SUB2API_SERVICE_HOME}

说明:
  - 程序启动前会导出 DATA_DIR=${SUB2API_DATA_DIR}
  - 若 ${SUB2API_ENV_FILE} 存在，将自动 source，可放 DATABASE_* / REDIS_* / JWT_SECRET 等
  - install 会额外保留上游 deploy/ 文档，并在缺失时写入示例配置 ${SUB2API_CONFIG_EXAMPLE_DEST}
  - install / update 默认会尝试校验 release 附带的 checksums.txt
  - 若未提供 config.yaml，Sub2API 可走上游 Setup Wizard / 默认环境变量路径
USAGE
}

ensure_dirs() {
  mkdir -p "${SUB2API_RUN_DIR}" "${SUB2API_LOG_DIR}" "${SUB2API_DATA_DIR}" "${SUB2API_SERVICE_HOME}/bin"
}

die() { echo "错误: $*" >&2; exit 1; }

process_alive() { kill -0 "$1" 2>/dev/null; }

listener_pid_for_port() {
  local port="$1"
  if command -v lsof >/dev/null 2>&1; then
    lsof -tiTCP:"${port}" -sTCP:LISTEN -n -P 2>/dev/null | head -1
    return 0
  fi
  if command -v ss >/dev/null 2>&1; then
    ss -ltnp "( sport = :${port} )" 2>/dev/null | awk -F 'pid=' 'NR>1 && NF>1 {split($2,a,","); print a[1]; exit}'
    return 0
  fi
  echo ""
}

read_pid() {
  [[ -f "$PID_FILE" ]] || { echo ""; return; }
  tr -d '[:space:]' <"$PID_FILE" || true
}

require_curl() { command -v curl >/dev/null 2>&1 || die "需要 curl"; }

require_tar() {
  command -v tar >/dev/null 2>&1 || die "需要 tar"
  command -v gzip >/dev/null 2>&1 || die "需要 gzip"
}

_detect_platform() {
  local os arch
  case "$(uname -s)" in
    Linux) os=linux ;;
    Darwin) os=darwin ;;
    *) die "不支持的操作系统: $(uname -s)" ;;
  esac
  case "$(uname -m)" in
    x86_64 | amd64) arch=amd64 ;;
    aarch64 | arm64) arch=arm64 ;;
    *) die "不支持的架构: $(uname -m)" ;;
  esac
  printf '%s_%s\n' "$os" "$arch"
}

_normalize_version_to_tag() {
  local v="${1:-}"
  v="${v#v}"
  [[ -n "$v" ]] || return 1
  printf 'v%s\n' "$v"
}

_fetch_latest_tag() {
  require_curl
  local out tag
  out="$(_nlt_github_download_curl -fsSL "https://api.github.com/repos/${SUB2API_GITHUB_REPO}/releases/latest")" || return 1
  tag="$(printf '%s' "$out" | sed -n 's/.*"tag_name": *"\(v[0-9][0-9.]*\)".*/\1/p' | head -1)"
  [[ -n "$tag" ]] || return 1
  printf '%s\n' "$tag"
}

_fetch_release_json_for_tag() {
  local tag="$1"
  require_curl
  _nlt_github_download_curl -fsSL "https://api.github.com/repos/${SUB2API_GITHUB_REPO}/releases/tags/${tag}"
}

_fetch_releases_json() {
  local page="${1:-1}"
  require_curl
  _nlt_github_download_curl -fsSL "https://api.github.com/repos/${SUB2API_GITHUB_REPO}/releases?per_page=100&page=${page}"
}

_resolve_tag() {
  if [[ -n "${SUB2API_VERSION:-}" ]]; then
    _normalize_version_to_tag "${SUB2API_VERSION}" || die "无效 SUB2API_VERSION: ${SUB2API_VERSION}"
    return
  fi
  local tag
  tag="$(_fetch_latest_tag)" || true
  if [[ -n "$tag" ]]; then
    printf '%s\n' "$tag"
    return
  fi
  printf '%s\n' "${SUB2API_FALLBACK_TAG}"
}

_asset_name_for_tag() {
  local tag="$1"
  local ver plat
  ver="${tag#v}"
  plat="$(_detect_platform)"
  printf 'sub2api_%s_%s.tar.gz\n' "$ver" "$plat"
}

_sha256_file() {
  local file="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$file" | awk '{print $1}'
    return 0
  fi
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$file" | awk '{print $1}'
    return 0
  fi
  if command -v openssl >/dev/null 2>&1; then
    openssl dgst -sha256 "$file" | awk '{print $NF}'
    return 0
  fi
  return 1
}

_pick_release_asset_urls() {
  local json="$1"
  local expected_asset="$2"
  if command -v python3 >/dev/null 2>&1; then
    RELEASE_JSON="$json" python3 -c '
import json, os, sys
expected = sys.argv[1]
data = json.loads(os.environ["RELEASE_JSON"])
assets = data.get("assets") or []
asset_url = ""
checksum_url = ""
for asset in assets:
    name = asset.get("name") or ""
    url = asset.get("browser_download_url") or ""
    if name == expected:
        asset_url = url
    elif name == "checksums.txt":
        checksum_url = url
if not asset_url:
    sys.exit(1)
print(asset_url)
print(checksum_url)
' "$expected_asset"
    return
  fi
  local asset_url checksum_url json_one
  json_one="$(printf '%s' "$json" | tr -d '\n')"
  asset_url="$(printf '%s' "$json_one" | sed -n "s/.*{\"url\":\"[^\"]*\",\"id\":[^}]*\"name\":\"${expected_asset//./\\.}\"[^}]*\"browser_download_url\":\"\\([^\"]*\\)\".*/\\1/p" | head -1)"
  checksum_url="$(printf '%s' "$json_one" | sed -n 's/.*{\"url\":\"[^"]*\",\"id\":[^}]*"name":"checksums.txt"[^}]*"browser_download_url":"\([^"]*\)".*/\1/p' | head -1)"
  [[ -n "$asset_url" ]] || return 1
  printf '%s\n%s\n' "$asset_url" "$checksum_url"
}

_verify_checksum_if_available() {
  local archive="$1"
  local checksum_file="$2"
  local asset_name="$3"
  [[ "${SUB2API_SKIP_CHECKSUM:-0}" == "1" ]] && {
    echo "跳过校验（SUB2API_SKIP_CHECKSUM=1）" >&2
    return 0
  }
  [[ -f "$checksum_file" ]] || {
    echo "警告: 未找到 checksums.txt，跳过校验。" >&2
    return 0
  }
  local expected actual
  expected="$(awk -v n="$asset_name" '$2 == n {print $1; exit}' "$checksum_file")"
  [[ -n "$expected" ]] || {
    echo "警告: checksums.txt 未找到 ${asset_name} 的条目，跳过校验。" >&2
    return 0
  }
  actual="$(_sha256_file "$archive")" || {
    echo "警告: 本机无 sha256 工具（sha256sum/shasum/openssl），跳过校验。" >&2
    return 0
  }
  [[ "$actual" == "$expected" ]] || die "校验失败: ${asset_name} sha256 不匹配"
  echo "校验通过: ${asset_name}" >&2
}

list_versions() {
  local page=1 json tags any=0
  while :; do
    json="$(_fetch_releases_json "$page")" || die "获取 release 列表失败"
    tags="$(printf '%s' "$json" | sed -n 's/.*"tag_name": *"\(v[0-9][0-9.]*\)".*/\1/p')"
    [[ -n "$tags" ]] || break
    printf '%s\n' "$tags"
    any=1
    page=$((page + 1))
  done
  [[ "$any" == "1" ]] || die "未解析到可用版本"
}

choose_rollback_version() {
  local versions chosen
  versions="$(list_versions)" || return 1
  [[ -n "$versions" ]] || return 1

  if [[ -t 0 && -t 1 ]]; then
    if _nlt_ensure_gum >/dev/null 2>&1; then
      chosen="$(printf '%s\n' "$versions" | gum choose --header "选择要回滚的 sub2api 版本")" || return 1
      [[ -n "$chosen" ]] || return 1
      printf '%s\n' "$chosen"
      return 0
    fi

    local options=()
    while IFS= read -r line; do
      [[ -n "$line" ]] && options+=("$line")
    done <<<"$versions"
    [[ "${#options[@]}" -gt 0 ]] || return 1
    echo "请选择要回滚的 sub2api 版本：" >&2
    select chosen in "${options[@]}"; do
      [[ -n "${chosen:-}" ]] || {
        echo "无效选择，请重试。" >&2
        continue
      }
      printf '%s\n' "$chosen"
      return 0
    done
  fi

  printf '%s\n' "$versions" >&2
  return 1
}

_maybe_seed_example_config() {
  if [[ ! -f "${SUB2API_CONFIG_EXAMPLE_DEST}" && -f "${SUB2API_DEPLOY_DIR}/config.example.yaml" ]]; then
    cp -f "${SUB2API_DEPLOY_DIR}/config.example.yaml" "${SUB2API_CONFIG_EXAMPLE_DEST}"
  fi
  if [[ ! -f "${SUB2API_ENV_FILE}" ]]; then
    cat >"${SUB2API_ENV_FILE}" <<'EOF'
# Sub2API 运行环境变量示例
# DATABASE_HOST=127.0.0.1
# DATABASE_PORT=5432
# DATABASE_USER=postgres
# DATABASE_PASSWORD=postgres
# DATABASE_NAME=sub2api
# REDIS_HOST=127.0.0.1
# REDIS_PORT=6379
# REDIS_PASSWORD=
# JWT_SECRET=replace-with-random-secret
# TOTP_ENCRYPTION_KEY=replace-with-random-secret
# ADMIN_EMAIL=admin@sub2api.local
# ADMIN_PASSWORD=change-me
EOF
  fi
}

_download_install() {
  require_curl
  require_tar
  local tag asset url checksum_url tmpdir release_json
  tag="$(_resolve_tag)"
  asset="$(_asset_name_for_tag "$tag")"
  release_json="$(_fetch_release_json_for_tag "$tag" || true)"
  if [[ -n "$release_json" ]]; then
    mapfile -t _picked_urls < <(_pick_release_asset_urls "$release_json" "$asset") || true
    url="${_picked_urls[0]:-}"
    checksum_url="${_picked_urls[1]:-}"
  fi
  url="${url:-https://github.com/${SUB2API_GITHUB_REPO}/releases/download/${tag}/${asset}}"
  checksum_url="${checksum_url:-https://github.com/${SUB2API_GITHUB_REPO}/releases/download/${tag}/checksums.txt}"
  echo "==> 下载 Sub2API ${tag} → ${SUB2API_SERVICE_HOME}" >&2
  echo "    ${url}" >&2
  tmpdir="$(mktemp -d)"
  trap '[[ -n "${tmpdir-}" ]] && rm -rf "${tmpdir}"' RETURN
  _nlt_github_download_print_accel_hint
  mkdir -p "${tmpdir}/unpack"
  if declare -F nlt_pb_curl_to_file >/dev/null 2>&1; then
    NLT_PB_LABEL="sub2api ${tag}" nlt_pb_curl_to_file "$url" "${tmpdir}/sub2api.tgz" || die "下载失败: ${url}"
  else
    _nlt_github_download_curl -fsSL "$url" -o "${tmpdir}/sub2api.tgz"
  fi
  _nlt_github_download_curl -fsSL "$checksum_url" -o "${tmpdir}/checksums.txt" 2>/dev/null || true
  _verify_checksum_if_available "${tmpdir}/sub2api.tgz" "${tmpdir}/checksums.txt" "${asset}"
  rm -rf "${SUB2API_SERVICE_HOME}/bin" "${SUB2API_DEPLOY_DIR}" 2>/dev/null || true
  mkdir -p "${SUB2API_SERVICE_HOME}/bin" "${SUB2API_DEPLOY_DIR}"
  tar -xzf "${tmpdir}/sub2api.tgz" -C "${tmpdir}/unpack"
  install -m 0755 "${tmpdir}/unpack/sub2api" "${SUB2API_BIN}"
  cp -R "${tmpdir}/unpack/deploy/." "${SUB2API_DEPLOY_DIR}/"
  _maybe_seed_example_config
  rm -rf "${tmpdir}"
  trap - RETURN
  [[ -x "${SUB2API_BIN}" ]] || die "解压后未找到可执行文件: ${SUB2API_BIN}"
  echo "已安装到 ${SUB2API_SERVICE_HOME}（${tag} / ${asset}）"
}

sub2api_export_runtime_env() {
  export DATA_DIR="${SUB2API_DATA_DIR}"
  export SERVER_HOST="${SUB2API_HOST}"
  export SERVER_PORT="${SUB2API_PORT}"
  export GIN_MODE="${GIN_MODE:-release}"
}

sub2api_source_env_file() {
  if [[ -f "${SUB2API_ENV_FILE}" ]]; then
    set -a
    # shellcheck disable=SC1090
    source "${SUB2API_ENV_FILE}"
    set +a
  fi
}

sub2api_http_code() {
  if ! command -v curl >/dev/null 2>&1; then
    echo "000"
    return
  fi
  curl -sS -o /dev/null -w '%{http_code}' -m 3 "http://${SUB2API_HOST}:${SUB2API_PORT}/health" 2>/dev/null || echo "000"
}

cmd_install() { ensure_dirs; _download_install; }

cmd_update() {
  ensure_dirs
  echo "==> 更新 Sub2API（重新下载）…" >&2
  _download_install
}

cmd_rollback() {
  if [[ -z "${SUB2API_VERSION:-}" ]]; then
    if [[ -t 0 && -t 1 ]]; then
      SUB2API_VERSION="$(choose_rollback_version)" || exit 1
    else
      echo "可回滚版本：" >&2
      list_versions >&2 || true
      die "rollback 需要 -v VERSION 或 --version VERSION"
    fi
  fi
  ensure_dirs
  echo "==> 回滚 Sub2API 到 ${SUB2API_VERSION}…" >&2
  _download_install
}

cmd_start() {
  [[ -x "${SUB2API_BIN}" ]] || die "未安装，请先: $0 install"
  ensure_dirs
  local existing listener_pid
  existing="$(read_pid)"
  if [[ -n "$existing" ]] && process_alive "$existing"; then
    echo "Sub2API 已在运行（PID ${existing}）。重启请: $0 restart" >&2
    exit 1
  fi
  listener_pid="$(listener_pid_for_port "${SUB2API_PORT}")"
  [[ -z "$listener_pid" ]] || die "端口 ${SUB2API_PORT} 已被 PID ${listener_pid} 占用，请先执行 $0 stop 或手动清理。"
  rm -f "$PID_FILE"
  echo "==> 启动 Sub2API，监听 ${SUB2API_HOST}:${SUB2API_PORT}，日志: ${LOG_FILE}" >&2
  (
    cd "${SUB2API_SERVICE_HOME}"
    sub2api_source_env_file
    sub2api_export_runtime_env
    nohup "${SUB2API_BIN}" >>"${LOG_FILE}" 2>&1 &
    echo $! >"${PID_FILE}"
  )
  sleep 1
  existing="$(read_pid)"
  if [[ -n "$existing" ]] && process_alive "$existing"; then
    echo "已启动 PID ${existing}"
  else
    echo "警告: 进程可能已退出，请查看: tail -80 ${LOG_FILE}" >&2
  fi
}

cmd_run() {
  [[ -x "${SUB2API_BIN}" ]] || die "未安装，请先: $0 install"
  ensure_dirs
  local existing
  existing="$(read_pid)"
  if [[ -n "$existing" ]] && process_alive "$existing"; then
    echo "Sub2API 已在后台运行（PID ${existing}）。请先 $0 stop，再使用 run。" >&2
    exit 1
  fi
  echo "==> 前台启动 Sub2API，监听 ${SUB2API_HOST}:${SUB2API_PORT}（Ctrl+C 退出；不写 PID）" >&2
  cd "${SUB2API_SERVICE_HOME}"
  sub2api_source_env_file
  sub2api_export_runtime_env
  exec "${SUB2API_BIN}"
}

cmd_stop() {
  local pid listener_pid
  pid="$(read_pid)"
  listener_pid="$(listener_pid_for_port "${SUB2API_PORT}")"
  if [[ -z "$pid" ]]; then
    echo "未找到 PID，视为未运行。" >&2
    rm -f "$PID_FILE"
  elif ! process_alive "$pid"; then
    rm -f "$PID_FILE"
  fi
  if [[ -n "$pid" || -n "$listener_pid" ]] && [[ "${NONINTERACTIVE:-}" != "1" ]] && [[ -t 0 ]]; then
    gum confirm "停止 Sub2API（PID ${pid:-unknown}）？" || exit 0
  fi
  if [[ -n "$pid" ]] && process_alive "$pid"; then
    kill -TERM "$pid" 2>/dev/null || true
    local w=0
    while process_alive "$pid" && (( w < 20 )); do
      sleep 1
      w=$((w + 1))
    done
    process_alive "$pid" && kill -KILL "$pid" 2>/dev/null || true
  fi
  if [[ -n "$listener_pid" ]] && { [[ -z "$pid" ]] || [[ "$listener_pid" != "$pid" ]]; }; then
    kill -TERM "$listener_pid" 2>/dev/null || true
  fi
  rm -f "$PID_FILE"
  echo "已停止。"
}

cmd_status() {
  local pid listener_pid state http_code
  pid="$(read_pid)"
  listener_pid="$(listener_pid_for_port "${SUB2API_PORT}")"
  state="未运行"
  if [[ -n "$pid" ]] && process_alive "$pid"; then
    state="运行中"
  elif [[ -n "$listener_pid" ]]; then
    state="端口占用"
  fi
  http_code="$(sub2api_http_code)"
  if [[ "$state" == "未运行" && "$http_code" != "000" ]]; then
    state="未运行（端口有响应）"
  fi
  echo "Sub2API 状态: ${state}"
  echo "PID 文件: ${PID_FILE}"
  echo "PID: ${pid:--}${listener_pid:+ / listen:${listener_pid}}"
  echo "监听: http://${SUB2API_HOST}:${SUB2API_PORT}"
  echo "健康检查: /health -> HTTP ${http_code}"
  echo "安装根: ${SUB2API_SERVICE_HOME}"
  echo "数据目录: ${SUB2API_DATA_DIR}"
  echo "环境文件: ${SUB2API_ENV_FILE} $( [[ -f "${SUB2API_ENV_FILE}" ]] && printf '%s' '(存在)' || printf '%s' '(不存在)' )"
  echo "配置示例: ${SUB2API_CONFIG_EXAMPLE_DEST} $( [[ -f "${SUB2API_CONFIG_EXAMPLE_DEST}" ]] && printf '%s' '(存在)' || printf '%s' '(不存在)' )"
  echo "实际配置: ${SUB2API_DATA_DIR}/config.yaml $( [[ -f "${SUB2API_DATA_DIR}/config.yaml" ]] && printf '%s' '(存在)' || printf '%s' '(不存在)' )"
  echo "日志: ${LOG_FILE}"
  echo "部署资料: ${SUB2API_DEPLOY_DIR}"
}

cmd_restart() { cmd_stop || true; cmd_start; }

cmd_uninstall() {
  if [[ ! -d "${SUB2API_SERVICE_HOME}" ]]; then
    echo "未找到安装目录，跳过: ${SUB2API_SERVICE_HOME}" >&2
    exit 0
  fi
  if [[ "${SUB2API_UNINSTALL_YES:-}" != "1" ]]; then
    if [[ -t 0 ]]; then
      gum confirm "将删除整个 ${SUB2API_SERVICE_HOME}，确认？" || exit 0
    else
      die "非交互卸载请设置 SUB2API_UNINSTALL_YES=1"
    fi
  fi
  cmd_stop || true
  rm -rf "${SUB2API_SERVICE_HOME}"
  echo "已卸载 ${SUB2API_SERVICE_HOME}"
}

interactive_main() {
  _nlt_ensure_gum || exit 1
  set +e
  while true; do
    local pick
    pick="$(gum choose --header "sub2api" "install" "update" "rollback" "start" "run" "stop" "restart" "status" "list-versions" "uninstall" "help" "quit")" || break
    [[ -z "$pick" ]] && break
    case "$pick" in
      quit) break ;;
      help) usage; echo "" ;;
      install) cmd_install ;;
      update) cmd_update ;;
      rollback) cmd_rollback ;;
      start) cmd_start ;;
      run) cmd_run ;;
      stop) cmd_stop ;;
      restart) cmd_restart ;;
      status) cmd_status ;;
      list-versions) list_versions ;;
      uninstall) cmd_uninstall ;;
    esac
    echo ""
  done
  set -e
}

main() {
  local cmd="${1:-}"
  if [[ $# -gt 0 ]]; then
    shift
  fi
  while [[ $# -gt 0 ]]; do
    case "$1" in
      -v|--version)
        [[ $# -ge 2 ]] || die "$1 需要版本参数"
        SUB2API_VERSION="$2"
        shift 2
        ;;
      --version=*)
        SUB2API_VERSION="${1#*=}"
        shift
        ;;
      *)
        die "未知参数: $1"
        ;;
    esac
  done
  if [[ -z "$cmd" ]]; then
    if [[ "${NONINTERACTIVE:-}" == "1" ]]; then
      usage >&2
      exit 1
    fi
    interactive_main
    return 0
  fi
  case "$cmd" in
    install) cmd_install ;;
    update) cmd_update ;;
    rollback) cmd_rollback ;;
    start) cmd_start ;;
    run) cmd_run ;;
    stop) cmd_stop ;;
    restart) cmd_restart ;;
    status) cmd_status ;;
    list-versions|versions) list_versions ;;
    uninstall) cmd_uninstall ;;
    help | -h | --help) usage ;;
    *)
      echo "未知命令: $cmd" >&2
      usage >&2
      exit 2
      ;;
  esac
}

main "$@"
