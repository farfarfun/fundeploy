#!/usr/bin/env bash
# 开发工具统一入口：委派 pip / Python，并路由多语言子脚本
set -euo pipefail

_DEV_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
_NLT_LIB=""
for _c in "${_DEV_ROOT}/../lib" "${_DEV_ROOT}/../../lib"; do
  if [[ -f "${_c}/nlt-common.sh" ]]; then
    _NLT_LIB="$(cd "${_c}" && pwd)"
    break
  fi
done
if [[ -n "${_NLT_LIB}" ]]; then
  # shellcheck source=../lib/nlt-common.sh
  source "${_NLT_LIB}/nlt-common.sh"
fi

die() { echo "错误: $*" >&2; exit 1; }

usage() {
  cat <<'EOF'
用法: nlt-dev [子命令] [参数…]

  推荐主入口（替代在文档中单独强调 nlt-pip-sources / nlt-python-env）:
    pip | pip-sources     pip 镜像与源配置（委派到 pip-sources）
    uv                    uv 多方式安装 / 升级 / 卸载（默认包管理器，source 到 ~/opt）
    python | python-env   uv 与 Python 虚拟环境（委派到 python-env；会按需自动装 uv）
    go                    Go 多方式安装 / 升级 / 卸载（默认包管理器，source 到 ~/opt）
    rust                  Rust 多方式安装 / 升级 / 卸载（默认包管理器，source 到 ~/opt）
    nodejs                Node.js 多方式安装 / 升级 / 卸载（默认包管理器，source 到 ~/opt）
    pnpm                  pnpm 多方式安装 / 升级 / 卸载（默认包管理器，source 到 ~/opt）

无子命令时：若已安装 gum 则弹出选择菜单；否则打印本说明。

说明见: scripts/dev/README.md
EOF
}

# 已安装布局: libexec/nltdeploy/{dev,pip-sources,...}
# 仓库布局: scripts/dev 与 scripts/tools/{pip-sources,python-env}
_resolve_tool_setup() {
  local name="$1"
  if [[ -f "${_DEV_ROOT}/../${name}/setup.sh" ]]; then
    echo "${_DEV_ROOT}/../${name}/setup.sh"
    return 0
  fi
  if [[ -f "${_DEV_ROOT}/../tools/${name}/setup.sh" ]]; then
    echo "${_DEV_ROOT}/../tools/${name}/setup.sh"
    return 0
  fi
  return 1
}

_dispatch_child() {
  local rel="$1"
  shift
  exec bash "${_DEV_ROOT}/${rel}/setup.sh" "$@"
}

_pick_menu() {
  if command -v gum >/dev/null 2>&1; then
    :
  elif [[ -n "${_NLT_LIB:-}" ]] && declare -F _nlt_ensure_gum >/dev/null 2>&1; then
    _nlt_ensure_gum || return 1
  else
    return 1
  fi
  if declare -F nlt_ui_banner >/dev/null 2>&1; then
    nlt_ui_banner "nlt-dev" "本机开发环境工具（语言 / 包管理器）" "↑/↓ 选择 · Enter 确认 · Esc/q 退出" >&2
  fi
  if declare -F nlt_ui_choose >/dev/null 2>&1; then
    nlt_ui_choose "选择要配置的开发工具" \
      "pip（pip 源 / 镜像）" \
      "uv（Astral 安装器）" \
      "python（uv / 虚拟环境）" \
      "go" \
      "rust（rustup）" \
      "nodejs" \
      "pnpm" \
      "取消"
    return $?
  fi
  gum choose --header "nlt-dev — 选择工具" \
    "pip（pip 源 / 镜像）" \
    "uv（Astral 安装器）" \
    "python（uv / 虚拟环境）" \
    "go" \
    "rust（rustup）" \
    "nodejs" \
    "pnpm" \
    "取消"
}

main() {
  local cmd="${1:-}"
  if [[ -z "$cmd" ]]; then
    local pick
    if pick="$(_pick_menu)"; then
      case "$pick" in
        pip（*) cmd="pip" ;;
        uv（*) cmd="uv" ;;
        python（*) cmd="python" ;;
        go) cmd="go" ;;
        rust（*) cmd="rust" ;;
        nodejs) cmd="nodejs" ;;
        pnpm) cmd="pnpm" ;;
        取消 | "") exit 0 ;;
        *) die "未知菜单项: $pick" ;;
      esac
    else
      usage
      exit 0
    fi
  else
    shift
  fi

  case "$cmd" in
    pip | pip-sources)
      local target
      target="$(_resolve_tool_setup pip-sources)" || die "找不到 pip-sources/setup.sh"
      exec bash "${target}" "$@"
      ;;
    python | python-env)
      local t2
      t2="$(_resolve_tool_setup python-env)" || die "找不到 python-env/setup.sh"
      exec bash "${t2}" "$@"
      ;;
    uv) _dispatch_child uv "$@" ;;
    go) _dispatch_child go "$@" ;;
    rust) _dispatch_child rust "$@" ;;
    nodejs | node) _dispatch_child nodejs "$@" ;;
    pnpm) _dispatch_child pnpm "$@" ;;
    -h | --help | help)
      usage
      ;;
    *) die "未知子命令: ${cmd}（见 nlt-dev --help）" ;;
  esac
}

main "$@"
