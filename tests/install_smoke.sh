#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "${TMP}"' EXIT
export NLTDEPLOY_ROOT="${TMP}/nd"
export NLTDEPLOY_SKIP_PROFILE_HINT=1
export NLTDEPLOY_SKIP_GIT_PULL=1
bash "${ROOT}/install.sh" install
bash "${ROOT}/install.sh" update
for f in \
  nlt-dev nlt-pip-sources nlt-python-env nlt-utils nlt-github-net nlt-port-kill nlt-download nlt-services \
  nlt-airflow nlt-celery nlt-paperclip nlt-code-server nlt-new-api nlt-open-pencil
do
  [[ -x "${NLTDEPLOY_ROOT}/bin/${f}" ]] || { echo "missing: bin/${f}" >&2; exit 1; }
  bash -n "${NLTDEPLOY_ROOT}/bin/${f}" || exit 1
done
for bad in \
  nlt-airflow-install \
  nlt-celery-install nlt-celery-update \
  nlt-paperclip-install nlt-code-server-install nlt-new-api-install
do
  [[ -e "${NLTDEPLOY_ROOT}/bin/${bad}" ]] && { echo "unexpected deprecated bin/${bad}" >&2; exit 1; }
done
bash -n "${NLTDEPLOY_ROOT}/libexec/nltdeploy/airflow/setup.sh" || exit 1
bash -n "${NLTDEPLOY_ROOT}/libexec/nltdeploy/code-server/setup.sh" || exit 1
bash -n "${NLTDEPLOY_ROOT}/libexec/nltdeploy/new-api/setup.sh" || exit 1
bash -n "${NLTDEPLOY_ROOT}/libexec/nltdeploy/open-pencil/setup.sh" || exit 1
bash -n "${NLTDEPLOY_ROOT}/libexec/nltdeploy/services/nlt-services.sh" || exit 1
bash -n "${NLTDEPLOY_ROOT}/libexec/nltdeploy/port-kill/setup.sh" || exit 1
bash -n "${NLTDEPLOY_ROOT}/libexec/nltdeploy/download/setup.sh" || exit 1
bash -n "${NLTDEPLOY_ROOT}/libexec/nltdeploy/dev/setup.sh" || exit 1
bash -n "${NLTDEPLOY_ROOT}/libexec/nltdeploy/dev/go/setup.sh" || exit 1
bash -n "${NLTDEPLOY_ROOT}/libexec/nltdeploy/dev/rust/setup.sh" || exit 1
bash -n "${NLTDEPLOY_ROOT}/libexec/nltdeploy/dev/nodejs/setup.sh" || exit 1
bash -n "${NLTDEPLOY_ROOT}/libexec/nltdeploy/dev/pnpm/setup.sh" || exit 1
bash -n "${NLTDEPLOY_ROOT}/libexec/nltdeploy/dev/uv/setup.sh" || exit 1
NONINTERACTIVE=1 "${NLTDEPLOY_ROOT}/bin/nlt-dev" --help >/dev/null || exit 1
NONINTERACTIVE=1 "${NLTDEPLOY_ROOT}/bin/nlt-dev" uv --help >/dev/null || exit 1
NONINTERACTIVE=1 "${NLTDEPLOY_ROOT}/bin/nlt-download" resolve-url "https://github.com/foo/bar" | grep -q "https://github.com/foo/bar" || exit 1
NONINTERACTIVE=1 "${NLTDEPLOY_ROOT}/bin/nlt-port-kill" list 59999 >/dev/null || exit 1
"${NLTDEPLOY_ROOT}/bin/nlt-services" status --no-http >/dev/null || exit 1
echo "install_smoke OK"
