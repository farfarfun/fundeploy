#!/bin/sh
# fundeploy 装机脚本。
#
# fundeploy 是一个静态二进制，不依赖 Python、Node 或者任何运行时，所以装机要做
# 的只有一件事：把本机对应的那一个放到 PATH 上。
#
#   curl -fsSL https://raw.githubusercontent.com/farfarfun/fundeploy/HEAD/install.sh | sh
#
# 装完以后升级、回退都交给它自己：`fundeploy upgrade`。卸载就是删掉那个文件。
#
# 用 sh 而不是 bash：这是装机前的第一条命令，不能假设机器上有什么。
set -eu

REPO="${FUNDEPLOY_RELEASE_REPO:-farfarfun/fundeploy}"
BIN_DIR="${FUNDEPLOY_BIN_DIR:-${HOME}/.local/bin}"
VERSION="${FUNDEPLOY_VERSION:-latest}"
# 直连 GitHub 不通时拿它兜底：整条 URL 会原样拼在这个前缀后面（ghproxy 那一类）。
# 名字和 fundeploy 自己用的那个变量一致，这样装完之后不必再设一遍。
PROXY="${FUNDEPLOY_GITHUB_HUB_PROXY_PREFIX:-}"

die() {
  printf '错误: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<'EOF'
用法: install.sh [--version 版本] [--bin-dir 目录]

环境变量:
  FUNDEPLOY_VERSION                   装哪一版，默认 latest
  FUNDEPLOY_BIN_DIR                   装到哪儿，默认 ~/.local/bin
  FUNDEPLOY_RELEASE_REPO              从哪个仓库的 release 取，默认 farfarfun/fundeploy
  FUNDEPLOY_GITHUB_HUB_PROXY_PREFIX   GitHub 直连不通时的加速前缀
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --version)
      [ $# -ge 2 ] || die "--version 后面要给版本号"
      VERSION="$2"
      shift 2
      ;;
    --version=*)
      VERSION="${1#*=}"
      shift
      ;;
    --bin-dir)
      [ $# -ge 2 ] || die "--bin-dir 后面要给目录"
      BIN_DIR="$2"
      shift 2
      ;;
    --bin-dir=*)
      BIN_DIR="${1#*=}"
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *) die "不认识的参数: $1（用 --help 看用法）" ;;
  esac
done

# 本机对应的 target 三元组。写全而不是拼出来，是为了让这份名单一眼能和
# release 里的附件对上。
case "$(uname -s)/$(uname -m)" in
  Linux/x86_64 | Linux/amd64) TARGET=x86_64-unknown-linux-musl ;;
  Linux/aarch64 | Linux/arm64) TARGET=aarch64-unknown-linux-musl ;;
  Darwin/x86_64) TARGET=x86_64-apple-darwin ;;
  Darwin/arm64 | Darwin/aarch64) TARGET=aarch64-apple-darwin ;;
  *) die "没有给 $(uname -s) $(uname -m) 发布过二进制" ;;
esac
ASSET="fundeploy-${TARGET}.tar.gz"

if [ "${VERSION}" = latest ]; then
  # latest/download 这条路由由 GitHub 自己重定向到最新 release，不用先查 API——
  # 匿名调 API 每小时只有 60 次，装机脚本不值得占一次。
  BASE="https://github.com/${REPO}/releases/latest/download"
else
  BASE="https://github.com/${REPO}/releases/download/v${VERSION#v}"
fi

# 下载。先整个下到临时文件再用，不边下边解：中途断线留下的半个包，下一步看着
# 和完整的包没区别。
fetch() {
  _url="$1"
  _dest="$2"
  if [ -n "${PROXY}" ]; then
    _url="${PROXY}${_url}"
  fi
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --retry 3 --connect-timeout 15 -o "${_dest}" "${_url}"
  elif command -v wget >/dev/null 2>&1; then
    wget -q --tries=3 --timeout=60 -O "${_dest}" "${_url}"
  else
    die "既没有 curl 也没有 wget，下不了东西。"
  fi
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

TMP="$(mktemp -d)"
STAGED="${BIN_DIR}/.fundeploy.new.$$"
trap 'rm -rf -- "${TMP}" "${STAGED}"' EXIT INT TERM

printf '==> 下载 %s\n' "${ASSET}"
fetch "${BASE}/${ASSET}" "${TMP}/${ASSET}" ||
  die "下不到 ${BASE}/${ASSET}（版本写错了，或者网络不通）"

# 校验。取不到清单或者本机没有 sha256 工具时只提醒不拦——那会让完全正常的安装
# 失败；对得上却不匹配则必须停下。
if fetch "${BASE}/SHA256SUMS" "${TMP}/SHA256SUMS" 2>/dev/null; then
  WANT="$(awk -v n="${ASSET}" '$2 == n || $2 == "*" n {print $1; exit}' "${TMP}/SHA256SUMS")"
  GOT="$(sha256_of "${TMP}/${ASSET}")"
  if [ -z "${WANT}" ]; then
    printf '警告: SHA256SUMS 里没有 %s，跳过校验。\n' "${ASSET}" >&2
  elif [ -z "${GOT}" ]; then
    printf '警告: 本机没有 sha256sum/shasum，跳过校验。\n' >&2
  elif [ "${WANT}" != "${GOT}" ]; then
    die "${ASSET} 校验失败
  期望 ${WANT}
  实际 ${GOT}"
  fi
else
  printf '警告: 取不到 SHA256SUMS，跳过校验。\n' >&2
fi

tar -xzf "${TMP}/${ASSET}" -C "${TMP}" || die "${ASSET} 解不开"
[ -f "${TMP}/fundeploy" ] || die "${ASSET} 里没有 fundeploy"

mkdir -p "${BIN_DIR}"
# 中转一次落在 BIN_DIR 里，最后那一步是同目录 rename。直接覆盖的话，正在跑的
# fundeploy 会在写到一半时被读到。
cp "${TMP}/fundeploy" "${STAGED}"
chmod 0755 "${STAGED}"
"${STAGED}" --version >/dev/null 2>&1 || die "下下来的二进制跑不起来，没有装上去。"
mv -f "${STAGED}" "${BIN_DIR}/fundeploy"

printf '✓ 装好了: %s\n' "${BIN_DIR}/fundeploy"
"${BIN_DIR}/fundeploy" --version

# PATH 不动别人的 shell 配置，只说该加哪一行。装机脚本偷偷往 ~/.bashrc 里写东西
# 是个常见做法，但那意味着卸载时还得把它找回来删掉，而卸载这件事这里做不到。
case ":${PATH}:" in
  *":${BIN_DIR}:"*) ;;
  *)
    printf '\n%s 不在 PATH 上。把这行加进 shell 配置（~/.bashrc 或 ~/.zshrc）:\n' "${BIN_DIR}"
    printf '  export PATH="%s:$PATH"\n' "${BIN_DIR}"
    ;;
esac
