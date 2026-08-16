#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "${TMP}"' EXIT
export HOME="${TMP}/home"
mkdir -p "$HOME"
export NLTDEPLOY_ROOT="${TMP}/nd"
export NLTDEPLOY_SKIP_PROFILE_HINT=1
export NLTDEPLOY_SKIP_GIT_PULL=1

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
touch "${NLTDEPLOY_SELF_UPDATE_MARKER:?}"
EOF
git -C "${UPSTREAM}" add install.sh
git -C "${UPSTREAM}" -c user.name=test -c user.email=test@example.com commit -qm update
NLTDEPLOY_ROOT="${TMP}/self-update-root" \
NLTDEPLOY_SELF_UPDATE_MARKER="${TMP}/self-update-ok" \
NLTDEPLOY_SKIP_GIT_PULL=0 \
  bash "${RUNNER}/install.sh" update
[[ -f "${TMP}/self-update-ok" ]] || { echo "updated install.sh did not take over" >&2; exit 1; }

# 即使源码仓库已提前更新、git pull 没有产生新提交，也必须由仓库内安装器接管。
mkdir -p "${TMP}/stale-runner"
cp "${ROOT}/install.sh" "${TMP}/stale-runner/install.sh"
NLTDEPLOY_ROOT="${TMP}/preupdated-root" \
NLTDEPLOY_SRC_DIR="${RUNNER}" \
NLTDEPLOY_SELF_UPDATE_MARKER="${TMP}/preupdated-ok" \
NLTDEPLOY_SKIP_GIT_PULL=0 \
  bash "${TMP}/stale-runner/install.sh" update
[[ -f "${TMP}/preupdated-ok" ]] || { echo "pre-updated source install.sh did not take over" >&2; exit 1; }

root_guard="$(sed -n '/^_guard_nltdeploy_root() {/,/^}/p' "${ROOT}/install.sh")"
for unsafe_root in relative / /tmp "${HOME}"; do
  if (die() { exit 1; }; eval "${root_guard}"; NLTDEPLOY_ROOT="${unsafe_root}"; _guard_nltdeploy_root) 2>/dev/null; then
    echo "unsafe NLTDEPLOY_ROOT accepted: ${unsafe_root}" >&2
    exit 1
  fi
done
mkdir -p "${TMP}/safe-root"
(die() { exit 1; }; eval "${root_guard}"; NLTDEPLOY_ROOT="${TMP}/safe-root"; _guard_nltdeploy_root) || exit 1

bash "${ROOT}/install.sh" install
legacy_bins=(
  nlt nlt-build nlt-tools nlt-dev nlt-ai-cli nlt-pip-sources nlt-python-env nlt-utils nlt-github-net nlt-port-kill nlt-download nlt-services nlt-cockpit-tools
  nlt-airflow nlt-celery nlt-paperclip nlt-code-server nlt-new-api nlt-sub2api nlt-open-pencil
  nlt-airflow-install nlt-celery-install nlt-celery-update nlt-paperclip-install nlt-code-server-install nlt-new-api-install nlt-sub2api-install
)
for f in "${legacy_bins[@]}"; do
  printf '#!/usr/bin/env bash\nexit 0\n' >"${NLTDEPLOY_ROOT}/bin/${f}"
  chmod +x "${NLTDEPLOY_ROOT}/bin/${f}"
done
bash "${ROOT}/install.sh" update
[[ -x "${NLTDEPLOY_ROOT}/bin/nltdeploy" ]] || { echo "missing: bin/nltdeploy" >&2; exit 1; }
bash -n "${NLTDEPLOY_ROOT}/bin/nltdeploy" || exit 1
[[ -x "${NLTDEPLOY_ROOT}/libexec/nltdeploy/skills-sync/setup.sh" ]] || { echo "missing: skills-sync/setup.sh" >&2; exit 1; }
NONINTERACTIVE=1 "${NLTDEPLOY_ROOT}/bin/nltdeploy" tool skills-sync --help >/dev/null || exit 1
for f in "${NLTDEPLOY_ROOT}/bin/"*; do
  [[ "$(basename "$f")" == "nltdeploy" ]] || { echo "unexpected command entry: bin/$(basename "$f")" >&2; exit 1; }
done
bash -n "${NLTDEPLOY_ROOT}/libexec/nltdeploy/airflow/setup.sh" || exit 1
bash -n "${NLTDEPLOY_ROOT}/libexec/nltdeploy/code-server/setup.sh" || exit 1
bash -n "${NLTDEPLOY_ROOT}/libexec/nltdeploy/code-server/setup-manual.sh" || exit 1
bash -n "${NLTDEPLOY_ROOT}/libexec/nltdeploy/code-server/setup-offical.sh" || exit 1
bash -n "${NLTDEPLOY_ROOT}/libexec/nltdeploy/new-api/setup.sh" || exit 1
bash -n "${NLTDEPLOY_ROOT}/libexec/nltdeploy/open-pencil/setup.sh" || exit 1
bash -n "${NLTDEPLOY_ROOT}/libexec/nltdeploy/paperclip/setup.sh" || exit 1
bash -n "${NLTDEPLOY_ROOT}/libexec/nltdeploy/sub2api/setup.sh" || exit 1
bash -n "${NLTDEPLOY_ROOT}/libexec/nltdeploy/sub2api/setup-manual.sh" || exit 1
bash -n "${NLTDEPLOY_ROOT}/libexec/nltdeploy/sub2api/setup-offical.sh" || exit 1
curl() {
  [[ "$*" == "--proto =https --proto-redir =https --tlsv1.2 -LsSf https://example.invalid/install.sh" ]] || return 64
  printf '%s\n' '[[ "$*" == "update --source github" ]]'
}
export -f curl
NLTDEPLOY_GITHUB_RAW="https://example.invalid/install.sh" "${NLTDEPLOY_ROOT}/bin/nltdeploy" upgrade github >/dev/null
curl() {
  [[ "$*" == "--proto =https --proto-redir =https --tlsv1.2 -LsSf https://gitee.example.invalid/install.sh" ]] || return 64
  printf '%s\n' '[[ "$*" == "update --source gitee" ]]'
}
export -f curl
NLTDEPLOY_GITEE_RAW="https://gitee.example.invalid/install.sh" "${NLTDEPLOY_ROOT}/bin/nltdeploy" upgrade gitee >/dev/null
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
"${NLTDEPLOY_ROOT}/bin/nltdeploy" service sub2api official install | grep -q "official-sub2api-installer-ran:install:8802" || exit 1
"${NLTDEPLOY_ROOT}/bin/nltdeploy" service sub2api install | grep -q "official-sub2api-installer-ran:install:8802" || exit 1
"${NLTDEPLOY_ROOT}/bin/nltdeploy" service sub2api official update | grep -q "official-sub2api-installer-ran:upgrade:8802" || exit 1
"${NLTDEPLOY_ROOT}/bin/nltdeploy" service sub2api official uninstall -y | grep -q "official-sub2api-installer-ran:uninstall:8802" || exit 1
unset -f curl sudo
curl() { printf '%s\n' 'echo "official-code-server-installer-ran:$*"'; }
export -f curl
"${NLTDEPLOY_ROOT}/bin/nltdeploy" service code-server official install --version 4.112.0 | grep -q "official-code-server-installer-ran:--version 4.112.0" || exit 1
"${NLTDEPLOY_ROOT}/bin/nltdeploy" service code-server install --version 4.112.0 | grep -q "official-code-server-installer-ran:--version 4.112.0" || exit 1
"${NLTDEPLOY_ROOT}/bin/nltdeploy" service install add code-server | grep -q "official-code-server-installer-ran:" || exit 1
unset -f curl
node() { [[ "${1:-}" == "-p" ]] && echo 20; }
pnpm() {
  [[ "$*" == "config get registry" ]] && { echo "https://registry.npmjs.org/"; return; }
  [[ "$*" == "add -g --registry https://registry.npmjs.org paperclipai@latest" ]]
}
paperclipai() { [[ "${1:-}" == "--version" ]]; }
export -f node pnpm paperclipai
bash "${NLTDEPLOY_ROOT}/libexec/nltdeploy/paperclip/setup.sh" install >/dev/null || exit 1
unset -f node pnpm paperclipai
node() { [[ "${1:-}" == "-p" ]] && echo 20; }
pnpm() {
  case "$*" in
    "config get registry") echo "https://packages.example.com/npm/" ;;
    "view paperclipai@latest version --registry https://registry.npmjs.org") echo "2026.810.0" ;;
    "view paperclipai@latest version --registry https://packages.example.com/npm/") echo "2026.811.0" ;;
    "view paperclipai@>2026.810.0 <=2026.811.0 version --registry https://packages.example.com/npm/") echo "2026.811.0" ;;
    "add -g --registry https://packages.example.com/npm/ paperclipai@2026.811.0") ;;
    *) return 1 ;;
  esac
}
paperclipai() { [[ "${1:-}" == "--version" ]]; }
export -f node pnpm paperclipai
bash "${NLTDEPLOY_ROOT}/libexec/nltdeploy/paperclip/setup.sh" install >/dev/null || exit 1
unset -f node pnpm paperclipai
PAPERCLIP_MIGRATION_MARKER="${TMP}/paperclip-migrated"
PAPERCLIP_MANAGED_ENTRYPOINT="${TMP}/paperclip-home/cli/current/node_modules/paperclipai/dist/index.js"
PAPERCLIP_PNPM_BIN="${TMP}/paperclip-pnpm-bin"
mkdir -p "${PAPERCLIP_PNPM_BIN}" "${TMP}/paperclip-home/instances/default" "$(dirname "${PAPERCLIP_MANAGED_ENTRYPOINT}")"
printf '{}\n' >"${TMP}/paperclip-home/instances/default/config.json"
printf '%s\n' 'paperclipai managed install store v1' >"${TMP}/paperclip-home/cli/.managed-install"
mkdir -p "${HOME}/.local/bin"
ln -s "${PAPERCLIP_MANAGED_ENTRYPOINT}" "${HOME}/.local/bin/paperclipai"
{
  printf '%s\n' '#!/usr/bin/env bash'
  printf '%s\n' 'case "${1:-}" in'
  printf '%s\n' '  uninstall) rm -rf "${PAPERCLIP_HOME}/cli" ;;'
  printf '%s\n' '  *) exit 1 ;;'
  printf '%s\n' 'esac'
} >"${PAPERCLIP_MANAGED_ENTRYPOINT}"
chmod +x "${PAPERCLIP_MANAGED_ENTRYPOINT}"
{
  printf '%s\n' '#!/usr/bin/env bash'
  printf '%s\n' 'case "${1:-}" in'
  printf '%s\n' '  --version) echo test ;;'
  printf '%s\n' '  run) printf "%s\n" "${PAPERCLIP_MIGRATION_AUTO_APPLY:-}" >"${PAPERCLIP_MIGRATION_MARKER}"; trap "exit 0" TERM INT; while :; do sleep 1; done ;;'
  printf '%s\n' '  *) exit 1 ;;'
  printf '%s\n' 'esac'
} >"${PAPERCLIP_PNPM_BIN}/paperclipai"
chmod +x "${PAPERCLIP_PNPM_BIN}/paperclipai"
node() {
  [[ "${1:-}" == "-p" ]] && { echo 20; return; }
  [[ "${1:-}" == "${PAPERCLIP_MANAGED_ENTRYPOINT}" ]] && { shift; bash "${PAPERCLIP_MANAGED_ENTRYPOINT}" "$@"; }
}
npm() {
  [[ "$*" == "uninstall -g paperclipai" ]] || return 1
  rm -f "${HOME}/.local/bin/paperclipai"
}
pnpm() {
  [[ "$*" == "config get registry" ]] && { echo "https://registry.npmjs.org/"; return; }
  [[ "$*" == "add -g --registry https://registry.npmjs.org paperclipai@latest" ]]
}
curl() { [[ "$*" == *"/api/health"* ]] && printf 200; }
export -f node npm pnpm curl
PATH="${PAPERCLIP_PNPM_BIN}:${PATH}" \
  PAPERCLIP_MANAGED_ENTRYPOINT="${PAPERCLIP_MANAGED_ENTRYPOINT}" \
  PAPERCLIP_MIGRATION_MARKER="${PAPERCLIP_MIGRATION_MARKER}" \
  PAPERCLIP_SERVICE_HOME="${TMP}/paperclip-service" \
  PAPERCLIP_HOME="${TMP}/paperclip-home" \
  PAPERCLIP_PORT=18884 \
  bash "${NLTDEPLOY_ROOT}/libexec/nltdeploy/paperclip/setup.sh" update >/dev/null || exit 1
[[ "$(<"${PAPERCLIP_MIGRATION_MARKER}")" == true ]] || exit 1
[[ ! -e "${TMP}/paperclip-home/cli" ]] || exit 1
[[ ! -e "${HOME}/.local/bin/paperclipai" ]] || exit 1
[[ ! -e "${TMP}/paperclip-service/run/paperclip.pid" ]] || exit 1
unset -f node npm pnpm curl
bash -n "${NLTDEPLOY_ROOT}/libexec/nltdeploy/services/nlt-services.sh" || exit 1
bash -n "${NLTDEPLOY_ROOT}/libexec/nltdeploy/port-kill/setup.sh" || exit 1
bash -n "${NLTDEPLOY_ROOT}/libexec/nltdeploy/brew/setup.sh" || exit 1
bash "${NLTDEPLOY_ROOT}/libexec/nltdeploy/brew/setup.sh" --help >/dev/null || exit 1
brew() { [[ "${1:-}" == "--version" || "${1:-}" == "update" ]]; }
export -f brew
bash "${NLTDEPLOY_ROOT}/libexec/nltdeploy/brew/setup.sh" install >/dev/null || exit 1
bash "${NLTDEPLOY_ROOT}/libexec/nltdeploy/brew/setup.sh" update || exit 1
unset -f brew
bash -n "${NLTDEPLOY_ROOT}/libexec/nltdeploy/download/setup.sh" || exit 1
bash -n "${NLTDEPLOY_ROOT}/libexec/nltdeploy/dev/setup.sh" || exit 1
bash -n "${NLTDEPLOY_ROOT}/libexec/nltdeploy/ai-cli/setup.sh" || exit 1
for tool in claude codex cursor; do
  bash -n "${NLTDEPLOY_ROOT}/libexec/nltdeploy/ai-cli/${tool}/setup.sh" || exit 1
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
"${NLTDEPLOY_ROOT}/bin/nltdeploy" ai claude official install | grep -q 'ai-official:claude' || exit 1
"${NLTDEPLOY_ROOT}/bin/nltdeploy" ai claude pnpm install | grep -q 'pnpm:add -g @anthropic-ai/claude-code@latest' || exit 1
"${NLTDEPLOY_ROOT}/bin/nltdeploy" ai codex install | grep -q 'ai-official:codex' || exit 1
"${NLTDEPLOY_ROOT}/bin/nltdeploy" ai codex npm install | grep -q 'npm:install -g @openai/codex@latest' || exit 1
"${NLTDEPLOY_ROOT}/bin/nltdeploy" ai codex brew update | grep -q 'brew:upgrade --cask codex' || exit 1
"${NLTDEPLOY_ROOT}/bin/nltdeploy" ai cursor official install | grep -q 'ai-official:cursor' || exit 1
unset -f curl npm pnpm brew
mkdir -p "${TMP}/ai-home/.local/bin" "${TMP}/ai-home/.codex/packages/standalone"
touch "${TMP}/ai-home/.local/bin/codex"
HOME="${TMP}/ai-home" NLT_ASSUME_YES=1 "${NLTDEPLOY_ROOT}/bin/nltdeploy" ai codex official uninstall
[[ ! -e "${TMP}/ai-home/.local/bin/codex" && ! -e "${TMP}/ai-home/.codex/packages/standalone" ]] || exit 1
bash -n "${NLTDEPLOY_ROOT}/libexec/nltdeploy/dev/go/setup.sh" || exit 1
bash -n "${NLTDEPLOY_ROOT}/libexec/nltdeploy/dev/rust/setup.sh" || exit 1
bash -n "${NLTDEPLOY_ROOT}/libexec/nltdeploy/dev/nodejs/setup.sh" || exit 1
bash -n "${NLTDEPLOY_ROOT}/libexec/nltdeploy/dev/pnpm/setup.sh" || exit 1
bash -n "${NLTDEPLOY_ROOT}/libexec/nltdeploy/dev/uv/setup.sh" || exit 1
out="$(NONINTERACTIVE=1 "${NLTDEPLOY_ROOT}/bin/nltdeploy" list)"
grep -q "service" <<<"${out}" || exit 1
out="$(NONINTERACTIVE=1 "${NLTDEPLOY_ROOT}/bin/nltdeploy" service list)"
grep -q "sub2api" <<<"${out}" || exit 1
NONINTERACTIVE=1 "${NLTDEPLOY_ROOT}/bin/nltdeploy" dev --help >/dev/null || exit 1
out="$(NONINTERACTIVE=1 "${NLTDEPLOY_ROOT}/bin/nltdeploy" tool list)"
grep -q "brew" <<<"${out}" || exit 1
out="$(NONINTERACTIVE=1 "${NLTDEPLOY_ROOT}/bin/nltdeploy" ai list)"
grep -q "codex" <<<"${out}" || exit 1
out="$(NLTDEPLOY_PACKAGE_MANAGER=apt NONINTERACTIVE=1 "${NLTDEPLOY_ROOT}/bin/nltdeploy" upgrade --source github)"
grep -q "apt install --only-upgrade nltdeploy" <<<"${out}" || exit 1
out="$(NLTDEPLOY_PACKAGE_MANAGER=apt NONINTERACTIVE=1 "${NLTDEPLOY_ROOT}/bin/nltdeploy" uninstall)"
grep -q "apt remove nltdeploy" <<<"${out}" || exit 1
out="$(NLTDEPLOY_PACKAGE_MANAGER=brew NONINTERACTIVE=1 "${NLTDEPLOY_ROOT}/bin/nltdeploy" upgrade)"
grep -q "brew upgrade nltdeploy" <<<"${out}" || exit 1
out="$(NLTDEPLOY_PACKAGE_MANAGER=brew NONINTERACTIVE=1 "${NLTDEPLOY_ROOT}/bin/nltdeploy" uninstall)"
grep -q "brew uninstall nltdeploy" <<<"${out}" || exit 1
systemctl() {
  [[ "${1:-}" == "cat" ]] && return 0
  printf 'systemctl:%s\n' "$*"
}
curl() { return 1; }
sudo() { "$@"; }
export -f systemctl curl sudo
CODE_SERVER_OFFICIAL_USER=tester "${NLTDEPLOY_ROOT}/bin/nltdeploy" service code-server official start | grep -q "systemctl:enable --now code-server@tester" || exit 1
CODE_SERVER_OFFICIAL_USER=tester "${NLTDEPLOY_ROOT}/bin/nltdeploy" service code-server official restart | grep -q "systemctl:restart code-server@tester" || exit 1
CODE_SERVER_OFFICIAL_USER=tester "${NLTDEPLOY_ROOT}/bin/nltdeploy" service code-server status | grep -q "systemctl:status code-server@tester --no-pager" || exit 1
out="$(HOME="${TMP}/manual-home" PATH=/usr/bin:/bin CODE_SERVER_SERVICE_HOME="${TMP}/manual-code-server" CODE_SERVER_BIND=127.0.0.1:59997 "${NLTDEPLOY_ROOT}/bin/nltdeploy" service code-server manual status)"
grep -q "CODE_SERVER_SERVICE_HOME" <<<"${out}" || exit 1
! grep -q "systemctl:" <<<"${out}" || exit 1
"${NLTDEPLOY_ROOT}/bin/nltdeploy" service sub2api official restart | grep -q "systemctl:restart sub2api" || exit 1
"${NLTDEPLOY_ROOT}/bin/nltdeploy" service sub2api status | grep -q "systemctl:status sub2api --no-pager" || exit 1
out="$(SUB2API_SERVICE_HOME="${TMP}/manual-sub2api" SUB2API_PORT=59998 "${NLTDEPLOY_ROOT}/bin/nltdeploy" service sub2api manual status)"
grep -q "PID 文件" <<<"${out}" || exit 1
! grep -q "systemctl:" <<<"${out}" || exit 1
unset -f systemctl curl sudo
out="$(NONINTERACTIVE=1 "${NLTDEPLOY_ROOT}/bin/nltdeploy" tool list)"
grep -q "github-net" <<<"${out}" || exit 1
grep -q "brew" <<<"${out}" || exit 1
NONINTERACTIVE=1 "${NLTDEPLOY_ROOT}/bin/nltdeploy" dev --help >/dev/null || exit 1
out="$(HOME="${TMP}/status-home" PATH=/usr/bin:/bin NONINTERACTIVE=1 "${NLTDEPLOY_ROOT}/bin/nltdeploy" service status --no-http)"
grep -q '^服务,状态,PID,端口/访问,HTTP$' <<<"${out}" || exit 1
NONINTERACTIVE=1 "${NLTDEPLOY_ROOT}/bin/nltdeploy" dev --help >/dev/null || exit 1
NONINTERACTIVE=1 "${NLTDEPLOY_ROOT}/bin/nltdeploy" dev uv --help >/dev/null || exit 1
NONINTERACTIVE=1 "${NLTDEPLOY_ROOT}/bin/nltdeploy" ai --help >/dev/null || exit 1
NONINTERACTIVE=1 "${NLTDEPLOY_ROOT}/bin/nltdeploy" ai list | grep -q "claude" || exit 1
NONINTERACTIVE=1 "${NLTDEPLOY_ROOT}/bin/nltdeploy" tool download resolve-url "https://github.com/foo/bar" | grep -q "https://github.com/foo/bar" || exit 1
NONINTERACTIVE=1 "${NLTDEPLOY_ROOT}/bin/nltdeploy" tool port-kill list 59999 >/dev/null || exit 1
mkdir -p "${TMP}/legacy-bin"
touch "${TMP}/legacy-bin/nlt-port-kill"
NLT_BIN_DIR="${TMP}/legacy-bin" NONINTERACTIVE=1 "${NLTDEPLOY_ROOT}/bin/nltdeploy" tool port-kill install >/dev/null || exit 1
[[ ! -e "${TMP}/legacy-bin/nlt-port-kill" ]] || exit 1
"${NLTDEPLOY_ROOT}/bin/nltdeploy" service status --no-http >/dev/null || exit 1
echo "install_smoke OK"
