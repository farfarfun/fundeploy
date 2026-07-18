#!/usr/bin/env bash
# Homebrew：通过官方安装器安装 / 升级 / 卸载。
set -euo pipefail

BREW_INSTALL_URL="${BREW_INSTALL_URL:-https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh}"
BREW_UNINSTALL_URL="${BREW_UNINSTALL_URL:-https://raw.githubusercontent.com/Homebrew/install/HEAD/uninstall.sh}"

die() { echo "错误: $*" >&2; exit 1; }

find_brew() {
  local brew
  if command -v brew >/dev/null 2>&1; then
    command -v brew
    return 0
  fi
  for brew in /opt/homebrew/bin/brew /usr/local/bin/brew /home/linuxbrew/.linuxbrew/bin/brew; do
    [[ -x "${brew}" ]] && { printf '%s\n' "${brew}"; return 0; }
  done
  return 1
}

run_official_script() {
  local url="$1" script
  shift
  command -v curl >/dev/null 2>&1 || die "需要 curl"
  script="$(curl -fsSL "${url}")" || die "下载安装脚本失败: ${url}"
  /bin/bash -c "${script}" nltdeploy "$@"
}

do_install() {
  local brew os
  if brew="$(find_brew)"; then
    echo "Homebrew 已安装: ${brew}"
    "${brew}" --version
    return 0
  fi
  os="$(uname -s)"
  [[ "${os}" == "Darwin" || "${os}" == "Linux" ]] || die "Homebrew 仅支持 macOS 和 Linux"
  run_official_script "${BREW_INSTALL_URL}"
  brew="$(find_brew)" || die "安装完成但未找到 brew，请按官方安装器输出配置 PATH"
  echo "Homebrew 安装完成: ${brew}"
  echo "请按需加入 shell 配置: eval \"\$(${brew} shellenv)\""
}

do_update() {
  local brew
  brew="$(find_brew)" || { do_install; return; }
  "${brew}" update
}

do_uninstall() {
  find_brew >/dev/null 2>&1 || { echo "Homebrew 未安装，跳过。"; return 0; }
  if [[ "${BREW_UNINSTALL_YES:-0}" != "1" ]]; then
    [[ -t 0 ]] || die "非交互卸载请设置 BREW_UNINSTALL_YES=1"
    read -r -p "将卸载 Homebrew 及其已安装软件，确认？[y/N] " answer
    [[ "${answer}" == "y" || "${answer}" == "Y" ]] || { echo "已取消。"; return 0; }
  fi
  NONINTERACTIVE=1 run_official_script "${BREW_UNINSTALL_URL}" --force
}

usage() {
  cat <<'EOF'
用法: brew/setup.sh [install|update|version|uninstall]

  install            使用 Homebrew 官方安装器安装；已安装则跳过
  update / upgrade   执行 brew update；未安装则先安装
  version            显示 brew 版本
  uninstall          使用官方卸载器删除 Homebrew 及其已安装软件

环境变量:
  NONINTERACTIVE=1       让官方安装器使用非交互模式
  BREW_UNINSTALL_YES=1   允许非交互卸载
  BREW_INSTALL_URL       覆盖官方安装脚本 URL
  BREW_UNINSTALL_URL     覆盖官方卸载脚本 URL
EOF
}

case "${1:-install}" in
  install) do_install ;;
  update | upgrade) do_update ;;
  version) brew="$(find_brew)" || die "Homebrew 未安装"; "${brew}" --version ;;
  uninstall | remove) do_uninstall ;;
  help | -h | --help) usage ;;
  *) die "未知子命令: $1（见 brew/setup.sh --help）" ;;
esac
