#!/usr/bin/env bash
# fundeploy service：服务总览、路由与安装入口聚合。
#
# 用法:
#   fundeploy service                    # gum：status / install / help / quit
#   fundeploy service status [--no-http]
#   fundeploy service <服务> [动作...]   # 透传给对应服务
#   fundeploy service install            # gum：先选「安装 / 卸载」，再选模块
#   fundeploy service install add <名>    # 安装类（install 与 add 同义）
#   fundeploy service install remove <名> # 卸载类（uninstall 与 remove 同义）
#   fundeploy service help
#
# 模块名: airflow, celery, paperclip, code-server, new-api, sub2api, open-pencil,
#         funflix-web, pip-sources, python-env, utils, github-net, cockpit-tools
# 卸载不支持: celery、utils（脚本未提供 uninstall）
#
# NONINTERACTIVE=1 且无参数时打印 help 并退出（不进入 gum）。

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../lib/fundeploy-common.sh
source "${SCRIPT_DIR}/../lib/fundeploy-common.sh"

die() { echo "错误: $*" >&2; exit 1; }

usage() {
  cat <<'EOF'
用法: fundeploy service <服务> [动作...]
      fundeploy service status [--no-http]
      fundeploy service list

  无参数：打开可搜索的服务菜单。

命令:
  status [--no-http]      汇总所有服务状态
  list                    列出服务
  <服务> [动作...]        透传 install/start/stop/restart/status 等动作
  install                 兼容原安装聚合菜单
  help / -h / --help      本说明

服务: airflow celery paperclip code-server new-api sub2api open-pencil funflix-web

示例:
  fundeploy service code-server official install
  fundeploy service code-server official start
  fundeploy service sub2api official install
  fundeploy service sub2api official restart
  fundeploy service funflix-web install
  fundeploy service funflix-web start
  fundeploy service status
EOF
}

_SERVICE_NAMES=(airflow celery paperclip code-server new-api sub2api open-pencil funflix-web)

_service_known() {
  local name="$1" service
  for service in "${_SERVICE_NAMES[@]}"; do
    [[ "$service" == "$name" ]] && return 0
  done
  return 1
}

cmd_list() { printf '%s\n' "${_SERVICE_NAMES[@]}"; }

_resolve_module_entry() {
  local name="$1" candidate
  for candidate in \
    "${SCRIPT_DIR}/${name}/setup.sh" \
    "${SCRIPT_DIR}/../${name}/setup.sh" \
    "${SCRIPT_DIR}/../tools/${name}/setup.sh"; do
    [[ -f "$candidate" ]] && { printf '%s\n' "$candidate"; return 0; }
  done
  return 1
}

_dispatch_service() {
  local name="$1" target
  shift
  target="$(_resolve_module_entry "$name")" || die "找不到服务入口: ${name}"
  exec bash "$target" "$@"
}

_open_service_menu() {
  local target
  target="$(_resolve_module_entry "$1")" || die "找不到服务入口: $1"
  bash "$target"
}

usage_status() {
  cat <<'EOF'
用法: fundeploy service status [--no-http]

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
  _fundeploy_listener_pid_for_port "$1"
}

service_state_from_pid_and_port() {
  local pid="${1:-}"
  local listener_pid="${2:-}"
  if proc_alive "$pid" || [[ -n "$listener_pid" ]]; then
    echo "运行中"
  else
    echo "未运行"
  fi
}

service_pid_display() {
  local pid="${1:-}"
  local listener_pid="${2:-}"
  if [[ -n "$pid" ]]; then
    printf '%s' "$pid"
    if [[ -n "$listener_pid" && "$listener_pid" != "$pid" ]]; then
      printf ' / listen:%s' "$listener_pid"
    fi
    return
  fi
  if [[ -n "$listener_pid" ]]; then
    printf 'listen:%s' "$listener_pid"
    return
  fi
  printf '%s' '-'
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
  if command -v gum >/dev/null 2>&1; then
    gum table -p -s ',' \
      --columns '服务,状态,PID,端口/访问,HTTP' \
      --border rounded
  else
    printf '%s\n' '服务,状态,PID,端口/访问,HTTP'
    cat
  fi
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
        echo "未知参数: $a（fundeploy service status --help）" >&2
        exit 2
        ;;
    esac
  done

  AIRFLOW_HOME="${AIRFLOW_HOME:-${HOME}/opt/airflow}"
  AIRFLOW_PID_FILE="${AIRFLOW_HOME}/run/standalone.pid"
  DEFAULT_AIRFLOW_PORT="8806"
  AIRFLOW_PORT="${AIRFLOW__WEBSERVER__WEB_SERVER_PORT:-$DEFAULT_AIRFLOW_PORT}"
  airflow_pid="$(read_pid_file "$AIRFLOW_PID_FILE")"
  airflow_listener_pid="$(listener_pid_for_port "${AIRFLOW_PORT}")"

  CELERY_HOME="${CELERY_HOME:-${HOME}/opt/celery}"
  CELERY_RUN="${CELERY_HOME}/run"
  FLOWER_PORT="${FLOWER_PORT:-8806}"
  FLOWER_ADDRESS="${FLOWER_ADDRESS:-0.0.0.0}"
  pid_cel_w="$(read_pid_file "${CELERY_RUN}/worker.pid")"
  pid_cel_b="$(read_pid_file "${CELERY_RUN}/beat.pid")"
  pid_cel_f="$(read_pid_file "${CELERY_RUN}/flower.pid")"

  PAPERCLIP_SERVICE_HOME="${PAPERCLIP_SERVICE_HOME:-${HOME}/opt/paperclip}"
  PAPERCLIP_PORT="${PAPERCLIP_PORT:-8804}"
  PAPERCLIP_HOST="${PAPERCLIP_HOST:-0.0.0.0}"
  pid_pc="$(read_pid_file "${PAPERCLIP_SERVICE_HOME}/run/paperclip.pid")"
  pc_listener_pid="$(listener_pid_for_port "${PAPERCLIP_PORT}")"

  CODE_SERVER_SERVICE_HOME="${CODE_SERVER_SERVICE_HOME:-${HOME}/opt/code-server}"
  CODE_SERVER_BIND="${CODE_SERVER_BIND:-127.0.0.1:8080}"
  CODE_SERVER_PORT="${CODE_SERVER_PORT:-${CODE_SERVER_BIND##*:}}"
  pid_cs="$(read_pid_file "${CODE_SERVER_SERVICE_HOME}/run/code-server.pid")"
  cs_listener_pid="$(listener_pid_for_port "${CODE_SERVER_PORT}")"

  NEW_API_SERVICE_HOME="${NEW_API_SERVICE_HOME:-${HOME}/opt/new-api}"
  NEW_API_PORT="${NEW_API_PORT:-8801}"
  pid_na="$(read_pid_file "${NEW_API_SERVICE_HOME}/run/new-api.pid")"
  na_listener_pid="$(listener_pid_for_port "${NEW_API_PORT}")"

  SUB2API_SERVICE_HOME="${SUB2API_SERVICE_HOME:-${HOME}/opt/sub2api}"
  SUB2API_HOST="${SUB2API_HOST:-127.0.0.1}"
  SUB2API_PORT="${SUB2API_PORT:-8802}"
  pid_s2a="$(read_pid_file "${SUB2API_SERVICE_HOME}/run/sub2api.pid")"
  s2a_listener_pid="$(listener_pid_for_port "${SUB2API_PORT}")"

  # funflix / funflix-web 各自管理自己的 PID 文件，这里只用端口探测判断是否在跑。
  FUNFLIX_WEB_BACKEND_HOST="${FUNFLIX_WEB_BACKEND_HOST:-127.0.0.1}"
  FUNFLIX_WEB_BACKEND_PORT="${FUNFLIX_WEB_BACKEND_PORT:-18810}"
  FUNFLIX_WEB_FRONTEND_HOST="${FUNFLIX_WEB_FRONTEND_HOST:-127.0.0.1}"
  FUNFLIX_WEB_FRONTEND_PORT="${FUNFLIX_WEB_FRONTEND_PORT:-8810}"
  ffx_be_listener_pid="$(listener_pid_for_port "${FUNFLIX_WEB_BACKEND_PORT}")"
  ffx_fe_listener_pid="$(listener_pid_for_port "${FUNFLIX_WEB_FRONTEND_PORT}")"

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
  cel_flower_listener_pid="$(listener_pid_for_port "${FLOWER_PORT}")"

  echo "fundeploy 服务概览  ${ts}"
  echo ""

  PATH="${HOME}/opt/gum/bin:${PATH}"
  {
    _status_csv_line \
      "airflow" \
      "$(service_state_from_pid_and_port "$airflow_pid" "$airflow_listener_pid")" \
      "$(service_pid_display "$airflow_pid" "$airflow_listener_pid")" \
      "${AIRFLOW_PORT} → 127.0.0.1:${AIRFLOW_PORT}" \
      "$(http_probe "http://127.0.0.1:${AIRFLOW_PORT}/")"
    _status_csv_line \
      "celery" \
      "wbf ${cel_wbf}" \
      "${cel_pids}${cel_flower_listener_pid:+ / flower-listen:${cel_flower_listener_pid}}" \
      "flower ${FLOWER_PORT} (${FLOWER_ADDRESS})" \
      "${cel_probe}"
    _status_csv_line \
      "paperclip" \
      "$(service_state_from_pid_and_port "$pid_pc" "$pc_listener_pid")" \
      "$(service_pid_display "$pid_pc" "$pc_listener_pid")" \
      "${PAPERCLIP_HOST}:${PAPERCLIP_PORT}" \
      "$(http_probe "http://127.0.0.1:${PAPERCLIP_PORT}/api/health")"
    _status_csv_line \
      "code-server" \
      "$(service_state_from_pid_and_port "$pid_cs" "$cs_listener_pid")" \
      "$(service_pid_display "$pid_cs" "$cs_listener_pid")" \
      "${CODE_SERVER_BIND}" \
      "$(http_probe "http://127.0.0.1:${CODE_SERVER_PORT}/")"
    _status_csv_line \
      "new-api" \
      "$(service_state_from_pid_and_port "$pid_na" "$na_listener_pid")" \
      "$(service_pid_display "$pid_na" "$na_listener_pid")" \
      "${NEW_API_PORT} → 127.0.0.1:${NEW_API_PORT}" \
      "$(http_probe "http://127.0.0.1:${NEW_API_PORT}/")"
    _status_csv_line \
      "sub2api" \
      "$(service_state_from_pid_and_port "$pid_s2a" "$s2a_listener_pid")" \
      "$(service_pid_display "$pid_s2a" "$s2a_listener_pid")" \
      "${SUB2API_PORT} → ${SUB2API_HOST}:${SUB2API_PORT}" \
      "$(http_probe "http://${SUB2API_HOST}:${SUB2API_PORT}/health")"
    _status_csv_line \
      "funflix" \
      "$(service_state_from_pid_and_port "" "$ffx_be_listener_pid")" \
      "$(service_pid_display "" "$ffx_be_listener_pid")" \
      "${FUNFLIX_WEB_BACKEND_HOST}:${FUNFLIX_WEB_BACKEND_PORT}" \
      "$(http_probe "http://${FUNFLIX_WEB_BACKEND_HOST}:${FUNFLIX_WEB_BACKEND_PORT}/docs")"
    _status_csv_line \
      "funflix-web" \
      "$(service_state_from_pid_and_port "" "$ffx_fe_listener_pid")" \
      "$(service_pid_display "" "$ffx_fe_listener_pid")" \
      "${FUNFLIX_WEB_FRONTEND_HOST}:${FUNFLIX_WEB_FRONTEND_PORT}" \
      "$(http_probe "http://${FUNFLIX_WEB_FRONTEND_HOST}:${FUNFLIX_WEB_FRONTEND_PORT}/web")"
  } | _render_status_table_from_csv

  echo ""
  echo "说明:"
  echo "  • celery 状态列 wbf 为 worker / beat / flower：√ 运行中，× 未运行；与 Airflow 同机时请区分 FLOWER_PORT。"
  echo "  • funflix / funflix-web 各自管理自己的 PID/日志，此表仅按端口探测判断存活；一起装/起/停请用 fundeploy service funflix-web。"
  echo "  • 安装路径: airflow ${AIRFLOW_HOME} | celery ${CELERY_HOME} | paperclip ${PAPERCLIP_SERVICE_HOME} | code-server ${CODE_SERVER_SERVICE_HOME} | new-api ${NEW_API_SERVICE_HOME} | sub2api ${SUB2API_SERVICE_HOME}"
  echo "  • 详情: fundeploy service <服务> status"
  echo ""
  echo "工具（无统一守护进程）: fundeploy dev / fundeploy tool"
  echo ""
}

# 是否支持 uninstall（上游脚本有该子命令）
_module_supports_uninstall() {
  case "$1" in
    airflow | paperclip | code-server | new-api | sub2api | open-pencil | funflix-web | pip-sources | python-env | github-net | cockpit-tools) return 0 ;;
    *) return 1 ;;
  esac
}

_dispatch_install_or_remove() {
  local action="$1"
  local name="$2"
  local target
  target="$(_resolve_module_entry "$name")" || die "找不到模块入口: ${name}"

  if [[ "$action" == "remove" ]]; then
    _module_supports_uninstall "$name" || die "模块「${name}」不支持卸载（或请使用各命令自带的 uninstall）"
    exec bash "$target" uninstall
  fi

  case "$name" in
    pip-sources | python-env | utils | github-net) exec bash "$target" ;;
    airflow | celery | paperclip | code-server | new-api | sub2api | open-pencil | funflix-web | cockpit-tools)
      exec bash "$target" install
      ;;
    *) die "未知模块: ${name}（见 fundeploy service help）" ;;
  esac
}

cmd_install() {
  local action="" name=""

  if [[ $# -eq 0 ]]; then
    if [[ "${NONINTERACTIVE:-}" == "1" ]]; then
      die "NONINTERACTIVE=1 时请使用: fundeploy service install add|remove <模块>"
    fi
    _fundeploy_ensure_gum || exit 1
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
        "airflow" "celery" "paperclip" "code-server" "new-api" "sub2api" "open-pencil" "funflix-web" \
        "pip-sources" "python-env" "utils" "github-net" "cockpit-tools" "取消")" || return 0
    else
      name="$(gum choose --header "选择要卸载的模块（celery、utils 请手动清理）" \
        "airflow" "paperclip" "code-server" "new-api" "sub2api" "open-pencil" "funflix-web" \
        "pip-sources" "python-env" "github-net" "cockpit-tools" "取消")" || return 0
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
        [[ "${NONINTERACTIVE:-}" == "1" ]] && die "请指定模块: fundeploy service install add <模块>"
        _fundeploy_ensure_gum || exit 1
        name="$(gum choose --header "选择要安装 / 初始化的模块" \
          "airflow" "celery" "paperclip" "code-server" "new-api" "sub2api" "open-pencil" "funflix-web" \
          "pip-sources" "python-env" "utils" "github-net" "cockpit-tools" "取消")" || return 0
        [[ -z "$name" || "$name" == "取消" ]] && return 0
      fi
      _dispatch_install_or_remove "add" "$name"
      ;;
    remove | uninstall)
      shift
      name="${1:-}"
      if [[ -z "$name" ]]; then
        [[ "${NONINTERACTIVE:-}" == "1" ]] && die "请指定模块: fundeploy service install remove <模块>"
        _fundeploy_ensure_gum || exit 1
        name="$(gum choose --header "选择要卸载的模块" \
          "airflow" "paperclip" "code-server" "new-api" "sub2api" "open-pencil" "funflix-web" \
          "pip-sources" "python-env" "github-net" "cockpit-tools" "取消")" || return 0
        [[ -z "$name" || "$name" == "取消" ]] && return 0
      fi
      _dispatch_install_or_remove "remove" "$name"
      ;;
    *)
      die "未知子命令: ${1}（使用: fundeploy service install add|remove <模块>）"
      ;;
  esac
}

interactive_main() {
  _fundeploy_ensure_gum || exit 1
  declare -F fundeploy_ui_apply_theme >/dev/null 2>&1 && fundeploy_ui_apply_theme
  if declare -F fundeploy_ui_banner >/dev/null 2>&1; then
    fundeploy_ui_banner "fundeploy / service" "安装、运行与查看本机服务" >&2
  fi
  set +e
  while true; do
    local pick name
    pick="$(fundeploy_ui_choose "fundeploy / service / 选择服务" \
      "status       全部服务状态" \
      "airflow      工作流调度" \
      "celery       异步任务队列" \
      "paperclip    AI 协作服务" \
      "code-server 浏览器 VS Code" \
      "new-api      API 网关" \
      "sub2api      订阅 API 网关" \
      "open-pencil  CLI、MCP 与桌面工具" \
      "funflix-web  影视库前后端一体化" \
      "help         命令帮助" \
      "back         返回")" || break
    [[ -z "$pick" ]] && break
    name="${pick%% *}"
    case "$name" in
      back) break ;;
      help) usage ;;
      status) cmd_status ;;
      *) _open_service_menu "$name" ;;
    esac
    echo ""
  done
  set -e
}

main() {
  local name
  if [[ $# -eq 0 ]]; then
    if [[ "${NONINTERACTIVE:-}" == "1" ]]; then
      usage >&2
      exit 1
    fi
    interactive_main
    return 0
  fi

  case "$1" in
    list | --list)
      cmd_list
      ;;
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
      if _service_known "$1"; then
        name="$1"
        shift
        _dispatch_service "$name" "$@"
      else
        echo "未知命令或服务: $1" >&2
        usage >&2
        exit 2
      fi
      ;;
  esac
}

main "$@"
