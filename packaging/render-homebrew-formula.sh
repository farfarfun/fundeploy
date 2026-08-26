#!/usr/bin/env bash
set -euo pipefail

VERSION="${1:-}"
SHA256="${2:-}"
OUTPUT="${3:-fundeploy.rb}"
REPOSITORY="${FUNDEPLOY_GITHUB_REPOSITORY:-farfarfun/fundeploy}"

die() { echo "错误: $*" >&2; exit 1; }

VERSION="${VERSION#v}"
[[ -n "$VERSION" && "$VERSION" =~ ^[0-9][0-9A-Za-z.+~-]*$ ]] || die "版本号无效: ${VERSION:-<空>}"
[[ "${#SHA256}" -eq 64 && "$SHA256" != *[!0-9A-Fa-f]* ]] || die "SHA256 必须是 64 位十六进制字符串"
mkdir -p "$(dirname "$OUTPUT")"

cat >"$OUTPUT" <<EOF
class Fundeploy < Formula
  desc "Bash tools for local development and service management"
  homepage "https://github.com/${REPOSITORY}"
  url "https://github.com/${REPOSITORY}/archive/refs/tags/v${VERSION}.tar.gz"
  sha256 "${SHA256}"
  license "MIT"

  def install
    system "env",
           "FUNDEPLOY_ROOT=#{prefix}",
           "FUNDEPLOY_WRAPPER_ROOT=#{opt_prefix}",
           "FUNDEPLOY_PACKAGE_MANAGER=brew",
           "FUNDEPLOY_SKIP_GIT_PULL=1",
           "FUNDEPLOY_SKIP_PROFILE_HINT=1",
           "/bin/bash", "install.sh", "install"
  end

  test do
    assert_match "service", shell_output("#{bin}/fundeploy list")
    assert_match "brew upgrade fundeploy", shell_output("#{bin}/fundeploy upgrade")
  end
end
EOF

echo "$OUTPUT"
