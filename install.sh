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
# 从哪个站点取 release。Gitee 上那份是 CI 从 GitHub 整仓镜像过去的，release
# 附件一起搬。留空＝先试 GitHub，连不上再换 Gitee。
#
# 默认先 GitHub 而不是先 Gitee：GitHub 是权威源，万一哪天镜像那条流水线断了，
# 默认不会把人静悄悄钉在旧版本上。国内嫌那次探测慢就点名 gitee。
SOURCE="${FUNDEPLOY_RELEASE_SOURCE:-}"
# 直连 GitHub 不通时拿它兜底：整条 URL 会原样拼在这个前缀后面（ghproxy 那一类）。
# 名字和 fundeploy 自己用的那个变量一致，这样装完之后不必再设一遍。
PROXY="${FUNDEPLOY_GITHUB_HUB_PROXY_PREFIX:-}"
# 建连超时和重试次数。探测 GitHub 那一下两个都会临时调小：连不上就换 Gitee，
# 没必要干等。重试也得一起关——不然 8 秒的超时重试三轮还是二十多秒。
CONNECT_TIMEOUT=15
RETRY=3

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
  FUNDEPLOY_RELEASE_SOURCE            github 或 gitee；默认先 GitHub，不通再换 Gitee
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

case "${SOURCE}" in
  '' | github | gitee) ;;
  *) die "FUNDEPLOY_RELEASE_SOURCE 只认 github 或 gitee（给的是 ${SOURCE}）" ;;
esac

# 下载。先整个下到临时文件再用，不边下边解：中途断线留下的半个包，下一步看着
# 和完整的包没区别。
fetch() {
  _url="$1"
  _dest="$2"
  # 加速前缀只往 GitHub 的地址上套。它是为 GitHub 配的，拿去套 Gitee 只会把一个
  # 本来通的地址绕坏——而走到 Gitee 的人，多半正是设了这个前缀的那批。
  if [ -n "${PROXY}" ]; then
    case "${_url}" in
      https://github.com/* | https://raw.githubusercontent.com/*)
        _url="${PROXY}${_url}"
        ;;
    esac
  fi
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --retry "${RETRY}" --connect-timeout "${CONNECT_TIMEOUT}" -o "${_dest}" "${_url}"
  elif command -v wget >/dev/null 2>&1; then
    # curl 的 --retry 是「首次之外再试几次」，wget 的 --tries 是总次数。
    wget -q --tries="$((RETRY + 1))" --connect-timeout="${CONNECT_TIMEOUT}" \
      --timeout=60 -O "${_dest}" "${_url}"
  else
    die "既没有 curl 也没有 wget，下不了东西。"
  fi
}

# Gitee 上最新那一版的 tag。
#
# 没有 jq 可用，但要取的就是一个顶层字符串字段：拆成一行一个再挑，比在 sh 里
# 写 JSON 解析器实在得多。
gitee_latest_tag() {
  fetch "https://gitee.com/api/v5/repos/${REPO}/releases/latest" "${TMP}/latest.json" || return 1
  tr ',' '\n' <"${TMP}/latest.json" |
    sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' |
    head -1
}

# 某个源上的附件目录地址。
base_of() {
  case "$1" in
    github)
      if [ "${VERSION}" = latest ]; then
        # latest/download 这条路由由 GitHub 自己重定向到最新 release，不用先查
        # API——匿名调 API 每小时只有 60 次，装机脚本不值得占一次。
        printf 'https://github.com/%s/releases/latest/download' "${REPO}"
      else
        printf 'https://github.com/%s/releases/download/v%s' "${REPO}" "${VERSION#v}"
      fi
      ;;
    gitee)
      _tag="${VERSION}"
      if [ "${_tag}" = latest ]; then
        # Gitee 没有 latest/download 这条路由：它会把 "latest" 当成分支名，
        # 重定向去取源码归档。所以这边只能先问一次 API 把版本号换出来。
        _tag="$(gitee_latest_tag)" || return 1
        [ -n "${_tag}" ] || return 1
      fi
      printf 'https://gitee.com/%s/releases/download/v%s' "${REPO}" "${_tag#v}"
      ;;
  esac
}

# 从某个源把附件下下来。成功则把 BASE 定在这个源上，后面取 SHA256SUMS 也走它，
# 保证校验和与包出自同一处。
grab() {
  _base="$(base_of "$1")" || return 1
  [ -n "${_base}" ] || return 1
  fetch "${_base}/${ASSET}" "${TMP}/${ASSET}" || return 1
  BASE="${_base}"
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
if [ -n "${SOURCE}" ]; then
  grab "${SOURCE}" || die "从 ${SOURCE} 下不到 ${ASSET}（版本写错了，或者网络不通）"
else
  # 先探 GitHub。探测而已，连不上就换下一个，没必要让人干等。
  CONNECT_TIMEOUT=8
  RETRY=0
  # 探测失败是预期内的一条岔路，不是故障，所以 curl/wget 那行原始报错咽掉，只留
  # 下面那句中文。真要是两边都不通，Gitee 那次的报错不咽，照样看得见。
  if grab github 2>/dev/null; then
    :
  else
    CONNECT_TIMEOUT=15
    RETRY=3
    printf '提示: GitHub 取不到，改用 Gitee 镜像。\n' >&2
    grab gitee || die "GitHub 和 Gitee 镜像都下不到 ${ASSET}（版本写错了，或者网络不通）"
  fi
fi
# 探测用的短超时到此为止，后面取 SHA256SUMS 按正常来。
#
# 代价说在明处：auto 模式下 GitHub 那一次是不重试的，所以链路时好时坏的人会被
# 换到 Gitee，而不是在 GitHub 上多试两轮。两边的包字节一致、校验和同一份，换过去
# 没有损失，而重试三轮却要让连不上的人多等二十多秒。
CONNECT_TIMEOUT=15
RETRY=3

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
