#!/usr/bin/env bash
# 本机 Apache Airflow 3.x：安装、启停、DAG 脚手架与常用 CLI 封装（仅 3.x，不兼容 2.x）。
# 与脚本所在仓库/业务无关，可置于任意目录单独使用。
# 约定与 .cursor/agents/software-ops.md 对齐：默认 AIRFLOW_HOME=~/opt/airflow，并创建 {bin,etc,data,log}；
# 依赖 gum：缺省时按 README 同款「curl -LsSf <…/scripts/tools/utils/setup.sh> | bash」安装（不经本地路径调用其它脚本）。
# 默认将 AIRFLOW_HOME 固定为 ~/opt/airflow（不再支持改到其它目录）。
# install 会安装核心 + apache-airflow-providers-fab（FAB），并执行 airflow fab-db migrate。
#
# 用法：
#   chmod +x airflow-setup.sh
#   ./airflow-setup.sh              # 无参数：gum 菜单
#   ./airflow-setup.sh install
#   ./airflow-setup.sh start
#   ./airflow-setup.sh run              # 前台 standalone（同 start 的环境变量；不写 PID；后台已在跑时拒绝）
#   ./airflow-setup.sh status
#   ./airflow-setup.sh dag-scaffold
#   ./airflow-setup.sh dags-list
#   ./airflow-setup.sh trigger example_minimal
#   ./airflow-setup.sh users-create              # 交互式（或传参同 airflow users create）
#   ./airflow-setup.sh users-list                # 列举用户（同 airflow users list）
#   ./airflow-setup.sh users-reset-password      # 重置密码（同 airflow users reset-password）
#   ./airflow-setup.sh http-trigger <dag_id> [payload.json]   # HTTP 触发 DAG（见下「HTTP 触发」）
#   ./airflow-setup.sh update          # 保留数据目录，升级依赖并执行 db / fab-db migrate
#   ./airflow-setup.sh stop
#   ./airflow-setup.sh uninstall        # 停止进程并删除 AIRFLOW_VENV 与 AIRFLOW_HOME（不可逆）
#
# 规范：NONINTERACTIVE=1 时跳过需确认的操作（见各命令说明）。
#
# HTTP 触发 DAG（Airflow 3 稳定 API，供其它系统调用；详见官方 Public API）：
#   文档: https://airflow.apache.org/docs/apache-airflow/stable/security/api.html
#        https://airflow.apache.org/docs/apache-airflow/stable/stable-rest-api-ref.html
#   1) 取 JWT:  POST {BASE}/auth/token  Content-Type: application/json
#              body: {"username":"...","password":"..."}
#              响应: {"access_token":"..."}
#   2) 触发运行: POST {BASE}/api/v2/dags/{dag_id}/dagRuns  Authorization: Bearer <token>
#              body 示例: {"conf":{"argv":["--date","2025-01-01"],"script":"/path/to/job.py"}}
#   本脚本封装: 设置 AIRFLOW_API_USERNAME / AIRFLOW_API_PASSWORD，可选 AIRFLOW_API_BASE（默认 http://127.0.0.1:8806）
#              执行 http-trigger；payload.json 缺省则用 {"conf":{}}。
#
# 环境变量：
#   AIRFLOW_VERSION           Airflow 版本（默认与下方 DEFAULT_AIRFLOW_VERSION 一致）
#   AIRFLOW_PYTHON_BIN        指定 uv venv 使用的 Python 解释器（例如 python3.11）
#
# Airflow 3 默认 SimpleAuthManager，不会注册 airflow users。本脚本在 venv 内已安装 FAB 提供方时，
# 会自动 export AIRFLOW__CORE__AUTH_MANAGER=FabAuthManager（可被你在环境中显式覆盖）。
#   AIRFLOW_ADMIN_USER        与下面两项同时设置时，install 会尝试 airflow users create（需 FAB 包与配置）
#   AIRFLOW_ADMIN_PASSWORD
#   AIRFLOW_ADMIN_EMAIL
#   AIRFLOW_API_BASE          http-trigger 用，API 根 URL（默认 http://127.0.0.1:8806）
#   AIRFLOW_API_USERNAME      http-trigger 用，与 AIRFLOW_API_PASSWORD 一起换 JWT
#   AIRFLOW_API_PASSWORD
#   AIRFLOW_UNINSTALL_YES=1   仅在非交互（无 TTY）时配合 uninstall 使用，表示确认删除
#
# 官方文档：https://airflow.apache.org/docs/apache-airflow/stable/

set -euo pipefail

DEFAULT_AIRFLOW_VERSION="3.1.8"
AIRFLOW_VERSION="${AIRFLOW_VERSION:-$DEFAULT_AIRFLOW_VERSION}"
DEFAULT_AIRFLOW_PORT="8806"

# Airflow 3：users / roles 等 CLI 仅在 FabAuthManager 下注册（需已安装 apache-airflow-providers-fab）
FAB_AUTH_MANAGER_CLASS="airflow.providers.fab.auth_manager.fab_auth_manager.FabAuthManager"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT_NAME="$(basename "${BASH_SOURCE[0]}")"

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

# 复用 lib/nlt-ui.sh 的统一输出层：原实现直接调 `gum style`，
# 缺 gum 时整条日志语句返回 127，在 set -e 下把脚本打断在无关位置。
say_info() { nlt_ui_info "$*"; }
say_warn() { nlt_ui_warn "$*"; }
die() { nlt_die "$*"; }

# 幂等操作（重装依赖 / stop）前的确认。非交互时自动通过，便于 CI/脚本 —— 这是
# 刻意行为，两个调用点（install 续做、stop）在自动化环境下本就应当继续。
# 真正不可逆的 uninstall 不走这里，另有 AIRFLOW_UNINSTALL_YES 把关。
# 交互路径改用 nlt_ui_confirm：缺 gum 时降级为 read y/N，而不是返回 127
# 被 `if ! confirm_yes` 判成「用户拒绝」从而无故中止。
confirm_yes() {
  local prompt="${1:-是否继续？}"
  nlt_interactive || return 0
  nlt_ui_confirm "$prompt"
}

usage() {
  cat <<EOF
用法: ./${SCRIPT_NAME} [command [args...]]
  不传参数时进入 gum 列表菜单。若无 gum，将按 README 用 curl 拉取 scripts/tools/utils/setup.sh 安装。

命令:
  install          在 ~/opt/airflow/venv 用 uv 创建/复用环境；按官方 constraints 安装 Airflow 核心 + FAB；db migrate + fab-db migrate
  update           保留 AIRFLOW_HOME 数据，按当前 AIRFLOW_VERSION 与 constraints 升级依赖并执行 db / fab-db migrate
  start            后台启动 airflow standalone（写入 run/standalone.pid）
  run              前台启动 airflow standalone（同 start 的 venv 与环境变量；不写 PID；后台已在跑时拒绝）
  stop             停止 standalone 进程组
  restart          stop 后 start
  status           显示 PID 与进程状态；尝试 airflow jobs check
  dag-scaffold     在 dags/ 下生成 example_minimal DAG（已存在则跳过）
  dags-list        airflow dags list
  trigger <id>     airflow dags trigger（dag_id 后参数原样透传，如 --conf '{"a":1}'）
  http-trigger     用 HTTP+JWT 触发 DAG：需 AIRFLOW_API_*；见脚本头部「HTTP 触发」
  task-test <...>  透传参数到 airflow tasks test（见官方文档）
  users-create     创建登录用户：无参数=交互问答；有参数=透传 airflow users create（需 FAB）
  users-list       列举用户（airflow users list；需 FAB + FabAuthManager，脚本在已装 fab 的 venv 内会自动设置）
  users-reset-password  重置用户密码：无参数=交互；有参数=透传 airflow users reset-password
  uninstall        停止 standalone 并删除固定目录 ~/opt/airflow（包含 venv，不可逆；非 TTY 须设 AIRFLOW_UNINSTALL_YES=1）
  help             显示本说明

交互模式: gum choose；http-trigger 需先 export AIRFLOW_API_USERNAME 与 PASSWORD。

环境变量见脚本头部注释。
EOF
}

resolve_airflow_home() {
  printf '%s' "${HOME}/opt/airflow"
}

export AIRFLOW_HOME="$(resolve_airflow_home)"
AIRFLOW_VENV="${AIRFLOW_HOME}/venv"
RUN_DIR="${AIRFLOW_HOME}/run"
LOG_DIR="${AIRFLOW_HOME}/logs"
DAG_DIR="${AIRFLOW_HOME}/dags"
PID_FILE="${RUN_DIR}/standalone.pid"
AIRFLOW_PYTHON=""

require_uv() {
  if ! command -v uv &>/dev/null; then
    echo "未找到 uv，请先安装 uv（https://docs.astral.sh/uv/）。" >&2
    exit 1
  fi
}

ensure_dirs() {
  # software-ops：在 AIRFLOW_HOME 下保留标准 opt 子目录（与 Airflow 自带 dags/logs/run 并存）
  mkdir -p "${AIRFLOW_HOME}/bin" "${AIRFLOW_HOME}/etc" "${AIRFLOW_HOME}/data" "${AIRFLOW_HOME}/log" \
    "$RUN_DIR" "$LOG_DIR" "$DAG_DIR"
}

require_python() {
  local requested="${AIRFLOW_PYTHON_BIN:-}"
  local found=""
  local major minor
  local candidates=()

  if [[ -n "$requested" ]]; then
    candidates+=("$requested")
  else
    candidates+=(
      "python3"
      "python3.13"
      "python3.12"
      "python3.11"
      "python3.10"
      "${HOME}/opt/py313/bin/python3"
      "${HOME}/opt/py312/bin/python3"
      "${HOME}/opt/py311/bin/python3"
      "${HOME}/opt/py310/bin/python3"
    )
  fi

  local py
  for py in "${candidates[@]}"; do
    if ! command -v "$py" &>/dev/null; then
      continue
    fi
    major="$("$py" -c 'import sys; print(sys.version_info[0])')"
    minor="$("$py" -c 'import sys; print(sys.version_info[1])')"
    if (( major > 3 || (major == 3 && minor >= 10) )); then
      found="$(command -v "$py")"
      break
    fi
  done

  if [[ -z "$found" ]]; then
    if command -v python3 &>/dev/null; then
      major="$(python3 -c 'import sys; print(sys.version_info[0])')"
      minor="$(python3 -c 'import sys; print(sys.version_info[1])')"
      echo "当前 Python 版本为 ${major}.${minor}，需要 >= 3.10（Airflow 3.1+ 要求 3.10–3.13）。" >&2
    else
      echo "未找到可用 Python，请先安装 Python 3.10+（Airflow 3.1+ 要求 3.10–3.13）。" >&2
    fi
    echo "可通过 AIRFLOW_PYTHON_BIN 指定解释器，例如: AIRFLOW_PYTHON_BIN=python3.11 $0 install" >&2
    exit 1
  fi

  AIRFLOW_PYTHON="$found"
  say_info "使用 Python: $("$AIRFLOW_PYTHON" --version) (${AIRFLOW_PYTHON})"
}

constraint_url() {
  local py_minor
  py_minor="$("${AIRFLOW_PYTHON}" -c 'import sys; print(f"{sys.version_info.major}.{sys.version_info.minor}")')"
  printf 'https://raw.githubusercontent.com/apache/airflow/constraints-%s/constraints-%s.txt' \
    "$AIRFLOW_VERSION" "$py_minor"
}

activate_venv() {
  # shellcheck source=/dev/null
  source "${AIRFLOW_VENV}/bin/activate"
  export AIRFLOW_HOME
  # 已装 FAB 提供方时切换为 FabAuthManager，否则 airflow users 在 Airflow 3 中不会出现
  if [[ -x "${AIRFLOW_VENV}/bin/python" ]] &&
    "${AIRFLOW_VENV}/bin/python" -c "import airflow.providers.fab" 2>/dev/null; then
    export AIRFLOW__CORE__AUTH_MANAGER="${AIRFLOW__CORE__AUTH_MANAGER:-$FAB_AUTH_MANAGER_CLASS}"
  fi
}

require_venv_airflow() {
  if [[ ! -x "${AIRFLOW_VENV}/bin/airflow" ]]; then
    echo "未找到 ${AIRFLOW_VENV}/bin/airflow，请先运行: $0 install" >&2
    exit 1
  fi
}

# airflow users 需：apache-airflow-providers-fab + FabAuthManager（Airflow 3 默认 SimpleAuthManager）
require_airflow_users_cli() {
  activate_venv
  if ! "${AIRFLOW_VENV}/bin/pip" show apache-airflow-providers-fab &>/dev/null; then
    echo "未安装 apache-airflow-providers-fab。" >&2
    echo "  venv: ${AIRFLOW_VENV}" >&2
    echo "请运行: $0 install" >&2
    exit 1
  fi
  if ! airflow users list --help &>/dev/null; then
    echo "FAB 已安装，但「airflow users」仍不可用。" >&2
    echo "  AIRFLOW_HOME=${AIRFLOW_HOME}" >&2
    echo "  AIRFLOW__CORE__AUTH_MANAGER=${AIRFLOW__CORE__AUTH_MANAGER:-（未设置，脚本会在激活 venv 且已装 fab 时设为 FabAuthManager）}" >&2
    echo "请自检: airflow users list --help" >&2
    exit 1
  fi
}

cmd_install() {
  require_uv
  require_python
  ensure_dirs
  say_info "AIRFLOW_HOME=${AIRFLOW_HOME}"
  say_info "AIRFLOW_VENV=${AIRFLOW_VENV}"
  if [[ -d "$AIRFLOW_VENV" ]] && [[ -x "${AIRFLOW_VENV}/bin/airflow" ]]; then
    if ! confirm_yes "检测到已有虚拟环境与 airflow，继续将重新安装/升级依赖，是否继续？"; then
      say_warn "已取消 install。"
      return 0
    fi
  fi
  if [[ ! -d "$AIRFLOW_VENV" ]]; then
    say_info "==> 使用 uv 创建虚拟环境..."
    uv venv --python "${AIRFLOW_PYTHON}" "${AIRFLOW_VENV}"
  fi
  activate_venv
  local curl_url
  curl_url="$(constraint_url)"
  say_info "==> 升级 pip（uv pip）..."
  uv pip install --python "${AIRFLOW_VENV}/bin/python" --upgrade pip
  say_info "==> 安装 Airflow 核心 + FAB（apache-airflow + apache-airflow-providers-fab，同一 constraints）..."
  # 不用 gum spin 包裹安装：需完整日志便于排错（software-ops：可观测性优先）
  uv pip install --python "${AIRFLOW_VENV}/bin/python" \
    "apache-airflow==${AIRFLOW_VERSION}" \
    "apache-airflow-providers-fab" \
    --constraint "${curl_url}"
  # 安装 FAB 后需再次激活环境变量（含 AIRFLOW__CORE__AUTH_MANAGER），否则 db migrate / users 仍按默认 SimpleAuthManager
  activate_venv
  say_info "==> 数据库迁移（Airflow 核心）..."
  airflow db migrate
  say_info "==> FAB 元数据迁移（用户/角色表，供 users-list / users-create）..."
  if airflow fab-db migrate; then
    :
  else
    echo "警告: airflow fab-db migrate 未成功，用户相关命令可能异常；请查看上方报错。" >&2
  fi
  if [[ -n "${AIRFLOW_ADMIN_USER:-}" && -n "${AIRFLOW_ADMIN_PASSWORD:-}" && -n "${AIRFLOW_ADMIN_EMAIL:-}" ]]; then
    say_info "==> 创建管理员用户（需 FAB 与 auth 配置；失败可改用 standalone 打印的账号）..."
    airflow users create \
      --username "$AIRFLOW_ADMIN_USER" \
      --firstname Admin \
      --lastname User \
      --role Admin \
      --email "$AIRFLOW_ADMIN_EMAIL" \
      --password "$AIRFLOW_ADMIN_PASSWORD" || {
      echo "警告: airflow users create 失败。可依赖 standalone 首次启动时在终端打印的账号。" >&2
    }
  fi
  say_info "安装完成。下一步: $0 start"
}

cmd_update() {
  require_uv
  require_venv_airflow
  ensure_dirs
  say_info "==> 更新 Airflow（保留 ${AIRFLOW_HOME} 数据与配置）"
  activate_venv
  local curl_url
  curl_url="$(constraint_url)"
  say_info "==> 升级 pip 与 Airflow 依赖（constraints）..."
  uv pip install --python "${AIRFLOW_VENV}/bin/python" --upgrade pip
  uv pip install --python "${AIRFLOW_VENV}/bin/python" \
    "apache-airflow==${AIRFLOW_VERSION}" \
    "apache-airflow-providers-fab" \
    --constraint "${curl_url}"
  activate_venv
  say_info "==> 数据库迁移（Airflow 核心）..."
  airflow db migrate
  say_info "==> FAB 元数据迁移..."
  if airflow fab-db migrate; then
    :
  else
    echo "警告: airflow fab-db migrate 未成功，请查看上方报错。" >&2
  fi
  say_info "更新完成。"
}

process_alive() {
  local pid="$1"
  kill -0 "$pid" 2>/dev/null
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

cmd_start() {
  require_venv_airflow
  ensure_dirs
  export AIRFLOW__WEBSERVER__WEB_SERVER_PORT="${AIRFLOW__WEBSERVER__WEB_SERVER_PORT:-$DEFAULT_AIRFLOW_PORT}"
  local existing
  existing="$(read_pid)"
  if [[ -n "$existing" ]] && process_alive "$existing"; then
    echo "standalone 已在运行（PID ${existing}）。如需重启请执行: $0 restart" >&2
    exit 1
  fi
  local listener_pid
  listener_pid="$(listener_pid_for_port "${AIRFLOW__WEBSERVER__WEB_SERVER_PORT}")"
  if [[ -n "$listener_pid" ]]; then
    die "端口 ${AIRFLOW__WEBSERVER__WEB_SERVER_PORT} 已被 PID ${listener_pid} 占用，请先执行 $0 stop 或手动清理。"
  fi
  activate_venv
  local airflow_cmd="${AIRFLOW_VENV}/bin/airflow"
  if [[ ! -x "$airflow_cmd" ]]; then
    echo "未找到可执行文件: ${airflow_cmd}" >&2
    exit 1
  fi
  # 本机稳定性优先：默认关闭 example DAG 并降低并发；用户显式 export 时可覆盖。
  export AIRFLOW__CORE__LOAD_EXAMPLES="${AIRFLOW__CORE__LOAD_EXAMPLES:-False}"
  export AIRFLOW__CORE__PARALLELISM="${AIRFLOW__CORE__PARALLELISM:-4}"
  export AIRFLOW__CORE__MAX_ACTIVE_TASKS_PER_DAG="${AIRFLOW__CORE__MAX_ACTIVE_TASKS_PER_DAG:-4}"
  export AIRFLOW__CORE__MAX_ACTIVE_RUNS_PER_DAG="${AIRFLOW__CORE__MAX_ACTIVE_RUNS_PER_DAG:-2}"
  export AIRFLOW__API__WORKERS="${AIRFLOW__API__WORKERS:-2}"
  local log_file="${LOG_DIR}/standalone.out.log"
  say_info "==> 启动 airflow standalone（日志: ${log_file}）..."
  # 先激活 venv，再显式调用 venv 内 airflow；nohup 保留 $! 为主进程
  nohup "${airflow_cmd}" standalone >>"$log_file" 2>&1 &
  local pid=$!
  echo "$pid" >"$PID_FILE"
  echo "已写入 PID ${pid} -> ${PID_FILE}"
  echo "UI 默认 http://localhost:${AIRFLOW__WEBSERVER__WEB_SERVER_PORT} （见 airflow.cfg）；standalone 首次运行可能在日志中打印管理员账号。"
}

cmd_run() {
  require_venv_airflow
  ensure_dirs
  local existing
  existing="$(read_pid)"
  if [[ -n "$existing" ]] && process_alive "$existing"; then
    echo "standalone 已在后台运行（PID ${existing}）。请先 $0 stop，再使用 run。" >&2
    exit 1
  fi
  activate_venv
  local airflow_cmd="${AIRFLOW_VENV}/bin/airflow"
  if [[ ! -x "$airflow_cmd" ]]; then
    echo "未找到可执行文件: ${airflow_cmd}" >&2
    exit 1
  fi
  export AIRFLOW__CORE__LOAD_EXAMPLES="${AIRFLOW__CORE__LOAD_EXAMPLES:-False}"
  export AIRFLOW__CORE__PARALLELISM="${AIRFLOW__CORE__PARALLELISM:-4}"
  export AIRFLOW__CORE__MAX_ACTIVE_TASKS_PER_DAG="${AIRFLOW__CORE__MAX_ACTIVE_TASKS_PER_DAG:-4}"
  export AIRFLOW__CORE__MAX_ACTIVE_RUNS_PER_DAG="${AIRFLOW__CORE__MAX_ACTIVE_RUNS_PER_DAG:-2}"
  export AIRFLOW__API__WORKERS="${AIRFLOW__API__WORKERS:-2}"
  export AIRFLOW__WEBSERVER__WEB_SERVER_PORT="${AIRFLOW__WEBSERVER__WEB_SERVER_PORT:-$DEFAULT_AIRFLOW_PORT}"
  say_info "==> 前台启动 airflow standalone（Ctrl+C 退出；不写 PID）…"
  exec "${airflow_cmd}" standalone
}

# 第二个参数为 1 时跳过确认（供 uninstall 在已二次确认后调用）
_stop_standalone_impl() {
  local _no_confirm="${1:-0}"
  export AIRFLOW__WEBSERVER__WEB_SERVER_PORT="${AIRFLOW__WEBSERVER__WEB_SERVER_PORT:-$DEFAULT_AIRFLOW_PORT}"
  local pid
  pid="$(read_pid)"
  local listener_pid
  listener_pid="$(listener_pid_for_port "${AIRFLOW__WEBSERVER__WEB_SERVER_PORT}")"
  if [[ -z "$pid" ]]; then
    echo "未找到 PID 文件（${PID_FILE}），视为未启动。" >&2
    rm -f "$PID_FILE"
  elif ! process_alive "$pid"; then
    echo "PID ${pid} 不存在，清理 PID 文件。"
    rm -f "$PID_FILE"
  fi
  if [[ -n "$pid" || -n "$listener_pid" ]] && [[ "${_no_confirm}" != "1" ]]; then
    if ! confirm_yes "确认停止 standalone（PID ${pid}）？"; then
      say_warn "已取消 stop。"
      return 0
    fi
  fi
  if [[ -n "$pid" ]] && process_alive "$pid"; then
    local pgid
    pgid="$(ps -o pgid= -p "$pid" 2>/dev/null | tr -d ' ' || true)"
    say_info "==> 停止 standalone（PID ${pid}, PGID ${pgid:-n/a}）..."
    if [[ -n "$pgid" ]] && [[ "$pgid" =~ ^[0-9]+$ ]]; then
      kill -TERM "-${pgid}" 2>/dev/null || kill -TERM "$pid" 2>/dev/null || true
    else
      kill -TERM "$pid" 2>/dev/null || true
    fi
    local waited=0
    while process_alive "$pid" && (( waited < 30 )); do
      sleep 1
      waited=$((waited + 1))
    done
    if process_alive "$pid"; then
      echo "优雅停止超时，发送 KILL..."
      if [[ -n "$pgid" ]] && [[ "$pgid" =~ ^[0-9]+$ ]]; then
        kill -KILL "-${pgid}" 2>/dev/null || kill -KILL "$pid" 2>/dev/null || true
      else
        kill -KILL "$pid" 2>/dev/null || true
      fi
    fi
  fi
  if [[ -n "$listener_pid" ]] && { [[ -z "$pid" ]] || [[ "$listener_pid" != "$pid" ]]; }; then
    kill -TERM "$listener_pid" 2>/dev/null || true
  fi
  rm -f "$PID_FILE"
  echo "已停止。"
}

cmd_stop() {
  _stop_standalone_impl 0
}

_uninstall_safety_check() {
  local ap vp hp
  if [[ ! -d "$AIRFLOW_HOME" ]]; then
    say_warn "AIRFLOW_HOME 不存在，无需卸载。"
    exit 0
  fi
  ap="$(cd "$AIRFLOW_HOME" 2>/dev/null && pwd -P)" || ap="$AIRFLOW_HOME"
  vp="$(cd "$AIRFLOW_VENV" 2>/dev/null && pwd -P)" || vp="$AIRFLOW_VENV"
  hp="$(cd "$HOME" && pwd -P)"
  if [[ "$ap" == "/" || "$ap" == "$hp" ]]; then
    echo "错误: AIRFLOW_HOME 解析为 / 或 \$HOME，禁止卸载。" >&2
    exit 1
  fi
  if [[ "$vp" == "/" || "$vp" == "$hp" ]]; then
    echo "错误: AIRFLOW_VENV 解析为 / 或 \$HOME，禁止卸载。" >&2
    exit 1
  fi
  if [[ "$ap" != "${HOME}/opt/airflow" ]]; then
    echo "错误: AIRFLOW_HOME 不是固定目录 ${HOME}/opt/airflow，禁止卸载。" >&2
    exit 1
  fi
  if [[ "$vp" != "${HOME}/opt/airflow/venv" ]]; then
    echo "错误: AIRFLOW_VENV 不是固定目录 ${HOME}/opt/airflow/venv，禁止卸载。" >&2
    exit 1
  fi
}

cmd_uninstall() {
  _uninstall_safety_check
  say_warn "===== Airflow 本地卸载（不可逆）====="
  echo "将停止 standalone（若运行中），并删除目录:"
  echo "  AIRFLOW_VENV=${AIRFLOW_VENV}"
  echo "  AIRFLOW_HOME=${AIRFLOW_HOME}"
  echo ""

  nlt_confirm_destructive \
    "确认永久删除以上路径？数据库、DAG、日志与虚拟环境将全部移除。" \
    AIRFLOW_UNINSTALL_YES || return 1

  _stop_standalone_impl 1

  say_info "==> 删除虚拟环境与 AIRFLOW_HOME …"
  if [[ -d "$AIRFLOW_VENV" ]]; then
    rm -rf "$AIRFLOW_VENV"
    echo "已删除 ${AIRFLOW_VENV}"
  fi
  if [[ -d "$AIRFLOW_HOME" ]]; then
    rm -rf "$AIRFLOW_HOME"
    echo "已删除 ${AIRFLOW_HOME}"
  fi

  say_info "卸载完成。若曾写入 shell profile 中的 gum PATH 等，请自行编辑删除。"
}

cmd_restart() {
  cmd_stop || die "停止失败，已中止 restart（服务可能仍在运行）"
  cmd_start
}

cmd_status() {
  require_venv_airflow
  export AIRFLOW__WEBSERVER__WEB_SERVER_PORT="${AIRFLOW__WEBSERVER__WEB_SERVER_PORT:-$DEFAULT_AIRFLOW_PORT}"
  local pid
  pid="$(read_pid)"
  local listener_pid
  listener_pid="$(listener_pid_for_port "${AIRFLOW__WEBSERVER__WEB_SERVER_PORT}")"
  if [[ -z "$pid" ]]; then
    if [[ -n "$listener_pid" ]]; then
      echo "PID 文件: 无（监听进程 PID ${listener_pid}）"
    else
      echo "PID 文件: 无"
    fi
  else
    if process_alive "$pid"; then
      if [[ -n "$listener_pid" && "$listener_pid" != "$pid" ]]; then
        echo "PID 文件: ${pid}（运行中；监听进程 PID ${listener_pid}）"
      else
        echo "PID 文件: ${pid}（运行中）"
      fi
    else
      echo "PID 文件: ${pid}（进程不存在，可删除 ${PID_FILE} 后重新 start）"
    fi
  fi
  activate_venv
  if airflow jobs check --job-type SchedulerJob --local &>/dev/null; then
    echo "airflow jobs check (--job-type SchedulerJob --local): 通过"
  elif airflow jobs check &>/dev/null; then
    echo "airflow jobs check: 通过"
  else
    echo "airflow jobs check: 不可用或未就绪（standalone 启动后稍等再试）"
  fi
}

cmd_dag_scaffold() {
  ensure_dirs
  local target="${DAG_DIR}/example_minimal.py"
  if [[ -f "$target" ]]; then
    echo "已存在 ${target}，跳过。"
    return 0
  fi
  cat >"$target" <<'PY'
"""Minimal example DAG (Airflow 3.x). Generated by airflow-setup.sh."""
from datetime import datetime

from airflow.providers.standard.operators.bash import BashOperator
from airflow.sdk import DAG

with DAG(
    dag_id="example_minimal",
    start_date=datetime(2024, 1, 1),
    schedule=None,
    catchup=False,
    tags=["example"],
) as dag:
    BashOperator(
        task_id="hello",
        bash_command="echo 'hello from example_minimal'",
    )
PY
  echo "已写入 ${target}"
}

cmd_dags_list() {
  require_venv_airflow
  activate_venv
  airflow dags list
}

cmd_trigger() {
  require_venv_airflow
  local dag_id="${1:-}"
  if [[ -z "$dag_id" ]]; then
    echo "用法: $0 trigger <dag_id> [传给 airflow dags trigger 的其它参数…]" >&2
    echo "示例: $0 trigger my_dag --conf '{\"k\":1}'" >&2
    exit 2
  fi
  shift
  activate_venv
  airflow dags trigger "$dag_id" "$@"
}

cmd_task_test() {
  require_venv_airflow
  if [[ $# -lt 1 ]]; then
    echo "用法: $0 task-test <dag_id> <task_id> <logical_date> [其他 airflow tasks test 参数]" >&2
    echo "示例: $0 task-test example_minimal hello 2024-01-01" >&2
    exit 2
  fi
  activate_venv
  airflow tasks test "$@"
}

cmd_users_create() {
  require_venv_airflow
  activate_venv
  require_airflow_users_cli
  if [[ $# -gt 0 ]]; then
    airflow users create "$@"
    return
  fi
  say_info "==> 创建用户（需启用 Flask AppBuilder 认证管理器；否则命令会失败，见官方 Quick Start 说明）"
  local u p p2 e fn ln role
  u="$(gum input --placeholder "用户名 username（默认 admin，可留空）")" || return 1
  u="${u:-admin}"
  p="$(gum input --password --placeholder "密码 password")" || return 1
  p2="$(gum input --password --placeholder "确认密码")" || return 1
  if [[ "$p" != "$p2" ]]; then
    echo "两次密码不一致。" >&2
    exit 1
  fi
  e="$(gum input --placeholder "邮箱 email（必填）")" || return 1
  if [[ -z "${e//[$' \t']/}" ]]; then
    echo "邮箱不能为空。" >&2
    exit 1
  fi
  fn="$(gum input --placeholder "名 firstname（默认 Admin，可留空）")" || return 1
  fn="${fn:-Admin}"
  ln="$(gum input --placeholder "姓 lastname（默认 User，可留空）")" || return 1
  ln="${ln:-User}"
  role="$(gum input --placeholder "角色 role（默认 Admin，可留空）")" || return 1
  role="${role:-Admin}"
  airflow users create \
    --username "$u" \
    --firstname "$fn" \
    --lastname "$ln" \
    --role "$role" \
    --email "$e" \
    --password "$p"
}

cmd_users_list() {
  require_venv_airflow
  activate_venv
  require_airflow_users_cli
  airflow users list "$@"
}

cmd_users_reset_password() {
  require_venv_airflow
  activate_venv
  require_airflow_users_cli
  if [[ $# -gt 0 ]]; then
    airflow users reset-password "$@"
    return
  fi
  say_info "==> 重置用户密码（airflow users reset-password）"
  local u p p2
  u="$(gum input --placeholder "用户名 username（默认 admin，可留空）")" || return 1
  u="${u:-admin}"
  p="$(gum input --password --placeholder "新密码")" || return 1
  p2="$(gum input --password --placeholder "确认新密码")" || return 1
  if [[ "$p" != "$p2" ]]; then
    echo "两次密码不一致。" >&2
    exit 1
  fi
  airflow users reset-password --username "$u" --password "$p"
}

cmd_http_trigger() {
  command -v curl &>/dev/null || {
    echo "http-trigger 需要系统命令 curl。" >&2
    exit 1
  }
  command -v python3 &>/dev/null || {
    echo "http-trigger 需要 python3（解析 JSON / 编码 URL）。" >&2
    exit 1
  }
  local base="${AIRFLOW_API_BASE:-http://127.0.0.1:${DEFAULT_AIRFLOW_PORT}}"
  base="${base%/}"
  local dag_id="${1:-}"
  local payload_path="${2:-}"
  if [[ -z "$dag_id" ]]; then
    echo "用法: $0 http-trigger <dag_id> [payload.json]" >&2
    echo "环境变量: AIRFLOW_API_USERNAME, AIRFLOW_API_PASSWORD（必填）" >&2
    echo "          AIRFLOW_API_BASE（可选，当前: ${base}）" >&2
    echo "payload.json 缺省则发送 {\"conf\":{}}；conf 内放本次作业参数即可。" >&2
    exit 2
  fi
  if [[ -z "${AIRFLOW_API_USERNAME:-}" || -z "${AIRFLOW_API_PASSWORD:-}" ]]; then
    echo "请设置 AIRFLOW_API_USERNAME 与 AIRFLOW_API_PASSWORD。" >&2
    exit 1
  fi
  local creds_json resp token payload enc_dag
  creds_json="$(
    AIRFLOW_API_USERNAME="$AIRFLOW_API_USERNAME" AIRFLOW_API_PASSWORD="$AIRFLOW_API_PASSWORD" \
      python3 -c 'import json, os; print(json.dumps({"username": os.environ["AIRFLOW_API_USERNAME"], "password": os.environ["AIRFLOW_API_PASSWORD"]}))'
  )"
  if ! resp="$(curl -sS -X POST "${base}/auth/token" -H "Content-Type: application/json" -d "$creds_json")"; then
    echo "请求 ${base}/auth/token 失败。" >&2
    exit 1
  fi
  if ! token="$(printf '%s' "$resp" | python3 -c 'import json,sys; print(json.load(sys.stdin)["access_token"])')"; then
    echo "解析 access_token 失败，响应: $resp" >&2
    exit 1
  fi
  if [[ -n "$payload_path" ]]; then
    if [[ ! -f "$payload_path" ]]; then
      echo "找不到 payload 文件: $payload_path" >&2
      exit 1
    fi
    payload="$(cat "$payload_path")"
  else
    payload='{"conf":{}}'
  fi
  enc_dag="$(python3 -c 'import urllib.parse, sys; print(urllib.parse.quote(sys.argv[1], safe=""))' "$dag_id")"
  say_info "==> POST ${base}/api/v2/dags/${dag_id}/dagRuns"
  curl -sS -X POST "${base}/api/v2/dags/${enc_dag}/dagRuns" \
    -H "Authorization: Bearer ${token}" \
    -H "Content-Type: application/json" \
    -d "$payload"
  echo ""
}

dispatch() {
  local cmd="$1"
  shift || true
  case "$cmd" in
    install) cmd_install ;;
    update) cmd_update ;;
    start) cmd_start ;;
    run) cmd_run ;;
    stop) cmd_stop ;;
    restart) cmd_restart ;;
    status) cmd_status ;;
    dag-scaffold) cmd_dag_scaffold ;;
    dags-list) cmd_dags_list ;;
    trigger) cmd_trigger "$@" ;;
    task-test) cmd_task_test "$@" ;;
    users-create) cmd_users_create "$@" ;;
    users-list) cmd_users_list "$@" ;;
    users-reset-password) cmd_users_reset_password "$@" ;;
    http-trigger) cmd_http_trigger "$@" ;;
    uninstall) cmd_uninstall ;;
    help|-h|--help) usage ;;
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
    nlt_ui_banner "Airflow 本地助手" "AIRFLOW_HOME=${AIRFLOW_HOME}"
  else
    gum style --bold --foreground 212 "Airflow 本地助手"
    gum style "AIRFLOW_HOME=${AIRFLOW_HOME}"
  fi
  set +e
  while true; do
    local pick
    pick="$(nlt_ui_choose "fundeploy / service / airflow / 选择动作" \
      "install" "update" "start" "run" "stop" "restart" "status" \
      "dag-scaffold" "dags-list" "trigger" "task-test" \
      "users-create" "users-list" "users-reset-password" "http-trigger" \
      "uninstall" "help" "quit")" || break
    [[ -z "$pick" ]] && break
    case "$pick" in
      quit) break ;;
      help) usage; continue ;;
      trigger)
        local tid
        tid="$(gum input --placeholder "dag_id")" || continue
        [[ -n "${tid//[$' \t']/}" ]] && ( dispatch trigger "$tid" ) || true
        continue ;;
      task-test)
        local tline
        tline="$(gum input --placeholder "dag_id task_id logical_date …（空格分隔）")" || continue
        [[ -z "${tline//[$' \t']/}" ]] && continue
        local ta_g=()
        read -ra ta_g <<<"$tline"
        ( dispatch task-test "${ta_g[@]}" )
        continue ;;
      http-trigger)
        if [[ -z "${AIRFLOW_API_USERNAME:-}" || -z "${AIRFLOW_API_PASSWORD:-}" ]]; then
          say_warn "请先: export AIRFLOW_API_USERNAME=… 与 AIRFLOW_API_PASSWORD=…"
          continue
        fi
        local hid_g pld_g
        hid_g="$(gum input --placeholder "dag_id")" || continue
        [[ -z "${hid_g//[$' \t']/}" ]] && continue
        pld_g="$(gum input --placeholder "payload.json 路径（可选，留空=默认 {\"conf\":{}}）")" || continue
        if [[ -n "${pld_g//[$' \t']/}" ]]; then
          ( dispatch http-trigger "$hid_g" "$pld_g" )
        else
          ( dispatch http-trigger "$hid_g" )
        fi
        continue ;;
    esac
    ( dispatch "$pick" )
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
