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
curl() { printf '%s\n' 'SERVER_PORT="8080"' 'echo "official-sub2api-installer-ran:${1}:${SERVER_PORT}"'; }
sudo() { "$@"; }
export -f curl sudo
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
npm() { [[ "$*" == "install -g --registry https://registry.npmjs.org paperclipai@latest" ]]; }
paperclipai() { [[ "${1:-}" == "--version" ]]; }
export -f node npm paperclipai
bash "${NLTDEPLOY_ROOT}/libexec/nltdeploy/paperclip/setup.sh" install >/dev/null || exit 1
unset -f node npm paperclipai
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
