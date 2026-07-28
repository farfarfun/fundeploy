#!/usr/bin/env bash
# nlt-ui：可复用的终端交互主题（gum 封装）。由各入口/服务脚本 source。
#
# 目标：让 nltdeploy / nlt-tools / nlt-dev / 各服务的 gum 菜单看起来是「同一套系统」——
#   统一配色、统一 banner、统一光标与选中样式、统一确认/输入风格。
#
# 设计约定:
#   - 纯装饰层，不改变任何业务分发逻辑；无 gum 时静默降级为朴素文本。
#   - 遵守 NO_COLOR（https://no-color.org/）与 NLT_UI_PLAIN=1：禁用一切颜色/边框。
#   - 主色沿用仓库既有服务脚本的品牌色 212（洋红），配套辅助色见下。
#   - 所有函数以 nlt_ui_ 前缀；内部实现以 _nlt_ui_ 前缀。
[[ -n "${_NLT_UI_LOADED:-}" ]] && return 0
_NLT_UI_LOADED=1

# ---- 主题色板（256 色）。可被环境变量覆盖以便定制。----
NLT_UI_C_PRIMARY="${NLT_UI_C_PRIMARY:-212}"   # 品牌主色（洋红）：banner 边框 / 标题
NLT_UI_C_ACCENT="${NLT_UI_C_ACCENT:-45}"      # 强调色（青）：光标 / 选中项
NLT_UI_C_MUTED="${NLT_UI_C_MUTED:-244}"       # 次要信息（灰）：副标题 / 描述
NLT_UI_C_OK="${NLT_UI_C_OK:-42}"              # 成功（绿）
NLT_UI_C_WARN="${NLT_UI_C_WARN:-214}"         # 警告（橙）
NLT_UI_C_ERR="${NLT_UI_C_ERR:-203}"           # 错误（红）

# 是否启用彩色/装饰：NO_COLOR 非空 或 NLT_UI_PLAIN=1 或 非 TTY(stderr) → 关闭。
_nlt_ui_styled() {
  [[ -n "${NO_COLOR:-}" ]] && return 1
  [[ "${NLT_UI_PLAIN:-0}" == "1" ]] && return 1
  return 0
}

nlt_ui_has_gum() { command -v gum >/dev/null 2>&1; }

# 一次性导出 gum 组件的主题环境变量，使 choose/confirm/input/filter 风格统一。
# 幂等：重复调用只是重设同样的值。
nlt_ui_apply_theme() {
  _nlt_ui_styled || return 0
  export GUM_CHOOSE_HEADER_FOREGROUND="${NLT_UI_C_PRIMARY}"
  export GUM_CHOOSE_CURSOR_FOREGROUND="${NLT_UI_C_ACCENT}"
  export GUM_CHOOSE_SELECTED_FOREGROUND="${NLT_UI_C_ACCENT}"
  export GUM_CHOOSE_CURSOR="${NLT_UI_CURSOR:-› }"
  export GUM_FILTER_HEADER_FOREGROUND="${NLT_UI_C_PRIMARY}"
  export GUM_FILTER_INDICATOR_FOREGROUND="${NLT_UI_C_ACCENT}"
  export GUM_FILTER_MATCH_FOREGROUND="${NLT_UI_C_ACCENT}"
  export GUM_CONFIRM_SELECTED_BACKGROUND="${NLT_UI_C_PRIMARY}"
  export GUM_CONFIRM_PROMPT_FOREGROUND="${NLT_UI_C_PRIMARY}"
  export GUM_INPUT_CURSOR_FOREGROUND="${NLT_UI_C_ACCENT}"
  export GUM_INPUT_PROMPT_FOREGROUND="${NLT_UI_C_PRIMARY}"
}

# nlt_ui_banner "标题" ["副标题"...]
# 有 gum 且启用样式：圆角边框盒 + 主色标题 + 灰色副标题行。
# 否则：朴素两行文本（带一条 ASCII 分隔线），保证在任何终端/管道下都可读。
nlt_ui_banner() {
  local title="${1:-nltdeploy}"; shift || true
  if nlt_ui_has_gum && _nlt_ui_styled; then
    local body
    body="$(gum style --bold --foreground "${NLT_UI_C_PRIMARY}" "${title}")"
    local line
    for line in "$@"; do
      body+=$'\n'"$(gum style --foreground "${NLT_UI_C_MUTED}" "${line}")"
    done
    gum style \
      --border rounded --border-foreground "${NLT_UI_C_PRIMARY}" \
      --padding "0 2" --margin "0 0" \
      "${body}"
  else
    printf '%s\n' "== ${title} =="
    local line
    for line in "$@"; do printf '%s\n' "   ${line}"; done
  fi
}

# nlt_ui_choose "<header>" 选项...   —— 可搜索的命令面板，回显所选项到 stdout。
# 无 gum：回退为 select（编号菜单），同样把所选项打印到 stdout；取消/EOF 返回非 0。
nlt_ui_choose() {
  local header="$1"; shift
  if nlt_ui_has_gum; then
    local height=$(( $# + 1 ))
    (( height < 4 )) && height=4
    (( height > 14 )) && height=14
    nlt_ui_apply_theme
    printf '%s\n' "$@" | gum filter \
      --header "${header}" \
      --placeholder "输入关键词筛选..." \
      --prompt "› " \
      --height "${height}" \
      --no-show-help \
      --limit 1 \
      --select-if-one
    return $?
  fi
  # 朴素回退：编号选择。
  local opt reply i=0
  printf '%s\n' "${header}" >&2
  for opt in "$@"; do
    i=$((i + 1)); printf '  %d) %s\n' "$i" "${opt}" >&2
  done
  printf '选择编号（回车取消）: ' >&2
  read -r reply || return 1
  [[ -z "${reply}" ]] && return 1
  [[ "${reply}" =~ ^[0-9]+$ ]] || return 1
  [[ "${reply}" -ge 1 && "${reply}" -le $# ]] || return 1
  printf '%s\n' "${!reply}"
}

# 主题化确认：有 gum 用 gum confirm；否则 read y/N。默认否（安全）。
nlt_ui_confirm() {
  local prompt="${1:-确认？}"
  if nlt_ui_has_gum; then
    nlt_ui_apply_theme
    gum confirm "${prompt}"
    return $?
  fi
  local a; read -r -p "${prompt} [y/N] " a || return 1
  [[ "${a,,}" == "y" || "${a,,}" == "yes" ]]
}

# 语义化单行提示（带颜色，输出到 stderr，便于与 stdout 数据分离）。
nlt_ui_info()  { if _nlt_ui_styled; then printf '\033[38;5;%sm%s\033[0m\n' "${NLT_UI_C_ACCENT}" "$*" >&2; else printf '%s\n' "$*" >&2; fi; }
nlt_ui_ok()    { if _nlt_ui_styled; then printf '\033[38;5;%sm✓ %s\033[0m\n' "${NLT_UI_C_OK}" "$*" >&2; else printf 'OK: %s\n' "$*" >&2; fi; }
nlt_ui_warn()  { if _nlt_ui_styled; then printf '\033[38;5;%sm! %s\033[0m\n' "${NLT_UI_C_WARN}" "$*" >&2; else printf '警告: %s\n' "$*" >&2; fi; }
nlt_ui_err()   { if _nlt_ui_styled; then printf '\033[38;5;%sm✗ %s\033[0m\n' "${NLT_UI_C_ERR}" "$*" >&2; else printf '错误: %s\n' "$*" >&2; fi; }
