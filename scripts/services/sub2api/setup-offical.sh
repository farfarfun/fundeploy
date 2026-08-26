#!/usr/bin/env bash
# Sub2API 官方模式：调用上游安装脚本，并通过 systemd 管理服务。

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ -f "${SCRIPT_DIR}/../lib/nlt-common.sh" ]]; then
  # shellcheck source=../lib/nlt-common.sh
  source "${SCRIPT_DIR}/../lib/nlt-common.sh"
elif [[ -f "${SCRIPT_DIR}/../../lib/nlt-common.sh" ]]; then
  # shellcheck source=../../lib/nlt-common.sh
  source "${SCRIPT_DIR}/../../lib/nlt-common.sh"
else
  echo "错误: 找不到 lib/nlt-common.sh" >&2
  exit 1
fi

# 上游安装脚本以 root 执行，因此必须锁定到不可变的 commit SHA，而不是可变的
# main 分支。raw.githubusercontent.com/<repo>/<sha>/<path> 是内容寻址的：给定
# SHA 就唯一确定内容。用 main 意味着上游仓库（或其账号）一旦被攻破，所有执行过
# `fundeploy service sub2api official install` 的机器都会被拿到 root。
#
# 升级步骤（人工复核后再改）：
#   1. 到 https://github.com/Wei-Shaw/sub2api/commits/main/deploy/install.sh 选定新提交
#   2. 阅读该提交的 diff
#   3. 同时更新下面的 REF 与 SHA256（sha256sum 新脚本可得）
SUB2API_INSTALLER_REF="${SUB2API_INSTALLER_REF:-510ee451bd9e42682838972f64bd1faf027bd244}"
SUB2API_INSTALLER_SHA256="${SUB2API_INSTALLER_SHA256-d6ca89c38111041e392463bbe1637453cd1576cb99dee0005b441ff5a20f8477}"
OFFICIAL_INSTALLER_URL="${SUB2API_INSTALLER_URL:-https://raw.githubusercontent.com/Wei-Shaw/sub2api/${SUB2API_INSTALLER_REF}/deploy/install.sh}"
SERVICE_NAME="sub2api"
SUB2API_OFFICIAL_PORT="${SUB2API_OFFICIAL_PORT:-8802}"

die() { echo "错误: $*" >&2; exit 1; }

usage() {
  cat <<'EOF'
用法: setup-offical.sh [command] [选项]

命令:
  install [上游选项]      使用官方脚本安装（/opt/sub2api + systemd，端口默认 8802）
  update [上游选项]       使用官方脚本升级
  start / stop / restart  通过 systemd 管理服务
  status                  查看 systemd 服务状态
  logs [journalctl 选项]  查看服务日志（默认持续跟踪）
  uninstall [上游选项]    使用官方脚本卸载；支持 -y、--purge

示例:
  fundeploy service sub2api official install
  fundeploy service sub2api official restart
  fundeploy service sub2api official logs -n 100
  fundeploy service sub2api official uninstall -y
EOF
}

as_root() {
  if [[ "$(id -u)" -eq 0 ]]; then
    "$@"
  else
    command -v sudo >/dev/null 2>&1 || die "需要 root 权限，但未找到 sudo"
    sudo "$@"
  fi
}

_sha256_of() {
  local f="$1"
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$f" | awk '{print $1}'; return 0; fi
  if command -v shasum >/dev/null 2>&1; then shasum -a 256 "$f" | awk '{print $1}'; return 0; fi
  if command -v openssl >/dev/null 2>&1; then openssl dgst -sha256 "$f" | awk '{print $NF}'; return 0; fi
  printf ''
}

run_official() {
  command -v curl >/dev/null 2>&1 || die "需要 curl"
  local command="$1"
  shift

  local tmpdir script
  tmpdir="$(mktemp -d)"
  # shellcheck disable=SC2064
  trap "rm -rf '${tmpdir}'" RETURN EXIT
  script="${tmpdir}/install.sh"

  echo "==> 下载 Sub2API 官方脚本" >&2
  echo "    ${OFFICIAL_INSTALLER_URL}" >&2
  # 先落盘再执行，而不是 `curl … | sudo bash`：这样才能在赋予 root 之前校验内容。
  _nlt_github_download_curl -fsSL "${OFFICIAL_INSTALLER_URL}" -o "${script}" \
    || die "下载官方脚本失败: ${OFFICIAL_INSTALLER_URL}"
  [[ -s "${script}" ]] || die "官方脚本为空: ${OFFICIAL_INSTALLER_URL}"

  local actual
  actual="$(_sha256_of "${script}")"
  if [[ -n "${SUB2API_INSTALLER_SHA256}" ]]; then
    if [[ -z "${actual}" ]]; then
      die "无法计算 sha256（缺少 sha256sum/shasum/openssl），拒绝以 root 执行未校验的脚本。设置 SUB2API_INSTALLER_SHA256= 可跳过校验（不推荐）。"
    fi
    if [[ "${actual}" != "${SUB2API_INSTALLER_SHA256}" ]]; then
      echo "错误: 官方脚本校验和不匹配 —— 拒绝以 root 执行。" >&2
      echo "  期望: ${SUB2API_INSTALLER_SHA256}" >&2
      echo "  实际: ${actual}" >&2
      echo "  若上游确已更新，请复核 diff 后同步更新本脚本中的 SUB2API_INSTALLER_REF 与 SUB2API_INSTALLER_SHA256。" >&2
      exit 1
    fi
    echo "    sha256 校验通过: ${actual}" >&2
  else
    echo "    警告: 未启用 sha256 校验（SUB2API_INSTALLER_SHA256 为空）。实际值: ${actual:-<无法计算>}" >&2
  fi

  # 端口改写必须验证是否真的生效。原实现直接 sed 后管道执行，一旦上游改了
  # 引号/缩进/默认值，sed 静默无操作 —— sub2api 装在 8080，而
  # `fundeploy service status` 一直探测 8802，永远显示「未运行」。
  if grep -qE '^SERVER_PORT="8080"$' "${script}"; then
    sed -i.bak "s/^SERVER_PORT=\"8080\"$/SERVER_PORT=\"${SUB2API_OFFICIAL_PORT}\"/" "${script}" \
      && rm -f "${script}.bak"
    grep -qE "^SERVER_PORT=\"${SUB2API_OFFICIAL_PORT}\"$" "${script}" \
      || die "端口改写失败（预期 SERVER_PORT=\"${SUB2API_OFFICIAL_PORT}\"）"
    echo "    已将 SERVER_PORT 改写为 ${SUB2API_OFFICIAL_PORT}" >&2
  else
    die "未在官方脚本中找到 'SERVER_PORT=\"8080\"'，无法确保端口为 ${SUB2API_OFFICIAL_PORT}。上游格式可能已变更，请复核后更新本脚本。"
  fi

  echo "==> 以 root 执行官方脚本: ${command}（端口 ${SUB2API_OFFICIAL_PORT}）" >&2
  as_root bash "${script}" "${command}" "$@"
}

cmd_uninstall() {
  local arg confirmed=0
  for arg in "$@"; do
    [[ "$arg" == "-y" || "$arg" == "--yes" ]] && confirmed=1
  done
  if [[ "$confirmed" == "0" ]]; then
    # 原判断是 `[[ "$confirmed" == "0" && -t 0 ]]`：非 TTY 且未加 -y 时整个
    # 分支被跳过，于是在毫无确认、也不带 -y 的情况下直接跑上游卸载器。
    # code-server 的同类路径在这种情形下是 die 的，这里对齐。
    nlt_confirm_destructive "确认使用官方脚本卸载 Sub2API？" SUB2API_UNINSTALL_YES || return 1
    set -- "$@" -y
  fi
  run_official uninstall "$@"
}

interactive_main() {
  _nlt_ensure_gum || exit 1
  nlt_ui_banner "fundeploy / service / sub2api / official" "官方脚本 · /opt/sub2api · systemd · 端口 8802" >&2
  set +e
  while true; do
    local pick
    pick="$(nlt_ui_choose "fundeploy / service / sub2api / official / 选择动作" \
      "install           安装" \
      "update            更新" \
      "start             启动" \
      "stop              停止" \
      "restart           重启" \
      "status            状态" \
      "logs              日志" \
      "uninstall         卸载" \
      "help              命令帮助" \
      "quit              返回")" || break
    pick="${pick%% *}"
    case "$pick" in
      install) run_official install ;;
      update) run_official upgrade ;;
      start|stop|restart) as_root systemctl "$pick" "${SERVICE_NAME}" ;;
      status) as_root systemctl status "${SERVICE_NAME}" --no-pager ;;
      logs) as_root journalctl -u "${SERVICE_NAME}" -f ;;
      uninstall) cmd_uninstall ;;
      help) usage ;;
      *) break ;;
    esac
    echo ""
  done
  set -e
}

main() {
  local cmd="${1:-}"
  [[ $# -eq 0 ]] || shift
  case "$cmd" in
    "")
      [[ "${NONINTERACTIVE:-}" != "1" ]] || { usage >&2; exit 1; }
      interactive_main
      ;;
    install) run_official install "$@" ;;
    update|upgrade) run_official upgrade "$@" ;;
    start|stop|restart) as_root systemctl "$cmd" "${SERVICE_NAME}" ;;
    status) as_root systemctl status "${SERVICE_NAME}" --no-pager ;;
    logs|log)
      if [[ $# -eq 0 ]]; then
        as_root journalctl -u "${SERVICE_NAME}" -f
      else
        as_root journalctl -u "${SERVICE_NAME}" "$@"
      fi
      ;;
    uninstall|remove) cmd_uninstall "$@" ;;
    help|-h|--help) usage ;;
    *) die "未知命令: ${cmd}（使用 --help 查看）" ;;
  esac
}

main "$@"
