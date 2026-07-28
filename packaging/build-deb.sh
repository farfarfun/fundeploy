#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="${1:-${NLTDEPLOY_VERSION:-}}"
OUTPUT_DIR="${2:-${ROOT}/dist}"

die() { echo "错误: $*" >&2; exit 1; }

VERSION="${VERSION#v}"
[[ -n "$VERSION" && "$VERSION" =~ ^[0-9][0-9A-Za-z.+:~-]*$ ]] || die "版本号无效: ${VERSION:-<空>}"
command -v dpkg-deb >/dev/null 2>&1 || die "缺少 dpkg-deb"

mkdir -p "$OUTPUT_DIR"
OUTPUT_DIR="$(cd "$OUTPUT_DIR" && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf -- "$WORK"' EXIT
STAGE="${WORK}/root"

NLTDEPLOY_ROOT="${STAGE}/usr" \
NLTDEPLOY_WRAPPER_ROOT="/usr" \
NLTDEPLOY_PACKAGE_MANAGER="apt" \
NLTDEPLOY_SKIP_GIT_PULL=1 \
NLTDEPLOY_SKIP_PROFILE_HINT=1 \
  bash "${ROOT}/install.sh" install

rmdir "${STAGE}/usr/etc/nltdeploy" "${STAGE}/usr/etc" 2>/dev/null || true
mkdir -p "${STAGE}/usr/share/doc/nltdeploy" "${STAGE}/DEBIAN"
install -m 0644 "${ROOT}/LICENSE" "${STAGE}/usr/share/doc/nltdeploy/copyright"

INSTALLED_SIZE="$(du -sk "${STAGE}/usr" | awk '{print $1}')"
cat >"${STAGE}/DEBIAN/control" <<EOF
Package: nltdeploy
Version: ${VERSION}
Section: admin
Priority: optional
Architecture: all
Maintainer: farfarfun <farfarfun@qq.com>
Installed-Size: ${INSTALLED_SIZE}
Depends: bash (>= 3.2), ca-certificates, curl, git
Homepage: https://github.com/farfarfun/nltdeploy
Description: Bash tools for local development and service management
 nltdeploy installs and manages development runtimes, local services,
 AI command-line tools, and common workstation utilities.
EOF

PACKAGE="${OUTPUT_DIR}/nltdeploy_${VERSION}_all.deb"
dpkg-deb --root-owner-group --build "${STAGE}" "${PACKAGE}" >/dev/null
echo "$PACKAGE"
