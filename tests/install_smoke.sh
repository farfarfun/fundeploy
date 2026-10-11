#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "${TMP}"' EXIT
export HOME="${TMP}/home"
mkdir -p "$HOME"
export FUNDEPLOY_ROOT="${TMP}/nd"
export FUNDEPLOY_SKIP_PROFILE_HINT=1
export FUNDEPLOY_SKIP_GIT_PULL=1

# 拉取更新后必须由新版 install.sh 接管，避免旧复制清单漏装新文件。
UPSTREAM="${TMP}/self-update-upstream"
RUNNER="${TMP}/self-update-runner"
mkdir -p "${UPSTREAM}/scripts"
cp "${ROOT}/install.sh" "${UPSTREAM}/install.sh"
touch "${UPSTREAM}/scripts/.keep"
git -C "${UPSTREAM}" init -q
git -C "${UPSTREAM}" add .
git -C "${UPSTREAM}" -c user.name=test -c user.email=test@example.com commit -qm initial
git clone -q "${UPSTREAM}" "${RUNNER}"
cat >"${UPSTREAM}/install.sh" <<'EOF'
#!/usr/bin/env bash
touch "${FUNDEPLOY_SELF_UPDATE_MARKER:?}"
EOF
git -C "${UPSTREAM}" add install.sh
git -C "${UPSTREAM}" -c user.name=test -c user.email=test@example.com commit -qm update
FUNDEPLOY_ROOT="${TMP}/self-update-root" \
FUNDEPLOY_SELF_UPDATE_MARKER="${TMP}/self-update-ok" \
FUNDEPLOY_SKIP_GIT_PULL=0 \
  bash "${RUNNER}/install.sh" update
[[ -f "${TMP}/self-update-ok" ]] || { echo "updated install.sh did not take over" >&2; exit 1; }

# 即使源码仓库已提前更新、git pull 没有产生新提交，也必须由仓库内安装器接管。
mkdir -p "${TMP}/stale-runner"
cp "${ROOT}/install.sh" "${TMP}/stale-runner/install.sh"
FUNDEPLOY_ROOT="${TMP}/preupdated-root" \
FUNDEPLOY_SRC_DIR="${RUNNER}" \
FUNDEPLOY_SELF_UPDATE_MARKER="${TMP}/preupdated-ok" \
FUNDEPLOY_SKIP_GIT_PULL=0 \
  bash "${TMP}/stale-runner/install.sh" update
[[ -f "${TMP}/preupdated-ok" ]] || { echo "pre-updated source install.sh did not take over" >&2; exit 1; }

root_guard="$(sed -n '/^_guard_fundeploy_root() {/,/^}/p' "${ROOT}/install.sh")"
for unsafe_root in relative / /etc /tmp /var "${HOME}"; do
  if (die() { exit 1; }; eval "${root_guard}"; FUNDEPLOY_ROOT="${unsafe_root}"; _guard_fundeploy_root) 2>/dev/null; then
    echo "unsafe FUNDEPLOY_ROOT accepted: ${unsafe_root}" >&2
    exit 1
  fi
done
mkdir -p "${TMP}/safe-root"
(die() { exit 1; }; eval "${root_guard}"; FUNDEPLOY_ROOT="${TMP}/safe-root"; _guard_fundeploy_root) || exit 1

bash "${ROOT}/install.sh" install
legacy_bins=(
  nlt fundeploy-build fundeploy-tools fundeploy-dev fundeploy-ai-cli fundeploy-pip-sources fundeploy-python-env fundeploy-utils fundeploy-github-net fundeploy-port-kill fundeploy-download fundeploy-services fundeploy-cockpit-tools
  fundeploy-airflow fundeploy-celery fundeploy-paperclip fundeploy-code-server fundeploy-new-api fundeploy-sub2api fundeploy-open-pencil
  fundeploy-airflow-install fundeploy-celery-install fundeploy-celery-update fundeploy-paperclip-install fundeploy-code-server-install fundeploy-new-api-install fundeploy-sub2api-install
)
for f in "${legacy_bins[@]}"; do
  printf '#!/usr/bin/env bash\nexit 0\n' >"${FUNDEPLOY_ROOT}/bin/${f}"
  chmod +x "${FUNDEPLOY_ROOT}/bin/${f}"
done
bash "${ROOT}/install.sh" update
[[ -x "${FUNDEPLOY_ROOT}/bin/fundeploy" ]] || { echo "missing: bin/fundeploy" >&2; exit 1; }
bash -n "${FUNDEPLOY_ROOT}/bin/fundeploy" || exit 1
[[ -x "${FUNDEPLOY_ROOT}/libexec/fundeploy/tools/skills-sync/setup.sh" ]] || { echo "missing: skills-sync/setup.sh" >&2; exit 1; }
NONINTERACTIVE=1 "${FUNDEPLOY_ROOT}/bin/fundeploy" tool skills-sync --help >/dev/null || exit 1
for f in "${FUNDEPLOY_ROOT}/bin/"*; do
  [[ "$(basename "$f")" == "fundeploy" ]] || { echo "unexpected command entry: bin/$(basename "$f")" >&2; exit 1; }
done
bash -n "${FUNDEPLOY_ROOT}/libexec/fundeploy/services/airflow/setup.sh" || exit 1
bash -n "${FUNDEPLOY_ROOT}/libexec/fundeploy/services/code-server/setup.sh" || exit 1
bash -n "${FUNDEPLOY_ROOT}/libexec/fundeploy/services/code-server/setup-manual.sh" || exit 1
bash -n "${FUNDEPLOY_ROOT}/libexec/fundeploy/services/code-server/setup-offical.sh" || exit 1
bash -n "${FUNDEPLOY_ROOT}/libexec/fundeploy/services/new-api/setup.sh" || exit 1
bash -n "${FUNDEPLOY_ROOT}/libexec/fundeploy/services/open-pencil/setup.sh" || exit 1
bash -n "${FUNDEPLOY_ROOT}/libexec/fundeploy/services/paperclip/setup.sh" || exit 1
bash -n "${FUNDEPLOY_ROOT}/libexec/fundeploy/services/sub2api/setup.sh" || exit 1
bash -n "${FUNDEPLOY_ROOT}/libexec/fundeploy/services/sub2api/setup-manual.sh" || exit 1
bash -n "${FUNDEPLOY_ROOT}/libexec/fundeploy/services/sub2api/setup-offical.sh" || exit 1
bash -n "${FUNDEPLOY_ROOT}/libexec/fundeploy/services/funflix-web/setup.sh" || exit 1
bash -n "${FUNDEPLOY_ROOT}/libexec/fundeploy/services/funread-web/setup.sh" || exit 1
bash -n "${FUNDEPLOY_ROOT}/libexec/fundeploy/services/funlesson-web/setup.sh" || exit 1
bash -n "${FUNDEPLOY_ROOT}/libexec/fundeploy/services/funfluid-web/setup.sh" || exit 1
bash -n "${FUNDEPLOY_ROOT}/libexec/fundeploy/services/fungame-web/setup.sh" || exit 1
curl() {
  [[ "$*" == "--proto =https --proto-redir =https --tlsv1.2 -LsSf https://example.invalid/install.sh" ]] || return 64
  printf '%s\n' '[[ "$*" == "update --source github" ]]'
}
export -f curl
FUNDEPLOY_GITHUB_RAW="https://example.invalid/install.sh" "${FUNDEPLOY_ROOT}/bin/fundeploy" upgrade github >/dev/null
curl() {
  [[ "$*" == "--proto =https --proto-redir =https --tlsv1.2 -LsSf https://gitee.example.invalid/install.sh" ]] || return 64
  printf '%s\n' '[[ "$*" == "update --source gitee" ]]'
}
export -f curl
FUNDEPLOY_GITEE_RAW="https://gitee.example.invalid/install.sh" "${FUNDEPLOY_ROOT}/bin/fundeploy" upgrade gitee >/dev/null
unset -f curl
# sub2api 官方模式现在会「先下载到文件、校验、改写端口，再以 root 执行文件」，
# 而不是 `curl … | sudo bash`。因此 stub 必须支持 -o <file>。
curl() {
  local out="" prev=""
  for a in "$@"; do
    [[ "$prev" == "-o" ]] && out="$a"
    prev="$a"
  done
  local body
  body="$(printf '%s\n' 'SERVER_PORT="8080"' 'echo "official-sub2api-installer-ran:${1}:${SERVER_PORT}"')"
  if [[ -n "$out" ]]; then printf '%s\n' "$body" >"$out"; else printf '%s\n' "$body"; fi
}
sudo() { "$@"; }
export -f curl sudo
# stub 内容与仓库记录的 pinned 校验和自然不同，这里显式关闭校验。
# 真实场景下的校验行为由 tests/supplychain_smoke.sh 覆盖。
export SUB2API_INSTALLER_SHA256=""
"${FUNDEPLOY_ROOT}/bin/fundeploy" service sub2api official install | grep -q "official-sub2api-installer-ran:install:8802" || exit 1
"${FUNDEPLOY_ROOT}/bin/fundeploy" service sub2api install | grep -q "official-sub2api-installer-ran:install:8802" || exit 1
"${FUNDEPLOY_ROOT}/bin/fundeploy" service sub2api official update | grep -q "official-sub2api-installer-ran:upgrade:8802" || exit 1
"${FUNDEPLOY_ROOT}/bin/fundeploy" service sub2api official uninstall -y | grep -q "official-sub2api-installer-ran:uninstall:8802" || exit 1
unset -f curl sudo
curl() { printf '%s\n' 'echo "official-code-server-installer-ran:$*"'; }
export -f curl
"${FUNDEPLOY_ROOT}/bin/fundeploy" service code-server official install --version 4.112.0 | grep -q "official-code-server-installer-ran:--version 4.112.0" || exit 1
"${FUNDEPLOY_ROOT}/bin/fundeploy" service code-server install --version 4.112.0 | grep -q "official-code-server-installer-ran:--version 4.112.0" || exit 1
"${FUNDEPLOY_ROOT}/bin/fundeploy" service install add code-server | grep -q "official-code-server-installer-ran:" || exit 1
unset -f curl
PAPERCLIP_NPX_MARKER="${TMP}/paperclip-npx"
PAPERCLIP_CLI_MARKER="${TMP}/paperclip-cli"
PAPERCLIP_ENV_MARKER="${TMP}/paperclip-env"
PAPERCLIP_STARTED_MARKER="${TMP}/paperclip-started"
PAPERCLIP_STOPPED_MARKER="${TMP}/paperclip-stopped"
# upgrade 先问官方 update --check --json 要不要动、是不是降级，再决定停不停服。两条渠道
# 各返回一种结果，用来分别覆盖「有新版可升」和「目标版本更旧（需确认）」两条分支。
# --check 刻意用退出码 10 表示「有可用更新」（payload 里 `if (options.check)
# process.exitCode = 10`，像 diff 那样的信号式退出码），不是失败 —— 把它当失败就恰好
# 把唯一需要升级的那种情况判成「查不到版本」，所以这里照搬这个返回值。
PAPERCLIP_CHECK_LATEST='{"mode":"npm","currentVersion":"2026.1001.0","targetVersion":"2026.1008.0","updateAvailable":true,"downgrade":false,"channel":"stable"}'
PAPERCLIP_CHECK_CANARY='{"mode":"npm","currentVersion":"2026.1008.0-canary.4","targetVersion":"2026.1001.0","updateAvailable":false,"downgrade":true,"channel":"canary"}'
npx() { printf '%s\n' "$*" >"${PAPERCLIP_NPX_MARKER}"; }
paperclipai() {
  printf '%s\n' "$*" >>"${PAPERCLIP_CLI_MARKER}"
  printf '%s|%s\n' "${DATABASE_URL-unset}" "$*" >>"${PAPERCLIP_ENV_MARKER}"
  case "$*" in
    'update --latest --check --json') printf '%s\n' "${PAPERCLIP_CHECK_LATEST}"; return 10 ;;
    'update --canary --check --json') printf '%s\n' "${PAPERCLIP_CHECK_CANARY}"; return 0 ;;
    # 供 nc 存根模拟端口时序，见下方 nc()。
    'service start') : >"${PAPERCLIP_STARTED_MARKER}"; rm -f "${PAPERCLIP_STOPPED_MARKER}" ;;
    'service stop') : >"${PAPERCLIP_STOPPED_MARKER}"; rm -f "${PAPERCLIP_STARTED_MARKER}" ;;
  esac
}
export PAPERCLIP_NPX_MARKER PAPERCLIP_CLI_MARKER PAPERCLIP_ENV_MARKER
export PAPERCLIP_STARTED_MARKER PAPERCLIP_STOPPED_MARKER
export PAPERCLIP_CHECK_LATEST PAPERCLIP_CHECK_CANARY
export -f npx paperclipai
# install 的执行器优先级：本机已装的 CLI > npx。本机已有能跑的 CLI 时必须复用它 ——
# npx paperclipai@latest 会把整棵依赖树（实测 1.4G）再下一份到 ~/.npm/_npx，而它唯一的
# 用途只是执行一次 install 子命令。
PAPERCLIP_NODE_MIN_VERSION=0 bash "${FUNDEPLOY_ROOT}/libexec/fundeploy/services/paperclip/setup.sh" install-canary >/dev/null || exit 1
[[ ! -f "${PAPERCLIP_NPX_MARKER}" ]] || exit 1
grep -Fxq -- 'install --yes --canary' "${PAPERCLIP_CLI_MARKER}" || exit 1
grep -Fxq -- '--version' "${PAPERCLIP_CLI_MARKER}" || exit 1

# PAPERCLIP_INSTALL_VIA_NPX=1 仍要能强制回到 npx 路径（本机 CLI 已损坏时的出口）。
: >"${PAPERCLIP_CLI_MARKER}"
PAPERCLIP_INSTALL_VIA_NPX=1 PAPERCLIP_NODE_MIN_VERSION=0 bash "${FUNDEPLOY_ROOT}/libexec/fundeploy/services/paperclip/setup.sh" install-canary >/dev/null || exit 1
[[ "$(<"${PAPERCLIP_NPX_MARKER}")" == "--yes --registry https://registry.npmjs.org paperclipai@latest install --yes --canary" ]] || exit 1
grep -Fxq -- '--version' "${PAPERCLIP_CLI_MARKER}" || exit 1

PAPERCLIP_TEST_HOME="${TMP}/paperclip-home"
PAPERCLIP_TEST_DB="${PAPERCLIP_TEST_HOME}/instances/default/db"
mkdir -p "${PAPERCLIP_TEST_DB}"
printf '%s\n' "{\"database\":{\"mode\":\"embedded-postgres\",\"embeddedPostgresDataDir\":\"${PAPERCLIP_TEST_DB}\",\"embeddedPostgresPort\":5432},\"server\":{\"port\":8804}}" >"${PAPERCLIP_TEST_HOME}/instances/default/config.json"
: >"${PAPERCLIP_CLI_MARKER}"
for command in start stop; do
  DATABASE_URL=must-not-leak PAPERCLIP_HOME="${PAPERCLIP_TEST_HOME}" PAPERCLIP_NODE_MIN_VERSION=0 \
    bash "${FUNDEPLOY_ROOT}/libexec/fundeploy/services/paperclip/setup.sh" "$command" >/dev/null || exit 1
done
PAPERCLIP_HOME="${PAPERCLIP_TEST_HOME}" PAPERCLIP_NODE_MIN_VERSION=0 \
  bash "${FUNDEPLOY_ROOT}/libexec/fundeploy/services/paperclip/setup.sh" restart --wait >/dev/null || exit 1
PAPERCLIP_HOME="${PAPERCLIP_TEST_HOME}" PAPERCLIP_NODE_MIN_VERSION=0 \
  bash "${FUNDEPLOY_ROOT}/libexec/fundeploy/services/paperclip/setup.sh" status --json >/dev/null || exit 1
PAPERCLIP_HOME="${PAPERCLIP_TEST_HOME}" PAPERCLIP_NODE_MIN_VERSION=0 \
  bash "${FUNDEPLOY_ROOT}/libexec/fundeploy/services/paperclip/setup.sh" logs -f >/dev/null || exit 1

# 官方一等公民命令的透传：doctor / health / channels 直接转调，不做任何本地复现。
for command in doctor health channels; do
  PAPERCLIP_HOME="${PAPERCLIP_TEST_HOME}" PAPERCLIP_NODE_MIN_VERSION=0 \
    bash "${FUNDEPLOY_ROOT}/libexec/fundeploy/services/paperclip/setup.sh" "$command" >/dev/null || exit 1
done
# check 只读：问一次 update --check --json 就给结论，不许碰服务。
PAPERCLIP_HOME="${PAPERCLIP_TEST_HOME}" PAPERCLIP_NODE_MIN_VERSION=0 \
  bash "${FUNDEPLOY_ROOT}/libexec/fundeploy/services/paperclip/setup.sh" check prod |
  grep -Fq '2026.1008.0' || exit 1

# nc 既要记录探测过哪些端口，也要模拟真实时序：service stop 之前端口是开着的（升级前
# 备份要连得上 embedded PostgreSQL），停完必须变成关闭的（否则 upgrade 的「停服后端口
# 必须已释放」护栏会判定服务还在跑并中止升级），service start 之后又要重新开着（健康
# 检查才能通过）。
nc() {
  printf '%s\n' "$*" >>"${PAPERCLIP_PORT_MARKER}"
  [[ ! -f "${PAPERCLIP_STOPPED_MARKER}" ]] || [[ -f "${PAPERCLIP_STARTED_MARKER}" ]]
}
# 健康检查轮询刻意用 curl 而不是 paperclipai health：后者每次都要起一个 node 进程加载
# 整个 CLI bundle，按秒轮询最多 180 次扛不住。health 作为用户命令另行暴露（见上）。
curl() { printf '%s\n' "$*" >"${PAPERCLIP_HEALTH_MARKER}"; }
PAPERCLIP_PORT_MARKER="${TMP}/paperclip-ports"
PAPERCLIP_HEALTH_MARKER="${TMP}/paperclip-health"
export DATABASE_URL=must-not-leak PAPERCLIP_PORT_MARKER PAPERCLIP_HEALTH_MARKER
export -f nc curl
# 本次 upgrade 模拟最常见的场景：服务正在运行（端口开着），由 upgrade 自己停它。
rm -f "${PAPERCLIP_STARTED_MARKER}" "${PAPERCLIP_STOPPED_MARKER}"
PAPERCLIP_HOME="${PAPERCLIP_TEST_HOME}" PAPERCLIP_NODE_MIN_VERSION=0 \
  bash "${FUNDEPLOY_ROOT}/libexec/fundeploy/services/paperclip/setup.sh" update prod >/dev/null || exit 1
# 备份刻意在停服之前（embedded PostgreSQL 随服务一起停，停完再备份是空转），所以给官方
# update 传 --no-backup；doctor --repair 取代了此前三段 macOS arm64 专属补丁，且刻意在
# start 之前（官方 paperclipai run 自己也是 onboard + doctor --repair 后才启动）；start
# 先探一次 service status --json，才能决定走官方 supervisor 还是 fundeploy 自管的后台
# 进程（容器 / WSL1 / 无登录会话的机器上官方 service start 是空转）。
[[ "$(tail -n 6 "${PAPERCLIP_CLI_MARKER}")" == $'db:backup\nservice stop\nupdate --latest --no-backup\ndoctor --repair --yes\nservice status --json\nservice start' ]] || exit 1
grep -Fxq -- '-z 127.0.0.1 5432' "${PAPERCLIP_PORT_MARKER}" || exit 1
grep -Fxq -- '-z 127.0.0.1 8804' "${PAPERCLIP_PORT_MARKER}" || exit 1
grep -Fq -- 'http://127.0.0.1:8804/api/health' "${PAPERCLIP_HEALTH_MARKER}" || exit 1
! grep -Fq 'must-not-leak|' "${PAPERCLIP_ENV_MARKER}" || exit 1
PAPERCLIP_HOME="${PAPERCLIP_TEST_HOME}" PAPERCLIP_NODE_MIN_VERSION=0 PAPERCLIP_UNINSTALL_YES=1 \
  bash "${FUNDEPLOY_ROOT}/libexec/fundeploy/services/paperclip/setup.sh" uninstall >/dev/null || exit 1
for expected in "service start" "service stop" "service restart --wait" "service status --json" "service logs -f" \
  "doctor" "health" "channels" "update --latest --check --json" "db:backup" "update --latest --no-backup" \
  "doctor --repair --yes" "service uninstall" "uninstall"; do
  grep -Fxq -- "$expected" "${PAPERCLIP_CLI_MARKER}" || exit 1
done

# canary 机器上 --latest 指向的 stable 常比当前版本旧，官方 update 把这种情况当显式降级、
# 要求 -y 确认。非交互且未显式确认时必须在动手之前就拒绝（连备份都不做），否则会卡在
# update 自己的交互提示上。
: >"${PAPERCLIP_CLI_MARKER}"
rm -f "${PAPERCLIP_STARTED_MARKER}" "${PAPERCLIP_STOPPED_MARKER}"
NONINTERACTIVE=1 PAPERCLIP_HOME="${PAPERCLIP_TEST_HOME}" PAPERCLIP_NODE_MIN_VERSION=0 \
  bash "${FUNDEPLOY_ROOT}/libexec/fundeploy/services/paperclip/setup.sh" upgrade canary >/dev/null 2>&1 && exit 1
grep -Fxq -- 'update --canary --check --json' "${PAPERCLIP_CLI_MARKER}" || exit 1
! grep -Fxq -- 'db:backup' "${PAPERCLIP_CLI_MARKER}" || exit 1
# 显式确认后才补 -y 交给官方 update。
: >"${PAPERCLIP_CLI_MARKER}"
NONINTERACTIVE=1 PAPERCLIP_UPGRADE_YES=1 PAPERCLIP_HOME="${PAPERCLIP_TEST_HOME}" PAPERCLIP_NODE_MIN_VERSION=0 \
  bash "${FUNDEPLOY_ROOT}/libexec/fundeploy/services/paperclip/setup.sh" upgrade canary >/dev/null || exit 1
grep -Fxq -- 'update --canary -y --no-backup' "${PAPERCLIP_CLI_MARKER}" || exit 1

# rollback 与 upgrade 共用同一套停服 → update → doctor --repair → 重启编排。
# 这次从「服务已经停着」开始：embedded PostgreSQL 随服务一起停，连不上，升级前备份必须
# 跳过而不是让 db:backup 失败把整个 rollback 中止掉。
: >"${PAPERCLIP_CLI_MARKER}"
rm -f "${PAPERCLIP_STARTED_MARKER}"
: >"${PAPERCLIP_STOPPED_MARKER}"
PAPERCLIP_HOME="${PAPERCLIP_TEST_HOME}" PAPERCLIP_NODE_MIN_VERSION=0 \
  bash "${FUNDEPLOY_ROOT}/libexec/fundeploy/services/paperclip/setup.sh" rollback >/dev/null || exit 1
[[ "$(<"${PAPERCLIP_CLI_MARKER}")" == $'service stop\nupdate --rollback --no-backup\ndoctor --repair --yes\nservice status --json\nservice start' ]] || exit 1

PAPERCLIP_PLUGIN_MARKER="${TMP}/paperclip-plugin-installed"
gum() {
  local line pick=""
  while IFS= read -r line; do
    [[ "$line" == paperclip-aperture\ -* ]] && pick="$line"
  done
  [[ -n "$pick" ]] && printf '%s\n' "$pick"
}
paperclipai() { printf '%s\n' "$*" >"${PAPERCLIP_PLUGIN_MARKER}"; }
export PAPERCLIP_PLUGIN_MARKER
export -f gum paperclipai
bash "${FUNDEPLOY_ROOT}/libexec/fundeploy/services/paperclip/setup.sh" plugin list | grep -q '^paperclip-aperture (@tomismeta/paperclip-aperture)$' || exit 1
PAPERCLIP_NODE_MIN_VERSION=0 bash "${FUNDEPLOY_ROOT}/libexec/fundeploy/services/paperclip/setup.sh" plugin install >/dev/null || exit 1
[[ "$(<"${PAPERCLIP_PLUGIN_MARKER}")" == "plugin install @tomismeta/paperclip-aperture" ]] || exit 1
unset DATABASE_URL
unset -f npx gum paperclipai nc curl
bash -n "${FUNDEPLOY_ROOT}/libexec/fundeploy/services/fundeploy-services.sh" || exit 1
bash -n "${FUNDEPLOY_ROOT}/libexec/fundeploy/tools/port-kill/setup.sh" || exit 1
bash -n "${FUNDEPLOY_ROOT}/libexec/fundeploy/tools/brew/setup.sh" || exit 1
bash "${FUNDEPLOY_ROOT}/libexec/fundeploy/tools/brew/setup.sh" --help >/dev/null || exit 1
brew() { [[ "${1:-}" == "--version" || "${1:-}" == "update" ]]; }
export -f brew
bash "${FUNDEPLOY_ROOT}/libexec/fundeploy/tools/brew/setup.sh" install >/dev/null || exit 1
bash "${FUNDEPLOY_ROOT}/libexec/fundeploy/tools/brew/setup.sh" update || exit 1
unset -f brew
bash -n "${FUNDEPLOY_ROOT}/libexec/fundeploy/tools/download/setup.sh" || exit 1
bash -n "${FUNDEPLOY_ROOT}/libexec/fundeploy/dev/setup.sh" || exit 1
bash -n "${FUNDEPLOY_ROOT}/libexec/fundeploy/ai-cli/setup.sh" || exit 1
for tool in claude codex cursor; do
  bash -n "${FUNDEPLOY_ROOT}/libexec/fundeploy/ai-cli/${tool}/setup.sh" || exit 1
done
curl() {
  case "$*" in
    *claude.ai*) echo 'echo ai-official:claude' ;;
    *chatgpt.com*) echo 'echo ai-official:codex' ;;
    *cursor.com*) echo 'echo ai-official:cursor' ;;
  esac
}
npm() { printf 'npm:%s\n' "$*"; }
pnpm() { printf 'pnpm:%s\n' "$*"; }
brew() { printf 'brew:%s\n' "$*"; }
export -f curl npm pnpm brew
"${FUNDEPLOY_ROOT}/bin/fundeploy" ai claude official install | grep -q 'ai-official:claude' || exit 1
"${FUNDEPLOY_ROOT}/bin/fundeploy" ai claude pnpm install | grep -q 'pnpm:add -g @anthropic-ai/claude-code@latest' || exit 1
"${FUNDEPLOY_ROOT}/bin/fundeploy" ai codex install | grep -q 'ai-official:codex' || exit 1
"${FUNDEPLOY_ROOT}/bin/fundeploy" ai codex npm install | grep -q 'npm:install -g @openai/codex@latest' || exit 1
"${FUNDEPLOY_ROOT}/bin/fundeploy" ai codex brew update | grep -q 'brew:upgrade --cask codex' || exit 1
"${FUNDEPLOY_ROOT}/bin/fundeploy" ai cursor official install | grep -q 'ai-official:cursor' || exit 1
unset -f curl npm pnpm brew
mkdir -p "${TMP}/ai-home/.local/bin" "${TMP}/ai-home/.codex/packages/standalone"
touch "${TMP}/ai-home/.local/bin/codex"
HOME="${TMP}/ai-home" FUNDEPLOY_ASSUME_YES=1 "${FUNDEPLOY_ROOT}/bin/fundeploy" ai codex official uninstall
[[ ! -e "${TMP}/ai-home/.local/bin/codex" && ! -e "${TMP}/ai-home/.codex/packages/standalone" ]] || exit 1
bash -n "${FUNDEPLOY_ROOT}/libexec/fundeploy/dev/go/setup.sh" || exit 1
bash -n "${FUNDEPLOY_ROOT}/libexec/fundeploy/dev/rust/setup.sh" || exit 1
bash -n "${FUNDEPLOY_ROOT}/libexec/fundeploy/dev/nodejs/setup.sh" || exit 1
bash -n "${FUNDEPLOY_ROOT}/libexec/fundeploy/dev/pnpm/setup.sh" || exit 1
bash -n "${FUNDEPLOY_ROOT}/libexec/fundeploy/dev/uv/setup.sh" || exit 1
out="$(NONINTERACTIVE=1 "${FUNDEPLOY_ROOT}/bin/fundeploy" list)"
grep -q "service" <<<"${out}" || exit 1
out="$(NONINTERACTIVE=1 "${FUNDEPLOY_ROOT}/bin/fundeploy" service list)"
grep -q "sub2api" <<<"${out}" || exit 1
grep -q "funflix-web" <<<"${out}" || exit 1
grep -q "funread-web" <<<"${out}" || exit 1
grep -q "funlesson-web" <<<"${out}" || exit 1

for service in funflix-web funread-web funlesson-web funfluid-web fungame-web; do
  out="$(NONINTERACTIVE=1 "${FUNDEPLOY_ROOT}/bin/fundeploy" service "${service}" --help)"
  grep -q '^  run ' <<<"${out}" || exit 1
done

# funread-web 已和 funflix-web / funlesson-web 一样改为「装正式包 + 转发上游 CLI」：
# start/run/stop/status 都是转发给 funread-api / funread-web 自己的 server 子命令，
# PID 与日志归那两个 CLI 管，脚本里不再有 Bash 托管的 .run/ 目录。这里正反各断一条，
# 防止哪天又退回到脚本自己 nohup + 写 PID 文件的旧形态。负向断言特意写成带斜杠的
# `.run/` 目录形式，而不是裸 `nohup`：文件头注释里就有「不 nohup」这句话，按词断言会误命中。
out="$(NONINTERACTIVE=1 "${FUNDEPLOY_ROOT}/bin/fundeploy" service funread-web --help)"
grep -q '^  rollback ' <<<"${out}" || exit 1
funread_setup="${FUNDEPLOY_ROOT}/libexec/fundeploy/services/funread-web/setup.sh"
grep -q 'funread-api server start' "${funread_setup}" || exit 1
grep -q 'funread-web server start' "${funread_setup}" || exit 1
! grep -qE '^[^#]*\.run/' "${funread_setup}" || exit 1
# funread 1.1.111 的 server start 要求默认 config.toml 已存在（缺了只报一行
# config file not found，而同一份安装 server run 完全正常），生成它的责任落在部署侧。
# install / start / run 三处都得调，少一处就是「装完起不来」或「旧机器起不来」。
# 这里只能按内容断言：真跑一次需要网络装包，而不装包的话 start 会先在 _require_cli
# 上退出，根本走不到生成配置那一步。
grep -q '^_ensure_default_configs()' "${funread_setup}" || exit 1
[[ "$(grep -c '^  _ensure_default_configs$' "${funread_setup}")" == 3 ]] || exit 1
# 那份 config.toml 写到哪个目录不写死，问 CLI 自己（两个 CLI 的 `server status` 输出里
# 都带 `config <路径>`，服务没在跑也会打）。funread 1.1.114 把布局从
# ${XDG_CONFIG_HOME:-~/.config}/farfarfun/funread-{api,web}/ 换到了
# ~/.farfarfun/funread/{api,web}/，而 `rollback 1.1.113` 能把一台机器退回旧布局，所以
# 两边任选一个写死都会让配置建到 CLI 不看的地方 —— 症状和它要修的那个缺陷一模一样。
# 这条不用按内容断言：--help 的说明里就会打出解析到的目录，拿假 CLI 顶上去就能真跑。
fake_cli="${TMP}/fake-funread-cli"
mkdir -p "${fake_cli}"
printf '#!/usr/bin/env bash\necho "funread-api not running (config /opt/probe/api/config.toml)"\n' \
  >"${fake_cli}/funread-api"
printf '#!/usr/bin/env bash\necho "funread-web not running (config /opt/probe/web/config.toml)"\n' \
  >"${fake_cli}/funread-web"
chmod +x "${fake_cli}/funread-api" "${fake_cli}/funread-web"
out="$(PATH="${fake_cli}:${PATH}" NONINTERACTIVE=1 "${FUNDEPLOY_ROOT}/bin/fundeploy" service funread-web --help)"
grep -qF '/opt/probe/api/funread-api.{pid,log}' <<<"${out}" || exit 1
grep -qF '/opt/probe/web/funread-web.{pid,log}' <<<"${out}" || exit 1
# 问不出来（CLI 没装、跑挂了、或上游改了输出格式）必须退到新布局的默认值，不能退成空串：
# 空串会让 _ensure_default_config 去写 /config.toml。两个假 CLI 分别覆盖「退出码非 0」
# 与「有输出但格式不对」这两种问不出来的方式。
printf '#!/usr/bin/env bash\nexit 1\n' >"${fake_cli}/funread-api"
printf '#!/usr/bin/env bash\necho "unparseable"\n' >"${fake_cli}/funread-web"
out="$(PATH="${fake_cli}:${PATH}" NONINTERACTIVE=1 "${FUNDEPLOY_ROOT}/bin/fundeploy" service funread-web --help)"
grep -qF "${HOME}/.farfarfun/funread/api/funread-api.{pid,log}" <<<"${out}" || exit 1
grep -qF "${HOME}/.farfarfun/funread/web/funread-web.{pid,log}" <<<"${out}" || exit 1
# upgrade 与 rollback 都得先停服再换包：在跑着的进程底下换全局 npm 包，node 还开着那棵树
# 里的文件；而且旧进程的 PID 文件在旧配置目录里，换完包的 CLI 只看新目录，stop 会报
# 「not running」而端口还被占着。存活判断必须走端口而不是 PID 文件 —— 路径本身会变。
[[ "$(grep -c '^  _stop_if_running$' "${funread_setup}")" == 2 ]] || exit 1
sed -n '/^_stop_if_running() {/,/^}/p' "${funread_setup}" | grep -q 'listener_pid_for_port' || exit 1
NONINTERACTIVE=1 "${FUNDEPLOY_ROOT}/bin/fundeploy" dev --help >/dev/null || exit 1
out="$(NONINTERACTIVE=1 "${FUNDEPLOY_ROOT}/bin/fundeploy" tool list)"
grep -q "brew" <<<"${out}" || exit 1
out="$(NONINTERACTIVE=1 "${FUNDEPLOY_ROOT}/bin/fundeploy" ai list)"
grep -q "codex" <<<"${out}" || exit 1
out="$(FUNDEPLOY_PACKAGE_MANAGER=apt NONINTERACTIVE=1 "${FUNDEPLOY_ROOT}/bin/fundeploy" upgrade --source github)"
grep -q "apt install --only-upgrade fundeploy" <<<"${out}" || exit 1
out="$(FUNDEPLOY_PACKAGE_MANAGER=apt NONINTERACTIVE=1 "${FUNDEPLOY_ROOT}/bin/fundeploy" uninstall)"
grep -q "apt remove fundeploy" <<<"${out}" || exit 1
out="$(FUNDEPLOY_PACKAGE_MANAGER=brew NONINTERACTIVE=1 "${FUNDEPLOY_ROOT}/bin/fundeploy" upgrade)"
grep -q "brew upgrade fundeploy" <<<"${out}" || exit 1
out="$(FUNDEPLOY_PACKAGE_MANAGER=brew NONINTERACTIVE=1 "${FUNDEPLOY_ROOT}/bin/fundeploy" uninstall)"
grep -q "brew uninstall fundeploy" <<<"${out}" || exit 1
systemctl() {
  [[ "${1:-}" == "cat" ]] && return 0
  printf 'systemctl:%s\n' "$*"
}
brew() { printf 'brew:%s\n' "$*"; }
curl() { return 1; }
sudo() { "$@"; }
export -f systemctl brew curl sudo
if [[ "$(uname -s)" == "Darwin" ]]; then
  "${FUNDEPLOY_ROOT}/bin/fundeploy" service code-server official start | grep -q "brew:services start code-server" || exit 1
  "${FUNDEPLOY_ROOT}/bin/fundeploy" service code-server official restart | grep -q "brew:services restart code-server" || exit 1
  "${FUNDEPLOY_ROOT}/bin/fundeploy" service code-server status | grep -q "brew:services info code-server" || exit 1
else
  CODE_SERVER_OFFICIAL_USER=tester "${FUNDEPLOY_ROOT}/bin/fundeploy" service code-server official start | grep -q "systemctl:enable --now code-server@tester" || exit 1
  CODE_SERVER_OFFICIAL_USER=tester "${FUNDEPLOY_ROOT}/bin/fundeploy" service code-server official restart | grep -q "systemctl:restart code-server@tester" || exit 1
  CODE_SERVER_OFFICIAL_USER=tester "${FUNDEPLOY_ROOT}/bin/fundeploy" service code-server status | grep -q "systemctl:status code-server@tester --no-pager" || exit 1
fi
out="$(HOME="${TMP}/manual-home" PATH=/usr/bin:/bin CODE_SERVER_SERVICE_HOME="${TMP}/manual-code-server" CODE_SERVER_BIND=127.0.0.1:59997 "${FUNDEPLOY_ROOT}/bin/fundeploy" service code-server manual status)"
grep -q "CODE_SERVER_SERVICE_HOME" <<<"${out}" || exit 1
! grep -q "systemctl:" <<<"${out}" || exit 1
"${FUNDEPLOY_ROOT}/bin/fundeploy" service sub2api official restart | grep -q "systemctl:restart sub2api" || exit 1
"${FUNDEPLOY_ROOT}/bin/fundeploy" service sub2api status | grep -q "systemctl:status sub2api --no-pager" || exit 1
out="$(SUB2API_SERVICE_HOME="${TMP}/manual-sub2api" SUB2API_PORT=59998 "${FUNDEPLOY_ROOT}/bin/fundeploy" service sub2api manual status)"
grep -q "PID 文件" <<<"${out}" || exit 1
! grep -q "systemctl:" <<<"${out}" || exit 1
unset -f systemctl brew curl sudo
out="$(NONINTERACTIVE=1 "${FUNDEPLOY_ROOT}/bin/fundeploy" tool list)"
grep -q "github-net" <<<"${out}" || exit 1
grep -q "brew" <<<"${out}" || exit 1
NONINTERACTIVE=1 "${FUNDEPLOY_ROOT}/bin/fundeploy" dev --help >/dev/null || exit 1
mkdir -p "${TMP}/status-home/.paperclip/instances/default"
printf '%s\n' '{"server":{"host":"127.0.0.2","port":43100}}' >"${TMP}/status-home/.paperclip/instances/default/config.json"
# status 用 node 解析 paperclip 的 config.json；GitHub Actions 的 node 装在
# hostedtoolcache 里而非 /usr/bin，PATH=/usr/bin:/bin 会让 command -v node
# 找不到它，host/port 静默回退到默认值，下面的 grep 就再也匹配不到。
status_path="/usr/bin:/bin"
node_bin="$(command -v node 2>/dev/null || true)"
[[ -n "${node_bin}" ]] && status_path="$(dirname "${node_bin}"):${status_path}"
out="$(HOME="${TMP}/status-home" PATH="${status_path}" NONINTERACTIVE=1 "${FUNDEPLOY_ROOT}/bin/fundeploy" service status --no-http)"
grep -q '^服务,状态,PID,端口/访问,HTTP$' <<<"${out}" || exit 1
grep -q '127.0.0.2:43100' <<<"${out}" || exit 1
NONINTERACTIVE=1 "${FUNDEPLOY_ROOT}/bin/fundeploy" dev --help >/dev/null || exit 1
NONINTERACTIVE=1 "${FUNDEPLOY_ROOT}/bin/fundeploy" dev uv --help >/dev/null || exit 1
NONINTERACTIVE=1 "${FUNDEPLOY_ROOT}/bin/fundeploy" ai --help >/dev/null || exit 1
NONINTERACTIVE=1 "${FUNDEPLOY_ROOT}/bin/fundeploy" ai list | grep -q "claude" || exit 1
NONINTERACTIVE=1 "${FUNDEPLOY_ROOT}/bin/fundeploy" tool download resolve-url "https://github.com/foo/bar" | grep -q "https://github.com/foo/bar" || exit 1
NONINTERACTIVE=1 "${FUNDEPLOY_ROOT}/bin/fundeploy" tool port-kill list 59999 >/dev/null || exit 1
NONINTERACTIVE=1 "${FUNDEPLOY_ROOT}/bin/fundeploy" tool port-kill install >/dev/null || exit 1
"${FUNDEPLOY_ROOT}/bin/fundeploy" service status --no-http >/dev/null || exit 1
echo "install_smoke OK"
