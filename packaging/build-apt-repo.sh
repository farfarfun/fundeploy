#!/usr/bin/env bash
# 把一批 .deb 组装成一个签名好的 APT 仓库（给 GitHub Pages 托管用）。
#
#   build-apt-repo.sh <deb目录> <输出目录> <GPG密钥ID>
set -euo pipefail

DEB_DIR="${1:-}"
OUTPUT_DIR="${2:-}"
GPG_KEY_ID="${3:-}"

die() {
  echo "错误: $*" >&2
  exit 1
}

[[ -d "$DEB_DIR" ]] || die "找不到 deb 目录: ${DEB_DIR:-<空>}"
[[ -n "$OUTPUT_DIR" ]] || die "请指定 APT 仓库输出目录"
[[ -n "$GPG_KEY_ID" ]] || die "请指定 GPG 签名密钥 ID"
for cmd in apt-ftparchive dpkg-scanpackages gpg gzip; do
  command -v "$cmd" >/dev/null 2>&1 || die "缺少命令: $cmd"
done

shopt -s nullglob
DEBS=("$DEB_DIR"/*.deb)
shopt -u nullglob
[[ "${#DEBS[@]}" -gt 0 ]] || die "${DEB_DIR} 里一个 .deb 都没有"

if [[ -d "$OUTPUT_DIR" ]] && find "$OUTPUT_DIR" -mindepth 1 -print -quit | grep -q .; then
  die "输出目录必须为空: $OUTPUT_DIR"
fi
mkdir -p "$OUTPUT_DIR"
OUTPUT_DIR="$(cd "$OUTPUT_DIR" && pwd)"
POOL="${OUTPUT_DIR}/pool/main/n/fundeploy"
DIST="${OUTPUT_DIR}/dists/stable"
ARCHES=(amd64 arm64)

mkdir -p "$POOL"
cp -f "${DEBS[@]}" "$POOL/"

# 每个架构扫出自己的 Packages。以前的包是 Architecture: all，两个架构共用一份
# 索引没问题；现在二进制是静态链接的、每个架构一个包，索引再共用的话 arm64 的
# 机器会在列表里看到 amd64 的包，apt 直接判定「没有可安装的候选」。
for arch in "${ARCHES[@]}"; do
  mkdir -p "${DIST}/main/binary-${arch}"
  (
    cd "$OUTPUT_DIR"
    dpkg-scanpackages --arch "$arch" pool
  ) >"${DIST}/main/binary-${arch}/Packages"
  [[ -s "${DIST}/main/binary-${arch}/Packages" ]] || die "${arch} 没有对应的包"
  gzip -9 -c "${DIST}/main/binary-${arch}/Packages" >"${DIST}/main/binary-${arch}/Packages.gz"
done

apt-ftparchive \
  -o APT::FTPArchive::Release::Origin=fundeploy \
  -o APT::FTPArchive::Release::Label=fundeploy \
  -o APT::FTPArchive::Release::Suite=stable \
  -o APT::FTPArchive::Release::Codename=stable \
  -o "APT::FTPArchive::Release::Architectures=${ARCHES[*]}" \
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
