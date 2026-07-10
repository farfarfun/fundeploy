#!/usr/bin/env bash
# 修复「GitHub 网页能访问，但 git clone 失败」的常见网络问题。
#
# 用法：
#   ./setup.sh                  # 无参：gum 交互诊断 + 选择修复
#   ./setup.sh install          # 自动诊断并应用推荐修复（同 fix_auto）
#   ./setup.sh update           # 仅诊断
#   ./setup.sh reinstall        # 再次自动修复（交互确认）
#   ./setup.sh uninstall        # 提示如何撤销本脚本写入的 SSH/Git 片段
#   NONINTERACTIVE=1             # 跳过 gum 确认（install/reinstall 直接执行）
#
# 自动处理流程（已固化到脚本）：
#   1) 诊断三条通道：HTTPS(443) / SSH(22) / SSH(443)
#   2) 判定优先级：SSH(22) > SSH(443) > HTTPS
#   3) 自动应用对应修复（修改 ~/.gitconfig / ~/.ssh/config）
#   4) 用 git ls-remote 做最终连通性验证并给出结果

set -euo pipefail

_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ -f "${_SCRIPT_DIR}/../lib/nlt-common.sh" ]]; then
  # shellcheck source=../lib/nlt-common.sh
  source "${_SCRIPT_DIR}/../lib/nlt-common.sh"
elif [[ -f "${_SCRIPT_DIR}/../../lib/nlt-common.sh" ]]; then
  # shellcheck source=../../lib/nlt-common.sh
  source "${_SCRIPT_DIR}/../../lib/nlt-common.sh"
else
  echo "错误: 找不到 lib/nlt-common.sh（已检查 ${_SCRIPT_DIR}/../lib 与 ${_SCRIPT_DIR}/../../lib）" >&2
  exit 1
fi

SCRIPT_NAME="$(basename "${BASH_SOURCE[0]}")"
SSH_CONFIG_PATH="${HOME}/.ssh/config"
SSH_CONFIG_BACKUP="${HOME}/.ssh/config.bak.nltdeploy.$(date +%Y%m%d%H%M%S)"
KNOWN_HOSTS_PATH="${HOME}/.ssh/known_hosts"

say() { printf '%s\n' "$*"; }
warn() { printf '警告: %s\n' "$*" >&2; }
err() { printf '错误: %s\n' "$*" >&2; }
say_info() { gum style --foreground 212 "$*"; }
say_warn() { gum style --foreground 214 "$*" >&2; }

has_cmd() {
  command -v "$1" >/dev/null 2>&1
}

print_header() {
  gum style --bold --foreground 212 "$1"
}

verify_clone_path() {
  # 用公开仓库做低成本验证，避免影响用户业务仓库。
  local probe_repo="github/gitignore.git"
  local target="$1"
  case "$target" in
    ssh22)
      git ls-remote --exit-code "git@github.com:${probe_repo}" HEAD >/dev/null 2>&1
      ;;
    ssh443)
      GIT_SSH_COMMAND="ssh -p 443" git ls-remote --exit-code "git@github.com:${probe_repo}" HEAD >/dev/null 2>&1
      ;;
    https)
      git ls-remote --exit-code "https://github.com/${probe_repo}" HEAD >/dev/null 2>&1
      ;;
    *)
      return 2
      ;;
  esac
}

test_https() {
  if ! has_cmd git; then
    err "未找到 git，请先安装。"
    return 2
  fi
  if git ls-remote --exit-code https://github.com/github/gitignore.git HEAD >/dev/null 2>&1; then
    say "[OK] HTTPS 克隆测试通过"
    return 0
  fi
  warn "HTTPS 克隆测试失败（可能是代理/证书/网络策略问题）"
  return 1
}

test_ssh_22() {
  if ! has_cmd ssh; then
    err "未找到 ssh。"
    return 2
  fi

  # GitHub 在认证成功时也可能返回非 0，这里通过输出内容判断。
  local out
  set +e
  out="$(ssh -o BatchMode=yes -o ConnectTimeout=8 -T git@github.com 2>&1)"
  set -e
  if [[ "$out" == *"successfully authenticated"* ]]; then
    say "[OK] SSH(22) 连通且认证成功"
    return 0
  fi
  if [[ "$out" == *"Connection refused"* ]] || [[ "$out" == *"Operation timed out"* ]] || [[ "$out" == *"No route to host"* ]]; then
    warn "SSH(22) 网络不可达（常见于公司网络屏蔽 22 端口）"
    return 1
  fi
  if [[ "$out" == *"Permission denied (publickey)"* ]]; then
    warn "SSH(22) 可达，但 SSH key 未配置或未加入 GitHub"
    return 1
  fi
  warn "SSH(22) 测试未通过：${out}"
  return 1
}

test_ssh_443() {
  if ! has_cmd ssh; then
    err "未找到 ssh。"
    return 2
  fi

  local out
  set +e
  out="$(ssh -o BatchMode=yes -o ConnectTimeout=8 -T -p 443 git@ssh.github.com 2>&1)"
  set -e
  if [[ "$out" == *"successfully authenticated"* ]]; then
    say "[OK] SSH(443) 连通且认证成功"
    return 0
  fi
  if [[ "$out" == *"Permission denied (publickey)"* ]]; then
    warn "SSH(443) 网络可达，但 SSH key 未配置或未加入 GitHub"
    return 1
  fi
  warn "SSH(443) 测试未通过：${out}"
  return 1
}

diagnose() {
  print_header "GitHub clone 诊断"
  local ok_https=1 ok_ssh22=1 ok_ssh443=1
  test_https && ok_https=0 || true
  test_ssh_22 && ok_ssh22=0 || true
  test_ssh_443 && ok_ssh443=0 || true
  say
  say_info "建议路径："
  if [[ $ok_ssh22 -eq 0 ]]; then
    say "  推荐：优先使用 SSH(22)（当前机器可用）"
  elif [[ $ok_ssh443 -eq 0 ]]; then
    say "  推荐：使用 SSH over 443"
  elif [[ $ok_https -eq 0 ]]; then
    say "  推荐：使用 HTTPS"
  else
    say_warn "  当前三种方式都失败，请检查本机网络代理/防火墙策略。"
  fi
}

fix_https() {
  print_header "应用修复：强制 GitHub 使用 HTTPS"
  has_cmd git || { err "未找到 git，请先安装。"; exit 1; }

  git config --global url."https://github.com/".insteadOf "git@github.com:"
  git config --global url."https://github.com/".insteadOf "ssh://git@github.com/"

  say_info "[OK] 已写入全局 Git 配置：GitHub SSH 地址自动改写为 HTTPS"
  say "你现在可以直接执行：git clone git@github.com:<owner>/<repo>.git"
  say "Git 会自动改写为 HTTPS。"
}

fix_ssh22() {
  print_header "应用修复：强制 GitHub 使用 SSH(22)"
  has_cmd git || { err "未找到 git，请先安装。"; exit 1; }

  # 清理把 SSH 改写到 HTTPS 的规则，避免走到不可达的 443。
  git config --global --unset-all url."https://github.com/".insteadOf >/dev/null 2>&1 || true

  # 将常见 GitHub HTTPS URL 自动改写为 SSH，适合「网页可访问但 clone HTTPS 不通」场景。
  git config --global url."git@github.com:".insteadOf "https://github.com/"
  git config --global url."git@github.com:".insteadOf "ssh://git@github.com/"

  say_info "[OK] 已配置 GitHub URL 优先走 SSH(22)"
  say "现在复制网页上的 HTTPS 地址也能被 Git 自动改写为 SSH。"
}

fix_ssh443() {
  print_header "应用修复：SSH 走 443 端口"
  has_cmd ssh || { err "未找到 ssh。"; exit 1; }

  mkdir -p "${HOME}/.ssh"
  chmod 700 "${HOME}/.ssh"

  if [[ -f "${SSH_CONFIG_PATH}" ]]; then
    cp "${SSH_CONFIG_PATH}" "${SSH_CONFIG_BACKUP}"
    say "已备份: ${SSH_CONFIG_BACKUP}"
  fi

  local marker_begin="# >>> nltdeploy github ssh443 >>>"
  local marker_end="# <<< nltdeploy github ssh443 <<<"

  # 移除旧块，避免重复写入。
  if [[ -f "${SSH_CONFIG_PATH}" ]] && has_cmd awk; then
    awk -v b="$marker_begin" -v e="$marker_end" '
      $0==b {skip=1; next}
      $0==e {skip=0; next}
      !skip {print}
    ' "${SSH_CONFIG_PATH}" > "${SSH_CONFIG_PATH}.tmp"
    mv "${SSH_CONFIG_PATH}.tmp" "${SSH_CONFIG_PATH}"
  fi

  cat >> "${SSH_CONFIG_PATH}" <<EOF

${marker_begin}
Host github.com
  HostName ssh.github.com
  Port 443
  User git
  IdentitiesOnly yes
${marker_end}
EOF

  chmod 600 "${SSH_CONFIG_PATH}"
  # 预置 443 主机指纹，避免首次连接阻塞或 host key 校验失败。
  if has_cmd ssh-keyscan && has_cmd ssh-keygen; then
    touch "${KNOWN_HOSTS_PATH}"
    chmod 600 "${KNOWN_HOSTS_PATH}"
    if ! ssh-keygen -F "[ssh.github.com]:443" -f "${KNOWN_HOSTS_PATH}" >/dev/null 2>&1; then
      ssh-keyscan -p 443 ssh.github.com >> "${KNOWN_HOSTS_PATH}" 2>/dev/null || true
    fi
  fi
  say_info "[OK] 已写入 ${SSH_CONFIG_PATH}"
  say "你现在可以继续使用 SSH 地址：git@github.com:<owner>/<repo>.git"
  say "如果首次连接，按提示确认主机指纹即可。"
}

fix_auto() {
  print_header "自动修复（诊断 -> 判定 -> 修复 -> 验证）"
  say "步骤 1/4: 诊断 HTTPS/SSH22/SSH443"
  if test_ssh_22 >/dev/null 2>&1; then
    say "步骤 2/4: 判定优先通道为 SSH(22)"
    say "步骤 3/4: 应用 SSH(22) 修复"
    fix_ssh22
    say "步骤 4/4: 执行连通性验证"
    if verify_clone_path ssh22; then
      say_info "[OK] 自动修复完成：当前可用通道 SSH(22)"
    else
      say_warn "已应用 SSH(22) 修复，但验证失败，请检查本机网络策略。"
    fi
    return
  fi
  if test_ssh_443 >/dev/null 2>&1; then
    say "步骤 2/4: 判定优先通道为 SSH(443)"
    say "步骤 3/4: 应用 SSH(443) 修复"
    fix_ssh443
    say "步骤 4/4: 执行连通性验证"
    if verify_clone_path ssh443; then
      say_info "[OK] 自动修复完成：当前可用通道 SSH(443)"
    else
      say_warn "已应用 SSH(443) 修复，但验证失败，请检查 known_hosts 或网络策略。"
    fi
    return
  fi
  if test_https >/dev/null 2>&1; then
    say "步骤 2/4: 判定优先通道为 HTTPS"
    say "步骤 3/4: 应用 HTTPS 修复"
    fix_https
    say "步骤 4/4: 执行连通性验证"
    if verify_clone_path https; then
      say_info "[OK] 自动修复完成：当前可用通道 HTTPS"
    else
      say_warn "已应用 HTTPS 修复，但验证失败，请检查代理/证书设置。"
    fi
    return
  fi
  say_warn "三种通道都不可用，未自动修改配置。"
}

interactive_main() {
  diagnose
  say
  while true; do
    local pick
    pick="$(gum choose --header "选择操作" \
      "应用修复：自动选择（推荐）" \
      "应用修复：强制 SSH(22)" \
      "应用修复：SSH over 443（继续用 SSH）" \
      "应用修复：HTTPS" \
      "仅重新诊断" \
      "退出")" || {
      say_warn "已取消。"
      return 0
    }
    case "$pick" in
      "应用修复：自动选择（推荐）")
        if gum confirm "确认按诊断结果自动修复？"; then
          fix_auto
        else
          say_warn "已取消。"
        fi
        ;;
      "应用修复：强制 SSH(22)")
        if gum confirm "确认应用 SSH(22) 修复？"; then
          fix_ssh22
        else
          say_warn "已取消。"
        fi
        ;;
      "应用修复：SSH over 443（继续用 SSH）")
        if gum confirm "确认应用 SSH over 443 修复？"; then
          fix_ssh443
        else
          say_warn "已取消。"
        fi
        ;;
      "应用修复：HTTPS")
        if gum confirm "确认应用 HTTPS 修复？"; then
          fix_https
        else
          say_warn "已取消。"
        fi
        ;;
      "仅重新诊断")
        diagnose
        ;;
      "退出")
        say "已退出。"
        return 0
        ;;
      *)
        warn "无效选项，已退出。"
        return 0
        ;;
    esac
    say
  done
}

usage_github() {
  cat <<EOF
用法: ${SCRIPT_NAME} [install|update|reinstall|uninstall|help]

  install    自动诊断并应用推荐修复（默认与非交互管线）
  update     仅运行诊断并给出建议
  reinstall  再次执行自动修复（TTY 下 gum 确认）
  uninstall  打印如何手动撤销 SSH/Git 配置中的 nltdeploy 片段
  help       本说明

环境: NONINTERACTIVE=1 时 install/reinstall 不弹出确认。
EOF
}

dispatch_github() {
  local c="${1:-install}"
  shift || true
  case "$c" in
    install)
      fix_auto
      ;;
    update|upgrade)
      diagnose
      ;;
    reinstall)
      if [[ "${NONINTERACTIVE:-}" == "1" ]] || ! [[ -t 0 ]]; then
        fix_auto
      else
        if gum confirm "将重新执行自动修复（可能改写 git/ssh 配置），继续？"; then
          fix_auto
        else
          say_warn "已取消。"
        fi
      fi
      ;;
    uninstall)
      say_warn "卸载请手动："
      say "  1) 编辑 ~/.ssh/config，删除标记为 # >>> nltdeploy github ssh443 >>> … # <<< nltdeploy github ssh443 <<< 的区块"
      say "  2) 运行 git config --global --list 检查 url.*.insteadOf，按需 git config --global --unset-all …"
      say "  3) SSH 配置备份可能在 ~/.ssh/config.bak.nltdeploy.*"
      ;;
    help | -h | --help)
      usage_github
      ;;
    *)
      err "未知命令: $c"
      usage_github >&2
      exit 2
      ;;
  esac
}

main() {
  _nlt_ensure_gum || exit 1
  if [[ $# -eq 0 ]]; then
    interactive_main
    return 0
  fi
  case "$1" in
    help | -h | --help)
      usage_github
      ;;
    *)
      dispatch_github "$@"
      ;;
  esac
}

main "$@"
