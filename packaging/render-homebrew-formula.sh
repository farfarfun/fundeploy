#!/usr/bin/env bash
# 生成 Homebrew formula。
#
#   render-homebrew-formula.sh <版本号> <SHA256SUMS> [输出路径]
#
# 装的是发布好的二进制，不是从源码编。brew 那边从源码编要先装 Rust 工具链，
# 而 fundeploy 本身就是用来装工具链的，这个顺序反了。
set -euo pipefail

VERSION="${1:-}"
SUMS="${2:-}"
OUTPUT="${3:-fundeploy.rb}"
REPOSITORY="${FUNDEPLOY_GITHUB_REPOSITORY:-farfarfun/fundeploy}"

die() {
  echo "错误: $*" >&2
  exit 1
}

VERSION="${VERSION#v}"
[[ -n "$VERSION" && "$VERSION" =~ ^[0-9][0-9A-Za-z.+~-]*$ ]] || die "版本号无效: ${VERSION:-<空>}"
[[ -f "$SUMS" ]] || die "读不到校验和清单: ${SUMS:-<空>}"

sum_of() {
  local name="$1" v
  v="$(awk -v n="$name" '$2 == n || $2 == "*" n {print $1; exit}' "$SUMS")"
  [[ "${#v}" -eq 64 && "$v" != *[!0-9A-Fa-f]* ]] || die "${SUMS} 里没有 ${name} 的校验和"
  printf '%s' "$v"
}

BASE="https://github.com/${REPOSITORY}/releases/download/v${VERSION}"
MAC_ARM="fundeploy-aarch64-apple-darwin.tar.gz"
MAC_X86="fundeploy-x86_64-apple-darwin.tar.gz"
LNX_ARM="fundeploy-aarch64-unknown-linux-musl.tar.gz"
LNX_X86="fundeploy-x86_64-unknown-linux-musl.tar.gz"

mkdir -p "$(dirname "$OUTPUT")"
cat >"$OUTPUT" <<EOF
class Fundeploy < Formula
  desc "Development environment, tool and service manager"
  homepage "https://github.com/${REPOSITORY}"
  version "${VERSION}"
  license "MIT"

  on_macos do
    on_arm do
      url "${BASE}/${MAC_ARM}"
      sha256 "$(sum_of "$MAC_ARM")"
    end
    on_intel do
      url "${BASE}/${MAC_X86}"
      sha256 "$(sum_of "$MAC_X86")"
    end
  end

  on_linux do
    on_arm do
      url "${BASE}/${LNX_ARM}"
      sha256 "$(sum_of "$LNX_ARM")"
    end
    on_intel do
      url "${BASE}/${LNX_X86}"
      sha256 "$(sum_of "$LNX_X86")"
    end
  end

  def install
    bin.install "fundeploy"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/fundeploy --version")
    assert_match "service", shell_output("#{bin}/fundeploy list")
  end
end
EOF

echo "$OUTPUT"
