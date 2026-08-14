#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf -- "${TMP}"' EXIT

SOURCE="${TMP}/source"
BARE="${TMP}/gitee.git"
mkdir -p "${SOURCE}" "${TMP}/home"
cp -R "${ROOT}/." "${SOURCE}/"
rm -rf -- "${SOURCE}/.git" "${SOURCE}/dist"
git -C "${SOURCE}" init -q
git -C "${SOURCE}" checkout -qb master
git -C "${SOURCE}" -c user.name=test -c user.email=test@example.com add .
git -C "${SOURCE}" -c user.name=test -c user.email=test@example.com commit -qm initial
git clone -q --bare "${SOURCE}" "${BARE}"
git -C "${SOURCE}" remote add mirror "${BARE}"

export HOME="${TMP}/home"
export NLTDEPLOY_ROOT="${TMP}/install"
export NLTDEPLOY_SKIP_PROFILE_HINT=1
export NLTDEPLOY_AUTO_EXEC_ZSH_AFTER_INSTALL=0
git config --global url."file://${BARE}".insteadOf "https://gitee.com/farfarfun/nltdeploy.git"
git config --global url."file://${TMP}/github-must-not-be-used".insteadOf "https://github.com/farfarfun/nltdeploy.git"

TRACE="${TMP}/git.trace"
GIT_TRACE="${TRACE}" bash -s -- install --source gitee <"${SOURCE}/install.sh" >/dev/null
[[ "$(<"${NLTDEPLOY_ROOT}/etc/nltdeploy/source")" == "gitee" ]]
[[ "$(git -C "${NLTDEPLOY_ROOT}/src/nltdeploy" config --get remote.origin.url)" == *gitee.com* ]]
! grep -qi 'github.com' "${TRACE}"

printf '%s\n' 'source-stickiness-update' >"${SOURCE}/VERSION"
git -C "${SOURCE}" add VERSION
git -C "${SOURCE}" -c user.name=test -c user.email=test@example.com commit -qm update
git -C "${SOURCE}" push -q mirror master
: >"${TRACE}"
GIT_TRACE="${TRACE}" "${NLTDEPLOY_ROOT}/bin/nltdeploy" upgrade >/dev/null
[[ "$(<"${NLTDEPLOY_ROOT}/src/nltdeploy/VERSION")" == "source-stickiness-update" ]]
[[ "$(<"${NLTDEPLOY_ROOT}/etc/nltdeploy/source")" == "gitee" ]]
! grep -qi 'github.com' "${TRACE}"

echo "source_stickiness_smoke OK"
