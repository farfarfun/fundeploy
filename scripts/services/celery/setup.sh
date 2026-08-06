#!/usr/bin/env bash
# Celery 安装、启停、状态管理工具。
# 与脚本所在仓库/业务无关，可置于任意目录单独使用。
# 默认安装路径 ~/opt/celery/（bin、etc、data、log 子目录）。
#
# 用法：
#   chmod +x setup.sh
#   ./setup.sh              # 无参数：菜单选择或输入命令名
#   ./setup.sh install
#   ./setup.sh start       # 二次交互：1=all, 2=worker, 3=beat, 4=flower
#   ./setup.sh start-worker  # 直接启动（非交互）
#   ./setup.sh start-beat
#   ./setup.sh start-flower
#   ./setup.sh run         # 前台：同 start 的 1–4 选择，但 1=all 会拒绝（须多终端 run-*）
#   ./setup.sh run-worker | run-beat | run-flower   # 前台单组件（无 nohup、不写 PID）
#   ./setup.sh stop
#   ./setup.sh restart
#   ./setup.sh status
#
# 环境变量：
#   NONINTERACTIVE=1         跳过 gum 确认（如 restart 中是否启动 beat/flower）
#   CELERY_HOME              覆盖 Celery 安装目录（默认 ~/opt/celery）
#   CELERY_VENV              虚拟环境路径（默认 ${CELERY_HOME}/venv）
#   CELERY_BROKER_URL        Broker URL（默认 redis://localhost:6379/0）
#   CELERY_APP               Celery 应用路径（如 myproject.celery:app），未设置时使用 scaffold 示例
#   CELERY_RESULT_BACKEND    结果后端（可选，默认与 broker 一致）
#   FLOWER_PORT              Flower 监控端口（默认 8806）
#   FLOWER_ADDRESS           Flower 监听地址（默认 127.0.0.1；对外暴露须同时设 FLOWER_BASIC_AUTH）
#   FLOWER_BASIC_AUTH        Flower 基本认证 user:pass（Flower 自身无认证，对外监听时必配）
#
# 官方文档：https://docs.celeryq.dev/

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

usage() {
  cat <<'USAGE'
用法: ./setup.sh [command [args...]]
  不传参数时进入菜单交互：输入序号或命令名（提示符 celery> ）。

命令:
  install          创建 venv；安装 celery、redis、flower（可选）；未设 CELERY_APP 时生成 scaffold 示例
  update           在已有 venv 内升级 pip、celery、redis、flower（需先 install）
  start [1-4]      无参时用 gum 选择；或 1=all, 2=worker, 3=beat, 4=flower
  start-beat       后台启动 Celery beat
  start-flower     后台启动 Flower（Web 监控，需已安装 flower）
  run [1-4]        前台：无参 gum 同 start；1=all 会拒绝（请分别 run-worker / run-beat / run-flower）
  run-worker       前台启动 worker（不写 PID；后台 worker 已在跑时拒绝）
  run-beat         前台启动 beat
  run-flower       前台启动 Flower
  stop             停止 worker/beat/flower 进程组
  restart          stop 后按需重新启动
  status           显示各组件 PID 与进程状态
  help             显示本说明

交互模式: 显示数字菜单；0=帮助 q=退出 ?=帮助
  安装前需确保 Redis 等 broker 已就绪；CELERY_BROKER_URL 未设置时默认 redis://localhost:6379/0

环境变量见脚本头部注释。
USAGE
}

# 安装路径
CELERY_HOME="${CELERY_HOME:-${HOME}/opt/celery}"
CELERY_VENV="${CELERY_VENV:-${CELERY_HOME}/venv}"
CELERY_BIN_DIR="${CELERY_HOME}/bin"
CELERY_ETC_DIR="${CELERY_HOME}/etc"
CELERY_DATA_DIR="${CELERY_HOME}/data"
CELERY_LOG_DIR="${CELERY_HOME}/log"
CELERY_RUN_DIR="${CELERY_HOME}/run"

# Broker 默认 Redis
CELERY_BROKER_URL="${CELERY_BROKER_URL:-redis://localhost:6379/0}"

# Flower 监控界面：端口与监听地址
FLOWER_PORT="${FLOWER_PORT:-8806}"
# 默认只监听回环。Flower 自身不带任何认证，却会暴露任务参数（其中经常含有
# 凭据）以及 revoke/terminate 操作 —— 绑 0.0.0.0 等于把这些交给整个局域网。
# 同仓库的 code-server 默认就是 127.0.0.1，这里对齐。
# 确需对外暴露时显式设置 FLOWER_ADDRESS=0.0.0.0，并务必配合
# FLOWER_BASIC_AUTH=user:pass。
FLOWER_ADDRESS="${FLOWER_ADDRESS:-127.0.0.1}"
FLOWER_BASIC_AUTH="${FLOWER_BASIC_AUTH:-}"

# PID 文件
PID_WORKER="${CELERY_RUN_DIR}/worker.pid"
PID_BEAT="${CELERY_RUN_DIR}/beat.pid"
PID_FLOWER="${CELERY_RUN_DIR}/flower.pid"

ensure_dirs() {
  mkdir -p "$CELERY_BIN_DIR" "$CELERY_ETC_DIR" "$CELERY_DATA_DIR" "$CELERY_LOG_DIR" "$CELERY_RUN_DIR"
}

require_python() {
  if ! command -v python3 &>/dev/null; then
    echo "未找到 python3，请先安装 Python 3.8+。" >&2
    exit 1
  fi
  echo "使用 Python: $(python3 --version)"
}

activate_venv() {
  # shellcheck source=/dev/null
  source "${CELERY_VENV}/bin/activate"
  export CELERY_BROKER_URL
  export CELERY_APP="${CELERY_APP:-celery_app:app}"
  export CELERY_RESULT_BACKEND="${CELERY_RESULT_BACKEND:-$CELERY_BROKER_URL}"
  # 使用 scaffold 时需能从 etc 目录找到 celery_app 模块（start 时以 etc 为 CWD 启动）
  if [[ "${CELERY_APP}" == "celery_app:app" ]] && [[ -f "${CELERY_ETC_DIR}/celery_app.py" ]]; then
    export PYTHONPATH="${CELERY_ETC_DIR}${PYTHONPATH:+:${PYTHONPATH}}"
  fi
}

# 检查 Redis 是否可达（默认 localhost:6379）
check_redis() {
  if command -v redis-cli &>/dev/null; then
    if redis-cli ping 2>/dev/null | grep -q PONG; then
      return 0
    fi
  fi
  return 1
}

require_venv_celery() {
  if [[ ! -x "${CELERY_VENV}/bin/celery" ]]; then
    echo "未找到 ${CELERY_VENV}/bin/celery，请先运行: $0 install" >&2
    exit 1
  fi
}

# 确保 scaffold 存在（使用 celery_app:app 且未设置 CELERY_APP 时）
ensure_scaffold() {
  if [[ "${CELERY_APP:-celery_app:app}" != "celery_app:app" ]]; then
    return 0
  fi
  local scaffold_file="${CELERY_ETC_DIR}/celery_app.py"
  if [[ -f "$scaffold_file" ]]; then
    return 0
  fi
  echo "==> 生成 scaffold 示例: ${scaffold_file}"
  ensure_dirs
  cat >"$scaffold_file" <<'PY'
"""Celery scaffold app. Set CELERY_APP=celery_app:app to use this."""
from celery import Celery
import os

broker = os.environ.get("CELERY_BROKER_URL", "redis://localhost:6379/0")
backend = os.environ.get("CELERY_RESULT_BACKEND", broker)

app = Celery("celery_app", broker=broker, backend=backend)

@app.task
def add(x, y):
    return x + y
PY
}

process_alive() {
  local pid="$1"
  kill -0 "$pid" 2>/dev/null
}

read_pid() {
  local f="$1"
  if [[ ! -f "$f" ]]; then
    echo ""
    return
  fi
  tr -d '[:space:]' <"$f" || true
}

status_show_one() {
  local pid_file="$1"
  local name="$2"
  local pid
  pid="$(read_pid "$pid_file")"
  if [[ -z "$pid" ]]; then
    echo "  ${name}: 未启动"
  elif process_alive "$pid"; then
    echo "  ${name}: PID ${pid}（运行中）"
  else
    echo "  ${name}: PID ${pid}（进程不存在，可删除 ${pid_file}）"
  fi
}

# 按 PID 文件停止进程组
stop_by_pid_file() {
  local pid_file="$1"
  local name="$2"
  local pid
  pid="$(read_pid "$pid_file")"
  if [[ -z "$pid" ]]; then
    return 0
  fi
  if ! process_alive "$pid"; then
    rm -f "$pid_file"
    return 0
  fi
  local pgid
  pgid="$(ps -o pgid= -p "$pid" 2>/dev/null | tr -d ' ' || true)"
  echo "  -> 停止 ${name}（PID ${pid}, PGID ${pgid:-n/a}）..."
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
    echo "  优雅停止超时，发送 KILL..."
    if [[ -n "$pgid" ]] && [[ "$pgid" =~ ^[0-9]+$ ]]; then
      kill -KILL "-${pgid}" 2>/dev/null || kill -KILL "$pid" 2>/dev/null || true
    else
      kill -KILL "$pid" 2>/dev/null || true
    fi
  fi
  rm -f "$pid_file"
}

cmd_install() {
  require_python
  ensure_dirs
  echo "CELERY_HOME=${CELERY_HOME}"
  echo "CELERY_VENV=${CELERY_VENV}"
  echo "CELERY_BROKER_URL=${CELERY_BROKER_URL}"
  if [[ ! -d "$CELERY_VENV" ]]; then
    echo "==> 创建虚拟环境..."
    python3 -m venv "$CELERY_VENV"
  fi
  activate_venv
  echo "==> 升级 pip..."
  pip install --upgrade pip
  echo "==> 安装 celery、redis、flower..."
  pip install celery redis flower
  echo "==> 检查 CELERY_APP..."
  if [[ -z "${CELERY_APP:-}" ]]; then
    local scaffold_file="${CELERY_HOME}/etc/celery_app.py"
    if [[ ! -f "$scaffold_file" ]]; then
      echo "未设置 CELERY_APP，生成 scaffold 示例: ${scaffold_file}"
      cat >"$scaffold_file" <<'PY'
"""Celery scaffold app. Set CELERY_APP=celery_app:app to use this."""
from celery import Celery
import os

broker = os.environ.get("CELERY_BROKER_URL", "redis://localhost:6379/0")
backend = os.environ.get("CELERY_RESULT_BACKEND", broker)

app = Celery("celery_app", broker=broker, backend=backend)

@app.task
def add(x, y):
    return x + y
PY
      echo "
# 如需自定义任务，请在 ${CELERY_ETC_DIR}/tasks/ 下添加模块，并在 scaffold 中 include
"
    fi
    export CELERY_APP="celery_app:app"
    # 需要从 etc 目录运行，或把路径加入 PYTHONPATH
    local env_example="${CELERY_ETC_DIR}/env.example"
    cat >"$env_example" <<ENV
# 复制为 .env 或在 shell 中 export
export CELERY_BROKER_URL="redis://localhost:6379/0"
export CELERY_APP="celery_app:app"
# 本脚本在 start-worker/start-beat/start-flower 时会自动设置 PYTHONPATH
ENV
    echo "Scaffold 已生成。CELERY_APP 未设置时将使用 celery_app:app。"
  else
    echo "CELERY_APP=${CELERY_APP}（已设置）"
  fi
  echo "安装完成。可执行: $0 start 一键启动 worker/beat/flower"
}

cmd_update() {
  require_venv_celery
  ensure_dirs
  echo "==> 更新 Celery 依赖（保留 ${CELERY_HOME} 数据）..."
  activate_venv
  pip install --upgrade pip
  pip install -U celery redis flower
  echo "更新完成。"
}

cmd_start() {
  local choice="${1:-}"
  if [[ -z "$choice" ]]; then
    local pick
    pick="$(gum choose --header "选择要启动的组件" \
      "1 all（worker+beat+flower）" \
      "2 worker" \
      "3 beat" \
      "4 flower" \
      "0 取消")" || return 0
    [[ -z "$pick" ]] && return 0
    choice="${pick%% *}"
  fi
  case "${choice:-0}" in
    1)
      echo ""
      cmd_start_worker
      echo ""
      cmd_start_beat
      echo ""
      cmd_start_flower
      echo ""
      echo "已启动全部。Flower 监控 http://localhost:${FLOWER_PORT}（监听 ${FLOWER_ADDRESS}）"
      ;;
    2) cmd_start_worker ;;
    3) cmd_start_beat ;;
    4) cmd_start_flower ;;
    0|"") echo "已取消。" ;;
    *)
      echo "无效选择，请输入 1-4 或 0。" >&2
      return 1
      ;;
  esac
}

cmd_start_worker() {
  require_venv_celery
  ensure_dirs
  activate_venv  # 提前激活以设置 CELERY_APP
  ensure_scaffold
  if ! check_redis; then
    echo "警告: 无法连接 Redis，worker 可能无法启动。请先启动 Redis 或检查 CELERY_BROKER_URL。" >&2
    if nlt_interactive; then
      # nlt_ui_confirm 在缺 gum 时降级为 read y/N；原来的裸 gum 会返回 127，
      # 被 `|| return 0` 判成「用户拒绝」，于是 Redis 不通时静默不启动。
      nlt_ui_confirm "仍尝试启动 worker？" || return 0
    fi
  fi
  local existing
  existing="$(read_pid "$PID_WORKER")"
  if [[ -n "$existing" ]] && process_alive "$existing"; then
    echo "Worker 已在运行（PID ${existing}）。如需重启请执行: $0 stop 后 $0 start-worker" >&2
    exit 1
  fi
  activate_venv
  local log_file="${CELERY_LOG_DIR}/worker.log"
  echo "==> 启动 Celery worker（日志: ${log_file}）..."
  echo "    CELERY_BROKER_URL=${CELERY_BROKER_URL}"
  # 使用 scaffold 时需从 etc 目录启动，以便找到 celery_app 模块
  if [[ "${CELERY_APP}" == "celery_app:app" ]] && [[ -f "${CELERY_ETC_DIR}/celery_app.py" ]]; then
    (cd "$CELERY_ETC_DIR" && exec nohup celery -A celery_app:app worker --loglevel=info >>"$log_file" 2>&1) &
  else
    nohup celery -A "$CELERY_APP" worker --loglevel=info >>"$log_file" 2>&1 &
  fi
  local pid=$!
  echo "$pid" >"$PID_WORKER"
  echo "已写入 PID ${pid} -> ${PID_WORKER}"
}

cmd_start_beat() {
  require_venv_celery
  ensure_dirs
  activate_venv
  ensure_scaffold
  local existing
  existing="$(read_pid "$PID_BEAT")"
  if [[ -n "$existing" ]] && process_alive "$existing"; then
    echo "Beat 已在运行（PID ${existing}）。如需重启请执行: $0 stop 后 $0 start-beat" >&2
    exit 1
  fi
  activate_venv
  local log_file="${CELERY_LOG_DIR}/beat.log"
  echo "==> 启动 Celery beat（日志: ${log_file}）..."
  if [[ "${CELERY_APP}" == "celery_app:app" ]] && [[ -f "${CELERY_ETC_DIR}/celery_app.py" ]]; then
    (cd "$CELERY_ETC_DIR" && exec nohup celery -A celery_app:app beat --loglevel=info >>"$log_file" 2>&1) &
  else
    nohup celery -A "$CELERY_APP" beat --loglevel=info >>"$log_file" 2>&1 &
  fi
  local pid=$!
  echo "$pid" >"$PID_BEAT"
  echo "已写入 PID ${pid} -> ${PID_BEAT}"
}

cmd_start_flower() {
  require_venv_celery
  if ! "${CELERY_VENV}/bin/python" -c "import flower" 2>/dev/null; then
    echo "未安装 flower，请先运行: $0 install" >&2
    exit 1
  fi
  ensure_dirs
  activate_venv
  ensure_scaffold
  local existing
  existing="$(read_pid "$PID_FLOWER")"
  if [[ -n "$existing" ]] && process_alive "$existing"; then
    echo "Flower 已在运行（PID ${existing}）。如需重启请执行: $0 stop 后 $0 start-flower" >&2
    exit 1
  fi
  local log_file="${CELERY_LOG_DIR}/flower.log"
  echo "==> 启动 Flower（日志: ${log_file}，http://${FLOWER_ADDRESS}:${FLOWER_PORT}）..."

  # Flower 无内建认证：一旦对外监听而又没配 basic auth，任何人都能看到任务
  # 参数（常含凭据）并 revoke/terminate 任务。
  local -a flower_args=(--port="${FLOWER_PORT}" --address="${FLOWER_ADDRESS}")
  if [[ -n "${FLOWER_BASIC_AUTH}" ]]; then
    flower_args+=(--basic-auth="${FLOWER_BASIC_AUTH}")
  elif [[ "${FLOWER_ADDRESS}" != "127.0.0.1" && "${FLOWER_ADDRESS}" != "localhost" ]]; then
    nlt_ui_warn "Flower 监听 ${FLOWER_ADDRESS} 且未设置 FLOWER_BASIC_AUTH：任务参数与 revoke/terminate 将对该网络完全开放。"
    nlt_ui_warn "建议: FLOWER_BASIC_AUTH=user:pass nltdeploy service celery start-flower"
  fi

  if [[ "${CELERY_APP}" == "celery_app:app" ]] && [[ -f "${CELERY_ETC_DIR}/celery_app.py" ]]; then
    (cd "$CELERY_ETC_DIR" && exec nohup celery -A celery_app:app flower "${flower_args[@]}" >>"$log_file" 2>&1) &
  else
    nohup celery -A "$CELERY_APP" flower "${flower_args[@]}" >>"$log_file" 2>&1 &
  fi
  local pid=$!
  echo "$pid" >"$PID_FLOWER"
  echo "已写入 PID ${pid} -> ${PID_FLOWER}"
}

cmd_run() {
  local choice="${1:-}"
  if [[ -z "$choice" ]]; then
    local pick
    pick="$(gum choose --header "前台运行（不写 PID；后台已在跑时拒绝）" \
      "1 all（不可用：请多终端分别 run-worker / run-beat / run-flower）" \
      "2 worker" \
      "3 beat" \
      "4 flower" \
      "0 取消")" || return 0
    [[ -z "$pick" ]] && return 0
    choice="${pick%% *}"
  fi
  case "${choice:-0}" in
    1)
      echo "错误: 无法在单终端前台同时运行 worker、beat、flower。请分别执行:" >&2
      echo "  $0 run-worker" >&2
      echo "  $0 run-beat" >&2
      echo "  $0 run-flower" >&2
      exit 1
      ;;
    2) cmd_run_worker ;;
    3) cmd_run_beat ;;
    4) cmd_run_flower ;;
    0|"") echo "已取消。" ;;
    *)
      echo "无效选择，请输入 1-4 或 0。" >&2
      return 1
      ;;
  esac
}

cmd_run_worker() {
  require_venv_celery
  ensure_dirs
  activate_venv
  ensure_scaffold
  if ! check_redis; then
    echo "警告: 无法连接 Redis，worker 可能无法正常启动。请先启动 Redis 或检查 CELERY_BROKER_URL。" >&2
    if nlt_interactive; then
      # nlt_ui_confirm 在缺 gum 时降级为 read y/N；原来的裸 gum 会返回 127，
      # 被 `|| return 0` 判成「用户拒绝」，于是 Redis 不通时静默不启动。
      nlt_ui_confirm "仍尝试启动 worker？" || return 0
    fi
  fi
  local existing
  existing="$(read_pid "$PID_WORKER")"
  if [[ -n "$existing" ]] && process_alive "$existing"; then
    echo "Worker 已在后台运行（PID ${existing}）。请先 $0 stop，再使用 run-worker。" >&2
    exit 1
  fi
  activate_venv
  echo "==> 前台 Celery worker（Ctrl+C 退出；不写 PID）…"
  echo "    CELERY_BROKER_URL=${CELERY_BROKER_URL}"
  if [[ "${CELERY_APP}" == "celery_app:app" ]] && [[ -f "${CELERY_ETC_DIR}/celery_app.py" ]]; then
    cd "$CELERY_ETC_DIR" && exec celery -A celery_app:app worker --loglevel=info
  fi
  exec celery -A "$CELERY_APP" worker --loglevel=info
}

cmd_run_beat() {
  require_venv_celery
  ensure_dirs
  activate_venv
  ensure_scaffold
  local existing
  existing="$(read_pid "$PID_BEAT")"
  if [[ -n "$existing" ]] && process_alive "$existing"; then
    echo "Beat 已在后台运行（PID ${existing}）。请先 $0 stop，再使用 run-beat。" >&2
    exit 1
  fi
  activate_venv
  echo "==> 前台 Celery beat（Ctrl+C 退出；不写 PID）…"
  if [[ "${CELERY_APP}" == "celery_app:app" ]] && [[ -f "${CELERY_ETC_DIR}/celery_app.py" ]]; then
    cd "$CELERY_ETC_DIR" && exec celery -A celery_app:app beat --loglevel=info
  fi
  exec celery -A "$CELERY_APP" beat --loglevel=info
}

cmd_run_flower() {
  require_venv_celery
  if ! "${CELERY_VENV}/bin/python" -c "import flower" 2>/dev/null; then
    echo "未安装 flower，请先运行: $0 install" >&2
    exit 1
  fi
  ensure_dirs
  activate_venv
  ensure_scaffold
  local existing
  existing="$(read_pid "$PID_FLOWER")"
  if [[ -n "$existing" ]] && process_alive "$existing"; then
    echo "Flower 已在后台运行（PID ${existing}）。请先 $0 stop，再使用 run-flower。" >&2
    exit 1
  fi
  activate_venv
  echo "==> 前台 Flower（http://${FLOWER_ADDRESS}:${FLOWER_PORT}；Ctrl+C 退出；不写 PID）…"
  if [[ "${CELERY_APP}" == "celery_app:app" ]] && [[ -f "${CELERY_ETC_DIR}/celery_app.py" ]]; then
    cd "$CELERY_ETC_DIR" && exec celery -A celery_app:app flower --port="${FLOWER_PORT}" --address="${FLOWER_ADDRESS}"
  fi
  exec celery -A "$CELERY_APP" flower --port="${FLOWER_PORT}" --address="${FLOWER_ADDRESS}"
}

cmd_stop() {
  echo "==> 停止 Celery 进程..."
  stop_by_pid_file "$PID_WORKER" "worker"
  stop_by_pid_file "$PID_BEAT" "beat"
  stop_by_pid_file "$PID_FLOWER" "flower"
  echo "已停止。"
}

cmd_restart() {
  cmd_stop || die "停止失败，已中止 restart（worker 可能仍在运行）"
  echo ""
  cmd_start_worker
  if [[ "${NONINTERACTIVE:-}" == "1" ]]; then
    return 0
  fi
  echo ""
  if nlt_ui_confirm "是否同时启动 beat？"; then
    cmd_start_beat
  fi
  echo ""
  if nlt_ui_confirm "是否同时启动 flower？"; then
    cmd_start_flower
  fi
}

cmd_status() {
  [[ -d "$CELERY_VENV" ]] && activate_venv
  echo "==> Celery 状态"
  echo ""
  # 基础信息表
  printf "┌────────────────┬%s┐\n" "$(printf '%.0s─' $(seq 1 50))"
  printf "│ %-14s │ %-48s │\n" "配置项" "值"
  printf "├────────────────┼%s┤\n" "$(printf '%.0s─' $(seq 1 50))"
  printf "│ %-14s │ %-48s │\n" "CELERY_HOME" "${CELERY_HOME:0:48}"
  printf "│ %-14s │ %-48s │\n" "CELERY_APP" "$(echo "${CELERY_APP:-（scaffold）}" | head -c 48)"
  printf "│ %-14s │ %-48s │\n" "Broker" "$(echo "$CELERY_BROKER_URL" | head -c 48)"
  printf "│ %-14s │ %-48s │\n" "Result" "$(echo "${CELERY_RESULT_BACKEND:-$CELERY_BROKER_URL}" | head -c 48)"
  printf "│ %-14s │ %-48s │\n" "日志目录" "$(echo "$CELERY_LOG_DIR" | head -c 48)"
  printf "└────────────────┴%s┘\n" "$(printf '%.0s─' $(seq 1 50))"
  echo ""
  # 进程状态表
  status_one() {
    local pid_file="$1" name="$2"
    local pid status
    pid="$(read_pid "$pid_file")"
    if [[ -z "$pid" ]]; then
      status="未启动"
    elif process_alive "$pid"; then
      status="运行中"
    else
      status="已退出"
    fi
    printf "│ %-10s │ %-12s │ %-8s │\n" "$name" "${pid:-—}" "$status"
  }
  printf "┌────────────┬────────────┬────────────┐\n"
  printf "│ %-10s │ %-10s │ %-10s │\n" "组件" "PID" "状态"
  printf "├────────────┼────────────┼────────────┤\n"
  status_one "$PID_WORKER" "worker"
  status_one "$PID_BEAT" "beat"
  status_one "$PID_FLOWER" "flower"
  printf "└────────────┴────────────┴────────────┘\n"
  echo ""
  # 图形化界面表
  local flower_pid flower_status
  flower_pid="$(read_pid "$PID_FLOWER")"
  if [[ -n "$flower_pid" ]] && process_alive "$flower_pid"; then
    flower_status="运行中"
  else
    flower_status="未启动"
  fi
  printf "┌──────────────────────────────────────────────────────┐\n"
  printf "│ %-20s │ %-41s │\n" "Flower 监控" "http://localhost:${FLOWER_PORT} (${FLOWER_ADDRESS})"
  printf "│ %-20s │ %-41s │\n" "状态" "$flower_status"
  printf "└──────────────────────────────────────────────────────┘\n"
  # 提示
  if ! check_redis; then
    echo ""
    echo "⚠ Redis 未连接，worker/beat/flower 可能无法运行。请先启动 Redis。"
  fi
  local any_dead=false
  for f in "$PID_WORKER" "$PID_BEAT" "$PID_FLOWER"; do
    local p
    p="$(read_pid "$f")"
    if [[ -n "$p" ]] && ! process_alive "$p"; then
      any_dead=true
      break
    fi
  done
  if [[ "$any_dead" == "true" ]]; then
    echo "⚠ 进程异常退出，可查看: tail -50 ${CELERY_LOG_DIR}/worker.log"
  fi
}

dispatch() {
  local cmd="$1"
  shift || true
  case "$cmd" in
    install) cmd_install ;;
    update) cmd_update ;;
    start) cmd_start "$@" ;;
    start-worker) cmd_start_worker ;;
    start-beat) cmd_start_beat ;;
    start-flower) cmd_start_flower ;;
    run) cmd_run "$@" ;;
    run-worker) cmd_run_worker ;;
    run-beat) cmd_run_beat ;;
    run-flower) cmd_run_flower ;;
    stop) cmd_stop ;;
    restart) cmd_restart ;;
    status) cmd_status ;;
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
    nlt_ui_banner "Celery 本地助手" "CELERY_HOME=${CELERY_HOME}" "CELERY_BROKER_URL=${CELERY_BROKER_URL}"
  else
    gum style --bold --foreground 212 "Celery 本地助手"
    gum style "CELERY_HOME=${CELERY_HOME}"
    gum style "CELERY_BROKER_URL=${CELERY_BROKER_URL}"
  fi
  echo ""
  set +e
  while true; do
    local pick
    pick="$(nlt_ui_choose "nltdeploy / service / celery / 选择动作" \
      "install" "update" "start" "run" "stop" "restart" "status" \
      "start-worker" "start-beat" "start-flower" \
      "run-worker" "run-beat" "run-flower" \
      "help" "quit")" || break
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
