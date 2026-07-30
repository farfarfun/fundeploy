#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf -- "$TMP"' EXIT

# 打包工具缺失时默认跳过（方便 macOS / 精简环境下跑其余测试）。
# 但 CI 必须设置 NLTDEPLOY_REQUIRE_PACKAGE_TESTS=1 —— 否则本测试会静默 exit 0
# 绿灯通过，而实际发布用的 .deb 构建路径从未被验证过。
for cmd in apt-ftparchive dpkg-deb dpkg-scanpackages gpg gpgv; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    if [[ "${NLTDEPLOY_REQUIRE_PACKAGE_TESTS:-}" == "1" ]]; then
      echo "package_smoke FAIL: 缺少 $cmd（已设置 NLTDEPLOY_REQUIRE_PACKAGE_TESTS=1，不允许跳过）" >&2
      echo "  Debian/Ubuntu 安装: sudo apt-get install -y apt-utils dpkg-dev gnupg" >&2
      exit 1
    fi
    echo "package_smoke skip: missing $cmd"
    exit 0
  fi
done

VERSION="$(tr -d '[:space:]' < "${ROOT}/VERSION")"
DEB="$(bash "${ROOT}/packaging/build-deb.sh" "" "${TMP}/dist")"
[[ "$(dpkg-deb --field "$DEB" Package)" == "nltdeploy" ]]
[[ "$(dpkg-deb --field "$DEB" Version)" == "$VERSION" ]]
dpkg-deb --extract "$DEB" "${TMP}/extract"
[[ -x "${TMP}/extract/usr/bin/nltdeploy" ]]
for entry in "${TMP}/extract/usr/bin/"*; do
  [[ "$(basename "$entry")" == "nltdeploy" ]] || { echo "unexpected package command: $(basename "$entry")" >&2; exit 1; }
done
! grep -Fq "$TMP" "${TMP}/extract/usr/bin/nltdeploy"

OUT="$(NLTDEPLOY_ROOT="${TMP}/extract/usr" "${TMP}/extract/usr/bin/nltdeploy" list)"
grep -q "service" <<<"$OUT"
OUT="$(NLTDEPLOY_ROOT="${TMP}/extract/usr" "${TMP}/extract/usr/bin/nltdeploy" upgrade)"
grep -q "apt install --only-upgrade nltdeploy" <<<"$OUT"

GNUPGHOME="${TMP}/gnupg"
export GNUPGHOME
mkdir -m 0700 "$GNUPGHOME"
gpg --batch --passphrase '' --quick-generate-key 'nltdeploy package test' ed25519 sign 0 >/dev/null 2>&1
KEY_ID="$(gpg --batch --with-colons --list-secret-keys 2>/dev/null | awk -F: '$1 == "fpr" { print $10; exit }')"
bash "${ROOT}/packaging/build-apt-repo.sh" "$DEB" "${TMP}/apt" "$KEY_ID" >/dev/null
gpgv --keyring "${TMP}/apt/nltdeploy.gpg" "${TMP}/apt/dists/stable/InRelease" >/dev/null 2>&1
grep -q '^Package: nltdeploy$' "${TMP}/apt/dists/stable/main/binary-amd64/Packages"
if command -v apt-get >/dev/null 2>&1 && command -v apt-cache >/dev/null 2>&1; then
  mkdir -p "${TMP}/apt-client/lists/partial" "${TMP}/apt-client/cache/archives/partial"
  printf 'deb [signed-by=%s] file:%s stable main\n' \
    "${TMP}/apt/nltdeploy.gpg" "${TMP}/apt" >"${TMP}/apt-client/sources.list"
  APT_OPTIONS=(
    -o "Dir::Etc::sourcelist=${TMP}/apt-client/sources.list"
    -o "Dir::Etc::sourceparts=-"
    -o "Dir::State::lists=${TMP}/apt-client/lists"
    -o "Dir::Cache=${TMP}/apt-client/cache"
    -o "APT::Sandbox::User=${USER:-root}"
  )
  apt-get "${APT_OPTIONS[@]}" update >/dev/null
  apt-cache "${APT_OPTIONS[@]}" show nltdeploy | grep -q "^Version: ${VERSION}$"
fi

bash "${ROOT}/packaging/render-homebrew-formula.sh" "$VERSION" "$(printf '0%.0s' {1..64})" "${TMP}/nltdeploy.rb" >/dev/null
grep -q 'NLTDEPLOY_PACKAGE_MANAGER=brew' "${TMP}/nltdeploy.rb"
if command -v ruby >/dev/null 2>&1; then
  ruby -c "${TMP}/nltdeploy.rb" >/dev/null
fi

echo "package_smoke OK"
