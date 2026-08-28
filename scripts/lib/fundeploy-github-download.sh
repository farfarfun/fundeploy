#!/usr/bin/env bash
# GitHub 族下载 URL 改写（供其它脚本 source）。默认不改写；由环境变量显式启用。
# 参见 scripts/tools/download/README.md

[[ -n "${_FUNDEPLOY_GITHUB_DOWNLOAD_LIB_LOADED:-}" ]] && return 0
_FUNDEPLOY_GITHUB_DOWNLOAD_LIB_LOADED=1

_fundeploy_gh_dl_lc() {
  printf '%s' "$1" | tr '[:upper:]' '[:lower:]'
}

# 参数：主机名（小写）
_fundeploy_is_github_download_host() {
  case "$1" in
    github.com | www.github.com) return 0 ;;
    raw.githubusercontent.com) return 0 ;;
    api.github.com) return 0 ;;
    *) return 1 ;;
  esac
}

# 将 https URL 解析为 host（小写）与路径部分（以 / 开头，无路径则为 /）
_fundeploy_gh_dl_parse_https() {
  local url="$1" rest host_raw host path
  if [[ "$url" != https://* ]]; then
    return 1
  fi
  rest="${url#https://}"
  host_raw="${rest%%/*}"
  host="$(_fundeploy_gh_dl_lc "$host_raw")"
  path="${rest#"${host_raw}"}"
  [[ -z "$path" ]] && path="/"
  [[ "$path" != /* ]] && path="/${path}"
  printf '%s\t%s\n' "$host" "$path"
}

# 输出一行：改写后的 URL（stdout）。发生改写时 stderr 打一行诊断。
_fundeploy_github_download_resolve_url() {
  local url="$1"
  local mode hub_pre raw_base
  mode="${FUNDEPLOY_GITHUB_DOWNLOAD_MODE:-off}"
  hub_pre="${FUNDEPLOY_GITHUB_HUB_PROXY_PREFIX:-}"
  raw_base="${FUNDEPLOY_GITHUB_RAW_MIRROR_BASE:-}"

  if [[ -z "$url" ]]; then
    printf '%s\n' "$url"
    return 0
  fi

  if [[ "$url" != https://* ]]; then
    printf '%s\n' "$url"
    return 0
  fi

  local host path parsed
  if ! parsed="$(_fundeploy_gh_dl_parse_https "$url")"; then
    printf '%s\n' "$url"
    return 0
  fi
  host="${parsed%%$'\t'*}"
  path="${parsed#*$'\t'}"

  if ! _fundeploy_is_github_download_host "$host"; then
    printf '%s\n' "$url"
    return 0
  fi

  # 改写前缀/基址必须是 https://。这两个变量能把本仓库所有 GitHub 族下载
  # （gum、code-server、sub2api、new-api、cockpit-tools…）重定向到任意主机，
  # 而 README 又主动建议用户设置它们 —— "粘贴这行 ghproxy 就能提速" 正是最容易
  # 被从随手搜到的博客里抄走的配置。至少不能允许降级到明文 http://。
  if [[ -n "$hub_pre" && "$hub_pre" != https://* ]]; then
    printf '%s\n' "[fundeploy download] 拒绝非 https:// 的 FUNDEPLOY_GITHUB_HUB_PROXY_PREFIX（${hub_pre}），本次不改写。" >&2
    hub_pre=""
  fi
  if [[ -n "$raw_base" && "$raw_base" != https://* ]]; then
    printf '%s\n' "[fundeploy download] 拒绝非 https:// 的 FUNDEPLOY_GITHUB_RAW_MIRROR_BASE（${raw_base}），本次不改写。" >&2
    raw_base=""
  fi

  # 优先级：hub 前缀 > mirror_raw > off（与设计文档一致）
  if [[ -n "$hub_pre" ]]; then
    if [[ "$url" == "${hub_pre}"* ]]; then
      printf '%s\n' "$url"
      return 0
    fi
    local out="${hub_pre}${url}"
    printf '%s\n' "[fundeploy download] URL rewrite (hub proxy): ${url} -> ${out}" >&2
    printf '%s\n' "$out"
    return 0
  fi

  if [[ "$mode" == "mirror_raw" ]] && [[ -n "$raw_base" ]]; then
    if [[ "$host" != "raw.githubusercontent.com" ]]; then
      printf '%s\n' "$url"
      return 0
    fi
    local new_url="${raw_base%/}${path}"
    if [[ "$new_url" != "$url" ]]; then
      printf '%s\n' "[fundeploy download] URL rewrite (mirror_raw): ${url} -> ${new_url}" >&2
    fi
    printf '%s\n' "$new_url"
    return 0
  fi

  if [[ "$mode" == "hub_proxy" ]] && [[ -z "$hub_pre" ]]; then
    printf '%s\n' "[fundeploy download] FUNDEPLOY_GITHUB_DOWNLOAD_MODE=hub_proxy 但未设置 FUNDEPLOY_GITHUB_HUB_PROXY_PREFIX，跳过改写。" >&2
  fi

  printf '%s\n' "$url"
  return 0
}

# 与 fundeploy tool download curl 子命令相同：扫描参数中的 HTTPS URL 并改写后调用 curl。
_fundeploy_github_download_curl() {
  command -v curl >/dev/null 2>&1 || {
    echo "错误: 需要 curl。" >&2
    return 127
  }
  local args=() a new
  for a in "$@"; do
    if [[ "$a" == https://* ]]; then
      new="$(_fundeploy_github_download_resolve_url "$a")"
      args+=("$new")
    else
      args+=("$a")
    fi
  done
  # TLS 下限统一在这里设置，一处覆盖绝大多数调用点：
  #   --proto '=https'        只允许 https 发起
  #   --proto-redir '=https'  -L 跟随重定向时同样只允许 https（否则可被 302 到 http://）
  #   --tlsv1.2               拒绝已废弃的 TLS 版本
  curl --proto '=https' --proto-redir '=https' --tlsv1.2 "${args[@]}"
}

# 未启用任何镜像/前缀策略时，向 stderr 打一行说明（FUNDEPLOY_GITHUB_DOWNLOAD_HINT=0 可关）。
_fundeploy_github_download_print_accel_hint() {
  [[ "${FUNDEPLOY_GITHUB_DOWNLOAD_HINT:-1}" == "0" ]] && return 0
  if [[ -n "${FUNDEPLOY_GITHUB_HUB_PROXY_PREFIX:-}" ]]; then return 0; fi
  if [[ "${FUNDEPLOY_GITHUB_DOWNLOAD_MODE:-off}" != "off" ]]; then return 0; fi
  printf '%s\n' "提示: 当前为 GitHub 直连下载。受限网络可设置 FUNDEPLOY_GITHUB_HUB_PROXY_PREFIX，或 FUNDEPLOY_GITHUB_DOWNLOAD_MODE=mirror_raw 与 FUNDEPLOY_GITHUB_RAW_MIRROR_BASE（见 scripts/tools/download/README.md）。FUNDEPLOY_GITHUB_DOWNLOAD_HINT=0 可隐藏本行。" >&2
}
