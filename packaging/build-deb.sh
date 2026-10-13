#!/usr/bin/env bash
# 把一个已经构建好的静态二进制打成 .deb。
#
#   build-deb.sh <二进制> <版本号> <amd64|arm64> [输出目录]
set -euo pipefail

BIN="${1:-}"
VERSION="${2:-}"
ARCH="${3:-}"
OUTPUT_DIR="${4:-dist}"

die() {
  echo "错误: $*" >&2
  exit 1
}

[[ -f "$BIN" ]] || die "给的二进制不存在: ${BIN:-<空>}"
VERSION="${VERSION#v}"
[[ -n "$VERSION" && "$VERSION" =~ ^[0-9][0-9A-Za-z.+:~-]*$ ]] || die "版本号无效: ${VERSION:-<空>}"
case "$ARCH" in
  amd64 | arm64) ;;
  *) die "架构只认 amd64 / arm64，给的是: ${ARCH:-<空>}" ;;
esac
command -v dpkg-deb >/dev/null 2>&1 || die "缺少 dpkg-deb"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mkdir -p "$OUTPUT_DIR"
OUTPUT_DIR="$(cd "$OUTPUT_DIR" && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf -- "$WORK"' EXIT
STAGE="${WORK}/root"

install -D -m 0755 "$BIN" "${STAGE}/usr/bin/fundeploy"
install -D -m 0644 "${ROOT}/LICENSE" "${STAGE}/usr/share/doc/fundeploy/copyright"

INSTALLED_SIZE="$(du -sk "${STAGE}/usr" | awk '{print $1}')"
mkdir -p "${STAGE}/DEBIAN"
# 没有 Depends：二进制是静态链接的，装上就能跑。git / curl / bash 只有个别
# 子命令要用（skills 要 git，AI 官方安装器是 bash 脚本），那是 Recommends 的
# 语义——写成 Depends 会让一个不碰这些功能的人也被拖进一串依赖。
cat >"${STAGE}/DEBIAN/control" <<EOF
Package: fundeploy
Version: ${VERSION}
Section: admin
Priority: optional
Architecture: ${ARCH}
Maintainer: farfarfun <farfarfun@qq.com>
Installed-Size: ${INSTALLED_SIZE}
Recommends: ca-certificates, git, curl
Homepage: https://github.com/farfarfun/fundeploy
Description: Development environment, tool and service manager
 fundeploy installs and manages development toolchains, local services,
 AI command-line tools, package mirrors and common workstation utilities.
 Statically linked; no runtime dependencies.
EOF

PACKAGE="${OUTPUT_DIR}/fundeploy_${VERSION}_${ARCH}.deb"
dpkg-deb --root-owner-group --build "${STAGE}" "${PACKAGE}" >/dev/null
echo "$PACKAGE"
