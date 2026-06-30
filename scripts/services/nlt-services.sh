#!/usr/bin/env bash
# nlt-services：服务总览（原 nlt-services-status）与各模块安装入口聚合。
#
# 用法:
#   nlt-services                    # gum：status / install / help / quit
#   nlt-services status [--no-http]
#   nlt-services install            # gum：先选「安装 / 卸载」，再选模块
#   nlt-services install add <名>   # 安装类（install 与 add 同义）
#   nlt-services install remove <名> # 卸载类（uninstall 与 remove 同义）
#   nlt-services help
#
# 模块名: airflow, celery, paperclip, code-server, new-api,
#         pip-sources, python-env, utils, github-net
# 卸载不支持: celery、utils（脚本未提供 uninstall）
#
# NONINTERACTIVE=1 且无参数时打印 help 并退出（不进入 gum）。

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../lib/nlt-common.sh
source "${SCRIPT_DIR}/../lib/nlt-common.sh"

NLTDEPLOY_ROOT="${NLTDEPLOY_ROOT:-${HOME}/.local/nltdeploy}"
NLT_BIN="${NLTDEPLOY_ROOT}/bin"

die() { echo "错误: $*" >&2; exit 1; }

usage() {
  cat <<'EOF'
用法: nlt-services [command [args...]]

  无参数：gum 菜单（status / install / help / quit）。

命令:
  status [--no-http]    汇总 Airflow、Celery、Paperclip、code-server、new-api（gum table -p）
  install               无参：gum 先选「安装 / 卸载」，再选模块。
  install add <模块>    安装（add 可写 install）
  install remove <模块> 卸载（remove 可写 uninstall；celery/utils 不支持）
  help / -h / --help    本说明

说明:
  - status 与各域默认路径、环境变量一致，详见各服务脚本头部。
  - pip-sources / python-env / utils / github-net 无统一守护进程，status 中仅文末提示。
EOF
}

usage_status() {
  cat <<'EOF'
用法: nlt-services status [--no-http]

  --no-http   跳过 curl 探测。
EOF
}

# ---- status 实现（DO_HTTP 由 cmd_status 设置）----
DO_HTTP=1

read_pid_file() {
  local f="$1"
  [[ -f "$f" ]] || { echo ""; return; }
  tr -d '[:space:]' <"$f" || true
}

proc_alive() {
  [[ -n "${1:-}" ]] && kill -0 "$1" 2>/dev/null
}

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

status_word() {
  if proc_alive "$1"; then
    echo "运行中"
  else
    echo "未运行"
  fi
}

http_probe() {
  local url="$1"
  if [[ "$DO_HTTP" != "1" ]]; then
    echo "（已跳过）"
    return
  fi
  command -v curl >/dev/null 2>&1 || { echo "（无 curl）"; return; }
  local code
  code="$(curl -sS -m 2 -o /dev/null -w '%{http_code}' "$url" 2>/dev/null || true)"
  if [[ -n "$code" ]]; then
    echo "HTTP ${code}"
  else
    echo "（无响应）"
  fi
}

# CSV 单元格（RFC 风格双引号，避免内容含逗号时错乱）
_csv_cell() {
  local x="${1//\"/\"\"}"
  printf '"%s"' "$x"
}

_status_csv_line() {
  printf '%s,%s,%s,%s,%s\n' \
    "$(_csv_cell "$1")" "$(_csv_cell "$2")" "$(_csv_cell "$3")" "$(_csv_cell "$4")" "$(_csv_cell "$5")"
}

# stdin 为数据行 CSV（无表头；列名由 --columns 指定）→ gum table 静态打印
_render_status_table_from_csv() {
  PATH="${HOME}/opt/gum/bin:${PATH}"
  gum table -p -s ',' \
    --columns '服务,状态,PID,端口/访问,HTTP' \
    --border rounded
}

_mark_alive() {
  proc_alive "$1" && printf '%s' '√' || printf '%s' '×'
}

cmd_status() {
  DO_HTTP=1
  local a
  for a in "$@"; do
    case "$a" in
      -h | --help | help)
        usage_status
        exit 0
        ;;
      --no-http) DO_HTTP=0 ;;
      *)
        echo "未知参数: $a（nlt-services status --help）" >&2
        exit 2
        ;;
    esac
  done

  AIRFLOW_HOME="${AIRFLOW_HOME:-${HOME}/opt/airflow}"
  AIRFLOW_PID_FILE="${AIRFLOW_HOME}/run/standalone.pid"
  DEFAULT_AIRFLOW_PORT="8806"
  AIRFLOW_PORT="${AIRFLOW__WEBSERVER__WEB_SERVER_PORT:-$DEFAULT_AIRFLOW_PORT}"
  airflow_pid="$(read_pid_file "$AIRFLOW_PID_FILE")"

  CELERY_HOME="${CELERY_HOME:-${HOME}/opt/celery}"
  CELERY_RUN="${CELERY_HOME}/run"
  FLOWER_PORT="${FLOWER_PORT:-8806}"
  FLOWER_ADDRESS="${FLOWER_ADDRESS:-0.0.0.0}"
  pid_cel_w="$(read_pid_file "${CELERY_RUN}/worker.pid")"
  pid_cel_b="$(read_pid_file "${CELERY_RUN}/beat.pid")"
  pid_cel_f="$(read_pid_file "${CELERY_RUN}/flower.pid")"

  PAPERCLIP_SERVICE_HOME="${PAPERCLIP_SERVICE_HOME:-${HOME}/opt/paperclip}"
  PAPERCLIP_PORT="${PAPERCLIP_PORT:-18804}"
  PAPERCLIP_PUBLIC_PORT="${PAPERCLIP_PUBLIC_PORT:-8804}"
  pid_pc="$(read_pid_file "${PAPERCLIP_SERVICE_HOME}/run/paperclip.pid")"
  pc_listener_pid="$(listener_pid_for_port "${PAPERCLIP_PORT}")"

  CODE_SERVER_SERVICE_HOME="${CODE_SERVER_SERVICE_HOME:-${HOME}/opt/code-server}"
  CODE_SERVER_BIND="${CODE_SERVER_BIND:-127.0.0.1:8080}"
  CODE_SERVER_PORT="${CODE_SERVER_PORT:-${CODE_SERVER_BIND##*:}}"
  pid_cs="$(read_pid_file "${CODE_SERVER_SERVICE_HOME}/run/code-server.pid")"

  NEW_API_SERVICE_HOME="${NEW_API_SERVICE_HOME:-${HOME}/opt/new-api}"
  NEW_API_PORT="${NEW_API_PORT:-8801}"
  pid_na="$(read_pid_file "${NEW_API_SERVICE_HOME}/run/new-api.pid")"

  local ts flower_url cel_wbf cel_pids cel_probe
  ts="$(date '+%Y-%m-%d %H:%M:%S %z')"

  if [[ "$FLOWER_ADDRESS" == "0.0.0.0" ]]; then
    flower_url="http://127.0.0.1:${FLOWER_PORT}/"
  else
    flower_url="http://${FLOWER_ADDRESS}:${FLOWER_PORT}/"
  fi
  cel_probe="$(http_probe "$flower_url")"
  cel_wbf="$(_mark_alive "$pid_cel_w")$(_mark_alive "$pid_cel_b")$(_mark_alive "$pid_cel_f")"
  cel_pids="${pid_cel_w:--}/${pid_cel_b:--}/${pid_cel_f:--}"

  echo "nltdeploy 服务概览  ${ts}"
  echo ""

  PATH="${HOME}/opt/gum/bin:${PATH}"
  {
    _status_csv_line \
      "airflow" \
      "$(status_word "$airflow_pid")" \
      "${airflow_pid:--}" \
      "${AIRFLOW_PORT} → 127.0.0.1:${AIRFLOW_PORT}" \
      "$(http_probe "http://127.0.0.1:${AIRFLOW_PORT}/")"
    _status_csv_line \
      "celery" \
      "wbf ${cel_wbf}" \
      "${cel_pids}" \
      "flower ${FLOWER_PORT} (${FLOWER_ADDRESS})" \
      "${cel_probe}"
    _status_csv_line \
      "paperclip" \
      "$(if proc_alive "$pid_pc" || [[ -n "$pc_listener_pid" ]]; then echo "运行中"; else echo "未运行"; fi)" \
      "${pid_pc:--}${pc_listener_pid:+ / listen:${pc_listener_pid}}" \
      "${PAPERCLIP_PUBLIC_PORT} -> 127.0.0.1:${PAPERCLIP_PORT}" \
      "$(http_probe "http://127.0.0.1:${PAPERCLIP_PUBLIC_PORT}/api/health")"
    _status_csv_line \
      "code-server" \
      "$(status_word "$pid_cs")" \
      "${pid_cs:--}" \
      "${CODE_SERVER_BIND}" \
      "$(http_probe "http://127.0.0.1:${CODE_SERVER_PORT}/")"
    _status_csv_line \
      "new-api" \
      "$(status_word "$pid_na")" \
      "${pid_na:--}" \
      "${NEW_API_PORT} → 127.0.0.1:${NEW_API_PORT}" \
      "$(http_probe "http://127.0.0.1:${NEW_API_PORT}/")"
  } | _render_status_table_from_csv

  echo ""
  echo "说明:"
  echo "  • celery 状态列 wbf 为 worker / beat / flower：√ 运行中，× 未运行；与 Airflow 同机时请区分 FLOWER_PORT。"
  echo "  • 安装路径: airflow ${AIRFLOW_HOME} | celery ${CELERY_HOME} | paperclip ${PAPERCLIP_SERVICE_HOME} | code-server ${CODE_SERVER_SERVICE_HOME} | new-api ${NEW_API_SERVICE_HOME}"
  echo "  • 详情: nlt-airflow / nlt-celery / nlt-paperclip / nlt-code-server / nlt-new-api 各 status"
  echo ""
  echo "工具（无统一守护进程）: nlt-dev（推荐入口）/ nlt-pip-sources / nlt-python-env / nlt-utils / nlt-github-net — 请用各命令单独查看。"
  echo ""
}

# 是否支持 uninstall（上游脚本有该子命令）
_module_supports_uninstall() {
  case "$1" in
    airflow | paperclip | code-server | new-api | pip-sources | python-env | github-net) return 0 ;;
    *) return 1 ;;
  esac
}

_dispatch_install_or_remove() {
  local action="$1"
  local name="$2"

  [[ -d "$NLT_BIN" ]] || die "未找到 ${NLT_BIN}，请先执行 install.sh"

  if [[ "$action" == "remove" ]]; then
    _module_supports_uninstall "$name" || die "模块「${name}」不支持卸载（或请使用各命令自带的 uninstall）"
    case "$name" in
      airflow) exec "${NLT_BIN}/nlt-airflow" uninstall ;;
      paperclip) exec "${NLT_BIN}/nlt-paperclip" uninstall ;;
      code-server) exec "${NLT_BIN}/nlt-code-server" uninstall ;;
      new-api) exec "${NLT_BIN}/nlt-new-api" uninstall ;;
      pip-sources) exec "${NLT_BIN}/nlt-pip-sources" uninstall ;;
      python-env) exec "${NLT_BIN}/nlt-python-env" uninstall ;;
      github-net) exec "${NLT_BIN}/nlt-github-net" uninstall ;;
      *) die "内部错误: remove ${name}" ;;
    esac
  fi

  case "$name" in
    airflow) exec "${NLT_BIN}/nlt-airflow" install ;;
    celery) exec "${NLT_BIN}/nlt-celery" install ;;
    paperclip) exec "${NLT_BIN}/nlt-paperclip" install ;;
    code-server) exec "${NLT_BIN}/nlt-code-server" install ;;
    new-api) exec "${NLT_BIN}/nlt-new-api" install ;;
    pip-sources) exec "${NLT_BIN}/nlt-pip-sources" ;;
    python-env) exec "${NLT_BIN}/nlt-python-env" ;;
    utils) exec "${NLT_BIN}/nlt-utils" ;;
    github-net) exec "${NLT_BIN}/nlt-github-net" ;;
    *) die "未知模块: ${name}（见 nlt-services help）" ;;
  esac
}

cmd_install() {
  local action="" name=""
  [[ -d "$NLT_BIN" ]] || die "未找到 ${NLT_BIN}，请先执行 install.sh"

  if [[ $# -eq 0 ]]; then
    if [[ "${NONINTERACTIVE:-}" == "1" ]]; then
      die "NONINTERACTIVE=1 时请使用: nlt-services install add|remove <模块>"
    fi
    _nlt_ensure_gum || exit 1
    action="$(gum choose --header "要对模块做什么？" \
      "安装" \
      "卸载" \
      "取消")" || return 0
    [[ -z "$action" || "$action" == "取消" ]] && return 0
    case "$action" in
      安装) action="add" ;;
      卸载) action="remove" ;;
      *) return 0 ;;
    esac

    if [[ "$action" == "add" ]]; then
      name="$(gum choose --header "选择要安装 / 初始化的模块" \
        "airflow" "celery" "paperclip" "code-server" "new-api" \
        "pip-sources" "python-env" "utils" "github-net" "取消")" || return 0
    else
      name="$(gum choose --header "选择要卸载的模块（celery、utils 请手动清理）" \
        "airflow" "paperclip" "code-server" "new-api" \
        "pip-sources" "python-env" "github-net" "取消")" || return 0
    fi
    [[ -z "$name" || "$name" == "取消" ]] && return 0
    _dispatch_install_or_remove "$action" "$name"
    return
  fi

  case "${1:-}" in
    add | install)
      shift
      name="${1:-}"
      if [[ -z "$name" ]]; then
        [[ "${NONINTERACTIVE:-}" == "1" ]] && die "请指定模块: nlt-services install add <模块>"
        _nlt_ensure_gum || exit 1
        name="$(gum choose --header "选择要安装 / 初始化的模块" \
          "airflow" "celery" "paperclip" "code-server" "new-api" \
          "pip-sources" "python-env" "utils" "github-net" "取消")" || return 0
        [[ -z "$name" || "$name" == "取消" ]] && return 0
      fi
      _dispatch_install_or_remove "add" "$name"
      ;;
    remove | uninstall)
      shift
      name="${1:-}"
      if [[ -z "$name" ]]; then
        [[ "${NONINTERACTIVE:-}" == "1" ]] && die "请指定模块: nlt-services install remove <模块>"
        _nlt_ensure_gum || exit 1
        name="$(gum choose --header "选择要卸载的模块" \
          "airflow" "paperclip" "code-server" "new-api" \
          "pip-sources" "python-env" "github-net" "取消")" || return 0
        [[ -z "$name" || "$name" == "取消" ]] && return 0
      fi
      _dispatch_install_or_remove "remove" "$name"
      ;;
    *)
      die "未知子命令: ${1}（使用: nlt-services install add|remove <模块>）"
      ;;
  esac
}

interactive_main() {
  _nlt_ensure_gum || exit 1
  set +e
  while true; do
    local pick
    pick="$(gum choose --header "nlt-services" \
      "status" "install" "help" "quit")" || break
    [[ -z "$pick" ]] && break
    case "$pick" in
      quit) break ;;
      help) usage; echo "" ;;
      status) cmd_status ;;
      install) cmd_install ;;
    esac
    echo ""
  done
  set -e
}

main() {
  if [[ $# -eq 0 ]]; then
    if [[ "${NONINTERACTIVE:-}" == "1" ]]; then
      usage >&2
      exit 1
    fi
    interactive_main
    return 0
  fi

  case "$1" in
    status)
      shift
      cmd_status "$@"
      ;;
    install)
      shift
      cmd_install "$@"
      ;;
    help | -h | --help)
      usage
      ;;
    *)
      echo "未知命令: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
}

main "$@"
