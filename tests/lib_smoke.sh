#!/usr/bin/env bash
# lib/nlt-ui.sh 共享助手的回归测试。
#
# 覆盖的历史缺陷：
#   1. 服务脚本用裸 `gum confirm ... || exit 0`：缺 gum 时返回 127，
#      于是 stop/restart/uninstall 谎报成功却什么都没做。
#   2. 卸载守卫只比对「是否等于 / 或 $HOME」，SERVICE_HOME=/opt 之类会被放行。
#   3. nlt_ui_choose 对 "08"/"09" 走八进制，报 value too great for base。
set -uo pipefail

_TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
_REPO_ROOT="$(cd "${_TEST_DIR}/.." && pwd)"
# shellcheck source=lib/assert.sh
source "${_TEST_DIR}/lib/assert.sh"
_ASSERT_NAME="lib_smoke"

# shellcheck source=../scripts/lib/nlt-ui.sh
source "${_REPO_ROOT}/scripts/lib/nlt-ui.sh"

# 全程模拟「gum 未安装」——这正是历史缺陷的触发条件。
_no_gum_path="$(mktemp -d)"
trap 'rm -rf "${_no_gum_path}" "${_tmp_root:-}"' EXIT
export PATH="${_no_gum_path}:/usr/bin:/bin"
command -v gum >/dev/null 2>&1 && { echo "测试前置失败: PATH 中仍存在 gum" >&2; exit 1; }

echo "== nlt_confirm_destructive（无 gum）=="
# 缺 gum 时绝不能出现 127 / command not found。
_out="$(NONINTERACTIVE=1 nlt_confirm_destructive "删?" FOO_YES 2>&1)"; _rc=$?
assert_eq        "非交互且未授权 → 返回 1" 1 "${_rc}"
assert_not_contains "不得出现 command not found" "${_out}" "command not found"
assert_contains  "应提示所需环境变量" "${_out}" "FOO_YES"

assert_true  "FOO_YES=1 放行"      env NONINTERACTIVE=1 FOO_YES=1        bash -c 'source "'"${_REPO_ROOT}"'/scripts/lib/nlt-ui.sh"; nlt_confirm_destructive "x" FOO_YES'
assert_true  "NLT_ASSUME_YES=1 放行" env NONINTERACTIVE=1 NLT_ASSUME_YES=1 bash -c 'source "'"${_REPO_ROOT}"'/scripts/lib/nlt-ui.sh"; nlt_confirm_destructive "x" FOO_YES'
assert_false "未授权则不放行"       env NONINTERACTIVE=1                   bash -c 'source "'"${_REPO_ROOT}"'/scripts/lib/nlt-ui.sh"; nlt_confirm_destructive "x" FOO_YES'

echo "== nlt_safe_rm 守卫 =="
for p in "" "relative/path" "/" "${HOME}" "/opt" "/usr" "/etc" "/var" "/home"; do
  assert_false "拒绝删除 [${p:-<空>}]" nlt_safe_rm "${p}"
done

_tmp_root="$(mktemp -d)"
mkdir -p "${_tmp_root}/svc/bin"; : >"${_tmp_root}/svc/bin/x"
assert_true    "允许删除合法服务目录" nlt_safe_rm "${_tmp_root}/svc"
assert_no_file "目录确实已删除"       "${_tmp_root}/svc"

# 深度守卫：/opt 不可删，/opt/foo 可以。用临时根模拟，避免碰真实系统目录。
mkdir -p "${_tmp_root}/opt"
assert_false "拒绝层级过浅的路径" nlt_safe_rm "/nonexistent-toplevel"

echo "== nlt_ui_choose 数字解析 =="
# 历史缺陷：08/09 被当八进制。用管道喂输入（非 TTY → 走朴素编号分支）。
_pick="$(printf '08\n' | nlt_ui_choose "选一个" a b c d e f g h i 2>/dev/null)"; _rc=$?
assert_eq "输入 08 应选中第 8 项" "h" "${_pick}"
_pick="$(printf '2\n' | nlt_ui_choose "选一个" a b c 2>/dev/null)"
assert_eq "输入 2 应选中第 2 项" "b" "${_pick}"
_out="$(printf '99\n' | nlt_ui_choose "选一个" a b c 2>&1)"; _rc=$?
assert_eq "越界输入应返回非 0" 1 "${_rc}"
assert_not_contains "不得出现进制错误" "${_out}" "value too great"

echo "== nlt_interactive =="
assert_false "NONINTERACTIVE=1 → 非交互" env NONINTERACTIVE=1 bash -c 'source "'"${_REPO_ROOT}"'/scripts/lib/nlt-ui.sh"; nlt_interactive'
assert_false "stdin 非 TTY → 非交互"      bash -c 'source "'"${_REPO_ROOT}"'/scripts/lib/nlt-ui.sh"; nlt_interactive' </dev/null

echo "== 输出层不污染 stdout =="
# 语义化提示必须走 stderr，否则会污染被命令替换捕获的数据。
_stdout="$(nlt_ui_info "info 文本" 2>/dev/null)"
assert_eq "nlt_ui_info 不写 stdout" "" "${_stdout}"
_stdout="$(nlt_ui_warn "warn 文本" 2>/dev/null)"
assert_eq "nlt_ui_warn 不写 stdout" "" "${_stdout}"

echo "== 非 TTY 时不输出 ANSI 转义 =="
_out="$(nlt_ui_ok "成功" 2>&1)"
assert_not_contains "管道输出不含 ANSI" "${_out}" $'\033['

assert_summary
