#!/usr/bin/env bash
set -euo pipefail

DEB="${1:-}"
OUTPUT_DIR="${2:-}"
GPG_KEY_ID="${3:-}"

die() { echo "错误: $*" >&2; exit 1; }

[[ -f "$DEB" ]] || die "找不到 deb 包: ${DEB:-<空>}"
[[ -n "$OUTPUT_DIR" ]] || die "请指定 APT 仓库输出目录"
[[ -n "$GPG_KEY_ID" ]] || die "请指定 GPG 签名密钥 ID"
for cmd in apt-ftparchive dpkg-scanpackages gpg gzip; do
  command -v "$cmd" >/dev/null 2>&1 || die "缺少命令: $cmd"
done

if [[ -d "$OUTPUT_DIR" ]] && find "$OUTPUT_DIR" -mindepth 1 -print -quit | grep -q .; then
  die "输出目录必须为空: $OUTPUT_DIR"
fi
mkdir -p "$OUTPUT_DIR"
OUTPUT_DIR="$(cd "$OUTPUT_DIR" && pwd)"
POOL="${OUTPUT_DIR}/pool/main/n/fundeploy"
DIST="${OUTPUT_DIR}/dists/stable"
mkdir -p "$POOL" "${DIST}/main/binary-amd64" "${DIST}/main/binary-arm64"
cp -f "$DEB" "$POOL/"

(
  cd "$OUTPUT_DIR"
  dpkg-scanpackages --arch all pool >"${DIST}/main/binary-amd64/Packages"
)
cp -f "${DIST}/main/binary-amd64/Packages" "${DIST}/main/binary-arm64/Packages"
gzip -9 -c "${DIST}/main/binary-amd64/Packages" >"${DIST}/main/binary-amd64/Packages.gz"
gzip -9 -c "${DIST}/main/binary-arm64/Packages" >"${DIST}/main/binary-arm64/Packages.gz"

apt-ftparchive \
  -o APT::FTPArchive::Release::Origin=fundeploy \
  -o APT::FTPArchive::Release::Label=fundeploy \
  -o APT::FTPArchive::Release::Suite=stable \
  -o APT::FTPArchive::Release::Codename=stable \
  -o 'APT::FTPArchive::Release::Architectures=amd64 arm64' \
  -o APT::FTPArchive::Release::Components=main \
  -o 'APT::FTPArchive::Release::Description=fundeploy packages' \
  release "$DIST" >"${DIST}/Release"

GPG_ARGS=(--batch --yes --local-user "$GPG_KEY_ID")
if [[ -n "${APT_GPG_PASSPHRASE:-}" ]]; then
  GPG_ARGS+=(--pinentry-mode loopback --passphrase "$APT_GPG_PASSPHRASE")
fi
gpg "${GPG_ARGS[@]}" --armor --detach-sign --output "${DIST}/Release.gpg" "${DIST}/Release"
gpg "${GPG_ARGS[@]}" --clearsign --output "${DIST}/InRelease" "${DIST}/Release"
gpg --batch --export "$GPG_KEY_ID" >"${OUTPUT_DIR}/fundeploy.gpg"
gpg --batch --armor --export "$GPG_KEY_ID" >"${OUTPUT_DIR}/fundeploy.asc"

echo "$OUTPUT_DIR"
