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
export FUNDEPLOY_ROOT="${TMP}/install"
export FUNDEPLOY_SKIP_PROFILE_HINT=1
export FUNDEPLOY_AUTO_EXEC_ZSH_AFTER_INSTALL=0
git config --global url."file://${BARE}".insteadOf "https://gitee.com/farfarfun/fundeploy.git"
git config --global url."file://${TMP}/github-must-not-be-used".insteadOf "https://github.com/farfarfun/fundeploy.git"

TRACE="${TMP}/git.trace"
GIT_TRACE="${TRACE}" bash -s -- install --source gitee <"${SOURCE}/install.sh" >/dev/null
[[ "$(<"${FUNDEPLOY_ROOT}/etc/fundeploy/source")" == "gitee" ]]
[[ "$(git -C "${FUNDEPLOY_ROOT}/src/fundeploy" config --get remote.origin.url)" == *gitee.com* ]]
! grep -qi 'github.com' "${TRACE}"

printf '%s\n' 'source-stickiness-update' >"${SOURCE}/VERSION"
git -C "${SOURCE}" add VERSION
git -C "${SOURCE}" -c user.name=test -c user.email=test@example.com commit -qm update
git -C "${SOURCE}" push -q mirror master
: >"${TRACE}"
GIT_TRACE="${TRACE}" "${FUNDEPLOY_ROOT}/bin/fundeploy" upgrade >/dev/null
[[ "$(<"${FUNDEPLOY_ROOT}/src/fundeploy/VERSION")" == "source-stickiness-update" ]]
[[ "$(<"${FUNDEPLOY_ROOT}/etc/fundeploy/source")" == "gitee" ]]
! grep -qi 'github.com' "${TRACE}"

echo "source_stickiness_smoke OK"
