#!/usr/bin/env bash
# 统一测试入口：发现并运行 tests/*_smoke.sh。
#
# 此前测试清单被手工重复在三处（.github/workflows/release.yml、README.md、
# packaging/README.md），新增一个测试就要同步改三个地方，且极易漏。
# 现在三处都只调用本脚本。
#
# 环境变量:
#   NLTDEPLOY_REQUIRE_PACKAGE_TESTS=1  缺打包工具时不再跳过，而是判失败（CI 用）
#   TEST_FILTER=<子串>                 只运行名字匹配该子串的测试
set -uo pipefail

TESTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${TESTS_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

pass=0 fail=0 skipped=0
failed_names=()

echo "=== nltdeploy 测试套件 ==="
echo "仓库: ${REPO_ROOT}"
echo ""

# ── 0. 全量语法检查（最便宜的护栏，覆盖每一个 .sh）─────────────────────────────
echo "--- bash -n 全量语法检查 ---"
syntax_bad=0
while IFS= read -r f; do
  if ! bash -n "$f" 2>/dev/null; then
    echo "  语法错误: $f"
    bash -n "$f" 2>&1 | sed 's/^/      /'
    syntax_bad=$((syntax_bad + 1))
  fi
done < <(git ls-files '*.sh' 2>/dev/null || find . -name '*.sh' -not -path './.git/*')
if (( syntax_bad == 0 )); then
  echo "  ok  所有 .sh 均通过 bash -n"
  pass=$((pass + 1))
else
  echo "  FAIL ${syntax_bad} 个文件存在语法错误"
  fail=$((fail + 1)); failed_names+=("bash -n")
fi
echo ""

# ── 1. 各 smoke 测试 ─────────────────────────────────────────────────────────
for t in "${TESTS_DIR}"/*_smoke.sh; do
  [[ -f "$t" ]] || continue
  name="$(basename "$t")"
  if [[ -n "${TEST_FILTER:-}" && "${name}" != *"${TEST_FILTER}"* ]]; then
    continue
  fi
  echo "--- ${name} ---"
  if out="$(bash "$t" 2>&1)"; then
    if [[ "${out}" == *"skip"* && "${out}" != *"OK"* ]]; then
      echo "${out}" | sed 's/^/  /'
      skipped=$((skipped + 1))
    else
      echo "${out}" | tail -3 | sed 's/^/  /'
      pass=$((pass + 1))
    fi
  else
    echo "${out}" | sed 's/^/  /'
    fail=$((fail + 1)); failed_names+=("${name}")
  fi
  echo ""
done

# ── 2. download 自检（此前只在用户 install 时跑，CI 从未调用）───────────────────
_selftest="${REPO_ROOT}/scripts/tools/download/selftest.sh"
if [[ -f "${_selftest}" ]]; then
  echo "--- download/selftest.sh ---"
  if out="$(bash "${_selftest}" 2>&1)"; then
    echo "${out}" | tail -2 | sed 's/^/  /'
    pass=$((pass + 1))
  else
    echo "${out}" | tail -10 | sed 's/^/  /'
    fail=$((fail + 1)); failed_names+=("download/selftest.sh")
  fi
  echo ""
fi

# ── 汇总 ────────────────────────────────────────────────────────────────────
echo "=========================================="
printf '通过 %d, 失败 %d, 跳过 %d\n' "${pass}" "${fail}" "${skipped}"
if (( fail > 0 )); then
  printf '失败项: %s\n' "${failed_names[*]}"
  exit 1
fi
echo "全部通过。"
