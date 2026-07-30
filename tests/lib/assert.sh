#!/usr/bin/env bash
# 测试断言助手：被 tests/*.sh source。
#
# 目的：原有测试大量使用无消息的 `... || exit 1`，CI 失败时只给一个退出码，
# 必须靠数行号定位。这里的每个断言都会打印自己在断言什么。
#
# 用法:
#   source "$(dirname "${BASH_SOURCE[0]}")/lib/assert.sh"
#   assert_ok "gum 缺失时 stop 应失败" test_stop_without_gum
#   assert_eq  "退出码" 0 "$rc"
#   assert_contains "输出含 PID" "$out" "PID"
#   assert_summary            # 结尾调用；有失败则退出 1

_ASSERT_PASS=0
_ASSERT_FAIL=0
_ASSERT_NAME="${_ASSERT_NAME:-$(basename "${0}")}"

_assert_pass() { _ASSERT_PASS=$((_ASSERT_PASS + 1)); printf '  \033[32mok\033[0m   %s\n' "$1"; }
_assert_fail() {
  _ASSERT_FAIL=$((_ASSERT_FAIL + 1))
  printf '  \033[31mFAIL\033[0m %s\n' "$1"
  [[ -n "${2:-}" ]] && printf '       %s\n' "$2"
  return 0
}

# assert_true <描述> <命令...>
assert_true() {
  local desc="$1"; shift
  if "$@" >/dev/null 2>&1; then _assert_pass "${desc}"; else _assert_fail "${desc}" "命令返回非 0: $*"; fi
}

# assert_false <描述> <命令...>
assert_false() {
  local desc="$1"; shift
  if "$@" >/dev/null 2>&1; then _assert_fail "${desc}" "命令意外返回 0: $*"; else _assert_pass "${desc}"; fi
}

# assert_eq <描述> <期望> <实际>
assert_eq() {
  local desc="$1" want="$2" got="$3"
  if [[ "${want}" == "${got}" ]]; then _assert_pass "${desc}"; else _assert_fail "${desc}" "期望 [${want}]，实际 [${got}]"; fi
}

# assert_contains <描述> <文本> <子串>
assert_contains() {
  local desc="$1" hay="$2" needle="$3"
  if [[ "${hay}" == *"${needle}"* ]]; then _assert_pass "${desc}"; else _assert_fail "${desc}" "输出中未找到 [${needle}]，实际: ${hay:0:400}"; fi
}

# assert_not_contains <描述> <文本> <子串>
assert_not_contains() {
  local desc="$1" hay="$2" needle="$3"
  if [[ "${hay}" != *"${needle}"* ]]; then _assert_pass "${desc}"; else _assert_fail "${desc}" "输出中不应出现 [${needle}]，实际: ${hay:0:400}"; fi
}

# assert_file <描述> <路径>
assert_file() {
  local desc="$1" p="$2"
  if [[ -f "${p}" ]]; then _assert_pass "${desc}"; else _assert_fail "${desc}" "文件不存在: ${p}"; fi
}

# assert_no_file <描述> <路径>
assert_no_file() {
  local desc="$1" p="$2"
  if [[ ! -e "${p}" ]]; then _assert_pass "${desc}"; else _assert_fail "${desc}" "路径本应不存在: ${p}"; fi
}

assert_summary() {
  printf '\n%s: %d passed, %d failed\n' "${_ASSERT_NAME}" "${_ASSERT_PASS}" "${_ASSERT_FAIL}"
  (( _ASSERT_FAIL == 0 )) || exit 1
  printf '%s OK\n' "${_ASSERT_NAME}"
}
