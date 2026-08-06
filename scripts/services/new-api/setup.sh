#!/usr/bin/env bash
# new-api（https://github.com/QuantumNous/new-api）本机服务：从 GitHub Releases 下载预编译二进制。
#
# 依赖：curl；可选 python3（用于在「latest 无资源包」时挑选最近一条带二进制附件的 Release）。
#
# 用法：
#   ./setup.sh              # gum 菜单
#   ./setup.sh install      # 下载二进制到 ${NEW_API_SERVICE_HOME}/bin/new-api
#   ./setup.sh update       # 重新下载（同 install）
#   ./setup.sh start        # 后台启动（工作目录为数据目录，默认端口 8801）
#   ./setup.sh run          # 前台启动（同目录与 PORT；不写 PID；后台已在跑时拒绝）
#   ./setup.sh stop | restart | status | uninstall
#
# 环境变量：
#   NEW_API_SERVICE_HOME   安装根（默认 ~/opt/new-api），内含 bin/new-api、data/（SQLite 等工作目录）
#   NEW_API_DATA_DIR       运行时的 cwd（默认 ${NEW_API_SERVICE_HOME}/data）；库文件、以及 new-api 官方文档中的 .env 均放此目录。
#                          二进制启动后会在该目录下执行 godotenv.Load(".env")，把 new-api 内部需要的变量（如 SQL_DSN、
#                          SESSION_SECRET、REDIS_CONN_STRING、各类业务开关等）写在 ${NEW_API_DATA_DIR}/.env 即可。
#   NEW_API_PORT / PORT    无 .env 时：启动前注入 PORT（默认 8801）。若存在 ${NEW_API_DATA_DIR}/.env，默认不注入
#                          PORT，由程序从 .env 读入；且启动时用 env -u PORT 去掉 shell 里已 export 的 PORT，避免挡住 .env。
#                          注意：其它键若你已在当前 shell 里 export 过同名变量，godotenv 同样不会覆盖——需 unset 该变量或换干净终端。
#   NEW_API_FORCE_SCRIPT_PORT=1  即使存在 .env 也强制使用 NEW_API_PORT（等价于以前行为）
#   NEW_API_VERSION        强制版本，如 0.12.6 或 v0.12.6（不设则从 Releases 解析）
#   NEW_API_GITHUB_REPO    owner/repo（默认 QuantumNous/new-api）
#   NONINTERACTIVE=1
#   NEW_API_UNINSTALL_YES=1   非 TTY 卸载确认

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# 仓库内为 scripts/.../<域>/；install 同步后为 libexec/nltdeploy/<域>/（与 lib/ 同级）→ 先试 ../lib 再 ../../lib
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

NEW_API_GITHUB_REPO="${NEW_API_GITHUB_REPO:-QuantumNous/new-api}"
NEW_API_SERVICE_HOME="${NEW_API_SERVICE_HOME:-${HOME}/opt/new-api}"
NEW_API_DATA_DIR="${NEW_API_DATA_DIR:-${NEW_API_SERVICE_HOME}/data}"
NEW_API_PORT="${NEW_API_PORT:-8801}"

NEW_API_RUN_DIR="${NEW_API_SERVICE_HOME}/run"
NEW_API_LOG_DIR="${NEW_API_SERVICE_HOME}/log"
PID_FILE="${NEW_API_RUN_DIR}/new-api.pid"
LOG_FILE="${NEW_API_LOG_DIR}/new-api.log"
NEW_API_BIN="${NEW_API_SERVICE_HOME}/bin/new-api"

# 无 python3 或 API 失败时的回退 tag（须对应 Release 上存在各平台二进制）
NEW_API_FALLBACK_TAG="${NEW_API_FALLBACK_TAG:-v0.12.6}"

usage() {
  cat <<USAGE
用法: ./setup.sh [command]

  无参数：gum 菜单。

命令:
  install / update   从 GitHub Releases 下载预编译二进制到 ${NEW_API_BIN}
  start              后台启动（cwd ${NEW_API_DATA_DIR}；无 .env 时注入 PORT=${NEW_API_PORT}；有 .env 时由程序读 .env；日志 ${LOG_FILE}）
  run                前台启动（与 start 相同 cwd；终端附着；不写 PID；后台已在跑时拒绝）
  stop / restart / status
  uninstall          停止并删除 ${NEW_API_SERVICE_HOME}

上游: https://github.com/QuantumNous/new-api
文档: https://docs.newapi.pro/
USAGE
}

ensure_dirs() {
  mkdir -p "${NEW_API_RUN_DIR}" "${NEW_API_LOG_DIR}" "${NEW_API_DATA_DIR}"
}

die() { echo "错误: $*" >&2; exit 1; }

# new-api 使用 godotenv.Load(".env")：默认不覆盖已在环境中的变量。本脚本若先 env PORT=… 会导致 .env 不生效。
# 返回用于探测 HTTP 的端口（优先 .env 中 PORT，否则 NEW_API_PORT）。
_new_api_effective_port() {
  local p="${NEW_API_PORT}"
  if [[ -f "${NEW_API_DATA_DIR}/.env" ]]; then
    local raw
    raw="$(grep -E '^[[:space:]]*PORT[[:space:]]*=' "${NEW_API_DATA_DIR}/.env" 2>/dev/null | tail -1 || true)"
    raw="$(printf '%s' "$raw" | sed -E 's/^[[:space:]]*PORT[[:space:]]*=[[:space:]]*//;s/#.*$//;s/^["'\'']//;s/["'\'']$//;s/[[:space:]]*$//')"
    [[ -n "$raw" ]] && p="$raw"
  fi
  printf '%s' "$p"
}

process_alive() {
  kill -0 "$1" 2>/dev/null
}

listener_pid_for_port() {
  _nlt_listener_pid_for_port "$1"
}

read_pid() {
  if [[ ! -f "$PID_FILE" ]]; then
    echo ""
    return
  fi
  tr -d '[:space:]' <"$PID_FILE" || true
}

require_curl() {
  command -v curl >/dev/null 2>&1 || die "需要 curl"
}

# 输出平台键：linux-amd64 | linux-arm64 | darwin
_detect_platform_kind() {
  local os arch
  case "$(uname -s)" in
    Linux) os=linux ;;
    Darwin) os=darwin ;;
    *) die "不支持的操作系统: $(uname -s)" ;;
  esac
  if [[ "$os" == "darwin" ]]; then
    echo "darwin"
    return
  fi
  case "$(uname -m)" in
    x86_64 | amd64) echo "linux-amd64" ;;
    aarch64 | arm64) echo "linux-arm64" ;;
    *) die "不支持的架构: $(uname -m)" ;;
  esac
}

# 根据 release tag（如 v0.12.6）与平台得到 GitHub 上的资源文件名
_asset_name_for_tag() {
  local tag="$1"
  local kind ver
  kind="$(_detect_platform_kind)"
  ver="${tag#v}"
  case "$kind" in
    linux-amd64) echo "new-api-v${ver}" ;;
    linux-arm64) echo "new-api-arm64-v${ver}" ;;
    darwin) echo "new-api-macos-v${ver}" ;;
    *) die "内部错误: kind=${kind}" ;;
  esac
}

# 从 API JSON 中选出第一个包含当前平台二进制的 release tag；失败打印空
_pick_tag_from_releases_json() {
  local json="$1"
  command -v python3 >/dev/null 2>&1 || return 1
  local kind
  kind="$(_detect_platform_kind)"
  NEW_API_PLATFORM_KIND="$kind" python3 -c '
import json, os, sys
releases = json.loads(sys.stdin.read())
kind = os.environ.get("NEW_API_PLATFORM_KIND", "")

def want_name(tag: str) -> str:
    ver = tag[1:] if tag.startswith("v") else tag
    if kind == "linux-amd64":
        return f"new-api-v{ver}"
    if kind == "linux-arm64":
        return f"new-api-arm64-v{ver}"
    if kind == "darwin":
        return f"new-api-macos-v{ver}"
    raise SystemExit(1)

for r in releases:
    if r.get("draft"):
        continue
    tag = r.get("tag_name") or ""
    if not tag:
        continue
    names = {a.get("name") for a in (r.get("assets") or []) if a.get("name")}
    if not names:
        continue
    try:
        w = want_name(tag)
    except SystemExit:
        continue
    if w in names:
        print(tag)
        sys.exit(0)
sys.exit(1)
' <<<"$json"
}

_normalize_version_to_tag() {
  local v="${1:-}"
  v="${v#v}"
  [[ -n "$v" ]] || return 1
  echo "v${v}"
}

_resolve_tag() {
  if [[ -n "${NEW_API_VERSION:-}" ]]; then
    _normalize_version_to_tag "${NEW_API_VERSION}" || die "无效 NEW_API_VERSION: ${NEW_API_VERSION}"
    return
  fi
  require_curl
  local json picked
  json="$(_nlt_github_download_curl -fsSL "https://api.github.com/repos/${NEW_API_GITHUB_REPO}/releases?per_page=40")" || json=""
  if [[ -n "$json" ]]; then
    picked="$(_pick_tag_from_releases_json "$json" || true)"
    if [[ -n "$picked" ]]; then
      echo "$picked"
      return
    fi
  fi
  echo "${NEW_API_FALLBACK_TAG}"
}

_download_install() {
  require_curl
  local tag asset url tmp
  tag="$(_resolve_tag)"
  asset="$(_asset_name_for_tag "$tag")"
  url="https://github.com/${NEW_API_GITHUB_REPO}/releases/download/${tag}/${asset}"
  echo "==> 下载 new-api ${tag} → ${NEW_API_BIN}" >&2
  echo "    ${url}" >&2
  tmp="$(mktemp)"
  trap '[[ -n "${tmp-}" ]] && rm -f "${tmp}"' RETURN
  _nlt_github_download_print_accel_hint
  if declare -F nlt_pb_curl_to_file >/dev/null 2>&1; then
    NLT_PB_LABEL="new-api ${tag}" nlt_pb_curl_to_file "$url" "${tmp}" || die "下载失败: ${url}"
  else
    _nlt_github_download_curl -fsSL "$url" -o "${tmp}"
  fi
  mkdir -p "${NEW_API_SERVICE_HOME}/bin"
  install -m 0755 "${tmp}" "${NEW_API_BIN}"
  rm -f "${tmp}"
  trap - RETURN
  [[ -x "${NEW_API_BIN}" ]] || die "安装后二进制不可执行: ${NEW_API_BIN}"
  echo "已安装 ${NEW_API_BIN}（${tag} / ${asset}）"
}

cmd_install() {
  ensure_dirs
  _download_install
}

cmd_update() {
  ensure_dirs
  echo "==> 更新 new-api（重新下载）…" >&2
  _download_install
}

cmd_start() {
  [[ -x "${NEW_API_BIN}" ]] || die "未安装，请先: $0 install"
  ensure_dirs
  local existing
  existing="$(read_pid)"
  if [[ -n "$existing" ]] && process_alive "$existing"; then
    echo "new-api 已在运行（PID ${existing}）。重启请: $0 restart" >&2
    exit 1
  fi
  local effective_port listener_pid
  effective_port="$(_new_api_effective_port)"
  listener_pid="$(listener_pid_for_port "${effective_port}")"
  if [[ -n "$listener_pid" ]]; then
    die "端口 ${effective_port} 已被 PID ${listener_pid} 占用，请先执行 $0 stop 或手动清理。"
  fi
  rm -f "$PID_FILE"
  echo "==> 启动 new-api，必须先 cd 到数据目录再启动（否则 godotenv 找不到 .env）；日志 ${LOG_FILE}" >&2
  if [[ -f "${NEW_API_DATA_DIR}/.env" ]] && [[ "${NEW_API_FORCE_SCRIPT_PORT:-}" != "1" ]]; then
    echo "    检测到 .env：不注入 PORT，由程序加载 ${NEW_API_DATA_DIR}/.env（需覆盖请设 NEW_API_FORCE_SCRIPT_PORT=1）" >&2
  else
    echo "    PORT=${NEW_API_PORT}（注入进程环境）" >&2
  fi
  # 必须在子 shell 内先 cd 再 nohup：进程 cwd 才是 ${NEW_API_DATA_DIR}，与上游 godotenv.Load(\".env\") 一致
  local cpid
  cpid="$(
    cd "${NEW_API_DATA_DIR}" || {
      echo "错误: 无法 cd 到 ${NEW_API_DATA_DIR}" >&2
      exit 1
    }
    echo "    [启动] 工作目录: $(pwd -P)" >&2
    # 上游 main：先 godotenv.Load(".env")，且默认不覆盖已存在环境变量；故存在 .env 时不预先 export PORT
    if [[ -f "${NEW_API_DATA_DIR}/.env" ]] && [[ "${NEW_API_FORCE_SCRIPT_PORT:-}" != "1" ]]; then
      echo "    [启动] 将执行: cd $(printf '%q' "${NEW_API_DATA_DIR}") && nohup env -u PORT $(printf '%q' "${NEW_API_BIN}") >>$(printf '%q' "${LOG_FILE}") 2>&1 &" >&2
      nohup env -u PORT "${NEW_API_BIN}" >>"${LOG_FILE}" 2>&1 &
    else
      echo "    [启动] 将执行: cd $(printf '%q' "${NEW_API_DATA_DIR}") && nohup env PORT=$(printf '%q' "${NEW_API_PORT}") $(printf '%q' "${NEW_API_BIN}") >>$(printf '%q' "${LOG_FILE}") 2>&1 &" >&2
      nohup env PORT="${NEW_API_PORT}" "${NEW_API_BIN}" >>"${LOG_FILE}" 2>&1 &
    fi
    echo $!
  )" || die "启动失败"
  echo "$cpid" >"$PID_FILE"
  sleep 1
  existing="$(read_pid)"
  if [[ -n "$existing" ]] && process_alive "$existing"; then
    echo "已启动 PID ${existing}（探测 http://127.0.0.1:$(_new_api_effective_port)/ ）"
  else
    echo "警告: 进程可能已退出，请查看: tail -80 ${LOG_FILE}" >&2
  fi
}

cmd_run() {
  [[ -x "${NEW_API_BIN}" ]] || die "未安装，请先: $0 install"
  ensure_dirs
  local existing
  existing="$(read_pid)"
  if [[ -n "$existing" ]] && process_alive "$existing"; then
    echo "new-api 已在后台运行（PID ${existing}）。请先 $0 stop，再使用 run。" >&2
    exit 1
  fi
  if [[ -f "${NEW_API_DATA_DIR}/.env" ]] && [[ "${NEW_API_FORCE_SCRIPT_PORT:-}" != "1" ]]; then
    echo "==> 前台启动 new-api（先 cd 数据目录再 exec，否则 .env 不加载）；Ctrl+C 退出；不写 PID" >&2
    cd "${NEW_API_DATA_DIR}" || die "无法 cd 到 ${NEW_API_DATA_DIR}"
    echo "    [前台] 工作目录: $(pwd -P)" >&2
    echo "    [前台] 将执行: exec env -u PORT $(printf '%q' "${NEW_API_BIN}")" >&2
    exec env -u PORT "${NEW_API_BIN}"
  else
    echo "==> 前台启动 new-api，PORT=${NEW_API_PORT}（先 cd 数据目录再 exec）；Ctrl+C 退出；不写 PID" >&2
    cd "${NEW_API_DATA_DIR}" || die "无法 cd 到 ${NEW_API_DATA_DIR}"
    echo "    [前台] 工作目录: $(pwd -P)" >&2
    echo "    [前台] 将执行: exec env PORT=$(printf '%q' "${NEW_API_PORT}") $(printf '%q' "${NEW_API_BIN}")" >&2
    exec env PORT="${NEW_API_PORT}" "${NEW_API_BIN}"
  fi
}

# cmd_stop [--yes]
#   --yes 跳过确认（供 restart / uninstall 调用，避免二次追问）。
#   用户拒绝时返回 1（而非 exit 0），否则 restart/uninstall 会被静默截断。
cmd_stop() {
  local assume_yes=""
  [[ "${1:-}" == "--yes" ]] && assume_yes=1
  local pid
  pid="$(read_pid)"
  local effective_port listener_pid
  effective_port="$(_new_api_effective_port)"
  listener_pid="$(listener_pid_for_port "${effective_port}")"
  if [[ -z "$pid" ]]; then
    echo "未找到 PID，视为未运行。" >&2
    rm -f "$PID_FILE"
  elif ! process_alive "$pid"; then
    rm -f "$PID_FILE"
  fi
  if [[ -z "$assume_yes" ]] && [[ -n "$pid" || -n "$listener_pid" ]] && nlt_interactive; then
    # 用 nlt_ui_confirm 而非裸 gum：缺 gum 时降级为 read y/N，
    # 而不是返回 127 让 `|| exit 0` 谎报「已停止」。
    nlt_ui_confirm "停止 new-api（PID ${pid}）？" || return 1
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

cmd_restart() {
  cmd_stop --yes || die "停止失败，已中止 restart（服务可能仍在运行）"
  cmd_start
}

cmd_status() {
  local pid
  pid="$(read_pid)"
  local effective_port listener_pid
  effective_port="$(_new_api_effective_port)"
  listener_pid="$(listener_pid_for_port "${effective_port}")"
  echo "NEW_API_SERVICE_HOME=${NEW_API_SERVICE_HOME}"
  echo "NEW_API_DATA_DIR=${NEW_API_DATA_DIR}"
  echo "NEW_API_PORT=${NEW_API_PORT}（无 .env 或未解析时用；实际监听见下）"
  echo "探测端口（.env 优先）: ${effective_port}"
  if [[ -n "$pid" ]] && process_alive "$pid"; then
    if [[ -n "$listener_pid" && "$listener_pid" != "$pid" ]]; then
      echo "状态: 运行中 PID ${pid}（监听进程 PID ${listener_pid}）"
    else
      echo "状态: 运行中 PID ${pid}"
    fi
  elif [[ -n "$listener_pid" ]]; then
    echo "状态: 监听中（PID 文件缺失/失效，监听进程 PID ${listener_pid}）"
  else
    echo "状态: 未运行"
    rm -f "$PID_FILE"
  fi
  if [[ -x "${NEW_API_BIN}" ]]; then
    echo "二进制: $("${NEW_API_BIN}" --version 2>/dev/null | head -1 || echo ok)"
  fi
  if command -v curl >/dev/null 2>&1; then
    echo ""
    echo "==> 探测 http://127.0.0.1:${effective_port}/"
    curl -sS -m 3 -o /dev/null -w "HTTP %{http_code}\n" "http://127.0.0.1:${effective_port}/" || echo "（无法连接）"
  fi
}

cmd_uninstall() {
  echo "将删除: ${NEW_API_SERVICE_HOME}" >&2
  # 先确认再动手：确认通过后 cmd_stop 用 --yes，避免二次追问把卸载拦腰截断。
  nlt_confirm_destructive "确认删除 ${NEW_API_SERVICE_HOME}？" NEW_API_UNINSTALL_YES || return 1
  cmd_stop --yes || nlt_ui_warn "停止失败，仍继续删除。"
  nlt_safe_rm "${NEW_API_SERVICE_HOME}" || return 1
  echo "已删除。"
}

dispatch() {
  local cmd="${1:-}"
  shift || true
  case "$cmd" in
    install) cmd_install ;;
    update) cmd_update ;;
    start) cmd_start ;;
    run) cmd_run ;;
    stop) cmd_stop ;;
    restart) cmd_restart ;;
    status) cmd_status ;;
    uninstall) cmd_uninstall ;;
    help | -h | --help) usage ;;
    *)
      echo "未知命令: ${cmd}" >&2
      usage >&2
      exit 2
      ;;
  esac
}

interactive_main() {
  declare -F nlt_ui_apply_theme >/dev/null 2>&1 && nlt_ui_apply_theme
  if declare -F nlt_ui_banner >/dev/null 2>&1; then
    nlt_ui_banner "new-api 本地服务（QuantumNous/new-api）" "安装目录: ${NEW_API_SERVICE_HOME}" "数据目录: ${NEW_API_DATA_DIR}  端口: ${NEW_API_PORT}"
  else
    gum style --bold --foreground 212 "new-api 本地服务（QuantumNous/new-api）"
    gum style "安装目录: ${NEW_API_SERVICE_HOME}"
    gum style "数据目录: ${NEW_API_DATA_DIR}  端口: ${NEW_API_PORT}"
  fi
  echo ""
  set +e
  while true; do
    local pick
    pick="$(nlt_ui_choose "nltdeploy / service / new-api / 选择动作" \
      "install" "update" "start" "run" "stop" "restart" "status" "uninstall" "help" "quit")" || break
    [[ -z "$pick" ]] && break
    case "$pick" in
      quit) break ;;
      help) usage; continue ;;
    esac
    ( dispatch "$pick" )
    echo ""
  done
  set -e
}

main() {
  if [[ $# -gt 0 ]]; then
    case "$1" in
      help | -h | --help)
        dispatch "$@"
        return 0
        ;;
    esac
  fi
  _nlt_ensure_gum || exit 1
  if [[ $# -eq 0 ]]; then
    interactive_main
    return 0
  fi
  dispatch "$@"
}

main "$@"
