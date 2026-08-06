#!/usr/bin/env bash
# 服务生命周期回归测试：stop / restart / uninstall 在「缺 gum」下的行为。
#
# 历史缺陷（本测试即为其护栏）：
#   1. cmd_stop 里裸调 `gum confirm ... || exit 0`。缺 gum 时 gum 返回 127，
#      `|| exit 0` 触发 → 脚本打印「已停止」并 exit 0，而进程仍在运行。
#   2. `cmd_stop || true` 拦不住 `exit`，于是 restart 既没停也没启，退出码 0。
#   3. sub2api uninstall 确认两次，对第二次答 No 会 exit 0，rm -rf 永不执行。
set -uo pipefail

_TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
_REPO_ROOT="$(cd "${_TEST_DIR}/.." && pwd)"
# shellcheck source=lib/assert.sh
source "${_TEST_DIR}/lib/assert.sh"
_ASSERT_NAME="lifecycle_smoke"

_NOGUM="$(mktemp -d)"          # 空目录，用来把 gum 挤出 PATH
_WORK="$(mktemp -d)"
trap 'rm -rf "${_NOGUM}" "${_WORK}"; [[ -n "${_PID:-}" ]] && kill "${_PID}" 2>/dev/null' EXIT

_SCRIPT="${_REPO_ROOT}/scripts/services/code-server/setup-manual.sh"
_HAS_PTY=0
command -v script >/dev/null 2>&1 && _HAS_PTY=1

# 起一个占位进程并写好 PID 文件，模拟「服务正在运行」。
_spawn_fake_service() {
  local home="$1"
  sleep 300 &
  _PID=$!
  mkdir -p "${home}/opt/code-server/run"
  printf '%s\n' "${_PID}" >"${home}/opt/code-server/run/code-server.pid"
}

_alive() { kill -0 "$1" 2>/dev/null; }

# 在无 gum 的环境里运行服务脚本。
_run_nogum() {
  local home="$1"; shift
  env -i PATH="${_NOGUM}:/usr/bin:/bin" HOME="${home}" NONINTERACTIVE=1 \
    CODE_SERVER_SERVICE_HOME="${home}/opt/code-server" \
    bash "${_SCRIPT}" "$@" 2>&1
}

echo "== 缺 gum 时 stop 必须真正停止进程（非交互）=="
_H="${_WORK}/h1"; mkdir -p "${_H}"
_spawn_fake_service "${_H}"
_out="$(_run_nogum "${_H}" stop)"; _rc=$?
sleep 1
assert_eq           "stop 退出码为 0"        0 "${_rc}"
assert_not_contains "不得出现 gum 未找到"    "${_out}" "command not found"
assert_false        "进程必须已被终止"       _alive "${_PID}"
assert_no_file      "PID 文件应被清理"       "${_H}/opt/code-server/run/code-server.pid"

echo "== 缺 gum + 有 TTY，回答 y 时 stop 必须真正停止 =="
if (( _HAS_PTY )); then
  _H="${_WORK}/h2"; mkdir -p "${_H}"
  _spawn_fake_service "${_H}"
  _out="$(printf 'y\n' | script -q -e -c \
    "env PATH='${_NOGUM}:/usr/bin:/bin' HOME='${_H}' CODE_SERVER_SERVICE_HOME='${_H}/opt/code-server' bash '${_SCRIPT}' stop" \
    /dev/null 2>&1)"
  sleep 1
  assert_not_contains "不得出现 gum 未找到"       "${_out}" "command not found"
  assert_contains     "应降级为 read y/N 提问"    "${_out}" "[y/N]"
  assert_false        "回答 y 后进程必须已终止"   _alive "${_PID}"
else
  echo "  skip  未找到 script(1)，跳过 pty 场景"
fi

echo "== 缺 gum + 有 TTY，回答 n 时必须保留进程且不谎报 =="
if (( _HAS_PTY )); then
  _H="${_WORK}/h3"; mkdir -p "${_H}"
  _spawn_fake_service "${_H}"
  _out="$(printf 'n\n' | script -q -e -c \
    "env PATH='${_NOGUM}:/usr/bin:/bin' HOME='${_H}' CODE_SERVER_SERVICE_HOME='${_H}/opt/code-server' bash '${_SCRIPT}' stop" \
    /dev/null 2>&1)"
  sleep 1
  assert_true         "回答 n 后进程应仍在运行" _alive "${_PID}"
  assert_not_contains "拒绝时不得谎称已停止"    "${_out}" "已停止"
  kill "${_PID}" 2>/dev/null
else
  echo "  skip  未找到 script(1)，跳过 pty 场景"
fi

echo "== uninstall：未授权时不得删除 =="
_H="${_WORK}/h4"; mkdir -p "${_H}/opt/code-server/lib"
: >"${_H}/opt/code-server/lib/marker"
_out="$(_run_nogum "${_H}" uninstall)"; _rc=$?
assert_file     "未授权时目录必须保留"     "${_H}/opt/code-server/lib/marker"
assert_contains "应提示所需环境变量"       "${_out}" "CODE_SERVER_UNINSTALL_YES"

echo "== uninstall：授权后必须真正删除（一次确认，不得二次追问）=="
_H="${_WORK}/h5"; mkdir -p "${_H}/opt/code-server/lib"
: >"${_H}/opt/code-server/lib/marker"
_out="$(env -i PATH="${_NOGUM}:/usr/bin:/bin" HOME="${_H}" NONINTERACTIVE=1 \
  CODE_SERVER_UNINSTALL_YES=1 CODE_SERVER_SERVICE_HOME="${_H}/opt/code-server" \
  bash "${_SCRIPT}" uninstall 2>&1)"
assert_no_file      "授权后目录必须已删除"  "${_H}/opt/code-server"
assert_not_contains "不得出现 gum 未找到"   "${_out}" "command not found"

echo "== 所有服务脚本不得再出现裸 gum confirm =="
_hits="$(grep -rn 'gum confirm' "${_REPO_ROOT}/scripts/services/" 2>/dev/null || true)"
assert_eq "services/ 下裸 gum confirm 数量" "" "${_hits}"

echo "== 不得再出现 cmd_stop || true =="
_hits="$(grep -rn 'cmd_stop || true' "${_REPO_ROOT}/scripts/" 2>/dev/null || true)"
assert_eq "cmd_stop || true 数量" "" "${_hits}"

assert_summary
