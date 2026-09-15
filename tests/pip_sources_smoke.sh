#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "${TMP}"' EXIT
mkdir -p "${TMP}/bin"
export PIP_LOG="${TMP}/pip.log"

cat >"${TMP}/bin/python3" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"${PIP_LOG}"
case "$*" in
  "-m pip --version") printf '%s\n' "pip 1.0" ;;
  *"config --user get"*) exit 1 ;;
  *"config --user list"*) printf '%s\n' "global.index-url='https://example.invalid/simple/'" ;;
esac
EOF

cat >"${TMP}/bin/curl" <<'EOF'
#!/usr/bin/env bash
printf '%s' '200|0.123'
EOF
chmod 0755 "${TMP}/bin/python3" "${TMP}/bin/curl"

PATH="${TMP}/bin:/usr/bin:/bin" NONINTERACTIVE=1 PIP_SOURCES_PARALLEL_JOBS=4 \
  bash "${ROOT}/scripts/tools/pip-sources/setup.sh" install >/dev/null

grep -q -- '-m pip config --user set global.index-url https://' "${PIP_LOG}"
grep -q -- '-m pip config --user set global.extra-index-url ' "${PIP_LOG}"
! grep -q -- 'config --user set global.trusted-host' "${PIP_LOG}"
! grep -q -- 'http://pypi.hustunique.com' "${PIP_LOG}"

: >"${PIP_LOG}"
PATH="${TMP}/bin:/usr/bin:/bin" NONINTERACTIVE=1 PIP_SOURCES_ALLOW_INSECURE=1 \
  bash "${ROOT}/scripts/tools/pip-sources/setup.sh" install >/dev/null
grep -q -- 'config --user set global.trusted-host .*pypi.hustunique.com' "${PIP_LOG}"
grep -q -- 'http://pypi.hustunique.com' "${PIP_LOG}"

PATH="${TMP}/bin:/usr/bin:/bin" bash "${ROOT}/scripts/tools/pip-sources/setup.sh" status \
  | grep -q 'global.index-url'
PATH="${TMP}/bin:/usr/bin:/bin" NONINTERACTIVE=1 \
  bash "${ROOT}/scripts/tools/pip-sources/setup.sh" uninstall >/dev/null
grep -q -- 'config --user unset global.index-url' "${PIP_LOG}"

echo "pip_sources_smoke OK"
